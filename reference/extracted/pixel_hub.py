#!/usr/bin/env python3
"""
Pixel Hub 2 - Pixel e Fedora insieme.

  - Schermo:   duplicazione e controllo Android con scrcpy (USB/ADB)
  - Condividi: LocalSend integrato (protocollo v2): ricevi e invia file e testi
               via Wi-Fi o hotspot, con richieste Accetta/Rifiuta
  - Pixel:    via cavo USB: batteria, spazio, importazione foto e video
  - Impostazioni, componenti, avvio automatico

Struttura: questo file fa da "motore" (server locale su 127.0.0.1 con token) e apre
una finestra WebKitGTK con l'interfaccia (index.html, accanto a questo file).

  python3 pixel_hub.py               apre l'app
  python3 pixel_hub.py --background  parte nascosta (per l'avvio automatico)
  python3 pixel_hub.py --browser     usa il browser al posto della finestra (debug)
"""
import collections
import copy
import ipaddress
import hashlib
import http.client
import http.server
import json
import mimetypes
import os
import pwd
import re
import secrets
import shlex
import shutil
import socket
import ssl
import struct
import subprocess
import sys
import threading
import time
import urllib.parse
import uuid
from pathlib import Path

VERSIONE = "3.1-pairing-fix"
APP_ID = "it.frank.PixelHub"
HOME = Path.home()
QUI = Path(__file__).resolve().parent
UI_FILE = QUI / "index.html"
CONF_FILE = HOME / ".config" / "pixel-hub-v3" / "config.json"
CACHE = HOME / ".cache" / "pixel-hub-v3"
AUTOSTART = HOME / ".config" / "autostart" / (APP_ID + ".desktop")
LS_PORT = int(os.environ.get("HUB_LS_PORT", "53317"))
MCAST = "224.0.0.167"

def cartella_xdg(kind, fallback):
    try:
        result = subprocess.run(["xdg-user-dir", kind], capture_output=True, text=True, timeout=5)
        value = result.stdout.strip()
        if result.returncode == 0 and value.startswith("/"):
            return Path(value)
    except (OSError, subprocess.TimeoutExpired):
        pass
    return HOME / fallback

DEFAULT = {
    "nome_pc": "Fedora Pixel Hub",
    "pairing": {},
    "cartella": str(cartella_xdg("DOWNLOAD", "Downloads") / "Da-Pixel"),
    "cartella_foto": str(cartella_xdg("PICTURES", "Pictures") / "Pixel"),
    "vetro": 45,              # 0 = trasparente, 100 = colorato
    "tema": "auto",           # auto | chiaro | scuro
    "auto_accetta": False,     # accetta senza chiedere dai dispositivi fidati
    "fidati": {},             # fingerprint -> nome
    "notifiche": True,
    "schermo_intero": False,
    "background": True,       # chiudendo la finestra resta pronta a ricevere
    "avvio_login": False,
}


def log(*a):
    if os.environ.get("HUB_DEBUG"):
        print("[hub]", *a, file=sys.stderr, flush=True)


def ha(cmd):
    return shutil.which(cmd) is not None


def esegui(argv, timeout=20):
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return p.returncode, (p.stdout + p.stderr).strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        return 1, str(e)


def in_thread(f, *a):
    t = threading.Thread(target=f, args=a, daemon=True)
    t.start()
    return t


def nome_utente():
    try:
        g = pwd.getpwuid(os.getuid()).pw_gecos.split(",")[0].strip()
        if g:
            return g.split()[0]
    except (KeyError, IndexError):
        pass
    return ""


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


def nome_sicuro(nome):
    """Toglie percorsi pericolosi (../) e caratteri strani da un nome ricevuto."""
    parti = [p for p in re.split(r"[\\/]+", nome or "") if p not in ("", ".", "..")]
    parti = [re.sub(r"[\x00-\x1f]", "", p)[:180] for p in parti]
    parti = [p for p in parti if p]
    return parti or ["file"]


def libera(dest):
    """Se dest esiste restituisce 'nome (1).ext', 'nome (2).ext'..."""
    if not dest.exists():
        return dest
    stem, suf = dest.stem, dest.suffix
    for i in range(1, 10000):
        c = dest.with_name(f"{stem} ({i}){suf}")
        if not c.exists():
            return c
    return dest.with_name(f"{stem}-{uuid.uuid4().hex[:6]}{suf}")


def ip_locali():
    rc, out = esegui(["ip", "-4", "-o", "addr", "show", "scope", "global"], 5)
    ips = []
    for l in out.splitlines() if rc == 0 else []:
        p = l.split()
        if len(p) > 3:
            ips.append((p[1], p[3].split("/")[0]))
    return ips


def gateway():
    rc, out = esegui(["ip", "-4", "route", "show", "default"], 5)
    m = re.search(r"via (\d+\.\d+\.\d+\.\d+)", out) if rc == 0 else None
    return m.group(1) if m else None


def nome_rete():
    rc, out = esegui(["nmcli", "-t", "-f", "NAME,TYPE", "connection", "show", "--active"], 5)
    for l in out.splitlines() if rc == 0 else []:
        n, _, t = l.rpartition(":")
        if "wireless" in t or "ethernet" in t:
            return n.replace("\\:", ":")
    return ""


def notifica(titolo, testo):
    if ha("notify-send"):
        subprocess.Popen(["notify-send", "-a", "Pixel Hub", "-i", APP_ID, titolo, testo],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


# =============================================================== configurazione
class Conf:
    def __init__(self):
        self.lock = threading.Lock()
        self.d = copy.deepcopy(DEFAULT)
        try:
            self.d.update(json.loads(CONF_FILE.read_text()))
        except (OSError, ValueError):
            pass

    def __getitem__(self, k):
        return self.d.get(k, DEFAULT.get(k))

    def salva(self):
        try:
            CONF_FILE.parent.mkdir(parents=True, exist_ok=True)
            tmp = CONF_FILE.with_suffix(".tmp")
            tmp.write_text(json.dumps(self.d, indent=2, ensure_ascii=False))
            os.chmod(tmp, 0o600)
            tmp.replace(CONF_FILE)
        except OSError:
            pass

    def aggiorna(self, nuovi):
        ok = {}
        for k, v in nuovi.items():
            if k == "nome_pc" and isinstance(v, str) and v.strip():
                ok[k] = v.strip()[:40]
            elif k in ("cartella", "cartella_foto") and isinstance(v, str) and v.startswith("/"):
                ok[k] = v
            elif k == "vetro" and isinstance(v, (int, float)):
                ok[k] = max(0, min(100, int(v)))
            elif k == "tema" and v in ("auto", "chiaro", "scuro"):
                ok[k] = v
            elif k in ("auto_accetta", "notifiche", "schermo_intero", "background",
                       "avvio_login") and isinstance(v, bool):
                ok[k] = v
        with self.lock:
            self.d.update(ok)
            self.salva()
        return ok


# =============================================================== LocalSend
def contesto_client():
    c = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    c.check_hostname = False
    c.verify_mode = ssl.CERT_NONE  # LocalSend usa certificati auto-firmati
    # LocalSend in modalita https riconosce chi si collega dal suo certificato:
    # presentiamo il nostro (lo stesso del server), come fa l'app ufficiale.
    cert, key = CACHE / "cert.pem", CACHE / "key.pem"
    if cert.exists() and key.exists():
        try:
            c.load_cert_chain(str(cert), str(key))
        except (OSError, ssl.SSLError):
            pass
    return c


def collega_verificato(conn, dev):
    if dev.get("protocol", "https") != "https":
        raise ssl.SSLError("È richiesta la cifratura HTTPS di LocalSend")
    conn.connect()
    actual = hashlib.sha256(conn.sock.getpeercert(binary_form=True)).hexdigest().upper()
    expected = dev.get("fp") or dev.get("fingerprint")
    if expected and not secrets.compare_digest(actual, expected.upper()):
        conn.close()
        raise ssl.SSLError("Il certificato del dispositivo è cambiato: rifai il pairing")
    return actual


def richiesta_ls(dev, metodo, percorso, corpo=None, timeout=10):
    """Richiesta verso un altro dispositivo LocalSend. Restituisce (stato, json|None)."""
    if dev.get("protocol", "https") == "https":
        conn = http.client.HTTPSConnection(dev["ip"], dev.get("port", 53317),
                                           timeout=timeout, context=contesto_client())
    else:
        conn = http.client.HTTPConnection(dev["ip"], dev.get("port", 53317), timeout=timeout)
    try:
        actual = collega_verificato(conn, dev)
        dati = json.dumps(corpo).encode() if corpo is not None else None
        conn.request(metodo, percorso, body=dati,
                     headers={"Content-Type": "application/json"} if dati else {})
        r = conn.getresponse()
        raw = r.read()
        try:
            parsed = json.loads(raw) if raw else None
            if isinstance(parsed, dict) and percorso.endswith(("/register", "/info")):
                parsed["fingerprint"] = actual
            return r.status, parsed
        except ValueError:
            return r.status, None
    finally:
        conn.close()


class Certificato:
    def __init__(self):
        self.cert = CACHE / "cert.pem"
        self.key = CACHE / "key.pem"
        self.protocollo = "http"
        self.fingerprint = secrets.token_hex(32).upper()
        CACHE.mkdir(parents=True, exist_ok=True)
        if not (self.cert.exists() and self.key.exists()) and ha("openssl"):
            esegui(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                    "-keyout", str(self.key), "-out", str(self.cert), "-days", "3650",
                    "-subj", "/CN=Pixel Hub"], 60)
            try:
                os.chmod(self.key, 0o600)
            except OSError:
                pass
        if self.cert.exists() and self.key.exists():
            try:
                der = ssl.PEM_cert_to_DER_cert(self.cert.read_text())
                self.fingerprint = hashlib.sha256(der).hexdigest().upper()
                self.protocollo = "https"
            except (OSError, ValueError):
                pass


class Nodo:
    """Implementazione di LocalSend v2 (ricevere, inviare, scoperta)."""

    def __init__(self, hub):
        self.hub = hub
        self.cert = Certificato()
        self.lock = threading.RLock()
        self.dispositivi = {}     # fingerprint -> info
        self.richieste = {}       # id -> richiesta in arrivo
        self.sessioni = {}        # sessionId -> sessione di ricezione
        self.invii = {}           # id -> invio in uscita
        self.messaggi = collections.deque(maxlen=30)
        self.errore = None
        self.attivo = False
        self.server = None

    # ---------------------------------------------------------------- info
    def info(self, con_porta=True):
        d = {"alias": self.hub.conf["nome_pc"], "version": "2.0", "deviceModel": os.environ.get("HUB_MODELLO", "Linux"),
             "deviceType": os.environ.get("HUB_TIPO", "desktop"), "fingerprint": self.cert.fingerprint, "download": False}
        if con_porta:
            d.update(port=LS_PORT, protocol=self.cert.protocollo)
        return d

    def aggiungi_dispositivo(self, info, ip, porta=None, protocollo=None):
        fp = str(info.get("fingerprint") or ip).upper()
        if fp == self.cert.fingerprint:
            return
        paired = self.hub.conf["pairing"] or {}
        if paired.get("fp") and fp != paired["fp"]:
            return
        if info.get("protocol", protocollo or "https") != "https":
            return
        with self.lock:
            vecchio = self.dispositivi.get(fp, {})
            self.dispositivi[fp] = {
                "fp": fp, "alias": str(info.get("alias") or "Dispositivo")[:60],
                "modello": info.get("deviceModel") or "", "tipo": info.get("deviceType") or "mobile",
                "ip": ip, "port": int(porta or info.get("port") or vecchio.get("port") or 53317),
                "protocol": protocollo or info.get("protocol") or vecchio.get("protocol") or "https",
                "visto": time.time()}

    # ---------------------------------------------------------------- avvio
    def avvia(self):
        if self.cert.protocollo != "https":
            self.errore = "Certificato HTTPS non disponibile: ricezione bloccata"
            return False
        try:
            self.server = ServerLS(("0.0.0.0", LS_PORT), GestoreLS, self)
        except OSError as e:
            self.errore = ("porta" if e.errno in (98, 13) else str(e))
            self.attivo = False
            log("LocalSend non avviato:", e)
            return False
        self.errore = None
        self.attivo = True
        in_thread(self.server.serve_forever)
        in_thread(self.ascolta_multicast)
        in_thread(self.annunci_periodici)
        return True

    def riprova(self):
        if self.attivo:
            return True
        return self.avvia()

    # ---------------------------------------------------------------- scoperta
    def _socket_mc(self):
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        if hasattr(socket, "SO_REUSEPORT"):
            try:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
            except OSError:
                pass
        return s

    def ascolta_multicast(self):
        s = self._socket_mc()
        try:
            s.bind(("", LS_PORT))
        except OSError as e:
            log("multicast bind:", e)
            return
        gruppi = set()

        def unisci():
            for _if, ip in ip_locali() + [("any", "0.0.0.0")]:
                if ip in gruppi:
                    continue
                try:
                    s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                                 socket.inet_aton(MCAST) + socket.inet_aton(ip))
                    gruppi.add(ip)
                except OSError:
                    pass
        unisci()
        s.settimeout(10)
        while True:
            try:
                dati, (ip, _p) = s.recvfrom(65535)
            except socket.timeout:
                unisci()  # la rete puo cambiare (hotspot acceso dopo)
                continue
            except OSError:
                time.sleep(2)
                continue
            try:
                info = json.loads(dati)
            except ValueError:
                continue
            if not isinstance(info, dict) or info.get("fingerprint") == self.cert.fingerprint:
                continue
            self.aggiungi_dispositivo(info, ip)
            if info.get("announce"):
                in_thread(self.rispondi_annuncio, info, ip)

    def rispondi_annuncio(self, info, ip):
        dev = {"ip": ip, "port": info.get("port", 53317), "protocol": info.get("protocol", "https")}
        try:
            st, r = richiesta_ls(dev, "POST", "/api/localsend/v2/register", self.info(), 4)
            if st == 200 and isinstance(r, dict):
                self.aggiungi_dispositivo(r, ip, dev["port"], dev["protocol"])
                return
        except (OSError, http.client.HTTPException, ssl.SSLError):
            pass
        self.annuncia(False)

    def annuncia(self, annuncio=True):
        msg = dict(self.info(), announce=annuncio)
        dati = json.dumps(msg).encode()
        s = self._socket_mc()
        s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)
        inviati = 0
        for _if, ip in ip_locali():
            try:
                s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(ip))
                s.sendto(dati, (MCAST, LS_PORT))
                inviati += 1
            except OSError:
                pass
        if not inviati:
            try:
                s.sendto(dati, (MCAST, LS_PORT))
            except OSError:
                pass
        s.close()

    def cerca(self, ip_manuale=None):
        """Annuncio multicast + prova diretta (utile con l'hotspot del Pixel)."""
        self.annuncia(True)
        bersagli = [ip_manuale] if ip_manuale else []
        gw = gateway()
        if gw and not ip_manuale:
            bersagli.append(gw)
        trovati = 0
        for ip in bersagli:
            for proto in ("https", "http"):
                dev = {"ip": ip, "port": 53317, "protocol": proto}
                try:
                    st, r = richiesta_ls(dev, "POST", "/api/localsend/v2/register", self.info(), 3)
                except (OSError, http.client.HTTPException, ssl.SSLError):
                    continue
                if st == 200 and isinstance(r, dict):
                    self.aggiungi_dispositivo(r, ip, 53317, proto)
                    trovati += 1
                    break
        return trovati

    def annunci_periodici(self):
        n = 0
        while True:
            attivo = time.time() - self.hub.ultimo_poll < 30
            if attivo or n % 6 == 0:
                self.annuncia(True)
            n += 1
            time.sleep(20)

    # ---------------------------------------------------------------- ricezione
    def nuova_richiesta(self, info, ip, files):
        fp = str(info.get("fingerprint") or ip).upper()
        alias = str(info.get("alias") or "Un dispositivo")[:60]
        self.aggiungi_dispositivo(info, ip)
        totale = sum(int(f.get("size") or 0) for f in files.values())
        r = {"id": uuid.uuid4().hex[:12], "fp": fp, "alias": alias, "ip": ip,
             "file": [{"nome": f.get("fileName", "file"), "size": int(f.get("size") or 0),
                       "tipo": tipo_file(f.get("fileName", ""))} for f in files.values()],
             "totale": totale, "stato": "attesa", "ricevuti": 0, "byte": 0,
             "creata": time.time(), "evento": threading.Event(), "fidato": False}
        with self.lock:
            self.richieste[r["id"]] = r
        conf = self.hub.conf
        if conf["auto_accetta"] and fp in conf["fidati"]:
            r["stato"] = "accettata"
            r["evento"].set()
        else:
            if conf["notifiche"]:
                n = len(files)
                notifica(alias, "vuole inviarti %s (%s)" % (
                    "un file" if n == 1 else "%d file" % n, dimensione(totale)))
            self.hub.chiama_finestra()
        return r

    def decidi(self, rid, accetta, fidati=False):
        with self.lock:
            r = self.richieste.get(rid)
            if not r or r["stato"] != "attesa":
                return False
            r["stato"] = "accettata" if accetta else "rifiutata"
            if accetta and fidati:
                with self.hub.conf.lock:
                    self.hub.conf.d.setdefault("fidati", {})[r["fp"]] = r["alias"]
                    self.hub.conf.salva()
            r["evento"].set()
        return True

    def messaggio(self, alias, testo, fp):
        self.messaggi.appendleft({"id": uuid.uuid4().hex[:10], "alias": alias, "testo": testo,
                                  "quando": time.time(), "fp": fp})
        if self.hub.conf["notifiche"]:
            notifica("Messaggio da " + alias, testo[:120])
        self.hub.chiama_finestra()

    # ---------------------------------------------------------------- invio
    def invia(self, fp, percorsi=None, testo=None):
        with self.lock:
            dev = dict(self.dispositivi.get(fp) or {})
        if not dev or not self.hub.pronto() or fp != self.hub.conf["pairing"].get("fp"):
            return None
        files, mappa = {}, {}
        if testo is not None:
            fid = uuid.uuid4().hex
            dati = testo.encode()
            files[fid] = {"id": fid, "fileName": fid[:8] + ".txt", "size": len(dati),
                          "fileType": "text/plain", "sha256": None, "preview": testo}
            mappa[fid] = ("testo", dati)
        for p in percorsi or []:
            p = Path(p)
            if not p.is_file():
                continue
            fid = uuid.uuid4().hex
            files[fid] = {"id": fid, "fileName": p.name, "size": p.stat().st_size,
                          "fileType": mimetypes.guess_type(p.name)[0] or "application/octet-stream",
                          "sha256": None, "preview": None,
                          "metadata": {"modified": time.strftime(
                              "%Y-%m-%dT%H:%M:%S", time.localtime(p.stat().st_mtime)),
                              "accessed": None}}
            mappa[fid] = ("file", p)
        if not files:
            return None
        job = {"id": uuid.uuid4().hex[:12], "verso": dev["alias"], "fp": fp,
               "n": len(files), "totale": sum(f["size"] for f in files.values()),
               "inviati": 0, "fatti": 0, "stato": "attesa", "errore": "",
               "testo": testo is not None and not percorsi, "annulla": False,
               "nomi": [f["fileName"] for f in files.values()], "creato": time.time()}
        with self.lock:
            self.invii[job["id"]] = job
        in_thread(self._esegui_invio, job, dev, files, mappa)
        return job["id"]

    def _esegui_invio(self, job, dev, files, mappa):
        corpo = {"info": self.info(), "files": files}
        try:
            st, r = richiesta_ls(dev, "POST", "/api/localsend/v2/prepare-upload", corpo, 120)
        except (OSError, http.client.HTTPException, ssl.SSLError) as e:
            dettaglio = getattr(e, "reason", None) or getattr(e, "strerror", None) or e.__class__.__name__
            log("invio fallito", dev, repr(e))
            job.update(stato="errore", errore="Non raggiungibile: tieni LocalSend aperto sul Pixel (%s)"
                       % str(dettaglio)[:60])
            return
        if job["annulla"]:
            job["stato"] = "annullato"
            return
        if st == 204:
            job.update(stato="fatto", inviati=job["totale"], fatti=job["n"])
            self._pulisci(mappa)
            return
        if st != 200 or not isinstance(r, dict):
            job.update(stato="errore", errore={
                403: "Rifiutato sul Pixel", 401: "Serve il PIN di LocalSend",
                409: "Il Pixel sta già ricevendo altro", 429: "Troppe richieste, riprova"
            }.get(st, "Errore %s" % st))
            return
        sid, tokens = r.get("sessionId"), r.get("files") or {}
        job["stato"] = "invio"
        for fid, token in tokens.items():
            if job["annulla"]:
                break
            tipo, sorgente = mappa.get(fid, (None, None))
            if tipo is None:
                continue
            q = urllib.parse.urlencode({"sessionId": sid, "fileId": fid, "token": token})
            try:
                self._carica(dev, "/api/localsend/v2/upload?" + q, tipo, sorgente, job)
                job["fatti"] += 1
            except Exception as e:  # noqa: BLE001
                job.update(stato="errore", errore="Invio interrotto (%s)" % e.__class__.__name__)
                return
        if job["annulla"]:
            try:
                richiesta_ls(dev, "POST", "/api/localsend/v2/cancel?" +
                             urllib.parse.urlencode({"sessionId": sid}), None, 5)
            except (OSError, http.client.HTTPException, ssl.SSLError):
                pass
            job["stato"] = "annullato"
            return
        job["stato"] = "fatto"
        self._pulisci(mappa)

    @staticmethod
    def _pulisci(mappa):
        for tipo, s in mappa.values():
            if tipo == "file" and str(s).startswith(str(CACHE / "uscita")):
                try:
                    s.unlink()
                    s.parent.rmdir()
                except OSError:
                    pass

    @staticmethod
    def _carica(dev, percorso, tipo, sorgente, job):
        if dev.get("protocol", "https") == "https":
            conn = http.client.HTTPSConnection(dev["ip"], dev["port"], timeout=60,
                                               context=contesto_client())
        else:
            conn = http.client.HTTPConnection(dev["ip"], dev["port"], timeout=60)
        try:
            collega_verificato(conn, dev)
            size = len(sorgente) if tipo == "testo" else sorgente.stat().st_size
            conn.putrequest("POST", percorso)
            conn.putheader("Content-Type", "application/octet-stream")
            conn.putheader("Content-Length", str(size))
            conn.endheaders()
            if tipo == "testo":
                conn.send(sorgente)
                job["inviati"] += size
            else:
                with open(sorgente, "rb") as f:
                    while True:
                        if job["annulla"]:
                            raise InterruptedError
                        b = f.read(256 * 1024)
                        if not b:
                            break
                        conn.send(b)
                        job["inviati"] += len(b)
            r = conn.getresponse()
            r.read()
            if r.status != 200:
                raise IOError("stato %s" % r.status)
        finally:
            conn.close()

    # ---------------------------------------------------------------- stato
    def stato(self):
        ora = time.time()
        with self.lock:
            for k in [k for k, d in self.dispositivi.items() if ora - d["visto"] > 900]:
                del self.dispositivi[k]
            for k in [k for k, r in self.richieste.items()
                      if r["stato"] in ("completata", "rifiutata", "scaduta", "annullata", "errore")
                      and ora - r.get("finita", r["creata"]) > 20]:
                del self.richieste[k]
            for k in [k for k, j in self.invii.items()
                      if j["stato"] in ("fatto", "errore", "annullato") and ora - j["creato"] > 600]:
                del self.invii[k]
            disp = sorted(self.dispositivi.values(), key=lambda d: -d["visto"])
            return {
                "attivo": self.attivo, "errore": self.errore, "protocollo": self.cert.protocollo,
                "dispositivi": [dict(d, vicino=ora - d["visto"] < 120,
                                     fidato=d["fp"] in self.hub.conf["fidati"]) for d in disp],
                "richieste": [{k: v for k, v in r.items() if k != "evento"}
                              for r in self.richieste.values()],
                "invii": sorted(({k: v for k, v in j.items() if k != "annulla"}
                                 for j in self.invii.values()), key=lambda j: -j["creato"]),
                "messaggi": list(self.messaggi),
            }


class ServerLS(http.server.ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, indirizzo, gestore, nodo):
        self.nodo = nodo
        self.ctx = None
        if nodo.cert.protocollo == "https":
            self.ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            self.ctx.load_cert_chain(str(nodo.cert.cert), str(nodo.cert.key))
        super().__init__(indirizzo, gestore)

    def get_request(self):
        sock, addr = self.socket.accept()
        if self.ctx:
            sock = self.ctx.wrap_socket(sock, server_side=True, do_handshake_on_connect=False)
        return sock, addr

    def handle_error(self, request, client_address):
        log("errore connessione da", client_address, sys.exc_info()[1])


class GestoreLS(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "PixelHub"

    def log_message(self, *a):
        log("LS", self.client_address[0], self.command, urllib.parse.urlsplit(self.path).path)

    def setup(self):
        super().setup()
        if isinstance(self.connection, ssl.SSLSocket):
            self.connection.settimeout(15)
            self.connection.do_handshake()
        self.connection.settimeout(300)

    def risposta(self, codice, obj=None):
        dati = b"" if obj is None else json.dumps(obj).encode()
        self.send_response(codice)
        if codice >= 400:
            self.close_connection = True
            self.send_header("Connection", "close")
        if obj is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(dati)))
        self.end_headers()
        if dati:
            self.wfile.write(dati)

    def corpo_json(self, limite=4_000_000):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0 or n > limite:
            return None
        try:
            return json.loads(self.rfile.read(n))
        except ValueError:
            return None

    def leggi_corpo(self, scrivi):
        if "chunked" in (self.headers.get("Transfer-Encoding") or "").lower():
            while True:
                riga = self.rfile.readline(65537)
                if not riga:
                    raise IOError("connessione chiusa")
                size = int(riga.split(b";")[0].strip() or b"0", 16)
                if size == 0:
                    while self.rfile.readline(65537) not in (b"\r\n", b"\n", b""):
                        pass
                    return
                resto = size
                while resto:
                    c = self.rfile.read(min(resto, 1 << 20))
                    if not c:
                        raise IOError("connessione chiusa")
                    scrivi(c)
                    resto -= len(c)
                self.rfile.readline()
        else:
            resto = int(self.headers.get("Content-Length") or 0)
            while resto > 0:
                c = self.rfile.read(min(resto, 1 << 20))
                if not c:
                    raise IOError("connessione chiusa")
                scrivi(c)
                resto -= len(c)

    @property
    def nodo(self):
        return self.server.nodo

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        if u.path in ("/api/localsend/v2/info", "/api/localsend/v1/info"):
            return self.risposta(200, self.nodo.info(False))
        self.risposta(404)

    def do_POST(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        ip = self.client_address[0]
        if u.path == "/api/localsend/v2/register":
            info = self.corpo_json() or {}
            if isinstance(info, dict) and info.get("fingerprint") != self.nodo.cert.fingerprint:
                self.nodo.aggiungi_dispositivo(info, ip)
            return self.risposta(200, self.nodo.info(False))
        if u.path == "/api/localsend/v2/prepare-upload":
            if not self.nodo.hub.pronto():
                return self.risposta(403)
            if not self.nodo.hub.verifica_pin(q.get("pin", ""), ip):
                return self.risposta(401)
            return self.prepara()
        if u.path == "/api/localsend/v2/upload":
            return self.carica(q, ip)
        if u.path == "/api/localsend/v2/cancel":
            with self.nodo.lock:
                sid = q.get("sessionId", "")
                s = self.nodo.sessioni.get(sid)
                if not s or s["ip"] != ip:
                    return self.risposta(403)
                self.nodo.sessioni.pop(sid, None)
                if s:
                    r = self.nodo.richieste.get(s["richiesta"])
                    if r and r["stato"] != "completata":
                        r.update(stato="annullata", finita=time.time())
            return self.risposta(200)
        self.risposta(404)

    def prepara(self):
        corpo = self.corpo_json()
        if not isinstance(corpo, dict) or not isinstance(corpo.get("files"), dict):
            return self.risposta(400)
        info = corpo.get("info") or {}
        if not isinstance(info, dict) or info.get("fingerprint", "").upper() != self.nodo.hub.conf["pairing"].get("fp"):
            return self.risposta(403)
        if not isinstance(self.connection, ssl.SSLSocket):
            return self.risposta(403)
        files = {k: v for k, v in corpo["files"].items() if isinstance(v, dict)}
        if not files:
            return self.risposta(400)
        ip = self.client_address[0]
        alias = str(info.get("alias") or "Un dispositivo")[:60]
        # solo testo con anteprima = messaggio: si mostra e basta (come fa LocalSend)
        if all(isinstance(f.get("preview"), str) and str(f.get("fileType", "")).startswith("text")
               for f in files.values()):
            self.nodo.aggiungi_dispositivo(info, ip)
            for f in files.values():
                self.nodo.messaggio(alias, f["preview"], info.get("fingerprint") or ip)
            return self.risposta(204)
        nodo = self.nodo
        cartella = Path(nodo.hub.conf["cartella"])
        try:
            cartella.mkdir(parents=True, exist_ok=True)
            spazio = shutil.disk_usage(cartella).free
        except OSError:
            return self.risposta(500)
        totale = sum(int(f.get("size") or 0) for f in files.values())
        if totale + 200 * 1024 * 1024 > spazio:
            notifica("Spazio insufficiente", "%s voleva inviarti %s" % (alias, dimensione(totale)))
            return self.risposta(403)
        r = nodo.nuova_richiesta(info, ip, files)
        if not r["evento"].wait(90):
            with nodo.lock:
                if r["stato"] == "attesa":
                    r.update(stato="scaduta", finita=time.time())
            return self.risposta(403)
        if r["stato"] != "accettata":
            r.setdefault("finita", time.time())
            return self.risposta(403)
        sid = uuid.uuid4().hex
        tokens = {fid: secrets.token_hex(16) for fid in files}
        with nodo.lock:
            nodo.sessioni[sid] = {
                "richiesta": r["id"], "ip": ip, "cartella": cartella,
                "file": {fid: {"token": tokens[fid], "nome": files[fid].get("fileName", "file"),
                               "size": int(files[fid].get("size") or 0), "fatto": False}
                         for fid in files}}
        r["stato"] = "ricezione"
        self.risposta(200, {"sessionId": sid, "files": tokens})

    def carica(self, q, ip):
        nodo = self.nodo
        with nodo.lock:
            s = nodo.sessioni.get(q.get("sessionId", ""))
            f = s["file"].get(q.get("fileId", "")) if s else None
        if not s or not f:
            return self.risposta(403 if s else 409)
        if f["token"] != q.get("token") or s["ip"] != ip:
            return self.risposta(403)
        r = nodo.richieste.get(s["richiesta"])
        parti = nome_sicuro(f["nome"])
        root = s["cartella"].resolve()
        dest = libera(root.joinpath(*parti)).resolve()
        if not dest.is_relative_to(root):
            return self.risposta(403)
        parz = dest.with_name(dest.name + ".part")
        try:
            dest.parent.mkdir(parents=True, exist_ok=True)
            with open(parz, "wb") as out:
                def scrivi(c):
                    out.write(c)
                    if r:
                        r["byte"] += len(c)
                self.leggi_corpo(scrivi)
            parz.replace(dest)
        except (OSError, IOError) as e:
            log("upload fallito", e)
            try:
                parz.unlink()
            except OSError:
                pass
            return self.risposta(500)
        with nodo.lock:
            f["fatto"] = True
            f["percorso"] = str(dest)
            if r:
                r["ricevuti"] += 1
            if all(x["fatto"] for x in s["file"].values()):
                nodo.sessioni.pop(q.get("sessionId", ""), None)
                if r:
                    r.update(stato="completata", finita=time.time())
                    if nodo.hub.conf["notifiche"]:
                        notifica("Ricevuto da " + r["alias"],
                                 "%d file in %s" % (r["ricevuti"], s["cartella"].name))
        self.risposta(200)


def dimensione(n):
    n = float(n)
    for u in ("B", "KB", "MB", "GB"):
        if n < 1024:
            return ("%.0f %s" if u == "B" else "%.1f %s") % (n, u)
        n /= 1024
    return "%.1f TB" % n


# =============================================================== schermo (scrcpy)
class Schermo:
    def __init__(self, hub):
        self.hub = hub
        self.proc = None
        self.connesso = False
        self.log = collections.deque(maxlen=120)
        self.dal = 0

    def avvia(self):
        if self.proc and self.proc.poll() is None:
            return True
        if not ha("scrcpy"):
            return False
        self.hub.cavo._controlla()
        if self.hub.cavo.stato_["stato"] != "collegato":
            self.log.append(self.hub.cavo.stato_.get("errore", "Collega un solo Pixel e autorizza il debug USB."))
            return False
        argv = ["scrcpy", "--serial", self.hub.cavo.udid, "--max-size=1920", "--max-fps=60"]
        if self.hub.conf["schermo_intero"]:
            argv.append("--fullscreen")
        try:
            self.proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                         text=True, bufsize=1, errors="replace")
        except OSError as e:
            self.log.append(str(e))
            return False
        self.connesso = False
        self.dal = time.time()
        self.log.append("$ " + " ".join(argv))
        in_thread(self._leggi, self.proc)
        return True

    def _leggi(self, p):
        for riga in p.stdout:
            riga = riga.rstrip()
            self.log.append(riga)
            low = riga.lower()
            if any(k in low for k in ("texture:", "renderer:")):
                self.connesso = True
            elif any(k in low for k in ("connection closed", "teardown", "client disconnected")):
                self.connesso = False
        if p is self.proc:
            self.connesso = False
            self.log.append("— ricevitore fermato —")

    def ferma(self):
        p = self.proc
        if p and p.poll() is None:
            p.terminate()
            try:
                p.wait(3)
            except subprocess.TimeoutExpired:
                p.kill()
        self.connesso = False

    def stato(self):
        attivo = bool(self.proc and self.proc.poll() is None)
        return {"installato": ha("scrcpy"), "attivo": attivo, "connesso": attivo and self.connesso,
                "dal": self.dal if attivo else 0, "log": list(self.log)[-40:]}


# =============================================================== cavo USB
class Cavo:
    def __init__(self, hub):
        self.hub = hub
        self.stato_ = {"stato": "assente", "info": {}}
        self.imp = {"attivo": False, "fatti": 0, "tot": 0, "msg": ""}
        self.udid = None
        in_thread(self._ciclo)

    def _ciclo(self):
        while True:
            if time.time() - self.hub.ultimo_poll < 15 and not self.imp["attivo"]:
                try:
                    self._controlla()
                except Exception as e:  # noqa: BLE001
                    log("cavo", e)
            time.sleep(3)

    def _controlla(self):
        self.udid = None
        identity = (self.hub.conf["pairing"] or {}).get("adb_serial")
        if not self.hub.pronto() or not identity:
            self.stato_ = {"stato": "errore", "info": {}, "errore": "ADB non associato: usa Configura Pixel nelle Impostazioni."}
            return
        if not ha("adb"):
            self.stato_ = {"stato": "mancante", "info": {}}
            return
        rc, out = esegui(["adb", "devices"], 10)
        devices = [line.split() for line in out.splitlines()
                   if len(line.split()) >= 2 and not line.startswith(("List ", "*", "adb:"))]
        devices = [d for d in devices if d[1] in ("device", "unauthorized", "offline", "no")]
        if rc or not devices:
            self.stato_ = {"stato": "assente", "info": {}, "errore": out if rc else "Nessun dispositivo ADB. Abilita Debug USB e usa un cavo dati."}
            return
        if len(devices) != 1:
            self.stato_ = {"stato": "errore", "info": {}, "errore": "Sono collegati più dispositivi ADB: lascia collegato solo il Pixel (anche una connessione Wi-Fi conta)."}
            return
        serial, status = devices[0][:2]
        self.udid = serial
        if status != "device":
            self.stato_ = {"stato": "fiducia" if status == "unauthorized" else "errore", "info": {},
                           "errore": "Autorizza Debug USB sul Pixel." if status == "unauthorized" else "ADB: " + " ".join(devices[0][1:]) + ". Ricollega il cavo e controlla i permessi USB."}
            return
        def shell(*args):
            code, value = esegui(["adb", "-s", serial, "shell", *args], 10)
            return value if code == 0 else ""
        if shell("getprop", "ro.serialno") != identity:
            self.udid = None
            self.stato_ = {"stato": "errore", "info": {}, "errore": "Questo dispositivo ADB non è il Pixel associato."}
            return
        model = shell("getprop", "ro.product.model")
        info = {"DeviceName": model or "Pixel", "ProductType": model,
                "ProductVersion": shell("getprop", "ro.build.version.release")}
        battery = dict(re.findall(r"^\s*([\w ]+):\s*(.+)$", shell("dumpsys", "battery"), re.M))
        info["BatteryCurrentCapacity"] = battery.get("level", "")
        info["BatteryIsCharging"] = str(battery.get("status") in ("2", "5")).lower()
        lines = shell("df", "-k", "/sdcard").splitlines()
        if len(lines) > 1:
            fields = lines[-1].split()
            try:
                info["TotalDiskCapacity"] = str(int(fields[1]) * 1024)
                info["AmountDataAvailable"] = str(int(fields[3]) * 1024)
            except (ValueError, IndexError):
                pass
        self.stato_ = {"stato": "collegato", "info": info}

    def associa(self):
        self._controlla()
        return self.stato_["stato"] == "collegato"

    def apri(self):
        # MTP is independent of ADB; the user selects File transfer on the phone.
        subprocess.Popen(["gio", "open", "computer:///"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True

    def importa(self):
        if self.imp["attivo"] or self.stato_["stato"] != "collegato" or not self.udid:
            return False
        self.imp = {"attivo": True, "fatti": 0, "tot": 0, "msg": "Leggo DCIM sul Pixel…"}
        in_thread(self._importa, self.udid)
        return True

    def _importa(self, serial):
        dest = Path(self.hub.conf["cartella_foto"]).resolve()
        msg = ""
        try:
            rc, listing = esegui(["adb", "-s", serial, "shell", "find /sdcard/DCIM -type f -print0"], 60)
            if rc:
                raise OSError(listing)
            paths = [s for s in listing.split("\0") if s.startswith("/sdcard/DCIM/")]
            self.imp.update(tot=len(paths), msg="Controllo e copio foto e video…")
            copied = skipped = 0
            for remote in paths:
                relative = Path(remote).relative_to("/sdcard/DCIM")
                q = (dest / relative).resolve()
                if not q.is_relative_to(dest):
                    raise OSError("Percorso DCIM non valido")
                rc, size = esegui(["adb", "-s", serial, "shell", "stat -c %s " + shlex.quote(remote)], 15)
                if rc or not size.isdigit():
                    raise OSError("Impossibile leggere " + remote)
                size = int(size)
                if q.exists() and q.stat().st_size == size:
                    skipped += 1
                else:
                    q.parent.mkdir(parents=True, exist_ok=True)
                    q = libera(q)
                    tmp = q.with_name("." + q.name + "." + uuid.uuid4().hex[:8] + ".part")
                    try:
                        rc, out = esegui(["adb", "-s", serial, "pull", remote, str(tmp)], 3600)
                        if rc or not tmp.exists() or tmp.stat().st_size != size:
                            raise OSError(out or "Copia incompleta")
                        tmp.replace(q)
                    finally:
                        tmp.unlink(missing_ok=True)
                    copied += 1
                self.imp["fatti"] += 1
            msg = f"Importati {copied}; già presenti {skipped}." if paths else "DCIM vuota: sono importabili solo i file locali, non quelli presenti solo su Google Foto."
        except Exception as e:
            msg = "Importazione interrotta: " + str(e)[:300]
        finally:
            self.imp.update(attivo=False, msg=msg)

    def stato(self):
        return dict(self.stato_, importa=dict(self.imp), destinazione=self.hub.conf["cartella_foto"])


# =============================================================== componenti
COMPONENTI = [
    ("scrcpy", "scrcpy", "Schermo e controllo del Pixel; installazione opzionale dal terminale", lambda: ha("scrcpy"), []),
    ("usb", "ADB", "Debug USB, informazioni e importazione DCIM", lambda: ha("adb"), ["android-tools"]),
    ("openssl", "OpenSSL", "Connessione cifrata LocalSend", lambda: ha("openssl"), ["openssl"]),
]



# =============================================================== hub
class Hub:
    def __init__(self):
        self.conf = Conf()
        self.pair_lock = threading.RLock()
        self.pending = None
        self.pin_attempts = collections.deque(maxlen=100)
        self.token = secrets.token_urlsafe(24)
        self.ultimo_poll = 0
        self.finestra_cb = None       # impostata dalla finestra GTK
        self.scegli_cartella_cb = None
        self.nodo = Nodo(self)
        self.schermo = Schermo(self)
        self.cavo = Cavo(self)
        self._comp = (0, [])
        self.installazione = {"attiva": False, "msg": ""}
        for k in ("cartella", "cartella_foto"):
            try:
                Path(self.conf[k]).mkdir(parents=True, exist_ok=True)
            except OSError:
                pass
        (CACHE / "uscita").mkdir(parents=True, exist_ok=True)

    def pronto(self):
        p = self.conf["pairing"] or {}
        return bool(p.get("fp") and p.get("pin_hash") and p.get("ready"))

    def pairing_stato(self):
        with self.pair_lock:
            q = self.pending
            if not q:
                return {"active": False, "seconds": 0, "attempts": 0}
            remaining = max(0, 300 - int(time.time() - q["created"]))
            return {"active": remaining > 0 and q["tries"] < 5,
                    "seconds": remaining, "attempts": max(0, 5 - q["tries"]),
                    "alias": q["dev"]["alias"]}

    def verifica_pin(self, pin, ip):
        # Global budget also limits attacks spread across local IP addresses.
        with self.pair_lock:
            now = time.monotonic()
            while self.pin_attempts and now - self.pin_attempts[0] > 300:
                self.pin_attempts.popleft()
            if len(self.pin_attempts) >= 10:
                return False
            if not re.fullmatch(r"[0-9]{6}", pin or ""):
                if pin:
                    self.pin_attempts.append(now)
                return False
            p = self.conf["pairing"] or {}
            digest = hashlib.pbkdf2_hmac("sha256", pin.encode(), p.get("pin_salt", "").encode(), 100000).hex()
            good = secrets.compare_digest(digest, p.get("pin_hash", ""))
            if not good:
                self.pin_attempts.append(now)
            return good

    def bluetooth_lista(self):
        rc, out = esegui(["bluetoothctl", "devices", "Paired"], 10)
        if rc:
            rc, out = esegui(["bluetoothctl", "paired-devices"], 10)
        return [{"mac": m[0], "nome": m[1]} for m in re.findall(r"^Device ([0-9A-Fa-f:]{17}) (.+)$", out, re.M)] if rc == 0 else []

    def bluetooth_apri(self):
        for command in (["gnome-control-center", "bluetooth"], ["blueman-manager"], ["systemsettings", "kcm_bluetooth"]):
            if ha(command[0]):
                subprocess.Popen(command, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                return {"ok": True}
        return {"ok": False, "errore": "Apri Bluetooth dalle Impostazioni di Fedora e abbina il Pixel, poi torna qui."}

    def pairing_inizia(self, d):
        with self.pair_lock:
            if self.conf["pairing"].get("fp"):
                return {"ok": False, "errore": "Pixel già associato. Per cambiarlo usa il reset esplicito da terminale."}
            mac = str(d.get("bluetooth") or "")
            if mac and mac not in [b["mac"] for b in self.bluetooth_lista()]:
                return {"ok": False, "errore": "Il dispositivo Bluetooth selezionato non risulta abbinato."}
            dev = dict(self.nodo.dispositivi.get(d.get("fp")) or {})
            if not dev or dev.get("protocol") != "https":
                return {"ok": False, "errore": "Seleziona il Pixel con LocalSend aperto e cifratura attiva."}
            old = self.pending
            if old and time.time() - old["created"] < 300 and old["tries"] < 5 and old["dev"]["fp"] == dev["fp"]:
                code = old["code"]
                old.update(dev=dev, mac=mac)
            else:
                code = str(secrets.randbelow(900000) + 100000)
                self.pending = {"created": time.time(), "dev": dev, "code": code, "mac": mac, "tries": 0}
            fid = uuid.uuid4().hex
            message = "Pixel Hub — codice di associazione: " + code + ". Inseriscilo SOLO nel wizard sul tuo PC Fedora. Scade tra 5 minuti."
            body = {"info": self.nodo.info(), "files": {fid: {"id": fid, "fileName": "pairing.txt", "size": len(message.encode()), "fileType": "text/plain", "preview": message}}}
            try:
                status, _ = richiesta_ls(dev, "POST", "/api/localsend/v2/prepare-upload", body, 30)
                # A phone can display the preview before accepting or responding.
                # Preserve this challenge even if its response is delayed/lost.
                if status != 204:
                    return {"ok": False, "errore": "LocalSend ha risposto %s. Se sul Pixel vedi il messaggio Pixel Hub con il codice, inseriscilo qui; altrimenti riprova l’invio." % status}
                return {"ok": True, "alias": dev["alias"]}
            except Exception:
                return {"ok": False, "errore": "Non ho ricevuto conferma da LocalSend. Se il codice è arrivato sul Pixel puoi comunque inserirlo; altrimenti controlla rete e cifratura e riprova."}

    def pairing_conferma(self, d):
        with self.pair_lock:
            q = self.pending
            if not q:
                return {"ok": False, "reason": "missing", "errore": "Nessun codice attivo in questa sessione. Premi Invia codice al Pixel selezionato. Non usare il codice Bluetooth o quello ADB."}
            if time.time() - q["created"] > 300:
                return {"ok": False, "reason": "expired", "errore": "Sono passati 5 minuti: premi Invia codice per riceverne uno nuovo."}
            if q["tries"] >= 5:
                return {"ok": False, "reason": "attempts", "errore": "Raggiunti 5 codici errati. Premi Invia codice per generarne uno nuovo."}
            pin = str(d.get("pin", "")).strip()
            if not re.fullmatch(r"[0-9]{6}", pin):
                return {"ok": False, "reason": "pin", "errore": "Scegli tu un PIN di ricezione di 6 cifre nel secondo campo. Il codice ricevuto resta valido."}
            code = re.sub(r"\s+", "", str(d.get("code", "")))
            if not re.fullmatch(r"[0-9]{6}", code):
                return {"ok": False, "reason": "format", "errore": "Inserisci le 6 cifre del messaggio Pixel Hub ricevuto in LocalSend, non il codice Bluetooth/ADB."}
            if not secrets.compare_digest(code, q["code"]):
                q["tries"] += 1
                return {"ok": False, "reason": "wrong", "errore": "Codice diverso da quello inviato al Pixel selezionato. Tentativi rimasti: %s." % (5 - q["tries"])}
            dev = q["dev"]
            salt = secrets.token_hex(16)
            record = {"fp": dev["fp"].upper(), "alias": dev["alias"], "bluetooth": q["mac"], "ready": False,
                      "pin_salt": salt, "pin_hash": hashlib.pbkdf2_hmac("sha256", pin.encode(), salt.encode(), 100000).hex()}
            with self.conf.lock:
                self.conf.d["pairing"] = record
                self.conf.d["fidati"] = {}
                self.conf.d["auto_accetta"] = False
                for name in ("cartella", "cartella_foto"):
                    self.conf.d[name] = str(Path(DEFAULT[name]) / ("Pixel-" + record["fp"][:12]))
                    Path(self.conf.d[name]).mkdir(parents=True, exist_ok=True)
                self.conf.salva()
            with self.nodo.lock:
                self.nodo.dispositivi = {dev["fp"]: dev}
                self.nodo.messaggi.clear()
                self.nodo.richieste.clear()
                self.nodo.sessioni.clear()
            self.pending = None
            return {"ok": True}

    def adb_candidati(self):
        rc, out = esegui(["adb", "devices"], 10)
        result = []
        for line in out.splitlines() if rc == 0 else []:
            fields = line.split()
            if len(fields) != 2 or fields[1] != "device":
                continue
            serial = fields[0]
            rc1, model = esegui(["adb", "-s", serial, "shell", "getprop", "ro.product.model"], 8)
            rc2, hardware = esegui(["adb", "-s", serial, "shell", "getprop", "ro.serialno"], 8)
            if not rc1 and not rc2 and hardware and hardware != "unknown":
                result.append({"serial": serial, "hardware": hardware, "model": model})
        return result

    def adb_wifi(self, d):
        # Pair code is passed via stdin, never command line, logs or config.
        try:
            endpoint, connection = str(d.get("pair", "")), str(d.get("connect", ""))
            for ep in (endpoint, connection):
                host, port = ep.rsplit(":", 1)
                if not ipaddress.IPv4Address(host).is_private or not 1 <= int(port) <= 65535:
                    raise ValueError()
            code = str(d.get("code", ""))
            if not re.fullmatch(r"[0-9]{6}", code):
                raise ValueError()
            r = subprocess.run(["adb", "pair", endpoint], input=code + "\n", capture_output=True, text=True, timeout=30)
            if r.returncode != 0 or "Successfully paired" not in r.stdout:
                return {"ok": False, "errore": "Pairing ADB non riuscito. Controlla IP, porta di abbinamento e codice sul Pixel."}
            rc, out = esegui(["adb", "connect", connection], 15)
            return {"ok": rc == 0 and "connected to" in out, "errore": "Se non compare, controlla la porta di connessione (diversa da quella di abbinamento)."}
        except (ValueError, OSError, subprocess.TimeoutExpired):
            return {"ok": False, "errore": "Usa IP locale:porta e il codice di 6 cifre mostrato dal Pixel."}

    def pairing_fine(self, d):
        with self.conf.lock:
            p = self.conf.d["pairing"]
            if not p.get("fp"):
                return {"ok": False, "errore": "Conferma prima LocalSend sul Pixel."}
            selected = str(d.get("serial", ""))
            if selected:
                choices = [x for x in self.adb_candidati() if x["serial"] == selected]
                if len(choices) != 1:
                    return {"ok": False, "errore": "Dispositivo ADB non disponibile."}
                if p.get("adb_serial") and p["adb_serial"] != choices[0]["hardware"]:
                    return {"ok": False, "errore": "ADB già associato a un altro telefono: usa il reset esplicito."}
                p["adb_serial"] = choices[0]["hardware"]
            p["ready"] = True
            self.conf.salva()
        return {"ok": True}

    def chiama_finestra(self):
        if self.finestra_cb:
            try:
                self.finestra_cb()
            except Exception:  # noqa: BLE001
                pass

    def componenti(self, forza=False):
        t, lista = self._comp
        if forza or time.time() - t > 30:
            lista = [{"id": i, "nome": n, "desc": d, "ok": bool(v())} for i, n, d, v, _p in COMPONENTI]
            self._comp = (time.time(), lista)
        return lista

    def installa(self):
        if self.installazione["attiva"]:
            return
        mancanti = [p for (_i, _n, _d, v, pk) in COMPONENTI if not v() for p in pk]
        if not mancanti:
            self.installazione["msg"] = "Dipendenze base presenti. Per scrcpy riesegui lo script di installazione nel terminale."
            return

        def lavoro():
            self.installazione = {"attiva": True, "msg": "Installo… (ti chiede la password)"}
            rc, out = esegui(["pkexec", "dnf", "install", "-y"] + mancanti, 1200)
            self.installazione = {"attiva": False,
                                  "msg": "Installato" if rc == 0 else "Installazione non riuscita"}
            self.componenti(True)
        in_thread(lavoro)

    def recenti(self, n=40):
        cart = Path(self.conf["cartella"])
        out = []
        try:
            voci = [e for e in os.scandir(cart)
                    if not e.name.startswith(".") and not e.name.endswith(".part")]
        except OSError:
            return out
        voci.sort(key=lambda e: e.stat().st_mtime, reverse=True)
        for e in voci[:n]:
            try:
                st = e.stat()
            except OSError:
                continue
            out.append({"nome": e.name, "path": e.path, "size": st.st_size, "mtime": st.st_mtime,
                        "dir": e.is_dir(), "tipo": "cartella" if e.is_dir() else tipo_file(e.name)})
        return out

    def consentito(self, p):
        try:
            p = Path(p).resolve()
        except (OSError, RuntimeError):
            return None
        for radice in (self.conf["cartella"], self.conf["cartella_foto"], str(CACHE / "uscita")):
            try:
                if p.is_relative_to(Path(radice).resolve()):
                    return p
            except (OSError, RuntimeError):
                pass
        return None

    def imposta_avvio(self, attivo):
        try:
            if attivo:
                AUTOSTART.parent.mkdir(parents=True, exist_ok=True)
                AUTOSTART.write_text(
                    "[Desktop Entry]\nType=Application\nName=Pixel Hub\n"
                    "Exec=python3 \"%s\" --background\nIcon=%s\nX-GNOME-Autostart-enabled=true\n"
                    "NoDisplay=true\n" % (Path(__file__).resolve(), APP_ID))
            elif AUTOSTART.exists():
                AUTOSTART.unlink()
        except OSError:
            pass

    def stato(self):
        self.ultimo_poll = time.time()
        c = self.conf
        reti = ip_locali()
        return {
            "versione": VERSIONE, "utente": nome_utente(),
            "pairing": {"ready": self.pronto(), "alias": (c["pairing"] or {}).get("alias", ""),
                        "fp": (c["pairing"] or {}).get("fp", ""),
                        "bluetooth": (c["pairing"] or {}).get("bluetooth", ""),
                        "adb": bool((c["pairing"] or {}).get("adb_serial")),
                        "challenge": self.pairing_stato()},
            "conf": {k: c[k] for k in ("nome_pc", "cartella", "cartella_foto", "vetro", "tema",
                                       "auto_accetta", "notifiche", "schermo_intero",
                                       "background", "avvio_login")},
            "fidati": [{"fp": k, "alias": v} for k, v in c["fidati"].items()],
            "ls": self.nodo.stato(),
            "schermo": self.schermo.stato(),
            "cavo": self.cavo.stato(),
            "recenti": self.recenti(),
            "rete": {"nome": nome_rete(), "ip": [ip for _i, ip in reti],
                     "hotspot": False},
            "componenti": self.componenti(),
            "installazione": self.installazione,
            "finestra": self.finestra_cb is not None,
        }


# =============================================================== server interfaccia
class ServerUI(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, hub):
        self.hub = hub
        super().__init__(("127.0.0.1", int(os.environ.get("HUB_UI_PORT", "0"))), GestoreUI)

    def url(self):
        return "http://127.0.0.1:%d/?t=%s" % (self.server_address[1], self.hub.token)


class GestoreUI(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        log("UI", self.command, urllib.parse.urlsplit(self.path).path)

    @property
    def hub(self):
        return self.server.hub

    def risposta(self, codice, obj=None, tipo="application/json", dati=None):
        if dati is None:
            dati = json.dumps(obj if obj is not None else {}, ensure_ascii=False).encode()
        self.send_response(codice)
        self.send_header("Content-Type", tipo)
        self.send_header("Content-Length", str(len(dati)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(dati)

    def autorizzato(self, q):
        t = self.headers.get("X-Token") or q.get("t", "")
        return secrets.compare_digest(t, self.hub.token)

    def corpo(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n <= 0 or n > 2_000_000:
            return {}
        try:
            d = json.loads(self.rfile.read(n))
            return d if isinstance(d, dict) else {}
        except ValueError:
            return {}

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path in ("/", "/index.html"):
            try:
                dati = UI_FILE.read_bytes()
            except OSError:
                dati = b"<h1>index.html mancante</h1>"
            return self.risposta(200, tipo="text/html; charset=utf-8", dati=dati)
        if u.path == "/icona.svg":
            p = HOME / ".local/share/icons/hicolor/scalable/apps" / (APP_ID + ".svg")
            if not p.exists():
                p = QUI / "icon.svg"
            try:
                return self.risposta(200, tipo="image/svg+xml", dati=p.read_bytes())
            except OSError:
                return self.risposta(404)
        if not self.autorizzato(q):
            return self.risposta(403, {"errore": "token"})
        if u.path == "/api/stato":
            return self.risposta(200, self.hub.stato())
        if u.path == "/api/anteprima":
            p = self.hub.consentito(q.get("path", ""))
            if not p or not p.is_file() or tipo_file(p.name) != "foto" or \
                    p.suffix.lower() in (".heic", ".heif") or p.stat().st_size > 40_000_000:
                return self.risposta(404)
            tipo = mimetypes.guess_type(p.name)[0] or "image/jpeg"
            return self.risposta(200, tipo=tipo, dati=p.read_bytes())
        self.risposta(404)

    def do_POST(self):
        u = urllib.parse.urlparse(self.path)
        q = dict(urllib.parse.parse_qsl(u.query))
        if not self.autorizzato(q):
            return self.risposta(403, {"errore": "token"})
        hub = self.hub
        if u.path == "/api/prepara" and not hub.pronto():
            return self.risposta(403, {"ok": False})
        if u.path == "/api/prepara":   # carica un file trascinato nell'app (da inviare)
            nome = nome_sicuro(q.get("nome", "file"))[-1]
            cart = CACHE / "uscita" / uuid.uuid4().hex[:10]
            cart.mkdir(parents=True, exist_ok=True)
            dest = cart / nome
            resto = int(self.headers.get("Content-Length") or 0)
            try:
                with open(dest, "wb") as f:
                    while resto > 0:
                        c = self.rfile.read(min(resto, 1 << 20))
                        if not c:
                            raise IOError
                        f.write(c)
                        resto -= len(c)
            except (OSError, IOError):
                return self.risposta(500, {"errore": "scrittura"})
            return self.risposta(200, {"path": str(dest), "nome": nome, "size": dest.stat().st_size})
        d = self.corpo()
        az = u.path[len("/api/"):]
        ok = True
        extra = {}
        if az.startswith("pairing/"):
            try:
                action = az.split("/", 1)[1]
                if action == "bluetooth": result = hub.bluetooth_apri()
                elif action == "lista": result = {"ok": True, "bluetooth": hub.bluetooth_lista(), "devices": list(hub.nodo.dispositivi.values()), "adb": hub.adb_candidati()}
                elif action == "inizia": result = hub.pairing_inizia(d)
                elif action == "conferma": result = hub.pairing_conferma(d)
                elif action == "wifi": result = hub.adb_wifi(d)
                elif action == "fine": result = hub.pairing_fine(d)
                else: result = {"ok": False}
            except Exception:
                result = {"ok": False, "errore": "Operazione non riuscita: controlla collegamento e permessi."}
            return self.risposta(200, result)
        if not hub.pronto() and az not in ("cerca", "ls/riprova"):
            return self.risposta(403, {"ok": False, "errore": "Completa il wizard di associazione."})
        if az == "conf":
            fatti = hub.conf.aggiorna(d)
            if "avvio_login" in fatti:
                hub.imposta_avvio(fatti["avvio_login"])
            if "nome_pc" in fatti:
                in_thread(hub.nodo.annuncia, True)
            extra = {"aggiornati": list(fatti)}
        elif az == "fidato/rimuovi":
            with hub.conf.lock:
                hub.conf.d.get("fidati", {}).pop(d.get("fp", ""), None)
                hub.conf.salva()
        elif az == "richiesta":
            ok = hub.nodo.decidi(d.get("id", ""), bool(d.get("accetta")), bool(d.get("fidati")))
        elif az == "invia":
            percorsi = [str(p) for p in (hub.consentito(x) for x in d.get("percorsi") or []) if p]
            testo = d.get("testo")
            if testo is not None and (not isinstance(testo, str) or not testo.strip()):
                testo = None
            jid = hub.nodo.invia(d.get("fp", ""), percorsi, testo)
            ok = jid is not None
            extra = {"id": jid}
        elif az == "invio/annulla":
            j = hub.nodo.invii.get(d.get("id", ""))
            if j:
                j["annulla"] = True
        elif az == "cerca":
            ip = d.get("ip")
            if ip and not re.fullmatch(r"\d{1,3}(\.\d{1,3}){3}", ip):
                return self.risposta(400, {"ok": False})
            extra = {"trovati": hub.nodo.cerca(ip)}
        elif az == "ls/riprova":
            ok = hub.nodo.riprova()
        elif az == "messaggio/rimuovi":
            m = [x for x in hub.nodo.messaggi if x["id"] == d.get("id")]
            for x in m:
                hub.nodo.messaggi.remove(x)
        elif az == "schermo":
            if d.get("azione") == "avvia":
                ok = hub.schermo.avvia()
            else:
                hub.schermo.ferma()
        elif az in ("apri", "mostra"):
            p = hub.consentito(d.get("path", ""))
            if not p or not p.exists():
                ok = False
            elif az == "apri":
                subprocess.Popen(["gio", "open", str(p)], stdout=subprocess.DEVNULL,
                                 stderr=subprocess.DEVNULL)
            else:
                rc, _o = esegui(["gdbus", "call", "--session", "--dest", "org.freedesktop.FileManager1",
                                 "--object-path", "/org/freedesktop/FileManager1", "--method",
                                 "org.freedesktop.FileManager1.ShowItems", "['%s']" % p.as_uri(), ""], 5)
                if rc != 0:
                    subprocess.Popen(["gio", "open", str(p.parent)], stdout=subprocess.DEVNULL,
                                     stderr=subprocess.DEVNULL)
        elif az == "apri-cartella":
            k = "cartella_foto" if d.get("quale") == "foto" else "cartella"
            p = Path(hub.conf[k])
            p.mkdir(parents=True, exist_ok=True)
            subprocess.Popen(["gio", "open", str(p)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        elif az == "scegli-cartella":
            k = "cartella_foto" if d.get("quale") == "foto" else "cartella"
            scelta = None
            if hub.scegli_cartella_cb:
                scelta = hub.scegli_cartella_cb(hub.conf[k])
            elif ha("zenity"):
                rc, out = esegui(["zenity", "--file-selection", "--directory",
                                  "--filename=%s/" % hub.conf[k]], 600)
                scelta = out.strip() if rc == 0 else None
            if scelta:
                hub.conf.aggiorna({k: scelta})
            ok = bool(scelta)
        elif az == "cavo/importa":
            ok = hub.cavo.importa()
        elif az == "cavo/apri":
            ok = hub.cavo.apri()
        elif az == "cavo/associa":
            ok = hub.cavo.associa()
        elif az == "installa":
            hub.installa()
        elif az == "componenti":
            extra = {"componenti": hub.componenti(True)}
        else:
            return self.risposta(404, {"ok": False})
        self.risposta(200, dict({"ok": ok}, **extra))


# =============================================================== finestra
def finestra(hub, server, background):
    import gi
    gi.require_version("Gtk", "4.0")
    gi.require_version("Adw", "1")
    gi.require_version("WebKit", "6.0")
    from gi.repository import Adw, Gio, GLib, Gtk, WebKit

    class App(Adw.Application):
        def __init__(self):
            super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.DEFAULT_FLAGS)
            self.win = None
            self.prima = True
            self.backend = False

        def do_activate(self):
            if self.prima and background:
                self.prima = False
                self.hold()
                return
            self.prima = False
            if self.win is None:
                self.crea()
            self.win.present()

        def crea(self):
            w = Adw.ApplicationWindow(application=self, title="Pixel Hub")
            w.set_default_size(1120, 780)
            w.set_size_request(400, 560)
            web = WebKit.WebView()
            s = web.get_settings()
            s.set_enable_developer_extras(bool(os.environ.get("HUB_DEBUG")))
            s.set_enable_back_forward_navigation_gestures(False)
            try:
                s.set_hardware_acceleration_policy(WebKit.HardwareAccelerationPolicy.ALWAYS)
            except AttributeError:
                pass

            def politica(_v, decisione, tipo):
                if tipo == WebKit.PolicyDecisionType.NAVIGATION_ACTION:
                    uri = decisione.get_navigation_action().get_request().get_uri()
                    if not uri.startswith("http://127.0.0.1:"):
                        decisione.ignore()
                        subprocess.Popen(["gio", "open", uri], stdout=subprocess.DEVNULL,
                                         stderr=subprocess.DEVNULL)
                        return True
                return False
            web.connect("decide-policy", politica)
            web.load_uri(server.url())

            over = Gtk.Overlay(child=web)
            maniglia = Gtk.WindowHandle(valign=Gtk.Align.START, height_request=44)
            barra = Gtk.Box(margin_start=10, margin_end=10)
            barra.append(Gtk.WindowControls(side=Gtk.PackType.START, valign=Gtk.Align.CENTER))
            barra.append(Gtk.Box(hexpand=True))
            barra.append(Gtk.WindowControls(side=Gtk.PackType.END, valign=Gtk.Align.CENTER))
            maniglia.set_child(barra)
            over.add_overlay(maniglia)
            w.set_content(over)

            def chiudi(_w):
                if hub.conf["background"]:
                    if not getattr(self, "tenuta", False):
                        self.hold()
                        self.tenuta = True
                    w.set_visible(False)
                    return True
                hub.schermo.ferma()
                self.quit()
                return False
            w.connect("close-request", chiudi)
            self.win = w

        def mostra(self):
            if self.win is None:
                self.crea()
            self.win.present()

    app = App()
    app.register(None)
    if app.get_is_remote():
        app.activate()       # l'app e gia aperta: la porto davanti
        return "remota"

    def scegli(attuale):
        risultato = {"v": None}
        ev = threading.Event()

        def apri():
            d = Gtk.FileDialog(title="Scegli una cartella")
            try:
                d.set_initial_folder(Gio.File.new_for_path(attuale))
            except GLib.Error:
                pass

            def fatto(dlg, res):
                try:
                    f = dlg.select_folder_finish(res)
                    risultato["v"] = f.get_path() if f else None
                except GLib.Error:
                    pass
                ev.set()
            d.select_folder(app.win, None, fatto)
            return False
        GLib.idle_add(apri)
        ev.wait(600)
        return risultato["v"]

    hub.finestra_cb = lambda: GLib.idle_add(lambda: (app.mostra(), False)[1])
    hub.scegli_cartella_cb = scegli
    in_thread(server.serve_forever)
    hub.nodo.avvia()
    rc = app.run(None)
    hub.schermo.ferma()
    return rc


def main():
    background = "--background" in sys.argv
    browser = "--browser" in sys.argv
    hub = Hub()
    if not hub.pronto():
        background = False
    if hub.conf["avvio_login"]:
        hub.imposta_avvio(True)
    server = ServerUI(hub)
    if not browser:
        try:
            r = finestra(hub, server, background)
            return 0 if r == "remota" else (r or 0)
        except (ImportError, ValueError) as e:
            print("Finestra non disponibile (%s): uso il browser." % e, file=sys.stderr)
    in_thread(server.serve_forever)
    hub.nodo.avvia()
    print(server.url(), flush=True)
    if not background and not os.environ.get("HUB_NO_OPEN"):
        subprocess.Popen(["xdg-open", server.url()], stdout=subprocess.DEVNULL,
                         stderr=subprocess.DEVNULL)
    try:
        while True:
            time.sleep(3600)
    except KeyboardInterrupt:
        hub.schermo.ferma()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
