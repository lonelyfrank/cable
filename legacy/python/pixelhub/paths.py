"""Percorsi dell'applicazione, risolti in modo pigro per poter essere sostituiti nei test."""
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path


def _xdg_user_dir(kind, fallback, home):
    try:
        r = subprocess.run(["xdg-user-dir", kind], capture_output=True, text=True, timeout=5)
        v = r.stdout.strip()
        if r.returncode == 0 and v.startswith("/") and Path(v) != home:
            return Path(v)
    except (OSError, subprocess.TimeoutExpired):
        pass
    return home / fallback


@dataclass
class Paths:
    home: Path
    config: Path
    data: Path
    cache: Path
    downloads: Path
    pictures: Path

    @classmethod
    def default(cls):
        home = Path.home()
        cfg = Path(os.environ.get("XDG_CONFIG_HOME") or home / ".config")
        data = Path(os.environ.get("XDG_DATA_HOME") or home / ".local" / "share")
        cache = Path(os.environ.get("XDG_CACHE_HOME") or home / ".cache")
        return cls(home, cfg / "pixel-hub", data / "pixel-hub", cache / "pixel-hub",
                   _xdg_user_dir("DOWNLOAD", "Downloads", home),
                   _xdg_user_dir("PICTURES", "Pictures", home))

    @classmethod
    def sotto(cls, base):
        """Percorsi isolati in una cartella (test e sviluppo)."""
        base = Path(base)
        return cls(base / "home", base / "config", base / "data", base / "cache",
                   base / "home" / "Downloads", base / "home" / "Pictures")

    # --- posizioni della versione 3.x (solo lettura, per la migrazione)
    @property
    def legacy_config(self):
        return self.home / ".config" / "pixel-hub-v3" / "config.json"

    @property
    def legacy_cache(self):
        return self.home / ".cache" / "pixel-hub-v3"

    @property
    def settings_file(self):
        return self.config / "settings.json"

    @property
    def backups(self):
        return self.config / "backups"

    @property
    def identity_dir(self):
        return self.data / "identity"
