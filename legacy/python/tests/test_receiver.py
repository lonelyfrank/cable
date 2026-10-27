import hashlib
import threading
import time
import unittest

from pixelhub.bus import Bus
from pixelhub.paths import Paths
from pixelhub.protocol import Limits, ProtocolError
from pixelhub.receiver import Receiver, StreamError
from pixelhub.settings import Settings
from tests.helpers import TmpCase, wait_for

IP = "10.0.0.5"


class Fake:
    """Corpo di upload simulato; può interrompersi a metà come una rete che cade."""

    def __init__(self, data, length="auto", fail_after=None, slow=None):
        self.data, self.fail_after, self.slow = data, fail_after, slow
        self.length = len(data) if length == "auto" else length

    def read(self, write):
        sent = 0
        for i in range(0, len(self.data), 4):
            if self.fail_after is not None and sent >= self.fail_after:
                raise StreamError("rete caduta")
            if self.slow:
                time.sleep(self.slow)
            c = self.data[i:i + 4]
            write(c)
            sent += len(c)


def body(files, alias="Mittente"):
    return {"info": {"alias": alias, "fingerprint": "C" * 64},
            "files": {k: {"id": k, "fileName": n, "size": len(d), "fileType": "application/octet-stream",
                          **({"sha256": hashlib.sha256(d).hexdigest()} if sha else {})}
                      for k, (n, d, sha) in files.items()}}


class ReceiverCase(TmpCase):
    def setUp(self):
        super().setUp()
        self.settings = Settings(Paths.sotto(self.tmp))
        self.settings.aggiorna({"cartella": str(self.tmp / "in")})
        self.limits = Limits(decision_timeout=5, session_idle=0.3)
        self.rx = Receiver(self.settings, Bus(), self.limits)

    def start(self, files, ip=IP, pin=None, accept=None, **kw):
        """Avvia prepare in un thread, decide e restituisce (stato, risposta)."""
        out = {}

        def run():
            try:
                out["r"] = self.rx.prepare(ip, pin, body(files, **kw))
            except ProtocolError as e:
                out["e"] = e
            except Exception as e:  # noqa: BLE001
                out["x"] = e
        t = threading.Thread(target=run)
        t.start()
        rid = wait_for(lambda: next((r["id"] for r in self.rx.snapshot()["ricezioni"] if r["stato"] == "attesa"), None)
                       if not out else True)
        if rid is not True:
            self.rx.decide(rid, accept)
        t.join(10)
        return out

    def session(self, files, **kw):
        out = self.start(files, **kw)
        self.assertIn("r", out, out)
        status, resp = out["r"]
        self.assertEqual(status, 200)
        return resp["sessionId"], resp["files"]

    def q(self, sid, fid, tok):
        return {"sessionId": sid, "fileId": fid, "token": tok}

    def state(self):
        return self.rx.snapshot()["ricezioni"][0]


class Upload(ReceiverCase):
    def test_happy_path_atomic_and_complete(self):
        data = b"ciao mondo!"
        sid, tok = self.session({"f": ("a.txt", data, True)})
        self.rx.upload(IP, self.q(sid, "f", tok["f"]), Fake(data))
        self.assertEqual((self.tmp / "in" / "a.txt").read_bytes(), data)
        self.assertEqual(self.state()["stato"], "completata")
        self.assertEqual(list((self.tmp / "in").glob(".pixelhub-*")), [])

    def test_zero_byte_and_unicode_and_subdir(self):
        sid, tok = self.session({"a": ("cartella/è già.txt", b"", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b""))
        self.assertEqual((self.tmp / "in" / "cartella" / "è già.txt").read_bytes(), b"")

    def test_traversal_name_stays_inside(self):
        sid, tok = self.session({"a": ("../../evil.txt", b"x", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"x"))
        self.assertTrue((self.tmp / "in" / "evil.txt").exists())
        self.assertFalse((self.tmp / "evil.txt").exists())

    def test_existing_file_not_overwritten(self):
        (self.tmp / "in").mkdir()
        (self.tmp / "in" / "a.txt").write_text("originale")
        sid, tok = self.session({"a": ("a.txt", b"nuovo", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"nuovo"))
        self.assertEqual((self.tmp / "in" / "a.txt").read_text(), "originale")
        self.assertEqual((self.tmp / "in" / "a (1).txt").read_text(), "nuovo")

    def test_symlink_in_destination_refused(self):
        (self.tmp / "in").mkdir()
        (self.tmp / "out").mkdir()
        (self.tmp / "in" / "d").symlink_to(self.tmp / "out")
        sid, tok = self.session({"a": ("d/x.txt", b"x", False)})
        with self.assertRaises(ProtocolError) as c:
            self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"x"))
        self.assertEqual(c.exception.status, 403)
        self.assertEqual(list((self.tmp / "out").iterdir()), [])

    def test_wrong_token_ip_session(self):
        sid, tok = self.session({"a": ("a", b"x", False)})
        for args, st in [((IP, self.q(sid, "a", "sbagliato")), 403), (("10.9.9.9", self.q(sid, "a", tok["a"])), 403),
                         ((IP, self.q("nope", "a", tok["a"])), 403), ((IP, self.q(sid, "zzz", tok["a"])), 403),
                         ((IP, {"sessionId": sid}), 400)]:
            with self.assertRaises(ProtocolError) as c:
                self.rx.upload(*args, Fake(b"x"))
            self.assertEqual(c.exception.status, st, args)

    def test_duplicate_after_completion_is_409(self):
        sid, tok = self.session({"a": ("a", b"x", False), "b": ("b", b"y", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"x"))
        with self.assertRaises(ProtocolError) as c:
            self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"x"))
        self.assertEqual(c.exception.status, 409)
        self.assertEqual(self.state()["stato"], "ricezione")      # b manca: non è completata

    def test_concurrent_same_file_only_one_wins(self):
        data = b"0123456789" * 10
        sid, tok = self.session({"a": ("a", data, False)})
        res = []

        def go(stream):
            try:
                self.rx.upload(IP, self.q(sid, "a", tok["a"]), stream)
                res.append(200)
            except ProtocolError as e:
                res.append(e.status)
        t1 = threading.Thread(target=go, args=(Fake(data, slow=0.01),))
        t1.start()
        wait_for(lambda: self.state()["file"][0]["stato"] == "ricezione")
        go(Fake(data))
        t1.join(10)
        self.assertEqual(sorted(res), [200, 409])
        self.assertEqual((self.tmp / "in" / "a").read_bytes(), data)
        self.assertEqual(len(list((self.tmp / "in").iterdir())), 1)

    def test_size_mismatches_leave_nothing(self):
        for length, data, declared in [(None, b"troppo lungo", 3), ("auto", b"ab", 5)]:
            sid, tok = self.session({"a": ("a%d" % declared, b"x" * declared, False)}, ip=IP)
            with self.assertRaises(ProtocolError) as c:
                self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(data, length=length))
            self.assertEqual(c.exception.status, 400)
            self.rx.cancel_from_sender(IP, sid)
        self.assertEqual([p.name for p in (self.tmp / "in").iterdir()], [])

    def test_content_length_must_match_declared(self):
        sid, tok = self.session({"a": ("a", b"xxx", False)})
        with self.assertRaises(ProtocolError) as c:
            self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"xxx", length=99))
        self.assertEqual(c.exception.status, 400)

    def test_sha256_mismatch_422_and_retry_allowed(self):
        sid, tok = self.session({"a": ("a", b"giusto", True)})
        with self.assertRaises(ProtocolError) as c:
            self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"sbagli"))
        self.assertEqual(c.exception.status, 422)
        self.assertEqual(list((self.tmp / "in").iterdir()), [])
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"giusto"))       # ritentativo valido
        self.assertEqual(self.state()["stato"], "completata")

    def test_interrupted_upload_discards_partial(self):
        sid, tok = self.session({"a": ("a", b"x" * 100, False)})
        with self.assertRaises(StreamError):
            self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"x" * 100, fail_after=40))
        self.assertEqual(list((self.tmp / "in").iterdir()), [])
        self.assertEqual(self.state()["file"][0]["stato"], "attesa")

    def test_cancel_by_sender_stops_inflight_upload(self):
        data = b"x" * 400
        sid, tok = self.session({"a": ("a", data, False)})
        err = []

        def go():
            try:
                self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(data, slow=0.01))
            except ProtocolError as e:
                err.append(e.status)
        t = threading.Thread(target=go)
        t.start()
        wait_for(lambda: self.state()["byte"] > 0)
        self.rx.cancel_from_sender(IP, sid)
        t.join(5)
        self.assertEqual(err, [409])
        self.assertEqual(list((self.tmp / "in").iterdir()), [])
        self.assertEqual(self.state()["stato"], "annullata")

    def test_cancel_wrong_ip_refused(self):
        sid, _ = self.session({"a": ("a", b"x", False)})
        with self.assertRaises(ProtocolError):
            self.rx.cancel_from_sender("10.9.9.9", sid)

    def test_local_cancel_after_some_files_is_partial_not_complete(self):
        sid, tok = self.session({"a": ("a", b"1", False), "b": ("b", b"2", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"1"))
        self.rx.annulla(self.state()["id"])
        st = self.state()
        self.assertEqual(st["stato"], "parziale")
        self.assertEqual(st["ricevuti"], 1)
        self.assertTrue((self.tmp / "in" / "a").exists())
        self.assertFalse((self.tmp / "in" / "b").exists())

    def test_idle_session_expires_to_partial(self):
        sid, tok = self.session({"a": ("a", b"1", False), "b": ("b", b"2", False)})
        self.rx.upload(IP, self.q(sid, "a", tok["a"]), Fake(b"1"))
        time.sleep(0.4)
        self.rx.sweep()
        self.assertEqual(self.state()["stato"], "parziale")
        with self.assertRaises(ProtocolError):
            self.rx.upload(IP, self.q(sid, "b", tok["b"]), Fake(b"2"))      # sessione chiusa

    def test_expired_without_any_file(self):
        sid, tok = self.session({"a": ("a", b"1", False)})
        time.sleep(0.4)
        self.rx.sweep()
        self.assertEqual(self.state()["stato"], "scaduta")


class Decisions(ReceiverCase):
    def test_partial_accept_only_selected_get_tokens(self):
        out = self.start({"a": ("a", b"1", False), "b": ("b", b"2", False), "c": ("c", b"3", False)}, accept=["a", "c"])
        status, resp = out["r"]
        self.assertEqual(set(resp["files"]), {"a", "c"})
        sid = resp["sessionId"]
        with self.assertRaises(ProtocolError) as c:
            self.rx.upload(IP, self.q(sid, "b", "x" * 32), Fake(b"2"))
        self.assertEqual(c.exception.status, 403)
        for k, d in (("a", b"1"), ("c", b"3")):
            self.rx.upload(IP, self.q(sid, k, resp["files"][k]), Fake(d))
        self.assertEqual(self.state()["stato"], "completata")
        self.assertEqual(sorted(p.name for p in (self.tmp / "in").iterdir()), ["a", "c"])

    def test_reject(self):
        out = self.start({"a": ("a", b"1", False)}, accept=[])
        self.assertEqual(out["e"].status, 403)
        self.assertEqual(self.state()["stato"], "rifiutata")

    def test_timeout_is_expired(self):
        self.rx.limits = Limits(decision_timeout=0.3)
        with self.assertRaises(ProtocolError) as c:
            self.rx.prepare(IP, None, body({"a": ("a", b"1", False)}))
        self.assertEqual(c.exception.status, 403)
        self.assertEqual(self.state()["stato"], "scaduta")

    def test_peer_gone_while_waiting(self):
        from pixelhub.receiver import PeerGone
        with self.assertRaises(PeerGone):
            self.rx.prepare(IP, None, body({"a": ("a", b"1", False)}), is_closed=lambda: True)
        self.assertEqual(self.state()["stato"], "annullata")

    def test_busy_session_blocks_others_409(self):
        self.session({"a": ("a", b"1", False)})
        with self.assertRaises(ProtocolError) as c:
            self.rx.prepare("10.0.0.9", None, body({"z": ("z", b"1", False)}))
        self.assertEqual(c.exception.status, 409)

    def test_too_many_pending_429(self):
        self.rx.limits = Limits(max_pending=2, decision_timeout=3)
        ths = []
        for i in range(2):
            t = threading.Thread(target=lambda ip=f"10.0.1.{i}": self._try(ip))
            t.start()
            ths.append(t)
        wait_for(lambda: sum(r["stato"] == "attesa" for r in self.rx.snapshot()["ricezioni"]) == 2)
        with self.assertRaises(ProtocolError) as c:
            self.rx.prepare("10.0.1.99", None, body({"z": ("z", b"1", False)}))
        self.assertEqual(c.exception.status, 429)
        for r in self.rx.snapshot()["ricezioni"]:
            self.rx.decide(r["id"], [])
        for t in ths:
            t.join(5)

    def _try(self, ip):
        try:
            self.rx.prepare(ip, None, body({"a": ("a", b"1", False)}))
        except ProtocolError:
            pass

    def test_invalid_body_400(self):
        for b in (None, {"info": {}, "files": {}}, {"files": {"a": {}}, "info": {}}):
            with self.assertRaises(ProtocolError) as c:
                self.rx.prepare(IP, None, b)
            self.assertEqual(c.exception.status, 400)

    def test_text_message_needs_accept_and_returns_204(self):
        b = {"info": {"alias": "Tel", "fingerprint": "C" * 64},
             "files": {"t": {"id": "t", "fileName": "t.txt", "size": 4, "fileType": "text/plain", "preview": "ciao"}}}
        out = {}
        t = threading.Thread(target=lambda: out.update(r=self.rx.prepare(IP, None, b)))
        t.start()
        rid = wait_for(lambda: next((r["id"] for r in self.rx.snapshot()["ricezioni"] if r["stato"] == "attesa"), None))
        self.assertEqual(self.rx.snapshot()["messaggi"], [])          # nulla consegnato prima dell'ok
        self.rx.decide(rid)
        t.join(5)
        self.assertEqual(out["r"], (204, None))
        self.assertEqual(self.rx.snapshot()["messaggi"][0]["testo"], "ciao")


class Pin(ReceiverCase):
    def test_pin_required_wrong_and_rate_limit(self):
        self.settings.imposta_pin("123456")
        b = body({"a": ("a", b"1", False)})
        with self.assertRaises(ProtocolError) as c:
            self.rx.prepare(IP, None, b)
        self.assertEqual(c.exception.status, 401)
        for _ in range(self.limits.pin_failures):
            with self.assertRaises(ProtocolError) as c:
                self.rx.prepare(IP, "000000", b)
            self.assertEqual(c.exception.status, 401)
        with self.assertRaises(ProtocolError) as c:
            self.rx.prepare(IP, "123456", b)               # anche il PIN giusto è bloccato dal limite
        self.assertEqual(c.exception.status, 429)

    def test_correct_pin_accepted(self):
        self.settings.imposta_pin("123456")
        out = self.start({"a": ("a", b"1", False)}, pin="123456")
        self.assertEqual(out["r"][0], 200)


class Space(ReceiverCase):
    def test_insufficient_space_507(self):
        import pixelhub.receiver as r
        old = r.DISK_MARGIN
        r.DISK_MARGIN = 2 ** 60
        try:
            out = self.start({"a": ("a", b"1", False)})
        finally:
            r.DISK_MARGIN = old
        self.assertEqual(out["e"].status, 507)
        st = self.state()
        self.assertEqual(st["stato"], "errore")
        self.assertIn("Spazio", st["errore"])


if __name__ == "__main__":
    unittest.main()
