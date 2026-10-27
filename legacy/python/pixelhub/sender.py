"""Invio LocalSend: lettura in streaming dalla sorgente, PIN, accettazione parziale, annullamento."""
import hashlib
import http.client
import mimetypes
import os
import socket
import ssl
import threading
import time
import urllib.parse
import uuid
from pathlib import Path

from .bus import log
from .client import IdentityMismatch, open_connection
from .protocol import API
from .util import Progress, safe_parts

CHUNK = 1 << 20
HASH_MAX = 32 * 1024 * 1024
MAX_FILES = 5000
PREPARE_TIMEOUT = 150
PIN_WAIT = 180
DONE = ("completato", "parziale", "rifiutato", "annullato", "errore")


class SourceChanged(Exception):
    pass


class JobEnd(Exception):
    def __init__(self, state, error=""):
        super().__init__(error)
        self.state, self.error = state, error


class OutFile:
    __slots__ = ("id", "name", "path", "size", "mtime_ns", "state", "error", "text", "sha256")

    def __init__(self, name, path=None, size=0, mtime_ns=0, text=None):
        self.id = uuid.uuid4().hex
        self.name, self.path, self.size, self.mtime_ns, self.text = name, path, size, mtime_ns, text
        self.state, self.error, self.sha256 = "attesa", "", None


class Job:
    def __init__(self, dev, items, is_text, on_progress):
        self.id = uuid.uuid4().hex[:12]
        self.fp, self.alias, self.ip, self.port = dev.fp, dev.alias, dev.ip, dev.port
        self.items = items
        self.is_text = is_text
        self.state = "preparazione"
        self.error = ""
        self.created = time.time()
        self.cancel = threading.Event()
        self.conn = None
        self.session = ""
        self.pin_cv = threading.Condition()
        self.pin = None
        self.pin_wrong = False
        self.progress = Progress(on_progress)

    def total(self):
        return sum(i.size for i in self.items)


def collect(paths):
    """Espande file e cartelle in (percorso, nome relativo). Ignora collegamenti e file speciali."""
    out, skipped = [], 0
    for raw in paths:
        p = Path(raw)
        try:
            if p.is_dir() and not p.is_symlink():
                base = p.name or "cartella"
                for dirpath, dirs, files in os.walk(p, followlinks=False):
                    dirs[:] = sorted(d for d in dirs if not os.path.islink(os.path.join(dirpath, d)))
                    for fn in sorted(files):
                        fp = Path(dirpath, fn)
                        if fp.is_symlink() or not fp.is_file():
                            skipped += 1
                            continue
                        out.append((fp, "/".join([base] + list(fp.relative_to(p).parts))))
                        if len(out) > MAX_FILES:
                            raise ValueError("Troppi file: massimo %d per invio" % MAX_FILES)
            elif p.is_file():
                out.append((p, p.name))
            else:
                skipped += 1
        except OSError:
            skipped += 1
    return out, skipped


class Sender:
    def __init__(self, engine):
        self.e = engine
        self.lock = threading.RLock()
        self.jobs = {}

    # ------------------------------------------------------------------ avvio
    def invia(self, dev, paths=None, text=None):
        """Restituisce (id, errore). Non legge i file: lo farà il thread, in streaming."""
        items = []
        if text is not None:
            data = text.encode("utf-8")
            items.append(OutFile(uuid.uuid4().hex[:8] + ".txt", size=len(data), text=text))
        try:
            found, skipped = collect(paths or [])
        except ValueError as e:
            return None, str(e)
        for p, name in found:
            try:
                st = p.stat()
            except OSError:
                continue
            items.append(OutFile(name, p, st.st_size, st.st_mtime_ns))
        if not items:
            return None, "Nessun file valido da inviare"
        job = Job(dev, items, text is not None and not found, lambda: self.e.bus.emit("invii"))
        with self.lock:
            self._prune()
            self.jobs[job.id] = job
        self.e.bus.emit("invii")
        threading.Thread(target=self._run, args=(job,), daemon=True).start()
        return job.id, ""

    # ------------------------------------------------------------------ azioni dell'utente
    def annulla(self, jid):
        job = self.jobs.get(jid)
        if not job or job.state in DONE:
            return False
        job.cancel.set()
        with job.pin_cv:
            job.pin_cv.notify_all()
        conn = job.conn
        if conn is not None and conn.sock is not None:
            try:
                conn.sock.shutdown(socket.SHUT_RDWR)   # sblocca subito recv/send in corso
            except OSError:
                pass
        return True

    def fornisci_pin(self, jid, pin):
        job = self.jobs.get(jid)
        if not job or job.state != "pin" or not isinstance(pin, str) or not pin or len(pin) > 64:
            return False
        with job.pin_cv:
            job.pin = pin
            job.pin_cv.notify_all()
        return True

    # ------------------------------------------------------------------ esecuzione
    def _run(self, job):
        try:
            self._execute(job)
        except JobEnd as end:
            self._finalize(job, end.state, end.error)
        except IdentityMismatch:
            self._finalize(job, "errore", "L'identità del dispositivo non corrisponde: aggiorna l'elenco e riprova")
        except (OSError, http.client.HTTPException, ssl.SSLError) as e:
            if job.cancel.is_set():
                self._finalize(job, "annullato")
            else:
                self._finalize(job, "errore", self._net_error(e))
        except Exception as e:  # noqa: BLE001
            log("invio", repr(e))
            self._finalize(job, "errore", "Errore imprevisto durante l'invio")

    @staticmethod
    def _net_error(e):
        if isinstance(e, (socket.timeout, TimeoutError)):
            return "Il dispositivo non risponde (timeout)"
        if isinstance(e, ConnectionRefusedError):
            return "Connessione rifiutata: apri LocalSend sul destinatario"
        if isinstance(e, (ConnectionError, http.client.HTTPException)):
            return "Connessione interrotta"
        if isinstance(e, ssl.SSLError):
            return "Errore di cifratura nella connessione"
        return "Non raggiungibile: %s" % (getattr(e, "strerror", None) or e.__class__.__name__)

    def _execute(self, job):
        files = {}
        for it in job.items:
            if it.text is None:
                try:
                    self._verify_source(it)
                    if it.size <= HASH_MAX:
                        it.sha256 = self._sha256(it, job)
                except SourceChanged as e:
                    raise JobEnd("errore", "«%s»: %s" % (it.name, e))
            meta = {"id": it.id, "fileName": it.name, "size": it.size,
                    "fileType": "text/plain" if it.text is not None else (mimetypes.guess_type(it.name)[0] or "application/octet-stream"),
                    "sha256": it.sha256, "preview": it.text}
            if it.path:
                meta["metadata"] = {"modified": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(it.mtime_ns / 1e9))}
            files[it.id] = meta
        self._check_cancel(job)
        body = {"info": self.e.info(), "files": files}
        status, resp = self._prepare(job, body)
        if status == 204:
            for it in job.items:
                it.state = "fatto"
            return self._finalize(job, "completato")
        tokens = resp.get("files") if isinstance(resp, dict) else None
        job.session = resp.get("sessionId", "") if isinstance(resp, dict) else ""
        if not job.session or not isinstance(tokens, dict):
            raise JobEnd("errore", "Risposta del destinatario non valida")
        for it in job.items:
            if it.id not in tokens:
                it.state = "rifiutato"
        job.state = "invio"
        self.e.bus.emit("invii")
        fatal = False
        for it in job.items:
            if it.state == "rifiutato":
                continue
            if job.cancel.is_set() or fatal:
                break
            it.state = "inviando"
            try:
                if it.text is not None:
                    self._upload_bytes(job, it, tokens[it.id])
                else:
                    self._upload_file(job, it, tokens[it.id])
                it.state = "fatto"
            except SourceChanged as e:
                it.state, it.error = "errore", str(e)
            except JobEnd as e:
                if e.state == "annullato":
                    it.state = "attesa"
                    break
                it.state, it.error = "errore", e.error
            except IdentityMismatch:
                raise
            except (OSError, http.client.HTTPException, ssl.SSLError) as e:
                if job.cancel.is_set():
                    break
                it.state, it.error = "errore", self._net_error(e)
                fatal = True        # il destinatario non risponde più: inutile insistere
            self.e.bus.emit("invii")
        if job.cancel.is_set():
            self._cancel_remote(job)
            return self._finalize(job, "annullato")
        self._finalize(job)

    def _finalize(self, job, state=None, error=""):
        ok = sum(1 for i in job.items if i.state == "fatto")
        refused = sum(1 for i in job.items if i.state == "rifiutato")
        bad = [i for i in job.items if i.state in ("errore", "inviando", "attesa")]
        if state is None:
            if bad and ok:
                state, error = "parziale", "Inviati %d file su %d" % (ok, len(job.items))
            elif bad:
                state, error = "errore", bad[0].error or "Invio non riuscito"
            elif refused and ok:
                state, error = "parziale", "Il destinatario ha accettato %d file su %d" % (ok, len(job.items))
            elif refused:
                state, error = "rifiutato", "Il destinatario non ha accettato nessun file"
            else:
                state = "completato"
        elif state in ("annullato", "errore") and ok:
            error = (error + " " if error else "") + "(%d file già inviati)" % ok
        job.state, job.error = state, error
        job.pin = None
        self.e.bus.emit("invii")
        self.e.notify("invio", id=job.id, alias=job.alias, stato=state, errore=error)

    # ------------------------------------------------------------------ prepare-upload con PIN
    def _prepare(self, job, body):
        job.state = "attesa"
        self.e.bus.emit("invii")
        pin = None
        for _ in range(5):
            self._check_cancel(job)
            path = API + "/prepare-upload" + ("?" + urllib.parse.urlencode({"pin": pin}) if pin else "")
            status, resp = self._request(job, "POST", path, body, PREPARE_TIMEOUT)
            if status in (200, 204):
                return status, resp
            if status == 401:
                job.pin_wrong = pin is not None
                job.state = "pin"
                self.e.bus.emit("invii")
                with job.pin_cv:
                    job.pin = None
                    deadline = time.monotonic() + PIN_WAIT
                    while job.pin is None and not job.cancel.is_set():
                        left = deadline - time.monotonic()
                        if left <= 0:
                            raise JobEnd("errore", "PIN non inserito in tempo")
                        job.pin_cv.wait(left)
                    pin, job.pin = job.pin, None
                self._check_cancel(job)
                job.state = "attesa"
                self.e.bus.emit("invii")
                continue
            raise JobEnd(*{
                403: ("rifiutato", "Il destinatario ha rifiutato"),
                409: ("errore", "Il destinatario sta già ricevendo altro: riprova tra poco"),
                429: ("errore", "Troppe richieste o tentativi di PIN: attendi qualche minuto"),
                507: ("errore", "Il destinatario non ha spazio sufficiente"),
            }.get(status, ("errore", "Errore del destinatario (%s)" % status)))
        raise JobEnd("errore", "PIN errato")

    def _request(self, job, method, path, body, timeout):
        import json
        conn, _actual = open_connection(job.ip, job.port, self.e.identity, timeout, job.fp)
        self.e.registry.mark_verified(job.fp)
        job.conn = conn
        try:
            data = json.dumps(body).encode() if body is not None else None
            conn.request(method, path, body=data, headers={"Content-Type": "application/json"} if data else {})
            r = conn.getresponse()
            raw = r.read(8 * 1024 * 1024)
            try:
                parsed = json.loads(raw) if raw else None
            except ValueError:
                parsed = None
            return r.status, parsed
        finally:
            job.conn = None
            conn.close()

    @staticmethod
    def _check_cancel(job):
        if job.cancel.is_set():
            raise JobEnd("annullato")

    def _cancel_remote(self, job):
        if not job.session:
            return
        try:
            self._request(job, "POST", API + "/cancel?" + urllib.parse.urlencode({"sessionId": job.session}), None, 5)
        except (OSError, http.client.HTTPException, ssl.SSLError):
            pass

    # ------------------------------------------------------------------ sorgente
    @staticmethod
    def _verify_source(it):
        try:
            st = os.stat(it.path)
        except FileNotFoundError:
            raise SourceChanged("Il file è stato rimosso")
        except PermissionError:
            raise JobEnd("errore", "Permesso negato su «%s»" % it.name)
        if st.st_size != it.size or st.st_mtime_ns != it.mtime_ns:
            raise SourceChanged("Il file è stato modificato dopo la selezione")

    def _sha256(self, it, job):
        h = hashlib.sha256()
        try:
            with open(it.path, "rb") as f:
                while True:
                    b = f.read(CHUNK)
                    if not b:
                        break
                    self._check_cancel(job)
                    h.update(b)
        except PermissionError:
            raise JobEnd("errore", "Permesso negato su «%s»" % it.name)
        except FileNotFoundError:
            raise JobEnd("errore", "Il file «%s» è stato rimosso" % it.name)
        return h.hexdigest()

    # ------------------------------------------------------------------ upload
    def _begin_upload(self, job, it, token, size):
        q = urllib.parse.urlencode({"sessionId": job.session, "fileId": it.id, "token": token})
        conn, _ = open_connection(job.ip, job.port, self.e.identity, 30, job.fp)
        job.conn = conn
        conn.putrequest("POST", API + "/upload?" + q)
        conn.putheader("Content-Type", "application/octet-stream")
        conn.putheader("Content-Length", str(size))
        conn.endheaders()
        return conn

    def _finish_upload(self, job, conn):
        try:
            r = conn.getresponse()
            r.read()
        finally:
            job.conn = None
            conn.close()
        if r.status != 200:
            raise JobEnd("errore", {
                403: "Token o indirizzo non accettati dal destinatario",
                409: "Il destinatario sta già ricevendo questo file",
                422: "Il destinatario ha rilevato un file danneggiato",
                507: "Il destinatario non ha spazio sufficiente",
            }.get(r.status, "Errore del destinatario (%s)" % r.status))

    def _upload_bytes(self, job, it, token):
        data = it.text.encode("utf-8")
        conn = self._begin_upload(job, it, token, len(data))
        try:
            conn.send(data)
            job.progress.add(len(data))
        except BaseException:
            job.conn = None
            conn.close()
            raise
        self._finish_upload(job, conn)

    def _upload_file(self, job, it, token):
        try:
            f = open(it.path, "rb")
        except FileNotFoundError:
            raise SourceChanged("Il file è stato rimosso")
        except PermissionError:
            raise JobEnd("errore", "Permesso negato")
        with f:
            st = os.fstat(f.fileno())
            if st.st_size != it.size or st.st_mtime_ns != it.mtime_ns:
                raise SourceChanged("Il file è stato modificato prima dell'invio")
            conn = self._begin_upload(job, it, token, it.size)
            changed_at_end = False
            try:
                left = it.size
                while left > 0:
                    if job.cancel.is_set():
                        raise JobEnd("annullato")
                    b = f.read(min(CHUNK, left))
                    if not b:
                        raise SourceChanged("Il file si è accorciato durante l'invio")
                    conn.send(b)
                    left -= len(b)
                    job.progress.add(len(b))
                    st2 = os.fstat(f.fileno())      # a ogni blocco: se cambia, si interrompe prima della fine
                    if st2.st_size != it.size or st2.st_mtime_ns != it.mtime_ns:
                        if left > 0:
                            raise SourceChanged("Il file è cambiato durante l'invio")
                        changed_at_end = True
            except BaseException:
                job.conn = None
                conn.close()        # il destinatario vede un upload interrotto e scarta il parziale
                raise
        self._finish_upload(job, conn)
        if changed_at_end:
            # tutti i byte erano già partiti: il destinatario ha potuto salvare una copia incoerente
            raise SourceChanged("Il file è cambiato durante l'ultimo blocco: il destinatario potrebbe aver salvato "
                                "una copia incoerente, controllala o inviala di nuovo")

    # ------------------------------------------------------------------ stato
    def _prune(self):
        now = time.time()
        done = sorted((j for j in self.jobs.values() if j.state in DONE), key=lambda j: j.created)
        for j in done:
            if now - j.created > 900 or len(done) > 30:
                self.jobs.pop(j.id, None)
                done.remove(j)

    def snapshot(self):
        with self.lock:
            jobs = sorted(self.jobs.values(), key=lambda j: -j.created)
            out = []
            for j in jobs:
                out.append({
                    "id": j.id, "verso": j.alias, "fp": j.fp, "stato": j.state, "errore": j.error,
                    "n": len(j.items), "totale": j.total(), "inviati": min(j.progress.total, j.total()),
                    "fatti": sum(1 for i in j.items if i.state == "fatto"),
                    "velocita": j.progress.speed() if j.state == "invio" else None,
                    "nomi": [i.name for i in j.items[:3]], "testo": j.is_text,
                    "pin_errato": j.pin_wrong, "creato": j.created,
                    "file": [{"nome": i.name, "stato": i.state, "errore": i.error} for i in j.items[:100]]})
            return out
