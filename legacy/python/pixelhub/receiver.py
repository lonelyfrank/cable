"""Ricezione LocalSend: richieste, sessioni, upload in streaming e pulizia.

Il modulo è indipendente da GTK e dal livello HTTP: `server.py` lo chiama e i test
lo esercitano direttamente.
"""
import collections
import errno
import hashlib
import os
import secrets
import shutil
import threading
import time
import uuid
from pathlib import Path

from .protocol import Limits, ProtocolError, is_text_message, parse_prepare_upload
from .settings import verify_pin_record
from .util import PathError, Progress, clean_label, commit_unique, prepare_target, tipo_file, dimensione
from .bus import log

DISK_MARGIN = 64 * 1024 * 1024
TERMINAL = ("completata", "parziale", "rifiutata", "scaduta", "annullata", "errore")


class Cancelled(Exception):
    pass


class SizeMismatch(Exception):
    pass


class HashMismatch(Exception):
    pass


class StreamError(Exception):
    """Errore di lettura dalla connessione del mittente (non un errore di disco)."""


class PeerGone(Exception):
    """Il mittente ha chiuso la connessione mentre attendeva la decisione."""


class InFile:
    __slots__ = ("id", "name", "parts", "size", "sha256", "file_type", "token", "state",
                 "received", "path", "tmp", "error")

    def __init__(self, meta):
        self.id, self.name, self.parts, self.size = meta.id, meta.name, meta.parts, meta.size
        self.sha256, self.file_type = meta.sha256, meta.file_type
        self.token = ""
        self.state = "attesa"      # attesa | ricezione | fatto | rifiutato
        self.received = 0
        self.path = ""
        self.tmp = None
        self.error = ""


class Incoming:
    def __init__(self, ip, alias, peer_fp, kind, metas, previews, on_progress):
        self.id = uuid.uuid4().hex[:12]
        self.sid = ""
        self.ip, self.alias, self.peer_fp, self.kind = ip, alias, peer_fp, kind
        self.files = {k: InFile(m) for k, m in metas.items()}
        self.previews = previews
        self.selected = []
        self.state = "attesa"      # attesa | accettata | ricezione | <TERMINAL>
        self.error = ""
        self.created = time.time()
        self.finished = 0.0
        self.root = None
        self.event = threading.Event()
        self.cancel = threading.Event()
        self.active = 0
        self.last = time.monotonic()
        self.progress = Progress(on_progress)

    def touch(self):
        self.last = time.monotonic()

    def total(self):
        return sum(self.files[k].size for k in (self.selected or self.files))

    def done_count(self):
        return sum(1 for k in self.selected if self.files[k].state == "fatto")

    def bytes(self):
        return sum((f.size if f.state == "fatto" else f.received) for k, f in self.files.items()
                   if k in self.selected)


class Receiver:
    def __init__(self, settings, bus, limits=Limits(), notify=None):
        self.settings = settings
        self.bus = bus
        self.limits = limits
        self.notify = notify or (lambda kind, **kw: None)
        self.lock = threading.RLock()
        self.incoming = {}
        self.by_sid = {}
        self.messages = collections.deque(maxlen=30)
        self._pin_failures = collections.deque()
        um = os.umask(0)
        os.umask(um)
        self._mode = 0o666 & ~um

    # ------------------------------------------------------------------ PIN
    def _check_pin(self, pin):
        rec = self.settings["pin_ricezione"]
        if not rec:
            return
        now = time.monotonic()
        with self.lock:
            while self._pin_failures and now - self._pin_failures[0] > self.limits.pin_window:
                self._pin_failures.popleft()
            if len(self._pin_failures) >= self.limits.pin_failures:
                raise ProtocolError(429, "troppi tentativi")
        if not pin:
            raise ProtocolError(401, "PIN richiesto")
        if not verify_pin_record(rec, pin):
            with self.lock:
                self._pin_failures.append(now)
            raise ProtocolError(401, "PIN errato")

    # ------------------------------------------------------------------ prepare-upload
    def prepare(self, ip, pin, body, is_closed=lambda: False):
        """Gestisce prepare-upload. Restituisce (stato, json|None)."""
        self._check_pin(pin)
        info, metas = parse_prepare_upload(body, self.limits)
        alias = clean_label(info.get("alias"), 60, "Un dispositivo")
        peer_fp = clean_label(info.get("fingerprint"), 64)
        kind = "testo" if is_text_message(metas) else "file"
        previews = {k: m.preview for k, m in metas.items()} if kind == "testo" else {}
        with self.lock:
            self.sweep()
            waiting = sum(1 for i in self.incoming.values() if i.state == "attesa")
            if waiting >= self.limits.max_pending:
                raise ProtocolError(429, "troppe richieste in attesa")
            if any(i.state in ("attesa", "accettata", "ricezione") and (i.ip == ip or i.state != "attesa")
                   for i in self.incoming.values()):
                raise ProtocolError(409, "un altro trasferimento è in corso")
            inc = Incoming(ip, alias, peer_fp, kind, metas, previews, lambda: self.bus.emit("ricezioni"))
            self.incoming[inc.id] = inc
        self.bus.emit("ricezioni")
        self.notify("richiesta", id=inc.id, alias=alias, n=len(metas), size=inc.total(), tipo=kind)
        deadline = time.monotonic() + self.limits.decision_timeout
        while not inc.event.wait(1.0):
            if time.monotonic() > deadline:
                self._finish(inc, "scaduta", "Nessuna risposta entro il tempo limite")
                raise ProtocolError(403, "scaduta")
            if is_closed():
                self._finish(inc, "annullata", "Il mittente ha annullato la richiesta")
                raise PeerGone()
        if inc.state != "accettata":
            raise ProtocolError(403, "rifiutata")
        if kind == "testo":
            return self._deliver_text(inc)
        return self._open_session(inc)

    def _deliver_text(self, inc):
        with self.lock:
            for k in inc.selected:
                self.messages.appendleft({"id": uuid.uuid4().hex[:10], "alias": inc.alias,
                                          "testo": inc.previews.get(k, ""), "quando": time.time()})
                inc.files[k].state = "fatto"
        self._finish(inc, "completata")
        self.notify("messaggio", alias=inc.alias, testo=self.messages[0]["testo"])
        return 204, None

    def _open_session(self, inc):
        root = Path(self.settings["cartella"])
        try:
            root.mkdir(parents=True, exist_ok=True)
            free = shutil.disk_usage(root).free
        except OSError as e:
            self._finish(inc, "errore", "Cartella di ricezione non utilizzabile: %s" % e.strerror)
            raise ProtocolError(500, "cartella") from e
        need = sum(inc.files[k].size for k in inc.selected)
        if need + DISK_MARGIN > free:
            self._finish(inc, "errore", "Spazio insufficiente: servono %s, liberi %s"
                         % (dimensione(need), dimensione(free)))
            raise ProtocolError(507, "spazio insufficiente")
        with self.lock:
            if inc.state != "accettata":     # annullata mentre si preparava la sessione
                raise ProtocolError(403, "annullata")
            inc.root = root
            inc.sid = uuid.uuid4().hex
            tokens = {}
            for k in inc.selected:
                inc.files[k].token = secrets.token_hex(16)
                tokens[k] = inc.files[k].token
            for k, f in inc.files.items():
                if k not in tokens:
                    f.state = "rifiutato"
            inc.state = "ricezione"
            inc.touch()
            self.by_sid[inc.sid] = inc
        self.bus.emit("ricezioni")
        return 200, {"sessionId": inc.sid, "files": tokens}

    # ------------------------------------------------------------------ decisione dell'utente
    def decide(self, rid, accept_ids=None):
        """accept_ids=None accetta tutto; lista vuota rifiuta; altrimenti accettazione parziale."""
        with self.lock:
            inc = self.incoming.get(rid)
            if not inc or inc.state != "attesa":
                return False
            ids = list(inc.files) if accept_ids is None else [k for k in accept_ids if k in inc.files]
            if ids:
                inc.selected = ids
                inc.state = "accettata"
            else:
                inc.state = "rifiutata"
                inc.finished = time.time()
            inc.event.set()
        self.bus.emit("ricezioni")
        return True

    def annulla(self, rid):
        """Annullamento locale: ferma subito l'I/O e scarta i parziali."""
        with self.lock:
            inc = self.incoming.get(rid)
        if not inc:
            return False
        if inc.state == "attesa":
            return self.decide(rid, [])
        if inc.state in ("accettata", "ricezione"):
            self._finish(inc, "annullata", "Annullato da te")
            return True
        return False

    # ------------------------------------------------------------------ upload
    def upload(self, ip, q, stream):
        sid, fid, token = q.get("sessionId", ""), q.get("fileId", ""), q.get("token", "")
        if not sid or not fid or not token:
            raise ProtocolError(400, "parametri mancanti")
        with self.lock:
            inc = self.by_sid.get(sid)
            f = inc.files.get(fid) if inc else None
            if not inc or not f or fid not in inc.selected:
                raise ProtocolError(403, "sessione o file sconosciuti")
            if inc.ip != ip or not secrets.compare_digest(f.token, token):
                raise ProtocolError(403, "token o indirizzo non validi")
            if inc.state != "ricezione" or f.state in ("ricezione", "fatto"):
                raise ProtocolError(409, "file già in ricezione o completato")
            if stream.length is not None and stream.length != f.size:
                raise ProtocolError(400, "lunghezza diversa da quella dichiarata")
            f.state, f.received, f.error = "ricezione", 0, ""
            inc.active += 1
            inc.touch()
        self.bus.emit("ricezioni")
        committed = False
        try:
            dest = self._write(inc, f, stream)
            committed = True
        except Cancelled:
            f.error = "Annullato"
            raise ProtocolError(409, "annullato")
        except SizeMismatch:
            f.error = "Dimensione diversa da quella dichiarata"
            raise ProtocolError(400, f.error)
        except HashMismatch:
            f.error = "Controllo di integrità fallito (sha256)"
            raise ProtocolError(422, f.error)
        except PathError as e:
            f.error = "Percorso rifiutato: %s" % e
            raise ProtocolError(403, f.error)
        except StreamError:
            f.error = "Connessione interrotta"
            raise
        except OSError as e:
            if e.errno == errno.ENOSPC:
                f.error = "Spazio insufficiente"
                raise ProtocolError(507, f.error)
            f.error = "Errore di scrittura: %s" % e.strerror
            log("upload", repr(e))
            raise ProtocolError(500, f.error)
        finally:
            with self.lock:
                inc.active -= 1
                if committed:
                    f.state, f.path, f.received = "fatto", str(dest), f.size
                    if inc.state == "ricezione" and inc.done_count() == len(inc.selected):
                        self._finish(inc, "completata")
                else:
                    f.state, f.received = "attesa", 0
                    if f.tmp:
                        self._unlink(f.tmp)
                        f.tmp = None
            self.bus.emit("ricezioni")
        return dest

    def _write(self, inc, f, stream):
        final = prepare_target(inc.root, f.parts)
        tmp = final.parent / (".pixelhub-%s.part" % secrets.token_hex(8))
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        f.tmp = tmp
        h = hashlib.sha256() if f.sha256 else None
        got = 0
        with os.fdopen(fd, "wb") as out:
            def write(chunk):
                nonlocal got
                if inc.cancel.is_set():
                    raise Cancelled()
                got += len(chunk)
                if got > f.size:
                    raise SizeMismatch()
                out.write(chunk)
                if h:
                    h.update(chunk)
                f.received = got
                inc.progress.add(len(chunk))
                inc.touch()
            stream.read(write)
            if got != f.size:
                raise SizeMismatch()
            out.flush()
            os.fsync(out.fileno())
        if h and h.hexdigest() != f.sha256:
            raise HashMismatch()
        if inc.cancel.is_set():
            raise Cancelled()
        os.chmod(tmp, self._mode)
        dest = commit_unique(tmp, final)
        f.tmp = None
        return dest

    @staticmethod
    def _unlink(p):
        try:
            os.unlink(p)
        except OSError:
            pass

    # ------------------------------------------------------------------ annullamento dal mittente
    def cancel_from_sender(self, ip, sid):
        with self.lock:
            inc = self.by_sid.get(sid)
            if not inc or inc.ip != ip:
                raise ProtocolError(403, "sessione sconosciuta")
        self._finish(inc, "annullata", "Annullato dal mittente")

    # ------------------------------------------------------------------ chiusura e pulizia
    def _finish(self, inc, state, error=""):
        with self.lock:
            if inc.state in TERMINAL:
                return
            done = inc.done_count()
            if state == "completata" and done < len(inc.selected):
                state = "parziale"
            elif state in ("annullata", "scaduta", "errore") and done > 0:
                state = "parziale"
            if state == "parziale" and not error:
                error = "Ricevuti %d file su %d" % (done, len(inc.selected))
            inc.state, inc.error, inc.finished = state, error, time.time()
            inc.cancel.set()
            inc.event.set()
            self.by_sid.pop(inc.sid, None)
            for f in inc.files.values():
                if f.tmp:
                    self._unlink(f.tmp)
        self.bus.emit("ricezioni")
        if inc.kind == "file":
            self.notify("fine", id=inc.id, alias=inc.alias, stato=state, ricevuti=done,
                        totale=len(inc.selected), errore=error)

    def sweep(self):
        """Scade le sessioni inattive e dimentica quelle concluse da tempo."""
        now_m, now = time.monotonic(), time.time()
        with self.lock:
            snapshot = list(self.incoming.values())
        for inc in snapshot:
            if inc.state == "ricezione" and inc.active == 0 and now_m - inc.last > self.limits.session_idle:
                self._finish(inc, "scaduta", "Il mittente non ha completato l'invio")
        with self.lock:
            finished = sorted((i for i in self.incoming.values() if i.state in TERMINAL),
                              key=lambda i: i.finished, reverse=True)
            for i in finished[:]:
                if now - i.finished > 3600 or finished.index(i) >= 30:
                    self.incoming.pop(i.id, None)

    def rimuovi_messaggio(self, mid):
        with self.lock:
            for m in list(self.messages):
                if m["id"] == mid:
                    self.messages.remove(m)
        self.bus.emit("ricezioni")

    def pulisci_residui(self):
        """All'avvio elimina i soli temporanei di Pixel Hub, vecchi, in cima alla cartella."""
        try:
            root = Path(self.settings["cartella"])
            for p in root.glob(".pixelhub-*.part"):
                if time.time() - p.stat().st_mtime > 86400:
                    p.unlink()
        except OSError:
            pass

    # ------------------------------------------------------------------ stato per la UI
    def snapshot(self):
        with self.lock:
            lst = sorted(self.incoming.values(), key=lambda i: -i.created)
            out = []
            for i in lst:
                total = i.total()
                out.append({
                    "id": i.id, "alias": i.alias, "ip": i.ip, "stato": i.state, "tipo": i.kind,
                    "totale": total, "byte": i.bytes(), "velocita": i.progress.speed() if i.state == "ricezione" else None,
                    "errore": i.error, "creata": i.created, "finita": i.finished,
                    "ricevuti": i.done_count(), "selezionati": len(i.selected),
                    "file": [{"id": k, "nome": f.name, "size": f.size, "tipo": tipo_file(f.name),
                              "stato": f.state, "errore": f.error, "percorso": f.path}
                             for k, f in list(i.files.items())[:200]],
                    "altri": max(0, len(i.files) - 200)})
            return {"ricezioni": out, "messaggi": list(self.messages)}
