"""Funzioni di supporto: nomi file sicuri, scrittura senza sovrascrittura, formati, velocità."""
import collections
import errno
import os
import re
import threading
import time
from pathlib import Path

MAX_COMPONENT_BYTES = 200


class PathError(Exception):
    """Il percorso richiesto uscirebbe dalla cartella autorizzata."""


def clean_label(v, limit=60, default=""):
    s = re.sub(r"[\x00-\x1f\x7f]", "", str(v if v is not None else "")).strip()
    return s[:limit] or default


def _truncate_bytes(s, limit):
    b = s.encode("utf-8")
    return s if len(b) <= limit else b[:limit].decode("utf-8", "ignore")


def truncate_name(name, limit=MAX_COMPONENT_BYTES):
    if len(name.encode("utf-8")) <= limit:
        return name
    stem, dot, ext = name.rpartition(".")
    if dot and stem and len(ext.encode("utf-8")) <= 20:
        return _truncate_bytes(stem, limit - len(ext.encode("utf-8")) - 1) + "." + ext
    return _truncate_bytes(name, limit)


def safe_parts(name):
    """Scompone un nome ricevuto in componenti sicuri (niente '..', separatori, controlli)."""
    parts = []
    for p in re.split(r"[\\/]+", str(name or "")):
        p = re.sub(r"[\x00-\x1f\x7f]", "", p).strip()
        if p in ("", ".", ".."):
            continue
        parts.append(truncate_name(p))
    return parts or ["file"]


def prepare_target(root, parts):
    """Crea le sottocartelle di `parts[:-1]` sotto `root` e restituisce il percorso finale.

    Rifiuta qualunque componente che sia un collegamento simbolico, anche se punta
    dentro la cartella: nessuna scrittura può uscire da `root`.
    """
    root = Path(root).resolve()
    cur = root
    for comp in parts[:-1]:
        nxt = cur / comp
        if nxt.is_symlink():
            raise PathError("collegamento simbolico nel percorso")
        try:
            nxt.mkdir(exist_ok=True)
        except FileExistsError:
            pass
        if not nxt.is_dir():
            raise PathError("il percorso contiene un file")
        cur = nxt
    if not cur.resolve().is_relative_to(root):
        raise PathError("percorso fuori dalla cartella autorizzata")
    final = cur / parts[-1]
    if final.is_symlink():
        raise PathError("collegamento simbolico come destinazione")
    return final


_NO_LINK = {errno.EPERM, errno.EXDEV, errno.EMLINK, getattr(errno, "ENOTSUP", 95), errno.ENOSYS}


def commit_unique(tmp, dest):
    """Sposta `tmp` in `dest` senza mai sovrascrivere: se esiste sceglie 'nome (1).ext'.

    Usa un hard link (atomico e fallisce se la destinazione esiste); dove non è
    supportato ripiega sulla rinomina dopo un controllo di esistenza.
    """
    tmp, dest = Path(tmp), Path(dest)
    for i in range(10000):
        cand = dest if i == 0 else dest.with_name(f"{dest.stem} ({i}){dest.suffix}")
        try:
            os.link(tmp, cand)
        except FileExistsError:
            continue
        except OSError as e:
            if e.errno not in _NO_LINK:
                raise
            if os.path.lexists(cand):
                continue
            os.rename(tmp, cand)
            return cand
        os.unlink(tmp)
        return cand
    raise FileExistsError(dest)


def dimensione(n):
    n = float(n)
    for u in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return ("%.0f %s" if u == "B" else "%.1f %s") % (n, u)
        n /= 1024
    return "%.1f TB" % n


def tipo_file(nome):
    e = Path(nome).suffix.lower()
    if e in (".jpg", ".jpeg", ".png", ".gif", ".webp", ".bmp", ".heic", ".heif", ".avif"):
        return "foto"
    if e in (".mov", ".mp4", ".m4v", ".avi", ".mkv", ".webm"):
        return "video"
    if e in (".mp3", ".m4a", ".wav", ".aac", ".flac", ".ogg", ".opus"):
        return "audio"
    if e == ".pdf":
        return "pdf"
    if e in (".zip", ".rar", ".7z", ".tar", ".gz"):
        return "archivio"
    if e in (".txt", ".md", ".json", ".csv", ".js", ".py", ".html", ".css", ".ts"):
        return "testo"
    return "file"


class Progress:
    """Contatore di byte con velocità su finestra mobile; emette al massimo ~10 volte/s."""

    WINDOW = 4.0

    def __init__(self, on_change=None):
        self.total = 0
        self._samples = collections.deque()
        self._on_change = on_change
        self._last_emit = 0.0
        self._lock = threading.Lock()

    def add(self, n):
        now = time.monotonic()
        with self._lock:
            self.total += n
            if not self._samples or now - self._samples[-1][0] >= 0.25:
                self._samples.append((now, self.total))
            while self._samples and now - self._samples[0][0] > self.WINDOW:
                self._samples.popleft()
        if self._on_change and now - self._last_emit >= 0.1:
            self._last_emit = now
            self._on_change()

    def reset(self, total=0):
        with self._lock:
            self.total = total
            self._samples.clear()

    def speed(self):
        """Byte/s se la misura è attendibile (almeno 1 s di campioni), altrimenti None."""
        now = time.monotonic()
        with self._lock:
            if len(self._samples) < 2 or now - self._samples[-1][0] > 2.0:
                return None
            t0, b0 = self._samples[0]
            t1, b1 = self._samples[-1]
            if t1 - t0 < 1.0:
                return None
            return max(0.0, (b1 - b0) / (t1 - t0))
