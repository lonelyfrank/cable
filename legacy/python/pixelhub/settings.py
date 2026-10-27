"""Impostazioni persistenti, con schema versionato e migrazione dalla 3.x."""
import copy
import hashlib
import json
import os
import re
import secrets
import shutil
import threading
import time
from pathlib import Path

SCHEMA = 4
FP_RE = re.compile(r"^[0-9A-F]{64}$")
PIN_ITERATIONS = 200_000


def defaults(paths):
    return {
        "schema": SCHEMA,
        "nome_pc": "Fedora Pixel Hub",
        "cartella": str(paths.downloads / "Pixel Hub"),
        "cartella_foto": str(paths.pictures / "Pixel"),
        "vetro": 45,
        "tema": "auto",
        "notifiche": True,
        "schermo_intero": False,
        "background": True,
        "avvio_login": False,
        "preferiti": {},          # fp -> {"alias","ip","port"}; NON è un'identità verificata
        "pin_ricezione": None,    # {"salt","hash","it"} oppure None
        "migrato_da": None,
    }


def hash_pin(pin, salt=None, iterations=PIN_ITERATIONS):
    salt = salt or secrets.token_hex(16)
    h = hashlib.pbkdf2_hmac("sha256", pin.encode(), salt.encode(), iterations).hex()
    return {"salt": salt, "hash": h, "it": iterations}


def verify_pin_record(rec, pin):
    if not rec or not isinstance(pin, str):
        return False
    h = hashlib.pbkdf2_hmac("sha256", pin.encode(), str(rec["salt"]).encode(), int(rec["it"])).hex()
    return secrets.compare_digest(h, str(rec["hash"]))


def _valid_pin_record(r):
    return (isinstance(r, dict) and isinstance(r.get("salt"), str) and isinstance(r.get("hash"), str)
            and isinstance(r.get("it"), int) and 1000 <= r["it"] <= 10_000_000)


def _clean_favorites(v):
    out = {}
    if isinstance(v, dict):
        for fp, rec in v.items():
            fp = str(fp).upper()
            if FP_RE.match(fp) and isinstance(rec, dict):
                port = rec.get("port")
                out[fp] = {"alias": str(rec.get("alias") or "Dispositivo")[:60],
                           "ip": str(rec.get("ip") or "")[:45],
                           "port": port if isinstance(port, int) and 0 < port < 65536 else 53317}
    return out


def validate(key, v):
    """Restituisce (ok, valore_normalizzato)."""
    if key == "nome_pc" and isinstance(v, str) and v.strip():
        return True, re.sub(r"[\x00-\x1f]", "", v).strip()[:40]
    if key in ("cartella", "cartella_foto") and isinstance(v, str) and v.startswith("/"):
        return True, v
    if key == "vetro" and isinstance(v, (int, float)) and not isinstance(v, bool):
        return True, max(0, min(100, int(v)))
    if key == "tema" and v in ("auto", "chiaro", "scuro"):
        return True, v
    if key in ("notifiche", "schermo_intero", "background", "avvio_login") and isinstance(v, bool):
        return True, v
    if key == "preferiti":
        return True, _clean_favorites(v)
    if key == "pin_ricezione" and (v is None or _valid_pin_record(v)):
        return True, v
    return False, None


def normalize(raw, paths):
    d = defaults(paths)
    if isinstance(raw, dict):
        for k, v in raw.items():
            ok, nv = validate(k, v)
            if ok:
                d[k] = nv
        if isinstance(raw.get("migrato_da"), dict):
            d["migrato_da"] = raw["migrato_da"]
    return d


def _atomic_write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".%d.tmp" % os.getpid())
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        f.write(text)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def backup_file(src, dest_dir, label):
    dest_dir.mkdir(parents=True, exist_ok=True)
    os.chmod(dest_dir, 0o700)
    dest = dest_dir / ("%s-%s%s" % (label, time.strftime("%Y%m%d-%H%M%S"), src.suffix or ".json"))
    shutil.copy2(src, dest)
    os.chmod(dest, 0o600)
    return dest


def migrate_v3(legacy_file, paths):
    """Converte la configurazione 3.x. Non modifica il file originale.

    Conserva cartelle e preferenze; i vecchi dispositivi 'fidati' e il telefono associato
    diventano semplici *preferiti non verificati*; l'auto-accettazione viene scartata.
    """
    old = json.loads(Path(legacy_file).read_text(encoding="utf-8"))
    new = normalize(old, paths)
    fav = {}
    pairing = old.get("pairing") if isinstance(old.get("pairing"), dict) else {}
    cand = dict(old.get("fidati") or {}) if isinstance(old.get("fidati"), dict) else {}
    if pairing.get("fp"):
        cand.setdefault(pairing["fp"], pairing.get("alias") or "Pixel")
    for fp, alias in cand.items():
        fp = str(fp).upper()
        if FP_RE.match(fp):
            fav[fp] = {"alias": str(alias)[:60], "ip": "", "port": 53317}
    new["preferiti"] = fav
    pin = {"salt": pairing.get("pin_salt"), "hash": pairing.get("pin_hash"), "it": 100000}
    new["pin_ricezione"] = pin if _valid_pin_record(pin) else None
    new["migrato_da"] = {"versione": 3, "quando": time.strftime("%Y-%m-%dT%H:%M:%S"),
                         "preferiti_importati": len(fav), "pin_conservato": bool(new["pin_ricezione"])}
    return new


class Settings:
    def __init__(self, paths):
        self.paths = paths
        self.file = paths.settings_file
        self.lock = threading.RLock()
        self.d = defaults(paths)
        self.avvisi = []
        self._load()

    def _load(self):
        raw = None
        if self.file.exists():
            try:
                raw = json.loads(self.file.read_text(encoding="utf-8"))
                if not isinstance(raw, dict):
                    raise ValueError("formato")
            except (OSError, ValueError):
                bad = self.file.with_name("settings.json.corrotto-" + time.strftime("%Y%m%d-%H%M%S"))
                try:
                    os.replace(self.file, bad)
                except OSError:
                    pass
                self.avvisi.append("Le impostazioni erano illeggibili: sono state messe da parte in %s." % bad.name)
                raw = None
            else:
                version = int(raw.get("schema") or 0)
                self.d = normalize(raw, self.paths)
                if version != SCHEMA:
                    backup_file(self.file, self.paths.backups, "settings-v%d" % version)
                    self.d["schema"] = SCHEMA
                    self.salva()
                return
        legacy = self.paths.legacy_config
        if legacy.exists():
            try:
                backup_file(legacy, self.paths.backups, "config-v3")
                self.d = migrate_v3(legacy, self.paths)
                self.salva()
                self.avvisi.append("Impostazioni importate dalla versione 3.x (copia di sicurezza in %s)."
                                   % self.paths.backups)
            except (OSError, ValueError) as e:
                self.avvisi.append("Migrazione dalla 3.x non riuscita (%s): partenza con valori predefiniti." % e)

    def __getitem__(self, k):
        with self.lock:
            return copy.deepcopy(self.d.get(k))

    def snapshot(self):
        with self.lock:
            return copy.deepcopy(self.d)

    def salva(self):
        with self.lock:
            _atomic_write(self.file, json.dumps(self.d, indent=2, ensure_ascii=False))

    def aggiorna(self, patch):
        done = {}
        with self.lock:
            for k, v in patch.items():
                if k in ("schema", "migrato_da"):
                    continue
                ok, nv = validate(k, v)
                if ok:
                    self.d[k] = nv
                    done[k] = nv
            if done:
                self.salva()
        return done

    def imposta_pin(self, pin):
        """pin None/'' = nessun PIN. Salva solo hash con sale."""
        if pin:
            if not isinstance(pin, str) or not (4 <= len(pin) <= 32):
                return False
            return bool(self.aggiorna({"pin_ricezione": hash_pin(pin)}))
        self.aggiorna({"pin_ricezione": None})
        return True

    def preferito(self, fp, on, info=None):
        fp = str(fp).upper()
        if not FP_RE.match(fp):
            return False
        with self.lock:
            fav = self.d["preferiti"]
            if on:
                info = info or {}
                fav[fp] = {"alias": str(info.get("alias") or fav.get(fp, {}).get("alias") or "Dispositivo")[:60],
                           "ip": str(info.get("ip") or fav.get(fp, {}).get("ip") or "")[:45],
                           "port": info.get("port") or fav.get(fp, {}).get("port") or 53317}
            else:
                fav.pop(fp, None)
            self.salva()
        return True

    def ripristina(self):
        """Riporta tutto ai valori predefiniti dopo un backup. Non tocca file ricevuti né identità."""
        with self.lock:
            bk = None
            if self.file.exists():
                bk = backup_file(self.file, self.paths.backups, "settings-prima-del-ripristino")
            self.d = defaults(self.paths)
            self.salva()
        return bk
