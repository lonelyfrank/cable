import json
import os
import unittest

from pixelhub import desktop
from pixelhub.engine import Engine
from pixelhub.paths import Paths
from pixelhub.ui.api import UiApi
from tests.helpers import TmpCase, free_port

FP = "A" * 64


class FavoritesAndReset(TmpCase):
    def test_favorite_is_not_verified_identity_and_forget(self):
        a, b = self.engine("a"), self.engine("b")
        self.pair(a, b)
        fp = b.identity.fingerprint
        a.preferito(fp, True)
        snap = [d for d in a.snapshot()["dispositivi"] if d["fp"] == fp][0]
        self.assertTrue(snap["preferito"])
        self.assertTrue(snap["verificato"])        # visto con certificato coerente in questa sessione
        # un preferito riletto da disco non è mai "verificato"
        a2 = Engine(a.paths, port=free_port(), multicast=False)
        d = [x for x in a2.snapshot()["dispositivi"] if x["fp"] == fp][0]
        self.assertTrue(d["preferito"])
        self.assertFalse(d["verificato"])
        self.assertFalse(d["vicino"])
        a.dimentica(fp)
        self.assertEqual([d for d in a.snapshot()["dispositivi"] if d["fp"] == fp], [])
        self.assertNotIn(fp, a.settings["preferiti"])

    def test_reset_settings_keeps_files_and_identity(self):
        e = self.engine("a")
        e.aggiorna_impostazioni({"nome_pc": "Altro"})
        e.imposta_pin("123456")
        fp = e.identity.fingerprint
        marker = os.path.join(e.settings["cartella"], "mio.txt")
        open(marker, "w").write("x")
        bk = e.ripristina_impostazioni()
        self.assertTrue(os.path.exists(bk))
        self.assertEqual(e.settings["nome_pc"], "Fedora Pixel Hub")
        self.assertFalse(e.snapshot()["pin_impostato"])
        self.assertTrue(os.path.exists(marker))
        self.assertEqual(e.identity.fingerprint, fp)

    def test_regenerate_identity_is_explicit(self):
        e = self.engine("a")
        old = e.identity.fingerprint
        e.rigenera_identita()
        self.assertNotEqual(e.identity.fingerprint, old)
        self.assertEqual(e.registry.own_fp, e.identity.fingerprint)
        self.assertTrue(e.server)                 # la ricezione riparte con la nuova identità

    def test_corrupt_identity_blocks_server_not_silently_regenerated(self):
        paths = Paths.sotto(self.tmp / "x")
        e0 = Engine(paths, port=free_port(), multicast=False)
        e0.identity.key.write_text("rotta")
        e = Engine(paths, port=free_port(), multicast=False)
        self.engines.append(e)
        self.assertFalse(e.avvia())
        s = e.snapshot()
        self.assertEqual(s["identita"]["stato"], "invalida")
        self.assertEqual(s["server"]["errore"], "identita")
        self.assertEqual(e.identity.key.read_text(), "rotta")
        jid, err = e.invia("B" * 64, testo="x")
        self.assertIsNone(jid)

    def test_port_in_use_reports_cause(self):
        a = self.engine("a")
        b = self.engine("b", start=False)
        b.port = a.port
        self.assertFalse(b.avvia())
        self.assertEqual(b.snapshot()["server"]["errore"], "porta")
        self.assertIn("porta", b.diagnostica().lower())

    def test_recovers_after_port_freed(self):
        a = self.engine("a")
        b = self.engine("b", start=False)
        b.port = a.port
        b.avvia()
        a.ferma()
        import time
        time.sleep(0.3)
        self.assertTrue(b.riprova_server())


class Migration(TmpCase):
    def test_full_v3_upgrade_keeps_data_and_fixes_autostart(self):
        paths = Paths.sotto(self.tmp)
        paths.legacy_config.parent.mkdir(parents=True)
        recv = self.tmp / "ricevuti"
        recv.mkdir()
        (recv / "foto.jpg").write_text("preziosa")
        paths.legacy_config.write_text(json.dumps({"cartella": str(recv), "avvio_login": True,
                                                   "pairing": {"fp": FP, "alias": "Pixel", "ready": True}}))
        auto = desktop.autostart_file(paths)
        auto.parent.mkdir(parents=True)
        auto.write_text("[Desktop Entry]\nExec=python3 /home/u/.local/share/pixel-hub/pixel_hub.py --background\n")
        e = Engine(paths, port=free_port(), multicast=False)
        self.engines.append(e)
        e.avvia()
        self.assertEqual(e.settings["cartella"], str(recv))
        self.assertEqual((recv / "foto.jpg").read_text(), "preziosa")
        self.assertNotIn("pixel_hub.py", auto.read_text())             # non rilancia più la 3.x
        self.assertIn("--background", auto.read_text())
        saved = list(paths.backups.glob("autostart-v3-*"))
        self.assertEqual(len(saved), 1)
        self.assertIn("pixel_hub.py", saved[0].read_text())             # copia per il rollback
        snap = e.snapshot()
        self.assertEqual(snap["migrazione"]["versione"], 3)
        self.assertTrue(any("importate" in a.lower() for a in snap["avvisi"]))
        self.assertTrue(snap["dispositivi"][0]["preferito"])
        self.assertFalse(snap["dispositivi"][0]["verificato"])
        self.assertTrue(list(paths.backups.glob("config-v3-*")))


class UiApiSafety(TmpCase):
    def setUp(self):
        super().setUp()
        self.e = self.engine("a")
        self.api = UiApi(self.e)

    def test_paths_limited_to_configured_folders(self):
        root = self.e.settings["cartella"]
        ok = os.path.join(root, "f.txt")
        open(ok, "w").write("x")
        self.assertIsNotNone(self.api.consentito(ok))
        self.assertIsNone(self.api.consentito("/etc/passwd"))
        self.assertIsNone(self.api.consentito(os.path.join(root, "..", "..", "x")))
        os.symlink("/etc/passwd", os.path.join(root, "link"))
        self.assertIsNone(self.api.consentito(os.path.join(root, "link")))
        self.assertFalse(self.api.chiama("apri", {"path": "/etc/passwd"})["ok"])
        self.assertFalse(self.api.chiama("mostra", {"path": "/etc"})["ok"])

    def test_preview_only_for_images_inside_folders(self):
        root = self.e.settings["cartella"]
        open(os.path.join(root, "a.png"), "wb").write(b"\x89PNG")
        open(os.path.join(root, "a.txt"), "w").write("t")
        self.assertEqual(self.api.anteprima(os.path.join(root, "a.png"))[1], "image/png")
        self.assertIsNone(self.api.anteprima(os.path.join(root, "a.txt")))
        self.assertIsNone(self.api.anteprima("/etc/hostname"))

    def test_dispatch_rejects_unknown_and_bad_input(self):
        self.assertFalse(self.api.chiama("__class__", {})["ok"])
        self.assertFalse(self.api.chiama("cerca", {"ip": "not-an-ip"})["ok"])
        self.assertFalse(self.api.chiama("stato", "x")["ok"])
        self.assertFalse(self.api.chiama("decidi", {"id": "x", "ids": "tutti"})["ok"])

    def test_snapshot_is_json_serializable_and_has_no_secrets(self):
        self.e.imposta_pin("987654")
        txt = json.dumps(self.api.chiama("stato", {}))
        self.assertNotIn("987654", txt)
        self.assertNotIn("pin_ricezione", txt)
        self.assertNotIn("PRIVATE KEY", txt)

    def test_conf_ignores_unknown_keys(self):
        r = self.api.chiama("conf", {"nome_pc": "Nuovo", "pin_ricezione": None, "schema": 1, "preferiti": {"x": 1}})
        self.assertEqual(r["aggiornati"], ["nome_pc"])


if __name__ == "__main__":
    unittest.main()
