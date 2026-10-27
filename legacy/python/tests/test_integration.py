"""Prove end-to-end fra due motori reali su HTTPS (loopback). Non provano la compatibilità
con i client LocalSend ufficiali: per quella vedi docs/CHECKLIST-MANUALE.md."""
import hashlib
import http.client
import json
import os
import resource
import socket
import ssl
import threading
import time
import unittest

from pixelhub.client import IdentityMismatch, json_request
from tests.helpers import TmpCase, free_port, sha, wait_for


class E2E(TmpCase):
    def setUp(self):
        super().setUp()
        self.a = self.engine("sorgente")
        self.b = self.engine("destinazione")
        self.dev = self.pair(self.a, self.b)

    def send(self, paths=None, text=None):
        jid, err = self.a.invia(self.b.identity.fingerprint, paths, text)
        self.assertEqual(err, "")
        return jid

    def make(self, name, data):
        p = self.tmp / "src" / name
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data)
        return p

    def test_discovery_register_both_ways(self):
        self.assertTrue(self.dev.verified)                        # certificato visto e coerente
        peer = wait_for(lambda: self.b.registry.get(self.a.identity.fingerprint))
        self.assertEqual(peer.alias, "sorgente")
        self.assertFalse(peer.verified)                            # b non ha mai visto il certificato di a

    def test_send_file_end_to_end(self):
        data = os.urandom(3 * 1024 * 1024 + 17)
        src = self.make("dati.bin", data)
        jid = self.send([src])
        self.accept_when_pending(self.b)
        j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "completato", j)
        self.assertEqual(sha(self.b.settings["cartella"] + "/dati.bin"), hashlib.sha256(data).hexdigest())
        rx = self.b.receiver.snapshot()["ricezioni"][0]
        self.assertEqual(rx["stato"], "completata")
        self.assertEqual(rx["ricevuti"], 1)

    def test_send_text(self):
        jid = self.send(text="ciao dal PC")
        self.accept_when_pending(self.b)
        self.assertEqual(self.wait_job(self.a, jid)["stato"], "completato")
        self.assertEqual(self.b.receiver.snapshot()["messaggi"][0]["testo"], "ciao dal PC")

    def test_text_rejected(self):
        jid = self.send(text="no grazie")
        self.accept_when_pending(self.b, ids=[])
        self.assertEqual(self.wait_job(self.a, jid)["stato"], "rifiutato")
        self.assertEqual(self.b.receiver.snapshot()["messaggi"], [])

    def test_reject_files(self):
        jid = self.send([self.make("x.txt", b"x")])
        self.accept_when_pending(self.b, ids=[])
        j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "rifiutato")
        self.assertEqual(os.listdir(self.b.settings["cartella"]), [])

    def test_partial_selection_makes_job_partial(self):
        paths = [self.make(n, n.encode() * 100) for n in ("a.txt", "b.txt", "c.txt")]
        jid = self.send(paths)

        def pend():
            return next((r for r in self.b.receiver.snapshot()["ricezioni"] if r["stato"] == "attesa"), None)
        r = wait_for(pend)
        pick = [f["id"] for f in r["file"] if f["nome"] in ("a.txt", "c.txt")]
        self.b.decidi(r["id"], pick)
        j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "parziale", j)
        self.assertEqual(sorted(os.listdir(self.b.settings["cartella"])), ["a.txt", "c.txt"])

    def test_folder_send_keeps_structure(self):
        self.make("album/uno.txt", b"1")
        self.make("album/sotto/due.txt", b"2")
        jid = self.send([self.tmp / "src" / "album"])
        self.accept_when_pending(self.b)
        self.assertEqual(self.wait_job(self.a, jid)["stato"], "completato")
        root = self.b.settings["cartella"]
        self.assertEqual(open(root + "/album/sotto/due.txt").read(), "2")

    def test_pin_flow_wrong_then_right(self):
        self.b.imposta_pin("424242")
        jid = self.send([self.make("p.txt", b"pin")])
        j = wait_for(lambda: (j := self.job(self.a, jid)) and j["stato"] == "pin" and j)
        self.assertFalse(j["pin_errato"])
        self.assertTrue(self.a.sender.fornisci_pin(jid, "000000"))
        j = wait_for(lambda: (j := self.job(self.a, jid)) and j["stato"] == "pin" and j["pin_errato"] and j)
        self.assertTrue(self.a.sender.fornisci_pin(jid, "424242"))
        self.accept_when_pending(self.b)
        self.assertEqual(self.wait_job(self.a, jid)["stato"], "completato")
        self.assertNotIn("424242", json.dumps(self.a.snapshot()))
        self.assertNotIn("424242", (self.a.paths.config / "settings.json").read_text())

    def test_pin_prompt_can_be_cancelled(self):
        self.b.imposta_pin("424242")
        jid = self.send([self.make("p.txt", b"pin")])
        wait_for(lambda: (j := self.job(self.a, jid)) and j["stato"] == "pin")
        self.a.sender.annulla(jid)
        self.assertEqual(self.wait_job(self.a, jid)["stato"], "annullato")

    def test_cancel_during_large_transfer_stops_io_and_cleans_up(self):
        src = self.tmp / "src" / "grande.bin"
        src.parent.mkdir(parents=True, exist_ok=True)
        with open(src, "wb") as f:
            f.truncate(600 * 1024 * 1024)               # sparso: veloce da creare
        jid = self.send([src])
        self.accept_when_pending(self.b)
        wait_for(lambda: (j := self.job(self.a, jid)) and j["inviati"] > 1024 * 1024)
        t0 = time.monotonic()
        self.a.sender.annulla(jid)
        j = self.wait_job(self.a, jid, timeout=10)
        self.assertEqual(j["stato"], "annullato")
        self.assertLess(time.monotonic() - t0, 5)
        rx = wait_for(lambda: (r := self.b.receiver.snapshot()["ricezioni"][0]) and r["stato"] in ("annullata", "parziale") and r)
        root = self.b.settings["cartella"]
        wait_for(lambda: os.listdir(root) == [])         # nessun parziale né file finale

    def test_receiver_cancel_stops_sender(self):
        src = self.tmp / "src" / "grande.bin"
        src.parent.mkdir(parents=True, exist_ok=True)
        with open(src, "wb") as f:
            f.truncate(600 * 1024 * 1024)
        jid = self.send([src])
        r = self.accept_when_pending(self.b)
        wait_for(lambda: self.b.receiver.snapshot()["ricezioni"][0]["byte"] > 1024 * 1024)
        self.b.receiver.annulla(r["id"])
        j = self.wait_job(self.a, jid)
        self.assertIn(j["stato"], ("errore", "annullato"))
        self.assertNotEqual(j["stato"], "completato")
        wait_for(lambda: os.listdir(self.b.settings["cartella"]) == [])

    def test_source_modified_after_selection(self):
        from unittest import mock
        from pixelhub.sender import Sender
        src = self.make("m.txt", b"originale")
        orig = Sender._verify_source

        def modify_then_verify(it):
            src.write_bytes(b"cambiato dopo la selezione")
            return orig(it)
        with mock.patch.object(Sender, "_verify_source", staticmethod(modify_then_verify)):
            jid = self.send([src])
            j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "errore")
        self.assertIn("modificato", j["errore"])
        self.assertEqual(os.listdir(self.b.settings["cartella"]), [])

    def test_source_removed_before_hashing(self):
        from unittest import mock
        from pixelhub.sender import Sender
        src = self.make("sparisce.txt", b"x")
        orig = Sender._sha256

        def remove_then_hash(self_, it, job):
            src.unlink()
            return orig(self_, it, job)
        with mock.patch.object(Sender, "_sha256", remove_then_hash):
            jid = self.send([src])
            j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "errore")
        self.assertIn("rimosso", j["errore"])

    def test_source_changes_during_upload(self):
        src = self.tmp / "src" / "vivo.bin"
        src.parent.mkdir(parents=True, exist_ok=True)
        with open(src, "wb") as f:
            f.truncate(400 * 1024 * 1024)
        jid = self.send([src])
        self.accept_when_pending(self.b)
        wait_for(lambda: (j := self.job(self.a, jid)) and j["inviati"] > 1024 * 1024)
        with open(src, "r+b") as f:
            f.write(b"modificato")
        j = self.wait_job(self.a, jid, timeout=60)
        self.assertEqual(j["stato"], "errore", j)
        self.assertIn("cambiato", j["file"][0]["errore"])
        # il destinatario deve aver scartato il parziale e NON aver salvato nulla
        wait_for(lambda: self.b.receiver.snapshot()["ricezioni"][0]["file"][0]["stato"] == "attesa")
        self.assertEqual(os.listdir(self.b.settings["cartella"]), [])
        self.assertLess(self.job(self.a, jid)["inviati"], 400 * 1024 * 1024)       # interrotto prima della fine

    def test_identity_mismatch_refused(self):
        dev = self.a.registry.get(self.b.identity.fingerprint)
        other = self.engine("altro")
        # b "diventa" un altro certificato: l'annuncio dichiarava l'impronta vecchia
        dev.fp = "F" * 64
        self.a.registry.devices["F" * 64] = dev
        jid, err = self.a.invia("F" * 64, testo="x")
        j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "errore")
        self.assertIn("identità", j["errore"].lower())
        self.assertEqual(os.listdir(self.b.settings["cartella"]), [])

    def test_unreachable_device_error_is_clear(self):
        self.b.ferma()
        jid = self.send(text="nessuno ascolta")
        j = self.wait_job(self.a, jid)
        self.assertEqual(j["stato"], "errore")
        self.assertTrue(j["errore"])

    def test_two_senders_second_gets_409_not_mixing(self):
        c = self.engine("terzo")
        self.pair(c, self.b)
        j1 = self.send([self.make("uno.txt", b"1")])
        r = wait_for(lambda: next((r for r in self.b.receiver.snapshot()["ricezioni"] if r["stato"] == "attesa"), None))
        self.b.decidi(r["id"])
        self.wait_job(self.a, j1)
        jid, _ = c.invia(self.b.identity.fingerprint, testo="altro")
        self.accept_when_pending(self.b)
        self.assertEqual(self.wait_job(c, jid)["stato"], "completato")

    def test_no_http_fallback_and_hostile_clients(self):
        # HTTP in chiaro sulla porta HTTPS non deve dare nulla di utile
        c = http.client.HTTPConnection("127.0.0.1", self.b.port, timeout=3)
        with self.assertRaises(Exception):
            c.request("GET", "/api/localsend/v2/info")
            c.getresponse().read()
        # garbage, intestazioni troncate, corpo enorme dichiarato
        ctx = ssl.create_default_context()
        ctx.check_hostname, ctx.verify_mode = False, ssl.CERT_NONE
        for payload in (b"\x00\xff" * 50, b"POST /api/localsend/v2/prepare-upload HTTP/1.1\r\nContent-Length: 99999999999\r\n\r\n",
                        b"POST /api/localsend/v2/upload?sessionId=a&fileId=b&token=c HTTP/1.1\r\nHost: x\r\n\r\n"):
            with socket.create_connection(("127.0.0.1", self.b.port), timeout=3) as s:
                with ctx.wrap_socket(s) as t:
                    t.sendall(payload)
                    try:
                        t.recv(1024)
                    except OSError:
                        pass
        # il server resta in piedi
        st, resp, fp = json_request("127.0.0.1", self.b.port, "GET", "/api/localsend/v2/info", self.a.identity)
        self.assertEqual(st, 200)
        self.assertEqual(fp, self.b.identity.fingerprint)

    def test_unsolicited_prepare_from_stranger_needs_manual_accept(self):
        """Nessuna auto-accettazione: neppure con fingerprint dichiarata uguale a un preferito."""
        self.b.preferito(self.a.identity.fingerprint, True)
        body = {"info": {"alias": "furbo", "fingerprint": self.a.identity.fingerprint},
                "files": {"i": {"id": "i", "fileName": "evil.txt", "size": 1, "fileType": "text/plain"}}}
        res = {}
        c = self.engine("estraneo")
        t = threading.Thread(target=lambda: res.update(r=json_request("127.0.0.1", self.b.port, "POST",
                             "/api/localsend/v2/prepare-upload", c.identity, body, timeout=20)))
        t.start()
        r = wait_for(lambda: next((r for r in self.b.receiver.snapshot()["ricezioni"] if r["stato"] == "attesa"), None))
        self.assertEqual(os.listdir(self.b.settings["cartella"]), [])
        self.b.decidi(r["id"], [])
        t.join(10)
        self.assertEqual(res["r"][0], 403)

    def test_memory_stays_bounded_for_big_transfer(self):
        src = self.tmp / "src" / "enorme.bin"
        src.parent.mkdir(parents=True, exist_ok=True)
        with open(src, "wb") as f:
            for _ in range(64):
                f.write(os.urandom(1024 * 1024))
            f.truncate(300 * 1024 * 1024)
        before = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        jid = self.send([src])
        self.accept_when_pending(self.b)
        self.assertEqual(self.wait_job(self.a, jid, timeout=120)["stato"], "completato")
        grown_mb = (resource.getrusage(resource.RUSAGE_SELF).ru_maxrss - before) / 1024
        self.assertLess(grown_mb, 120, "la memoria è cresciuta di %.0f MB" % grown_mb)
        self.assertEqual(os.path.getsize(self.b.settings["cartella"] + "/enorme.bin"), 300 * 1024 * 1024)

    def test_duplicate_names_not_overwritten(self):
        for n in range(2):
            jid = self.send([self.make("stesso.txt", b"v%d" % n)])
            self.accept_when_pending(self.b)
            self.assertEqual(self.wait_job(self.a, jid)["stato"], "completato")
            time.sleep(0.05)
        root = self.b.settings["cartella"]
        self.assertEqual(sorted(os.listdir(root)), ["stesso (1).txt", "stesso.txt"])


if __name__ == "__main__":
    unittest.main()
