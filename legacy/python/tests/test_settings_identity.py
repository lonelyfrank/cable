import json
import os
import stat
import unittest

from pixelhub.identity import Identity
from pixelhub.paths import Paths
from pixelhub.settings import SCHEMA, Settings, hash_pin, verify_pin_record
from tests.helpers import TmpCase

FP1, FP2 = "A" * 64, "B" * 64


class V3Migration(TmpCase):
    def legacy(self, **extra):
        paths = Paths.sotto(self.tmp)
        paths.legacy_config.parent.mkdir(parents=True)
        data = {"nome_pc": "Il mio PC", "cartella": "/data/ricevuti", "cartella_foto": "/data/foto",
                "tema": "scuro", "vetro": 70, "auto_accetta": True, "avvio_login": False,
                "fidati": {FP2.lower(): "Vecchio fidato", "non-valido": "x"},
                "pairing": {"fp": FP1, "alias": "Pixel di prova", "pin_salt": "s", "pin_hash": "h", "ready": True,
                            "adb_serial": "XYZ"}}
        data.update(extra)
        paths.legacy_config.write_text(json.dumps(data))
        return paths

    def test_migration_preserves_and_demotes(self):
        paths = self.legacy()
        original = paths.legacy_config.read_bytes()
        s = Settings(paths)
        self.assertEqual(s["cartella"], "/data/ricevuti")
        self.assertEqual(s["cartella_foto"], "/data/foto")
        self.assertEqual(s["nome_pc"], "Il mio PC")
        self.assertEqual(s["tema"], "scuro")
        self.assertEqual(s["schema"], SCHEMA)
        self.assertNotIn("auto_accetta", s.snapshot())
        fav = s["preferiti"]
        self.assertEqual(set(fav), {FP1, FP2})          # importati solo come preferiti
        self.assertEqual(s["migrato_da"]["versione"], 3)
        self.assertEqual(paths.legacy_config.read_bytes(), original)    # file 3.x intatto
        backups = list(paths.backups.glob("config-v3-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), original)
        self.assertEqual(stat.S_IMODE(os.stat(paths.settings_file).st_mode), 0o600)

    def test_idempotent(self):
        paths = self.legacy()
        Settings(paths)
        paths.legacy_config.write_text(json.dumps({"nome_pc": "Cambiato dopo"}))
        s2 = Settings(paths)
        self.assertEqual(s2["nome_pc"], "Il mio PC")        # non rimigra sopra le impostazioni nuove
        self.assertEqual(len(list(paths.backups.glob("config-v3-*"))), 1)

    def test_old_pin_hash_is_kept_compatible(self):
        import hashlib
        salt = "abcd"
        h = hashlib.pbkdf2_hmac("sha256", b"123456", salt.encode(), 100000).hex()
        paths = self.legacy(pairing={"fp": FP1, "alias": "P", "pin_salt": salt, "pin_hash": h})
        s = Settings(paths)
        self.assertTrue(verify_pin_record(s["pin_ricezione"], "123456"))
        self.assertFalse(verify_pin_record(s["pin_ricezione"], "654321"))

    def test_corrupted_new_settings_set_aside(self):
        paths = Paths.sotto(self.tmp)
        paths.config.mkdir(parents=True)
        paths.settings_file.write_text("{non json")
        s = Settings(paths)
        self.assertTrue(s.avvisi)
        self.assertTrue(list(paths.config.glob("settings.json.corrotto-*")))
        self.assertEqual(s["nome_pc"], "Fedora Pixel Hub")


class SettingsBehaviour(TmpCase):
    def test_validation_and_persistence(self):
        paths = Paths.sotto(self.tmp)
        s = Settings(paths)
        done = s.aggiorna({"tema": "rosa", "vetro": 999, "nome_pc": "  PC  ", "cartella": "relativa", "x": 1,
                           "schema": 99, "background": "si"})
        self.assertEqual(done, {"vetro": 100, "nome_pc": "PC"})
        self.assertEqual(Settings(paths)["vetro"], 100)

    def test_pin_is_hashed_never_plain(self):
        paths = Paths.sotto(self.tmp)
        s = Settings(paths)
        self.assertTrue(s.imposta_pin("123456"))
        self.assertNotIn("123456", paths.settings_file.read_text())
        self.assertTrue(verify_pin_record(s["pin_ricezione"], "123456"))
        self.assertFalse(s.imposta_pin("12"))
        s.imposta_pin(None)
        self.assertIsNone(s["pin_ricezione"])

    def test_reset_makes_backup_and_keeps_folder_files(self):
        paths = Paths.sotto(self.tmp)
        s = Settings(paths)
        s.aggiorna({"nome_pc": "Mio"})
        s.preferito(FP1, True, {"alias": "P"})
        bk = s.ripristina()
        self.assertTrue(bk.exists())
        self.assertIn("Mio", bk.read_text())
        self.assertEqual(s["nome_pc"], "Fedora Pixel Hub")
        self.assertEqual(s["preferiti"], {})

    def test_favorite_validation(self):
        s = Settings(Paths.sotto(self.tmp))
        self.assertFalse(s.preferito("zz", True))
        self.assertTrue(s.preferito(FP1.lower(), True, {"alias": "X"}))
        self.assertIn(FP1, s["preferiti"])


class IdentityTest(TmpCase):
    def test_create_reload_permissions(self):
        paths = Paths.sotto(self.tmp)
        a = Identity(paths)
        self.assertEqual(a.stato, "ok")
        self.assertEqual(stat.S_IMODE(os.stat(a.key).st_mode), 0o600)
        self.assertEqual(len(a.fingerprint), 64)
        self.assertEqual(Identity(paths).fingerprint, a.fingerprint)
        self.assertTrue(str(a.key).startswith(str(paths.data)))      # dati persistenti, non cache

    def test_import_from_v3_cache(self):
        paths = Paths.sotto(self.tmp)
        src = Identity(Paths.sotto(self.tmp / "altro"))
        paths.legacy_cache.mkdir(parents=True)
        (paths.legacy_cache / "cert.pem").write_bytes(src.cert.read_bytes())
        (paths.legacy_cache / "key.pem").write_bytes(src.key.read_bytes())
        i = Identity(paths)
        self.assertEqual(i.fingerprint, src.fingerprint)
        self.assertEqual(i.origine, "importata-v3")
        self.assertTrue((paths.legacy_cache / "key.pem").exists())    # originale non toccato

    def test_corrupt_identity_not_silently_replaced(self):
        paths = Paths.sotto(self.tmp)
        a = Identity(paths)
        a.key.write_text("spazzatura")
        b = Identity(paths)
        self.assertEqual(b.stato, "invalida")
        self.assertEqual(a.key.read_text(), "spazzatura")            # nessuna rigenerazione automatica

    def test_explicit_regeneration_keeps_backup(self):
        paths = Paths.sotto(self.tmp)
        a = Identity(paths)
        old = a.fingerprint
        new = a.rigenera()
        self.assertNotEqual(old, new)
        self.assertEqual(len(list(a.dir.glob("backup-*/key.pem"))), 1)

    def test_mismatched_pair_invalid(self):
        paths = Paths.sotto(self.tmp)
        a = Identity(paths)
        other = Identity(Paths.sotto(self.tmp / "x"))
        a.key.write_bytes(other.key.read_bytes())
        self.assertEqual(Identity(paths).stato, "invalida")


if __name__ == "__main__":
    unittest.main()
