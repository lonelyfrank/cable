"""Validazione dei messaggi LocalSend v2 e limiti applicati alle richieste in ingresso."""
import re
from dataclasses import dataclass

from .util import clean_label, safe_parts

API = "/api/localsend/v2"
SHA_RE = re.compile(r"^[0-9a-fA-F]{64}$")
FP_RE = re.compile(r"^[0-9A-Fa-f]{64}$")


@dataclass(frozen=True)
class Limits:
    max_json_body: int = 8 * 1024 * 1024
    max_files: int = 5000
    max_file_size: int = 2 ** 50
    max_total_size: int = 2 ** 50
    max_preview: int = 64 * 1024
    max_pending: int = 5
    decision_timeout: float = 120.0
    session_idle: float = 120.0
    max_connections: int = 32
    pin_failures: int = 10
    pin_window: float = 300.0


class ProtocolError(Exception):
    def __init__(self, status, message=""):
        super().__init__(message)
        self.status = status
        self.message = message


@dataclass
class FileMeta:
    id: str
    name: str
    parts: list
    size: int
    file_type: str
    sha256: str
    preview: str


def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


def parse_peer_info(info):
    """Valida le informazioni di un dispositivo; restituisce un dict pulito o None."""
    if not isinstance(info, dict):
        return None
    fp = info.get("fingerprint")
    if not isinstance(fp, str) or not FP_RE.match(fp):
        return None
    port = info.get("port", 53317)
    if not _is_int(port) or not 0 < port < 65536:
        return None
    proto = info.get("protocol", "https")
    if proto != "https":
        return None
    return {"fp": fp.upper(), "alias": clean_label(info.get("alias"), 60, "Dispositivo"),
            "model": clean_label(info.get("deviceModel"), 60),
            "type": clean_label(info.get("deviceType"), 20, "mobile"),
            "port": port, "protocol": "https"}


def parse_prepare_upload(body, limits=Limits()):
    """Valida il corpo di prepare-upload. Solleva ProtocolError(400)."""
    if not isinstance(body, dict) or not isinstance(body.get("files"), dict):
        raise ProtocolError(400, "corpo non valido")
    info = body.get("info")
    if not isinstance(info, dict):
        raise ProtocolError(400, "info mancante")
    raw = body["files"]
    if not raw or len(raw) > limits.max_files:
        raise ProtocolError(400, "numero di file non valido")
    files, total = {}, 0
    for key, f in raw.items():
        if not isinstance(f, dict) or not isinstance(key, str) or not 0 < len(key) <= 128:
            raise ProtocolError(400, "file non valido")
        size = f.get("size")
        if not _is_int(size) or not 0 <= size <= limits.max_file_size:
            raise ProtocolError(400, "dimensione non valida")
        name = f.get("fileName")
        if not isinstance(name, str) or not 0 < len(name) <= 1024:
            raise ProtocolError(400, "nome file non valido")
        sha = f.get("sha256")
        if sha is not None and not (isinstance(sha, str) and SHA_RE.match(sha)):
            raise ProtocolError(400, "sha256 non valido")
        prev = f.get("preview")
        if prev is not None and (not isinstance(prev, str) or len(prev.encode("utf-8")) > limits.max_preview):
            raise ProtocolError(400, "anteprima non valida")
        ftype = f.get("fileType")
        ftype = ftype[:100] if isinstance(ftype, str) else ""
        total += size
        if total > limits.max_total_size:
            raise ProtocolError(400, "totale troppo grande")
        files[key] = FileMeta(key, name, safe_parts(name), size, ftype,
                              sha.lower() if sha else "", prev if prev is not None else "")
    return info, files


def is_text_message(files):
    """Un invio di solo testo: ogni 'file' ha un'anteprima testuale (come l'app ufficiale)."""
    return bool(files) and all(f.preview and f.file_type.startswith("text/") for f in files.values())
