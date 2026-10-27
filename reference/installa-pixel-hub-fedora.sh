#!/usr/bin/env bash
# Pixel Hub 3.1 — Fedora e Android, associazione esplicita del telefono.
# Uso: bash installa-pixel-hub-fedora.sh [rimuovi|reset-pairing|firewall ZONA|diagnostica]
# Fedora tradizionale con DNF; Silverblue/Kinoite non supportati da questo installer.
set -euo pipefail
ID=it.frank.PixelHub
DIR="$HOME/.local/share/pixel-hub"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"
if (( EUID == 0 )); then
  echo 'Avvia questo script come utente normale, senza sudo. Lo script lo userà per i pacchetti.'; exit 1
fi
case "${1:-installa}" in
  reset-pairing)
    echo 'Chiudi Pixel Hub prima di proseguire. Il reset scollega il Pixel e azzera il PIN; foto e file restano.'
    read -r -p 'Scrivi RESET per confermare: ' answer
    [[ "$answer" == RESET ]] || exit 1
    pkill -f "^python3 $DIR/pixel_hub.py" 2>/dev/null || true
    python3 - <<'RESET_PY'
import json, os
from pathlib import Path
p = Path.home() / '.config/pixel-hub-v3/config.json'
if p.exists():
    data = json.loads(p.read_text())
    data.update(pairing={}, fidati={}, auto_accetta=False)
    tmp = p.with_suffix('.tmp')
    tmp.write_text(json.dumps(data, indent=2))
    os.chmod(tmp, 0o600)
    tmp.replace(p)
print('Reset completato. Riapri Pixel Hub per il wizard. Gli abbinamenti Bluetooth e ADB di sistema restano: puoi revocarli nelle impostazioni del Pixel.')
RESET_PY
    exit 0;;
  rimuovi)
    pkill -f "^python3 $DIR/pixel_hub.py" 2>/dev/null || true
    rm -rf -- "$DIR"
    rm -f -- "$APPS/$ID.desktop" "$ICONS/$ID.svg" "$HOME/.config/autostart/$ID.desktop"
    update-desktop-database "$APPS" 2>/dev/null || true
    echo 'Pixel Hub rimosso. File ricevuti, foto, configurazione e pacchetti conservati.'
    exit 0;;
  firewall)
    zone="${2:-}"
    if [[ -z "$zone" ]]; then
      echo 'Indica la zona della tua rete fidata: bash installa-pixel-hub-fedora.sh firewall NOME_ZONA'
      firewall-cmd --get-active-zones || true
      exit 1
    fi
    firewall-cmd --get-zones | tr ' ' '\n' | grep -Fxq -- "$zone" || { echo 'Zona non valida'; exit 1; }
    for proto in tcp udp; do
      sudo firewall-cmd --zone="$zone" --add-port="53317/$proto"
      sudo firewall-cmd --permanent --zone="$zone" --add-port="53317/$proto"
    done
    echo "LocalSend abilitato nella zona $zone. Non usare questa apertura su reti non fidate."
    exit 0;;
  diagnostica)
    cat /etc/fedora-release 2>/dev/null || true
    command -v adb >/dev/null && adb devices -l || true
    command -v scrcpy >/dev/null && scrcpy --version || true
    command -v firewall-cmd >/dev/null && firewall-cmd --get-active-zones || true
    ss -lntup 'sport = :53317' || true
    exit 0;;
  installa) ;;
  *) echo 'Uso: bash installa-pixel-hub-fedora.sh [rimuovi|reset-pairing|firewall ZONA|diagnostica]'; exit 1;;
esac
if [[ ! -f /etc/fedora-release ]] || ! command -v dnf >/dev/null; then
  echo 'Questo installer richiede Fedora con DNF.'; exit 1
fi
if [[ -e /run/ostree-booted ]]; then
  echo 'Fedora Atomic (Silverblue/Kinoite) richiede una procedura diversa: installazione interrotta.'; exit 1
fi
sudo dnf install -y python3 python3-gobject-base gtk4 libadwaita webkitgtk6.0 \
  bluez android-tools gvfs-mtp openssl libnotify xdg-utils xdg-user-dirs desktop-file-utils polkit
if ! command -v scrcpy >/dev/null; then
  echo 'Schermo Pixel: scrcpy è opzionale e richiede il repository comunitario COPR zeno/scrcpy.'
  echo 'È il metodo Fedora indicato dalla documentazione di scrcpy: https://github.com/Genymobile/scrcpy/blob/master/doc/linux.md'
  answer=n
  read -r -p 'Abilitare questo repository e installare scrcpy? [s/N] ' answer || true
  if [[ "$answer" =~ ^[sSyY]$ ]]; then
    if sudo dnf copr enable -y zeno/scrcpy && sudo dnf install -y scrcpy; then
      echo 'scrcpy installato.'
    else
      echo 'scrcpy non installato: Hub e trasferimento file restano disponibili. Puoi riprovare rieseguendo lo script.'
    fi
  fi
fi
pkill -f "^python3 $DIR/pixel_hub.py" 2>/dev/null || true
mkdir -p "$DIR" "$APPS" "$ICONS"
cat > "$DIR/pixel_hub.py" <<'PYEOF'
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
PYEOF
cat > "$DIR/index.html" <<'HTMLEOF'
<!doctype html>
<html lang="it">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pixel Hub</title>
<style>
:root{
  --tint:.45;
  --font:"SF Pro Text","SF Pro Display","Inter","Fedora Sans",system-ui,-apple-system,sans-serif;
  --bg:#eef1f7;
  --txt:#0b0b0f; --txt2:rgba(20,20,30,.62); --txt3:rgba(20,20,30,.38);
  --sep:rgba(20,20,40,.10);
  --blu:#007aff; --verde:#28c840; --rosso:#ff3b30; --arancio:#ff9500; --viola:#af52de;
  --glass:rgba(255,255,255,calc(.20 + var(--tint)*.52));
  --glass-forte:rgba(255,255,255,calc(.42 + var(--tint)*.45));
  --edge:rgba(0,0,0,.16);
  --spec:rgba(255,255,255,.85);
  --ombra:0 18px 40px -18px rgba(30,40,80,.30);
  --riempi:rgba(120,120,135,.13);
  --b1:#8fb4ff; --b2:#c8a2ff; --b3:#ffb2d0; --b4:#8fe6dc;
  color-scheme:light;
}
:root[data-tema="scuro"]{
  --bg:#07080d;
  --txt:#f5f5f7; --txt2:rgba(235,235,245,.62); --txt3:rgba(235,235,245,.34);
  --sep:rgba(255,255,255,.10);
  --blu:#0a84ff; --verde:#30d158; --rosso:#ff453a; --arancio:#ff9f0a; --viola:#bf5af2;
  --glass:rgba(30,30,38,calc(.28 + var(--tint)*.52));
  --glass-forte:rgba(44,44,54,calc(.50 + var(--tint)*.40));
  --edge:rgba(0,0,0,.55);
  --spec:rgba(255,255,255,.22);
  --ombra:0 22px 48px -20px rgba(0,0,0,.65);
  --riempi:rgba(120,120,135,.22);
  --b1:#1f4fd8; --b2:#6a2bd1; --b3:#b0306f; --b4:#0f7f78;
  color-scheme:dark;
}
:root[data-tema="scuro"] .sfondo i{opacity:.55}
*{box-sizing:border-box;margin:0;padding:0}
html,body{height:100%;overflow:hidden}
body{font-family:var(--font);background:var(--bg);color:var(--txt);font-size:15px;
  -webkit-font-smoothing:antialiased;letter-spacing:-.01em;user-select:none;-webkit-user-select:none}
button,input,textarea{font:inherit;color:inherit}
button{cursor:pointer;border:0;background:none}
::selection{background:color-mix(in srgb,var(--blu) 30%,transparent)}

/* ---------- sfondo vivo ---------- */
.sfondo{position:fixed;inset:-20%;z-index:0;filter:blur(70px) saturate(130%);pointer-events:none}
.sfondo i{position:absolute;border-radius:50%;opacity:.85;animation:vaga 38s ease-in-out infinite alternate}
.sfondo i:nth-child(1){width:52vw;height:52vw;left:2%;top:0;background:var(--b1)}
.sfondo i:nth-child(2){width:46vw;height:46vw;right:0;top:8%;background:var(--b2);animation-duration:44s;animation-delay:-8s}
.sfondo i:nth-child(3){width:40vw;height:40vw;left:30%;bottom:-4%;background:var(--b3);animation-duration:50s;animation-delay:-16s}
.sfondo i:nth-child(4){width:30vw;height:30vw;left:-4%;bottom:6%;background:var(--b4);animation-duration:41s;animation-delay:-22s}
@keyframes vaga{0%{transform:translate(0,0) scale(1)}50%{transform:translate(6vw,4vh) scale(1.08)}100%{transform:translate(-5vw,-3vh) scale(.95)}}
@media (prefers-reduced-motion:reduce){.sfondo i{animation:none}}

/* ---------- vetro ---------- */
.vetro{position:relative;background:var(--glass);
  -webkit-backdrop-filter:blur(30px) saturate(185%);backdrop-filter:blur(30px) saturate(185%);
  border-radius:28px;
  box-shadow:0 0 0 .5px var(--edge),inset 0 1px 0 var(--spec),inset 0 -1px 0 rgba(255,255,255,.08),var(--ombra)}
.vetro::before{content:"";position:absolute;inset:0;border-radius:inherit;pointer-events:none;
  background:linear-gradient(140deg,rgba(255,255,255,.35),rgba(255,255,255,0) 38%,rgba(255,255,255,0) 70%,rgba(255,255,255,.12));
  mix-blend-mode:soft-light}

/* ---------- struttura ---------- */
main{position:relative;z-index:1;height:100%;overflow-y:auto;overflow-x:hidden;scroll-behavior:smooth}
.barra{position:sticky;top:0;z-index:5;height:52px;display:flex;align-items:center;justify-content:center;
  transition:background .25s,box-shadow .25s}
.barra span{font-weight:600;font-size:15px;opacity:0;transform:translateY(4px);transition:.25s}
.barra.su{background:color-mix(in srgb,var(--bg) 55%,transparent);
  -webkit-backdrop-filter:blur(24px) saturate(180%);backdrop-filter:blur(24px) saturate(180%);
  box-shadow:0 .5px 0 var(--sep)}
.barra.su span{opacity:1;transform:none}
.pagina{display:none;max-width:1100px;margin:0 auto;padding:0 28px 130px}
.pagina.on{display:block;animation:entra .35s cubic-bezier(.2,.8,.2,1)}
@keyframes entra{from{opacity:0;transform:translateY(10px)}to{opacity:1;transform:none}}
.titolo{padding:6px 4px 20px}
.titolo h1{font-size:36px;font-weight:700;letter-spacing:-.025em;line-height:1.1}
.titolo p{color:var(--txt2);font-size:16px;margin-top:6px}
.griglia{display:grid;grid-template-columns:repeat(12,1fr);gap:18px}
.c12{grid-column:span 12}.c7{grid-column:span 7}.c5{grid-column:span 5}.c6{grid-column:span 6}.c4{grid-column:span 4}.c8{grid-column:span 8}
@media (max-width:860px){.c7,.c5,.c6,.c4,.c8{grid-column:span 12}.pagina{padding:0 16px 130px}.titolo h1{font-size:30px}}
.card{padding:22px}
.card h2{font-size:13px;font-weight:600;text-transform:uppercase;letter-spacing:.04em;color:var(--txt2);margin-bottom:14px;display:flex;align-items:center;gap:8px}
.card h2 .dx{margin-left:auto;text-transform:none;letter-spacing:0;font-weight:500}
.sezione{font-size:13px;font-weight:600;text-transform:uppercase;letter-spacing:.04em;color:var(--txt2);margin:28px 6px 10px}
.vuoto{color:var(--txt2);font-size:14px;line-height:1.45}

/* ---------- controlli ---------- */
.btn{display:inline-flex;align-items:center;justify-content:center;gap:8px;height:40px;padding:0 18px;border-radius:999px;
  font-weight:600;font-size:15px;background:var(--riempi);transition:transform .15s,filter .15s,background .2s}
.btn:hover{filter:brightness(1.06)}
.btn:active{transform:scale(.96)}
.btn.primario{background:var(--blu);color:#fff;box-shadow:0 6px 18px -6px color-mix(in srgb,var(--blu) 70%,transparent),inset 0 1px 0 rgba(255,255,255,.35)}
.btn.rosso{background:var(--rosso);color:#fff}
.btn.grande{height:52px;padding:0 26px;font-size:17px}
.btn.piccolo{height:32px;padding:0 13px;font-size:13px}
.btn.icona{width:36px;padding:0}
.btn[disabled]{opacity:.45;pointer-events:none}
.btn svg{width:18px;height:18px}
.link{color:var(--blu);font-weight:500;cursor:pointer}
.chip{display:inline-flex;align-items:center;gap:6px;height:26px;padding:0 11px;border-radius:999px;font-size:12.5px;font-weight:600;background:var(--riempi);color:var(--txt2)}
.chip.ok{background:color-mix(in srgb,var(--verde) 18%,transparent);color:var(--verde)}
.chip.blu{background:color-mix(in srgb,var(--blu) 16%,transparent);color:var(--blu)}
.chip.warn{background:color-mix(in srgb,var(--arancio) 18%,transparent);color:var(--arancio)}
.chip i{width:7px;height:7px;border-radius:50%;background:currentColor}
.chip i.pulsa{animation:pulsa 1.6s ease-in-out infinite}
@keyframes pulsa{50%{opacity:.25}}

/* liste raggruppate stile Impostazioni */
.lista{overflow:hidden;padding:0}
.riga{display:flex;align-items:center;gap:14px;min-height:54px;padding:10px 18px;position:relative}
.riga+.riga::before{content:"";position:absolute;top:0;left:62px;right:0;height:.5px;background:var(--sep)}
.lista.senza-icone .riga+.riga::before{left:18px}
.riga .ic{width:32px;height:32px;border-radius:9px;display:grid;place-items:center;color:#fff;flex:none}
.riga .ic svg{width:18px;height:18px}
.riga .testo{flex:1;min-width:0}
.riga .testo b{display:block;font-weight:500;font-size:15.5px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.riga .testo small{display:block;color:var(--txt2);font-size:13px;margin-top:2px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.riga .val{color:var(--txt2);font-size:14.5px}
.riga.cliccabile{cursor:pointer}
.riga.cliccabile:hover{background:var(--riempi)}
.freccia{color:var(--txt3);width:16px;height:16px}
.input{height:36px;border-radius:10px;border:0;background:var(--riempi);padding:0 12px;outline:none;min-width:0;width:220px;text-align:right}
.input:focus{box-shadow:0 0 0 3px color-mix(in srgb,var(--blu) 35%,transparent);text-align:left}

/* interruttore iOS */
.sw{position:relative;width:51px;height:31px;flex:none}
.sw input{opacity:0;width:0;height:0;position:absolute}
.sw span{position:absolute;inset:0;border-radius:999px;background:var(--riempi);transition:.25s;cursor:pointer;box-shadow:inset 0 0 0 .5px var(--sep)}
.sw span::after{content:"";position:absolute;top:2px;left:2px;width:27px;height:27px;border-radius:50%;background:#fff;
  box-shadow:0 3px 8px rgba(0,0,0,.15),0 1px 1px rgba(0,0,0,.16);transition:.25s cubic-bezier(.3,1.4,.5,1)}
.sw input:checked+span{background:var(--verde)}
.sw input:checked+span::after{transform:translateX(20px)}

/* segmentato */
.seg{display:inline-flex;background:var(--riempi);border-radius:10px;padding:2px;gap:2px}
.seg button{height:30px;padding:0 14px;border-radius:8px;font-size:13.5px;font-weight:500;color:var(--txt2)}
.seg button.on{background:var(--glass-forte);color:var(--txt);box-shadow:0 3px 8px rgba(0,0,0,.12),0 0 0 .5px var(--edge)}

/* cursore vetro */
.cursore{display:flex;align-items:center;gap:12px;width:100%}
.cursore small{color:var(--txt2);font-size:12.5px;white-space:nowrap}
input[type=range]{-webkit-appearance:none;appearance:none;flex:1;height:6px;border-radius:99px;
  background:linear-gradient(90deg,rgba(255,255,255,.35),var(--blu));outline:none;box-shadow:inset 0 0 0 .5px var(--edge)}
input[type=range]::-webkit-slider-thumb{-webkit-appearance:none;width:28px;height:28px;border-radius:50%;background:#fff;
  box-shadow:0 3px 10px rgba(0,0,0,.25),0 0 0 .5px rgba(0,0,0,.1);cursor:grab}
input[type=range]::-moz-range-thumb{width:28px;height:28px;border-radius:50%;background:#fff;border:0;box-shadow:0 3px 10px rgba(0,0,0,.25)}

.barra-prog{height:6px;border-radius:99px;background:var(--riempi);overflow:hidden}
.barra-prog i{display:block;height:100%;background:var(--blu);border-radius:99px;transition:width .4s}

/* ---------- tab bar fluttuante ---------- */
.tabbar{position:fixed;z-index:20;left:50%;bottom:18px;transform:translateX(-50%);display:flex;gap:2px;padding:6px;border-radius:999px}
.tab{position:relative;width:86px;height:56px;border-radius:999px;display:flex;flex-direction:column;align-items:center;justify-content:center;gap:3px;
  font-size:11px;font-weight:600;color:var(--txt2);transition:color .2s}
.tab svg{width:23px;height:23px}
.tab.on{color:var(--blu);background:var(--riempi);box-shadow:inset 0 1px 0 var(--spec)}
.tab .punto{position:absolute;top:8px;right:22px;min-width:16px;height:16px;padding:0 4px;border-radius:99px;background:var(--rosso);color:#fff;font-size:10px;line-height:16px;display:none}
.tab .punto.on{display:block}
@media (max-width:520px){.tab{width:64px}.tab span{font-size:10px}}

/* ---------- home ---------- */
.telefono{position:relative;width:92px;height:184px;border-radius:24px;flex:none;
  background:linear-gradient(160deg,#30323a,#0c0d10);box-shadow:0 0 0 2px rgba(255,255,255,.2) inset,0 18px 30px -12px rgba(0,0,0,.45)}
.telefono::before{content:"";position:absolute;inset:5px;border-radius:19px;background:var(--schermo,linear-gradient(160deg,#5b8def,#a46cf5 55%,#f37fb4))}
.telefono::after{content:"";position:absolute;top:10px;left:50%;transform:translateX(-50%);width:28px;height:8px;border-radius:9px;background:#000}
.telefono.spento::before{background:linear-gradient(160deg,#2b2d35,#15161a)}
.pixel{display:flex;gap:26px;align-items:center}
.pixel .info{flex:1;min-width:0}
.pixel h3{font-size:24px;font-weight:700;letter-spacing:-.02em;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.pixel .sub{color:var(--txt2);margin:4px 0 12px}
.misure{display:flex;gap:22px;margin-top:16px;flex-wrap:wrap}
.anello{display:flex;align-items:center;gap:10px}
.anello svg{width:44px;height:44px;transform:rotate(-90deg)}
.anello b{display:block;font-size:17px}
.anello small{color:var(--txt2);font-size:12px}
.spazio{flex:1;min-width:160px}
.spazio .barra-prog{height:8px;margin:8px 0 5px}
.spazio small{color:var(--txt2);font-size:12px}

.mirror{display:flex;flex-direction:column;align-items:center;text-align:center;gap:14px;height:100%;justify-content:center}
.tasto-tondo{width:96px;height:96px;border-radius:50%;display:grid;place-items:center;color:#fff;
  background:linear-gradient(160deg,color-mix(in srgb,var(--blu) 80%,#fff),var(--blu));
  box-shadow:0 14px 30px -10px color-mix(in srgb,var(--blu) 80%,transparent),inset 0 1px 0 rgba(255,255,255,.5),0 0 0 .5px var(--edge);transition:transform .2s}
.tasto-tondo:active{transform:scale(.93)}
.tasto-tondo svg{width:40px;height:40px}
.tasto-tondo.attivo{background:linear-gradient(160deg,#ff7a6e,var(--rosso));box-shadow:0 14px 30px -10px rgba(255,59,48,.7),inset 0 1px 0 rgba(255,255,255,.5)}
.tasto-tondo.attesa{animation:onda 2s ease-out infinite}
@keyframes onda{0%{box-shadow:0 0 0 0 color-mix(in srgb,var(--rosso) 55%,transparent)}100%{box-shadow:0 0 0 22px transparent}}
.mirror b{font-size:17px}
.mirror small{color:var(--txt2);font-size:13.5px;max-width:280px;line-height:1.4}

.drop{border-radius:22px;border:2px dashed color-mix(in srgb,var(--txt) 18%,transparent);padding:26px;display:flex;flex-direction:column;align-items:center;gap:10px;text-align:center;transition:.2s;cursor:pointer}
.drop:hover,.drop.sopra{border-color:var(--blu);background:color-mix(in srgb,var(--blu) 8%,transparent)}
.drop .ico{width:54px;height:54px;border-radius:16px;display:grid;place-items:center;background:color-mix(in srgb,var(--blu) 14%,transparent);color:var(--blu)}
.drop .ico svg{width:28px;height:28px}
.drop b{font-size:16.5px}
.drop small{color:var(--txt2);font-size:13.5px}

textarea{width:100%;min-height:92px;resize:none;border:0;outline:none;border-radius:16px;background:var(--riempi);padding:12px 14px;line-height:1.4;font-size:15px}
textarea:focus{box-shadow:0 0 0 3px color-mix(in srgb,var(--blu) 30%,transparent)}
.bolla{background:var(--riempi);border-radius:18px 18px 18px 6px;padding:11px 14px;line-height:1.4;font-size:14.5px;word-break:break-word;white-space:pre-wrap;max-height:120px;overflow:auto;user-select:text;-webkit-user-select:text}
.bolla-meta{display:flex;align-items:center;gap:8px;color:var(--txt2);font-size:12.5px;margin:6px 4px 12px}
.bolla-meta .dx{margin-left:auto;display:flex;gap:6px}

/* striscia di file recenti */
.strip{display:flex;gap:14px;overflow-x:auto;padding:4px 2px 8px;scrollbar-width:thin}
.tile{flex:none;width:132px;cursor:pointer}
.tile .img{width:132px;height:132px;border-radius:20px;background:var(--riempi) center/cover no-repeat;display:grid;place-items:center;color:var(--txt2);
  box-shadow:0 0 0 .5px var(--edge),0 10px 24px -14px rgba(0,0,0,.4);transition:transform .2s}
.tile:hover .img{transform:translateY(-3px)}
.tile .img svg{width:38px;height:38px}
.tile b{display:block;font-size:13px;font-weight:500;margin-top:8px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.tile small{color:var(--txt2);font-size:12px}

/* dispositivi vicini (stile AirDrop) */
.vicini{display:flex;gap:26px;flex-wrap:wrap;align-items:flex-start;min-height:120px}
.disp{width:104px;display:flex;flex-direction:column;align-items:center;gap:8px;text-align:center;cursor:pointer;border-radius:20px;padding:8px 4px;transition:.2s}
.disp:hover,.disp.sopra{background:var(--riempi)}
.disp.sel .av{box-shadow:0 0 0 3px var(--bg),0 0 0 5.5px var(--blu)}
.av{width:70px;height:70px;border-radius:50%;display:grid;place-items:center;color:#fff;font-size:26px;font-weight:600;
  background:linear-gradient(160deg,#7aa2ff,#5e5ce6);box-shadow:inset 0 1px 0 rgba(255,255,255,.5),0 10px 22px -10px rgba(40,40,120,.7);transition:box-shadow .2s}
.av svg{width:32px;height:32px}
.disp b{font-size:13px;font-weight:600;line-height:1.25;max-width:100px;overflow:hidden;text-overflow:ellipsis;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical}
.disp small{color:var(--txt2);font-size:11.5px}
.radar{position:relative;width:70px;height:70px;border-radius:50%;display:grid;place-items:center;color:var(--blu);background:color-mix(in srgb,var(--blu) 12%,transparent)}
.radar::before,.radar::after{content:"";position:absolute;inset:0;border-radius:50%;border:2px solid var(--blu);opacity:0;animation:radar 2.4s ease-out infinite}
.radar::after{animation-delay:1.2s}
@keyframes radar{0%{transform:scale(.6);opacity:.7}100%{transform:scale(1.5);opacity:0}}
.radar svg{width:30px;height:30px}

.coda{display:flex;flex-wrap:wrap;gap:8px;margin-top:14px}
.coda .chip{background:var(--glass-forte);color:var(--txt);box-shadow:0 0 0 .5px var(--edge);height:30px}
.coda .chip button{opacity:.5;display:grid;place-items:center}
.coda .chip button svg{width:14px;height:14px}

.trasf{display:flex;gap:14px;align-items:center;padding:12px 0}
.trasf+.trasf{border-top:.5px solid var(--sep)}
.trasf .ic{width:40px;height:40px;border-radius:12px;display:grid;place-items:center;flex:none}
.trasf .ic svg{width:20px;height:20px}
.trasf .t{flex:1;min-width:0}
.trasf .t b{display:block;font-size:14.5px;font-weight:500;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.trasf .t small{color:var(--txt2);font-size:12.5px}
.trasf .barra-prog{margin-top:7px}

/* passi */
.passi{display:flex;flex-direction:column;gap:14px}
.passo{display:flex;gap:14px;align-items:flex-start}
.passo .n{width:30px;height:30px;border-radius:50%;flex:none;display:grid;place-items:center;font-weight:700;font-size:14px;color:#fff;background:var(--blu)}
.passo b{display:block;font-weight:600}
.passo small{color:var(--txt2);font-size:13.5px}
.schermo-hero{display:flex;gap:34px;align-items:center}
.schermo-hero .telefono{width:120px;height:240px;border-radius:30px}
.schermo-hero .telefono::before{border-radius:25px}
.schermo-hero .telefono::after{width:36px;height:10px;top:13px}
.onde{position:relative}
.onde .w{position:absolute;right:-30px;top:50%;width:26px;height:60px;transform:translateY(-50%);color:var(--blu);opacity:0}
.onde.on .w{opacity:1;animation:pulsa 1.4s infinite}
pre.log{font:12px/1.5 ui-monospace,"JetBrains Mono","Fedora Mono",monospace;background:var(--riempi);border-radius:14px;padding:12px;max-height:220px;overflow:auto;white-space:pre-wrap;color:var(--txt2);user-select:text;-webkit-user-select:text}
details summary{cursor:pointer;color:var(--txt2);font-size:13.5px;list-style:none;padding:4px 2px}
details summary::-webkit-details-marker{display:none}

/* avviso */
.avviso{display:flex;gap:14px;align-items:center;padding:16px 18px;margin-bottom:18px}
.avviso .ic{width:36px;height:36px;border-radius:11px;display:grid;place-items:center;background:var(--arancio);color:#fff;flex:none}
.avviso .ic svg{width:20px;height:20px}
.avviso .t{flex:1}
.avviso b{display:block}
.avviso small{color:var(--txt2);font-size:13px}

/* ---------- sheet richiesta ---------- */
.velo{position:fixed;inset:0;z-index:40;background:rgba(0,0,0,.18);-webkit-backdrop-filter:blur(6px);backdrop-filter:blur(6px);display:none;align-items:flex-end;justify-content:center;padding:18px}
:root[data-tema="scuro"] .velo{background:rgba(0,0,0,.45)}
.velo.on{display:flex;animation:fade .25s}
@keyframes fade{from{opacity:0}}
.sheet{width:min(460px,100%);padding:26px 24px 22px;border-radius:36px;background:var(--glass-forte);text-align:center;animation:su .4s cubic-bezier(.2,.9,.25,1.1)}
@keyframes su{from{transform:translateY(60px);opacity:0}}
.sheet .av{margin:0 auto 12px;width:64px;height:64px}
.sheet h3{font-size:20px;font-weight:700;letter-spacing:-.02em}
.sheet p{color:var(--txt2);margin-top:4px}
.sheet .files{margin:18px 0;text-align:left;max-height:180px;overflow:auto;border-radius:16px;background:var(--riempi);padding:4px 14px}
.sheet .files div{display:flex;gap:10px;align-items:center;padding:9px 0;font-size:14px}
.sheet .files div+div{border-top:.5px solid var(--sep)}
.sheet .files svg{width:18px;height:18px;color:var(--txt2);flex:none}
.sheet .files span{flex:1;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.sheet .files em{font-style:normal;color:var(--txt2);font-size:12.5px}
.sheet .azioni{display:flex;gap:12px}
.sheet .azioni .btn{flex:1;height:50px;font-size:16px}
.sheet label{display:flex;align-items:center;justify-content:center;gap:10px;margin-top:16px;color:var(--txt2);font-size:13.5px;cursor:pointer}

/* ---------- toast ---------- */
.toasts{position:fixed;z-index:60;top:52px;left:50%;transform:translateX(-50%);display:flex;flex-direction:column;gap:8px;align-items:center;pointer-events:none}
.toast{padding:12px 20px;border-radius:999px;background:var(--glass-forte);font-weight:600;font-size:14px;display:flex;gap:10px;align-items:center;animation:giu .35s cubic-bezier(.2,.9,.25,1.1);pointer-events:auto}
.toast svg{width:18px;height:18px}
@keyframes giu{from{transform:translateY(-16px);opacity:0}}

/* rilascio file ovunque */
.rilascio{position:fixed;inset:12px;z-index:50;border-radius:34px;border:3px dashed var(--blu);background:color-mix(in srgb,var(--blu) 10%,transparent);
  -webkit-backdrop-filter:blur(8px);backdrop-filter:blur(8px);display:none;align-items:center;justify-content:center;flex-direction:column;gap:10px;pointer-events:none}
.rilascio.on{display:flex}
.rilascio b{font-size:24px;font-weight:700}
.rilascio small{color:var(--txt2);font-size:15px}
</style>
</head>
<body>
<div class="sfondo"><i></i><i></i><i></i><i></i></div>

<main id="main">
  <div class="barra" id="barra"><span id="barraTitolo">Home</span></div>

  <!-- ======================= HOME ======================= -->
  <section class="pagina on" data-p="home" data-t="Home">
    <div class="titolo"><h1 id="saluto">Ciao</h1><p id="data"></p></div>
    <div id="avvisoLS"></div>
    <div class="griglia">
      <div class="vetro card c7" id="cardPixel"></div>
      <div class="vetro card c5">
        <h2>Duplica schermo</h2>
        <div class="mirror" id="homeMirror"></div>
      </div>
      <div class="vetro card c7">
        <h2>Invia al Pixel <span class="dx" id="homeDest"></span></h2>
        <div class="drop" id="homeDrop">
          <div class="ico" data-i="su"></div>
          <b>Trascina qui foto e file</b>
          <small>oppure <span class="link">sfoglia il computer</span></small>
        </div>
        <div id="homeInvii"></div>
      </div>
      <div class="vetro card c5">
        <h2>Appunti condivisi</h2>
        <div id="homeMsg"></div>
        <textarea id="homeTesto" placeholder="Scrivi o incolla qui un testo, un link, un codice…"></textarea>
        <div style="display:flex;gap:10px;margin-top:12px">
          <button class="btn piccolo" id="homeIncolla"><span data-i="appunti"></span>Incolla</button>
          <button class="btn primario piccolo" id="homeInviaTesto" style="margin-left:auto"><span data-i="invia"></span>Invia al Pixel</button>
        </div>
      </div>
      <div class="vetro card c12">
        <h2>Arrivati di recente <span class="dx"><span class="link" data-azione="apriCartella">Apri cartella</span></span></h2>
        <div class="strip" id="homeRecenti"></div>
      </div>
    </div>
  </section>

  <!-- ======================= SCHERMO ======================= -->
  <section class="pagina" data-p="schermo" data-t="Schermo">
    <div class="titolo"><h1>Schermo</h1><p>Guarda il Pixel sul PC: perfetto per mostrare foto, video e app.</p></div>
    <div class="griglia">
      <div class="vetro card c12">
        <div class="schermo-hero">
          <div class="onde" id="onde"><div class="telefono" id="telSchermo"></div><svg class="w" viewBox="0 0 26 60" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><path d="M4 18c5 7 5 17 0 24"/><path d="M13 10c9 11 9 29 0 40"/><path d="M22 2c13 16 13 40 0 56"/></svg></div>
          <div style="flex:1;min-width:0">
            <div id="schermoStato"></div>
          </div>
        </div>
      </div>
      <div class="vetro card c6">
        <h2>Sul Pixel</h2>
        <div class="passi">
          <div class="passo"><div class="n">1</div><div><b>Abilita Debug USB</b><small>Impostazioni › Sistema › Opzioni sviluppatore</small></div></div>
          <div class="passo"><div class="n">2</div><div><b>Collega il cavo USB</b><small>Accetta «Consenti debug USB» sul Pixel</small></div></div>
          <div class="passo"><div class="n">3</div><div><b id="passo3">Premi Avvia in questa pagina</b><small>Dopo qualche secondo si apre la finestra</small></div></div>
        </div>
      </div>
      <div class="vetro lista c6" style="align-self:start">
        <div class="riga"><div class="ic" style="background:var(--blu)" data-i="schermo"></div><div class="testo"><b>Schermo intero</b><small>Premi Alt+F oppure ferma la duplicazione</small></div>
          <label class="sw"><input type="checkbox" data-conf="schermo_intero"><span></span></label></div>
        <div class="riga"><div class="ic" style="background:var(--viola)" data-i="pc"></div><div class="testo"><b>Nome del PC</b><small>Come lo vede il Pixel</small></div>
          <input class="input" data-conf-testo="nome_pc" maxlength="40"></div>
      </div>
      <div class="c12"><details><summary>Dettagli tecnici ›</summary><pre class="log" id="schermoLog"></pre></details></div>
    </div>
  </section>

  <!-- ======================= CONDIVIDI ======================= -->
  <section class="pagina" data-p="condividi" data-t="Condividi">
    <div class="titolo"><h1>Condividi</h1><p>File e testi tra il Pixel e questo PC. Sul Pixel tieni aperta l'app LocalSend.</p></div>
    <div id="avvisoLS2"></div>
    <div class="griglia">
      <div class="vetro card c12">
        <h2>Vicino a te <span class="dx"><span class="link" id="cerca">Cerca di nuovo</span></span></h2>
        <div class="vicini" id="vicini"></div>
        <details style="margin-top:10px"><summary>Non lo trovi? Inserisci l'indirizzo IP del Pixel ›</summary>
          <div style="display:flex;gap:10px;margin-top:10px;align-items:center;flex-wrap:wrap">
            <input class="input" id="ipManuale" placeholder="es. 192.168.1.50" style="text-align:left">
            <button class="btn piccolo" id="ipVai">Collega</button>
            <small class="vuoto">Lo trovi nell'app LocalSend del Pixel, scheda Ricevi.</small>
          </div></details>
      </div>
      <div class="vetro card c7">
        <h2>Da inviare</h2>
        <div class="drop" id="condDrop">
          <div class="ico" data-i="su"></div>
          <b>Trascina qui i file</b>
          <small>o <span class="link">sceglili</span> · puoi anche trascinarli direttamente sul dispositivo</small>
        </div>
        <div class="coda" id="coda"></div>
        <div style="display:flex;margin-top:14px"><button class="btn primario" id="inviaCoda" style="margin-left:auto" disabled><span data-i="invia"></span><span id="inviaCodaT">Invia</span></button></div>
      </div>
      <div class="vetro card c5">
        <h2>Trasferimenti</h2>
        <div id="trasferimenti"></div>
      </div>
      <div class="vetro card c12">
        <h2>Messaggi ricevuti</h2>
        <div id="messaggi"></div>
      </div>
      <div class="c12"><div class="sezione" style="display:flex">Ricevuti sul PC <span style="margin-left:auto;text-transform:none;letter-spacing:0" class="link" data-azione="apriCartella">Apri cartella</span></div>
        <div class="vetro lista" id="ricevuti"></div>
        <p class="vuoto" style="margin:10px 8px">Suggerimento: trascina una riga sul desktop o in un'altra app per copiarla lì.</p>
      </div>
    </div>
  </section>

  <!-- ======================= PIXEL (ADB) ======================= -->
  <section class="pagina" data-p="pixel" data-t="Pixel">
    <div class="titolo"><h1>Pixel</h1><p>Collegato tramite ADB: batteria, spazio e le tue foto sul PC.</p></div>
    <div id="cavo"></div>
  </section>

  <!-- ======================= IMPOSTAZIONI ======================= -->
  <section class="pagina" data-p="impostazioni" data-t="Impostazioni">
    <div class="titolo"><h1>Impostazioni</h1></div>

    <div class="sezione">Aspetto</div>
    <div class="vetro lista">
      <div class="riga"><div class="ic" style="background:#5e5ce6" data-i="luna"></div><div class="testo"><b>Tema</b></div>
        <div class="seg" id="segTema"><button data-v="auto">Automatico</button><button data-v="chiaro">Chiaro</button><button data-v="scuro">Scuro</button></div></div>
      <div class="riga" style="flex-wrap:wrap"><div class="ic" style="background:linear-gradient(160deg,#64d2ff,#0a84ff)" data-i="goccia"></div><div class="testo"><b>Vetro</b><small>Più trasparente o più colorato e leggibile</small></div>
        <div class="cursore" style="flex-basis:100%;padding:4px 0 6px 46px"><small>Trasparente</small><input type="range" min="0" max="100" id="vetro"><small>Colorato</small></div></div>
    </div>

    <div class="sezione">Ricezione</div>
    <div class="vetro lista">
      <div class="riga"><div class="ic" style="background:var(--viola)" data-i="pc"></div><div class="testo"><b>Nome del PC</b><small>Come appare sul Pixel in LocalSend</small></div>
        <input class="input" data-conf-testo="nome_pc" maxlength="40"></div>
      <div class="riga cliccabile" data-azione="scegliCartella" data-q="file"><div class="ic" style="background:var(--blu)" data-i="cartella"></div><div class="testo"><b>Cartella dei file ricevuti</b><small id="impCartella"></small></div><span class="link">Cambia</span></div>
      <div class="riga cliccabile" data-azione="scegliCartella" data-q="foto"><div class="ic" style="background:var(--arancio)" data-i="foto"></div><div class="testo"><b>Cartella delle foto importate</b><small id="impFoto"></small></div><span class="link">Cambia</span></div>
      <div class="riga"><div class="ic" style="background:var(--verde)" data-i="scudo"></div><div class="testo"><b>Accetta subito dai miei dispositivi</b><small>Niente conferma per i dispositivi che hai segnato come tuoi</small></div>
        <label class="sw"><input type="checkbox" data-conf="auto_accetta"><span></span></label></div>
      <div class="riga"><div class="ic" style="background:var(--rosso)" data-i="campana"></div><div class="testo"><b>Notifiche</b><small>Avvisa quando arriva qualcosa</small></div>
        <label class="sw"><input type="checkbox" data-conf="notifiche"><span></span></label></div>
    </div>
    <div class="vetro card"><h2>Il mio Pixel</h2><p>Per aggiungere ADB al Pixel già associato:</p><button class="btn" id="configurePixel">Configura Pixel</button><p>Per cambiare telefono o PIN, chiudi l’app ed esegui lo script con reset-pairing. I file già salvati restano nelle loro cartelle.</p></div><div id="fidati"></div>

    <div class="sezione">Avvio</div>
    <div class="vetro lista">
      <div class="riga"><div class="ic" style="background:#8e8e93" data-i="luna"></div><div class="testo"><b>Resta pronta in background</b><small>Chiudendo la finestra puoi continuare a ricevere dal Pixel</small></div>
        <label class="sw"><input type="checkbox" data-conf="background"><span></span></label></div>
      <div class="riga"><div class="ic" style="background:#30b0c7" data-i="accendi"></div><div class="testo"><b>Avvia all'accensione del PC</b><small>Parte nascosta, pronta a ricevere</small></div>
        <label class="sw"><input type="checkbox" data-conf="avvio_login"><span></span></label></div>
    </div>

    <div class="sezione" style="display:flex">Componenti <span style="margin-left:auto;text-transform:none;letter-spacing:0" class="link" id="installa"></span></div>
    <div class="vetro lista" id="componenti"></div>

    <div class="sezione">Rete</div>
    <div class="vetro lista" id="rete"></div>

    <p class="vuoto" style="text-align:center;margin:30px 0 0" id="versione"></p>
  </section>
</main>

<nav class="tabbar vetro" id="tabbar">
  <button class="tab on" data-p="home"><span data-i="casa"></span><span>Home</span></button>
  <button class="tab" data-p="schermo"><span data-i="schermo"></span><span>Schermo</span></button>
  <button class="tab" data-p="condividi"><span data-i="frecce"></span><span>Condividi</span><i class="punto" id="puntoCond"></i></button>
  <button class="tab" data-p="pixel"><span data-i="telefono"></span><span>Pixel</span></button>
  <button class="tab" data-p="impostazioni"><span data-i="ingranaggio"></span><span>Impostazioni</span></button>
</nav>

<div class="velo" id="velo"><div class="sheet vetro" id="sheet"></div></div>
<div class="toasts" id="toasts"></div>
<div class="rilascio" id="rilascio"><div class="drop" style="border:0;pointer-events:none"><div class="ico" data-i="su"></div></div><b id="rilascioT">Rilascia per inviare</b><small id="rilascioS"></small></div>
<input type="file" id="sceltaFile" multiple hidden>

<div id="wizard" style="position:fixed;inset:0;z-index:5000;background:var(--bg,#111827);overflow:auto;padding:30px 18px;display:block">
 <div style="max-width:760px;margin:auto">
 <h1>Abbina il tuo Pixel</h1><p style="margin:12px 0 24px">Le funzioni dell’app restano bloccate fino alla conferma. Nessun telefono viene scelto automaticamente.</p>
 <div id="wizardPair">
 <div class="vetro card" style="margin-bottom:16px"><h2>1. Bluetooth — facoltativo</h2>
 <p>Puoi iniziare senza cavo: apri Bluetooth, scegli il Pixel e verifica che il codice coincida sui due schermi. Poi torna qui e aggiorna l’elenco.</p>
 <p>Questo abbinamento non autentica LocalSend o ADB: li confermerai separatamente sullo stesso Pixel.</p>
 <button class="btn" id="wBT">Apri Bluetooth di Fedora</button> <button class="btn" id="wRefresh">Aggiorna elenchi</button>
 <label style="display:block;margin-top:14px">Dispositivo Bluetooth abbinato <select class="input" id="wBTChoice"><option value="">Salta Bluetooth</option></select></label></div>
 <div class="vetro card" style="margin-bottom:16px"><h2>2. Conferma LocalSend sul Pixel</h2>
 <p>Apri LocalSend sul Pixel, attiva la cifratura e usa la stessa rete Wi-Fi di Fedora. Chiudi LocalSend desktop. Seleziona il telefono: riceverà un codice da riportare qui.</p>
 <select class="input" id="wDevice"><option value="">Seleziona esplicitamente il Pixel…</option></select>
 <button class="btn primario" id="wSend">Invia codice al Pixel selezionato</button>
 <details style="margin-top:12px"><summary>Non compare?</summary><p>Controlla il firewall (porta 53317 TCP/UDP) e la rete. Puoi cercare l’IP mostrato da LocalSend sul Pixel.</p><input class="input" id="wIP" placeholder="IP locale del Pixel"><button class="btn" id="wFind">Cerca IP</button></details>
 <label style="display:block;margin-top:14px">Codice del messaggio Pixel Hub ricevuto in LocalSend (non Bluetooth/ADB) <input class="input" id="wCode" inputmode="numeric" maxlength="6" autocomplete="off"></label>
 <label style="display:block;margin-top:14px">Scegli un PIN di ricezione di 6 cifre <input class="input" id="wPIN" type="password" inputmode="numeric" maxlength="6" autocomplete="new-password"></label>
 <p>Conserva questo PIN: LocalSend sul Pixel te lo chiederà quando invii a Fedora. Il PC conserva solo un hash del PIN. I mittenti non associati vengono rifiutati, anche per i testi.</p>
 <p id="wCountdown" role="status"></p><button class="btn primario" id="wConfirm">Conferma questo Pixel</button></div>
 </div>
 <div id="wizardADB" class="vetro card" style="display:none;margin-bottom:16px"><h2>3. Schermo e foto — ADB facoltativo</h2>
 <p id="wPaired"></p><p>Puoi completare usando solo LocalSend. Per scrcpy e DCIM, autorizza ADB via USB oppure usa Debug wireless sullo stesso Pixel. Poi aggiorna e scegli il suo modello e seriale.</p>
 <details><summary>Collega ADB senza cavo</summary><p>Sul Pixel: Opzioni sviluppatore → Debug wireless → Associa dispositivo con codice. La porta di abbinamento e quella di connessione sono diverse; copia entrambe dal telefono.</p>
 <input class="input" id="wPairAddr" placeholder="IP:porta di abbinamento">
 <input class="input" id="wADBCode" type="password" inputmode="numeric" maxlength="6" placeholder="Codice ADB di 6 cifre" autocomplete="off">
 <input class="input" id="wConnectAddr" placeholder="IP:porta di connessione">
 <button class="btn" id="wWiFi">Associa e connetti ADB Wi-Fi</button></details>
 <button class="btn" id="wRefreshADB">Aggiorna dispositivi ADB</button>
 <select class="input" id="wADB"><option value="">Per ora usa solo LocalSend</option></select>
 <p>L’identità ADB viene salvata separatamente. Se colleghi un telefono diverso, schermo e importazione restano bloccati.</p>
 <button class="btn primario" id="wFinish">Completa e apri Pixel Hub</button></div>
 <p id="wStatus" role="status" style="padding:12px;white-space:pre-wrap"></p>
 </div>
</div>

<script>
"use strict";
/* ------------------------------------------------------------ icone (disegnate a mano) */
const P = (d, extra = "") => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" ${extra}>${d}</svg>`;
const I = {
  casa: P('<path d="M3.5 10.5 12 4l8.5 6.5"/><path d="M5.5 9v10.5h13V9"/><path d="M10 19.5v-5h4v5"/>'),
  schermo: P('<rect x="3" y="4.5" width="18" height="12" rx="2.5"/><path d="M8.5 20h7"/><path d="M12 16.5V20"/>'),
  frecce: P('<path d="M7 4v14"/><path d="m3.5 14.5 3.5 3.5 3.5-3.5"/><path d="M17 20V6"/><path d="m13.5 9.5 3.5-3.5 3.5 3.5"/>'),
  telefono: P('<rect x="6.5" y="2.5" width="11" height="19" rx="3"/><path d="M10.5 5.5h3"/>'),
  ingranaggio: P('<circle cx="12" cy="12" r="3"/><path d="M12 2.8v2.4M12 18.8v2.4M21.2 12h-2.4M5.2 12H2.8M18.5 5.5l-1.7 1.7M7.2 16.8l-1.7 1.7M18.5 18.5l-1.7-1.7M7.2 7.2 5.5 5.5"/>'),
  su: P('<path d="M12 15.5V4"/><path d="m7.5 8.5 4.5-4.5 4.5 4.5"/><path d="M4.5 14.5v3a2.5 2.5 0 0 0 2.5 2.5h10a2.5 2.5 0 0 0 2.5-2.5v-3"/>'),
  invia: P('<path d="M4 12 20 4l-6 16-3-7z"/><path d="m11 13 9-9"/>'),
  appunti: P('<rect x="6" y="4.5" width="12" height="16" rx="2.5"/><path d="M9.5 3.5h5v3h-5z"/>'),
  copia: P('<rect x="8.5" y="8.5" width="11" height="11" rx="2.5"/><path d="M15.5 8.5V6.5A2 2 0 0 0 13.5 4.5h-7a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>'),
  play: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/></svg>',
  stop: '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6.5" y="6.5" width="11" height="11" rx="2.5"/></svg>',
  mirror: P('<rect x="3" y="7" width="12" height="9" rx="2"/><path d="M9 4h10a2 2 0 0 1 2 2v7"/>'),
  foto: P('<rect x="3" y="4.5" width="18" height="15" rx="3"/><circle cx="9" cy="10" r="1.8"/><path d="m21 16-5-5-8.5 8.5"/>'),
  video: P('<rect x="3" y="6" width="13" height="12" rx="2.5"/><path d="m16 10.5 5-3v9l-5-3"/>'),
  audio: P('<path d="M9 18V6l11-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>'),
  pdf: P('<path d="M14 3.5H7A2.5 2.5 0 0 0 4.5 6v12A2.5 2.5 0 0 0 7 20.5h10a2.5 2.5 0 0 0 2.5-2.5V9z"/><path d="M14 3.5V9h5.5"/><path d="M8.5 14h7M8.5 17h4.5"/>'),
  file: P('<path d="M14 3.5H7A2.5 2.5 0 0 0 4.5 6v12A2.5 2.5 0 0 0 7 20.5h10a2.5 2.5 0 0 0 2.5-2.5V9z"/><path d="M14 3.5V9h5.5"/>'),
  testo: P('<path d="M5 6.5h14M5 11h14M5 15.5h9"/>'),
  archivio: P('<rect x="3.5" y="4.5" width="17" height="5" rx="1.5"/><path d="M5 9.5v8A2.5 2.5 0 0 0 7.5 20h9a2.5 2.5 0 0 0 2.5-2.5v-8"/><path d="M10 13h4"/>'),
  cartella: P('<path d="M3.5 7.5A2.5 2.5 0 0 1 6 5h3.5l2 2.5H18a2.5 2.5 0 0 1 2.5 2.5v7.5A2.5 2.5 0 0 1 18 20H6a2.5 2.5 0 0 1-2.5-2.5z"/>'),
  x: P('<path d="M6 6l12 12M18 6 6 18"/>'),
  ok: P('<path d="m5 12.5 4.5 4.5L19 7.5"/>'),
  attenzione: P('<path d="M12 4 2.8 19.5h18.4z"/><path d="M12 10v4.5M12 17.2v.1"/>'),
  pc: P('<rect x="3" y="4.5" width="18" height="12" rx="2"/><path d="M2 19.5h20"/>'),
  luna: P('<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>'),
  goccia: P('<path d="M12 3.5s6.5 7 6.5 11a6.5 6.5 0 0 1-13 0c0-4 6.5-11 6.5-11z"/>'),
  scudo: P('<path d="M12 3.5 5 6v5.5c0 4.5 3 7.7 7 9 4-1.3 7-4.5 7-9V6z"/><path d="m9 12 2 2 4-4"/>'),
  campana: P('<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>'),
  accendi: P('<path d="M12 3.5v8"/><path d="M7 6.5a7 7 0 1 0 10 0"/>'),
  wifi: P('<path d="M2.5 9a14 14 0 0 1 19 0"/><path d="M5.5 12.5a9.5 9.5 0 0 1 13 0"/><path d="M8.7 16a5 5 0 0 1 6.6 0"/><circle cx="12" cy="19.2" r=".9" fill="currentColor"/>'),
  cavo: P('<path d="M9 3.5v4M15 3.5v4"/><rect x="6.5" y="7.5" width="11" height="6" rx="2"/><path d="M12 13.5v3a3 3 0 0 1-3 3H7"/>'),
  lucchetto: P('<rect x="5" y="10.5" width="14" height="10" rx="2.5"/><path d="M8.5 10.5V7.5a3.5 3.5 0 0 1 7 0v3"/>'),
  occhio: P('<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="3"/>'),
  messaggio: P('<path d="M20.5 11.5c0 4.1-3.8 7.5-8.5 7.5-1.2 0-2.3-.2-3.3-.6L4 20l1.3-3.6A7 7 0 0 1 3.5 11.5C3.5 7.4 7.3 4 12 4s8.5 3.4 8.5 7.5z"/>'),
  rete: P('<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c2.5 2.5 3.5 5.5 3.5 8.5s-1 6-3.5 8.5c-2.5-2.5-3.5-5.5-3.5-8.5s1-6 3.5-8.5z"/>'),
  mondo: P('<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17"/>'),
};
function icone(root = document) { root.querySelectorAll("[data-i]").forEach(e => { if (!e.dataset.done) { e.innerHTML = I[e.dataset.i] || ""; e.dataset.done = 1; } }); }

/* ------------------------------------------------------------ utilità */
const $ = s => document.querySelector(s);
const T = new URLSearchParams(location.search).get("t") || "";
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const dim = n => { n = +n || 0; const u = ["B", "KB", "MB", "GB", "TB"]; let i = 0; while (n >= 1024 && i < 4) { n /= 1024; i++; } return (i ? n.toFixed(n < 10 ? 1 : 0) : n) + " " + u[i]; };
const fa = t => { const s = Date.now() / 1000 - t; if (s < 60) return "adesso"; if (s < 3600) return Math.floor(s / 60) + " min fa"; if (s < 86400) return Math.floor(s / 3600) + " h fa";
  const d = new Date(t * 1000); return d.toLocaleDateString("it-IT", { day: "numeric", month: "short" }); };
const iniziale = s => (String(s || "?").replace(/^(pixel|android)\s+(di\s+)?/i, "").trim()[0] || "?").toUpperCase();
function set(el, html) { if (typeof el === "string") el = $(el); if (el && el._h !== html) { el._h = html; el.innerHTML = html; icone(el); } }
async function api(path, body) {
  try {
    const r = await fetch("/api/" + path, { method: "POST", headers: { "X-Token": T, "Content-Type": "application/json" }, body: JSON.stringify(body || {}) });
    return await r.json();
  } catch (e) { return { ok: false }; }
}
function toast(testo, tipo = "ok") {
  const t = document.createElement("div");
  t.className = "toast vetro";
  t.innerHTML = (tipo === "ok" ? `<span style="color:var(--verde)">${I.ok}</span>` : `<span style="color:var(--arancio)">${I.attenzione}</span>`) + esc(testo);
  $("#toasts").appendChild(t);
  setTimeout(() => { t.style.transition = ".3s"; t.style.opacity = 0; t.style.transform = "translateY(-10px)"; }, 2800);
  setTimeout(() => t.remove(), 3200);
}
const MODELLI = {};

/* ------------------------------------------------------------ stato */
let S = null, visto = Date.now() / 1000, pagina = "home", destSel = null, coda = [], ultimoPoll = 0, sheetId = null;
const iconaTipo = t => I[{ foto: "foto", video: "video", audio: "audio", pdf: "pdf", testo: "testo", archivio: "archivio", cartella: "cartella" }[t] || "file"];
const coloreTipo = t => ({ foto: "#ff9f0a", video: "#ff375f", audio: "#bf5af2", pdf: "#ff453a", testo: "#64d2ff", archivio: "#a2845e", cartella: "#0a84ff" }[t] || "#8e8e93");

function vicini() { return (S?.ls.dispositivi || []).filter(d => d.vicino); }
function destinazione() {
  if (!S?.pairing.ready) return null;
  return vicini().find(d => d.fp === S.pairing.fp) || null;
}

/* ------------------------------------------------------------ navigazione */
function vai(p) {
  pagina = p;
  document.querySelectorAll(".pagina").forEach(s => s.classList.toggle("on", s.dataset.p === p));
  document.querySelectorAll(".tab").forEach(t => t.classList.toggle("on", t.dataset.p === p));
  $("#barraTitolo").textContent = document.querySelector(`.pagina[data-p="${p}"]`).dataset.t;
  $("#main").scrollTo({ top: 0, behavior: "instant" });
  try { localStorage.setItem("hub.pagina", p); } catch (e) {}
  if (S) render();
}
document.querySelectorAll(".tab").forEach(t => t.onclick = () => vai(t.dataset.p));
$("#main").addEventListener("scroll", () => $("#barra").classList.toggle("su", $("#main").scrollTop > 40));

/* ------------------------------------------------------------ tema e vetro */
function applicaAspetto() {
  const c = S.conf;
  const scuro = c.tema === "scuro" || (c.tema === "auto" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.dataset.tema = scuro ? "scuro" : "chiaro";
  if (!applicaAspetto.trascina) document.documentElement.style.setProperty("--tint", (c.vetro / 100).toFixed(2));
}
matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => S && applicaAspetto());

/* ------------------------------------------------------------ render */
function render() {
  wizardRender();
  applicaAspetto();
  renderHome(); renderSchermo(); renderCondividi(); renderCavo(); renderImpostazioni(); renderSheet();
  if (pagina === "condividi") visto = Date.now() / 1000;
  const pend = S.ls.richieste.filter(r => r.stato === "attesa").length + S.ls.messaggi.filter(m => m.quando > visto).length;
  const p = $("#puntoCond"); p.textContent = pend; p.classList.toggle("on", pend > 0 && pagina !== "condividi");
}

function saluto() {
  const h = new Date().getHours(), n = S.utente ? ", " + S.utente : "";
  return (h < 5 ? "Buonanotte" : h < 13 ? "Buongiorno" : h < 18 ? "Buon pomeriggio" : "Buonasera") + n;
}

function avvisoLS() {
  const ls = S.ls;
  if (ls.attivo) return "";
  const porta = ls.errore === "porta";
  return `<div class="vetro avviso"><div class="ic">${I.attenzione}</div><div class="t">
    <b>${porta ? "Un'altra app sta già ricevendo i file" : "Ricezione file non attiva"}</b>
    <small>${porta ? "Probabilmente LocalSend per PC è aperto: Pixel Hub lo sostituisce. Chiudilo e riprova." : esc(ls.errore || "")}</small></div>
    <button class="btn piccolo primario" data-azione="riprovaLS">${porta ? "Riprova" : "Riprova"}</button></div>`;
}

function renderHome() {
  set("#saluto", esc(saluto()));
  set("#data", esc(new Date().toLocaleDateString("it-IT", { weekday: "long", day: "numeric", month: "long" }).replace(/^./, c => c.toUpperCase())));
  set("#avvisoLS", avvisoLS());

  // card Pixel
  const cv = S.cavo, info = cv.info || {}, dest = destinazione();
  let h;
  if (cv.stato === "collegato") {
    const batt = +info.BatteryCurrentCapacity || 0, carica = info.BatteryIsCharging === "true";
    const tot = +info.TotalDiskCapacity || 0, lib = +info.AmountDataAvailable || 0, usato = tot ? Math.round((tot - lib) / tot * 100) : 0;
    const col = batt <= 20 ? "var(--rosso)" : carica ? "var(--verde)" : "var(--verde)";
    h = `<h2>Il tuo Pixel</h2><div class="pixel"><div class="telefono"></div><div class="info">
      <h3>${esc(info.DeviceName || "Pixel")}</h3><div class="sub">${esc(MODELLI[info.ProductType] || "Pixel")} · Android ${esc(info.ProductVersion)}</div>
      <span class="chip ok"><i></i>Collegato tramite ADB</span>${dest ? ' <span class="chip blu"><i></i>Anche in Wi-Fi</span>' : ""}
      <div class="misure"><div class="anello"><svg viewBox="0 0 44 44"><circle cx="22" cy="22" r="18" stroke="var(--riempi)" stroke-width="5" fill="none"/>
        <circle cx="22" cy="22" r="18" stroke="${col}" stroke-width="5" fill="none" stroke-linecap="round" stroke-dasharray="${(batt / 100 * 113).toFixed(1)} 113"/></svg>
        <div><b>${batt ? batt + "%" : "—"}</b><small>${carica ? "In carica" : "Batteria"}</small></div></div>
        ${tot ? `<div class="spazio"><b>${dim(lib)} liberi</b><div class="barra-prog"><i style="width:${usato}%;background:${usato > 90 ? "var(--rosso)" : "var(--blu)"}"></i></div><small>usati ${dim(tot - lib)} di ${dim(tot)}</small></div>` : ""}
      </div></div></div>`;
  } else if (dest) {
    h = `<h2>Il tuo Pixel</h2><div class="pixel"><div class="telefono"></div><div class="info">
      <h3>${esc(dest.alias)}</h3><div class="sub">${esc(dest.modello || "Pixel")} · via ${S.rete.hotspot ? "hotspot" : "Wi-Fi"}</div>
      <span class="chip ok"><i></i>Vicino e pronto</span>
      <p class="vuoto" style="margin-top:14px">Trascina file qui sotto per mandarglieli. Collega il cavo per vedere batteria e spazio.</p></div></div>`;
  } else {
    h = `<h2>Il tuo Pixel</h2><div class="pixel"><div class="telefono spento"></div><div class="info">
      <h3>Non lo vedo</h3><div class="sub">Due modi per collegarlo:</div>
      <div class="passi" style="gap:10px"><div class="passo"><div class="n" style="background:var(--verde)">${I.wifi.replace("<svg", '<svg width="16" height="16"')}</div><div><b>Apri LocalSend sul Pixel</b><small>Stessa Wi-Fi o hotspot del telefono</small></div></div>
      <div class="passo"><div class="n" style="background:var(--arancio)">${I.cavo.replace("<svg", '<svg width="16" height="16"')}</div><div><b>Oppure collega il cavo</b><small>Poi accetta «Consenti debug USB» sul telefono</small></div></div></div>
      </div></div>`;
  }
  set("#cardPixel", h);

  // mirror
  const sc = S.schermo;
  let m;
  if (!sc.installato) m = `<div class="tasto-tondo" style="background:var(--riempi);color:var(--txt3)">${I.mirror}</div><b>Da installare</b><small>Serve scrcpy: lo installi da Impostazioni › Componenti.</small>`;
  else if (sc.connesso) m = `<button class="tasto-tondo attivo" data-azione="schermo">${I.stop}</button><b>Schermo collegato</b><small>Lo schermo del Pixel è nella sua finestra. Tocca per fermare.</small>`;
  else if (sc.attivo) m = `<button class="tasto-tondo attivo" data-azione="schermo">${I.stop}</button><b>Avvio scrcpy…</b><small>Controlla il Pixel e i dettagli in Schermo.</small>`;
  else m = `<button class="tasto-tondo" data-azione="schermo">${I.play}</button><b>Schermo Pixel</b><small>Collega il Pixel con Debug USB e premi Avvia.</small>`;
  set("#homeMirror", m);

  set("#homeDest", dest ? `<span class="chip blu"><i></i>a ${esc(dest.alias)}</span>` : `<span class="chip warn"><i class="pulsa"></i>apri LocalSend sul Pixel</span>`);
  set("#homeInvii", listaInvii(S.ls.invii.slice(0, 2), true));

  const msg = S.ls.messaggi[0];
  set("#homeMsg", msg ? bolla(msg) : "");

  const rec = S.recenti.slice(0, 12);
  set("#homeRecenti", rec.length ? rec.map(tile).join("") : `<p class="vuoto">Quando il Pixel ti manda qualcosa, lo trovi qui. Sul Pixel: Condividi › LocalSend › «${esc(S.conf.nome_pc)}».</p>`);
}

function bolla(m) {
  return `<div class="bolla">${esc(m.testo)}</div><div class="bolla-meta">${I.messaggio.replace("<svg", '<svg width="14" height="14"')} ${esc(m.alias)} · ${fa(m.quando)}
    <span class="dx"><button class="btn piccolo" data-azione="copia" data-id="${esc(m.id)}">${I.copia}Copia</button>
    ${/^https?:\/\/\S+$/.test(m.testo.trim()) ? `<button class="btn piccolo" data-azione="apriLink" data-id="${esc(m.id)}">${I.mondo}Apri</button>` : ""}</span></div>`;
}

function tile(f) {
  const img = f.tipo === "foto" && !/\.hei[cf]$/i.test(f.nome);
  const sty = img ? `style="background-image:url('/api/anteprima?t=${encodeURIComponent(T)}&path=${encodeURIComponent(f.path)}')"` : `style="color:${coloreTipo(f.tipo)}"`;
  return `<div class="tile" draggable="true" data-file="${esc(f.path)}" title="${esc(f.nome)}"><div class="img" ${sty}>${img ? "" : iconaTipo(f.tipo)}</div>
    <b>${esc(f.nome)}</b><small>${f.dir ? "Cartella" : dim(f.size)} · ${fa(f.mtime)}</small></div>`;
}

function listaInvii(lista, compatta) {
  if (!lista.length) return compatta ? "" : `<p class="vuoto">Nessun trasferimento in corso.</p>`;
  return `<div style="margin-top:${compatta ? 14 : 0}px">` + lista.map(j => {
    const pct = j.totale ? Math.round(j.inviati / j.totale * 100) : 0;
    const st = { attesa: "Aspetto che accetti sul Pixel…", invio: `Invio… ${pct}% · ${dim(j.inviati)} di ${dim(j.totale)}`, fatto: "Inviato ✓", errore: j.errore, annullato: "Annullato" }[j.stato];
    const col = j.stato === "errore" ? "var(--rosso)" : j.stato === "fatto" ? "var(--verde)" : "var(--blu)";
    const nome = j.testo ? "Testo" : j.n === 1 ? j.nomi[0] : `${j.n} file`;
    return `<div class="trasf"><div class="ic" style="background:color-mix(in srgb,${col} 15%,transparent);color:${col}">${j.stato === "fatto" ? I.ok : j.stato === "errore" ? I.attenzione : I.su}</div>
      <div class="t"><b>${esc(nome)} → ${esc(j.verso)}</b><small style="${j.stato === "errore" ? "color:var(--rosso)" : ""}">${esc(st)}</small>
      ${j.stato === "invio" || j.stato === "attesa" ? `<div class="barra-prog"><i style="width:${j.stato === "attesa" ? 3 : pct}%"></i></div>` : ""}</div>
      ${j.stato === "invio" || j.stato === "attesa" ? `<button class="btn icona piccolo" data-azione="annullaInvio" data-id="${j.id}" title="Annulla">${I.x}</button>` : ""}</div>`;
  }).join("") + "</div>";
}

function listaRicezioni() {
  return S.ls.richieste.filter(r => r.stato === "ricezione" || r.stato === "accettata").map(r => {
    const pct = r.totale ? Math.round(r.byte / r.totale * 100) : 0;
    return `<div class="trasf"><div class="ic" style="background:color-mix(in srgb,var(--verde) 15%,transparent);color:var(--verde)">${I.su.replace("<svg", '<svg style="transform:rotate(180deg)"')}</div>
      <div class="t"><b>Da ${esc(r.alias)} · ${r.file.length} file</b><small>Ricevo… ${pct}% · ${dim(r.byte)} di ${dim(r.totale)}</small><div class="barra-prog"><i style="width:${pct}%;background:var(--verde)"></i></div></div></div>`;
  }).join("");
}

function renderSchermo() {
  const sc = S.schermo, nome = S.conf.nome_pc;
  set("#passo3", "Premi Avvia in questa pagina");
  $("#onde").classList.toggle("on", sc.attivo && !sc.connesso);
  $("#telSchermo").classList.toggle("spento", !sc.connesso);
  let h;
  if (!sc.installato) h = `<h3>Manca scrcpy</h3><p class="vuoto">Riesegui lo script di installazione dal terminale e scegli di installare scrcpy dal repository COPR zeno/scrcpy.</p>`;
  else if (sc.attivo) h = `<h3>${sc.connesso ? "Schermo collegato" : "Avvio scrcpy…"}</h3><p class="vuoto">Schermo e controllo del Pixel in una finestra separata. Se non compare, leggi i dettagli tecnici qui sotto.</p><button class="btn rosso grande" data-azione="schermo">${I.stop}Ferma</button>`;
  else h = `<h3>Il Pixel sul PC</h3><p class="vuoto">Collega un solo telefono, abilita Debug USB e autorizza Fedora sul Pixel. Poi premi Avvia.</p><button class="btn primario grande" data-azione="schermo">${I.play}Avvia</button>`;
  set("#schermoStato", h);
  const pre = $("#schermoLog"), txt = sc.log.join("\n") || "Nessun messaggio.";
  if (pre.textContent !== txt) { pre.textContent = txt; pre.scrollTop = pre.scrollHeight; }
}

function renderCondividi() {
  set("#avvisoLS2", avvisoLS());
  const v = vicini(), dest = destinazione();
  let h = v.map(d => `<div class="disp ${dest && d.fp === dest.fp ? "sel" : ""}" data-fp="${esc(d.fp)}" title="${esc(d.ip)}">
      <div class="av" style="${d.tipo === "desktop" ? "background:linear-gradient(160deg,#9aa0a6,#5f6368)" : ""}">${d.tipo === "desktop" ? I.pc : esc(iniziale(d.alias))}</div>
      <b>${esc(d.alias)}</b><small>${d.fidato ? "Tuo · " : ""}${esc(d.modello || (d.tipo === "mobile" ? "Telefono" : d.tipo))}</small></div>`).join("");
  h += `<div class="disp" style="cursor:default"><div class="radar">${I.telefono}</div><b>${v.length ? "Cerco altri…" : "Cerco…"}</b><small>${v.length ? "" : "Apri LocalSend sul Pixel"}</small></div>`;
  set("#vicini", h);

  set("#coda", coda.map((f, i) => `<span class="chip">${iconaTipo(f.tipo).replace("<svg", '<svg width="14" height="14"')}${esc(f.nome)} <em style="font-style:normal;opacity:.6">${dim(f.size)}</em><button data-azione="togliCoda" data-i="${i}" title="Togli">${I.x}</button></span>`).join(""));
  const b = $("#inviaCoda");
  b.disabled = !coda.length || !dest;
  $("#inviaCodaT").textContent = !coda.length ? "Invia" : dest ? `Invia a ${dest.alias}` : "Nessun dispositivo vicino";

  const t = listaRicezioni() + (S.ls.invii.length ? listaInvii(S.ls.invii.slice(0, 6), false) : "");
  set("#trasferimenti", t || `<p class="vuoto">Nessun trasferimento in corso.</p>`);

  set("#messaggi", S.ls.messaggi.length ? S.ls.messaggi.slice(0, 6).map(bolla).join("")
    : `<p class="vuoto">I testi che mandi dal Pixel con LocalSend (scheda Invia › Testo) compaiono qui, pronti da copiare.</p>`);

  set("#ricevuti", S.recenti.length ? S.recenti.map(f => `<div class="riga cliccabile" draggable="true" data-file="${esc(f.path)}">
      <div class="ic" style="background:${coloreTipo(f.tipo)}">${iconaTipo(f.tipo)}</div>
      <div class="testo"><b>${esc(f.nome)}</b><small>${f.dir ? "Cartella" : dim(f.size)} · ${fa(f.mtime)}</small></div>
      <button class="btn icona piccolo" data-azione="mostra" data-path="${esc(f.path)}" title="Mostra nella cartella">${I.cartella}</button></div>`).join("")
    : `<div class="riga"><div class="testo"><b style="color:var(--txt2)">Ancora niente</b><small>Cartella: ${esc(S.conf.cartella)}</small></div></div>`);
}

function renderCavo() {
  const cv = S.cavo, info = cv.info || {}, imp = cv.importa;
  let h;
  if (cv.stato === "mancante") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="telefono spento" style="margin:0 auto 20px"></div><h3 style="font-size:22px">Mancano gli strumenti USB</h3><p class="vuoto" style="margin:6px 0 18px">Installali da Impostazioni › Componenti.</p><button class="btn primario" data-azione="vai" data-p="impostazioni">Vai a Impostazioni</button></div>`;
  else if (cv.stato === "errore") h = `<div class="vetro card"><h3>Controlla il collegamento</h3><p>${esc(cv.errore)}</p></div>`;
  else if (cv.stato === "assente") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="telefono spento" style="margin:0 auto 20px"></div><h3 style="font-size:22px">Collega il Pixel con il cavo</h3><p class="vuoto" style="margin:6px auto 0;max-width:380px">Abilita Debug USB nelle Opzioni sviluppatore, collega un cavo dati e accetta «Consenti debug USB» sul Pixel.</p></div>`;
  else if (cv.stato === "fiducia") h = `<div class="vetro card" style="text-align:center;padding:50px 22px"><div class="tasto-tondo" style="margin:0 auto 20px;background:linear-gradient(160deg,#ffb340,var(--arancio))">${I.lucchetto}</div><h3 style="font-size:22px">Autorizza questo computer</h3><p class="vuoto" style="margin:6px auto 18px;max-width:380px">Sul Pixel sblocca lo schermo e accetta «Consenti debug USB». Poi premi Riprova.</p><button class="btn primario" data-azione="associa">Riprova</button></div>`;
  else {
    const batt = +info.BatteryCurrentCapacity || 0, carica = info.BatteryIsCharging === "true";
    const tot = +info.TotalDiskCapacity || 0, lib = +info.AmountDataAvailable || 0, usato = tot ? Math.round((tot - lib) / tot * 100) : 0;
    const pct = imp.tot ? Math.round(imp.fatti / imp.tot * 100) : 0;
    h = `<div class="griglia">
      <div class="vetro card c12"><div class="pixel"><div class="telefono"></div><div class="info">
        <h3 style="font-size:28px">${esc(info.DeviceName || "Pixel")}</h3><div class="sub">${esc(MODELLI[info.ProductType] || info.ProductType || "Pixel")} · Android ${esc(info.ProductVersion)}</div>
        <span class="chip ok"><i></i>Collegato tramite ADB</span></div></div></div>
      <div class="vetro card c6"><h2>Batteria</h2><div class="anello"><svg viewBox="0 0 44 44" style="width:64px;height:64px"><circle cx="22" cy="22" r="18" stroke="var(--riempi)" stroke-width="5" fill="none"/>
        <circle cx="22" cy="22" r="18" stroke="${batt <= 20 ? "var(--rosso)" : "var(--verde)"}" stroke-width="5" fill="none" stroke-linecap="round" stroke-dasharray="${(batt / 100 * 113).toFixed(1)} 113"/></svg>
        <div><b style="font-size:26px">${batt ? batt + "%" : "—"}</b><small>${carica ? "In carica" : "Non in carica"}</small></div></div></div>
      <div class="vetro card c6"><h2>Spazio</h2>${tot ? `<div class="spazio"><b style="font-size:26px">${dim(lib)}</b> <span class="vuoto">liberi</span><div class="barra-prog" style="height:10px"><i style="width:${usato}%;background:${usato > 90 ? "var(--rosso)" : "var(--blu)"}"></i></div><small>usati ${dim(tot - lib)} di ${dim(tot)}</small></div>` : `<p class="vuoto">Non disponibile</p>`}</div>
      <div class="vetro card c8"><h2>Foto e video</h2>
        <p class="vuoto" style="margin-bottom:14px">Importo DCIM locale (salto i file con stesso percorso e dimensione) in <b>${esc(cv.destinazione)}</b>. Sul Pixel non cancello niente.</p>
        ${imp.attivo ? `<div class="barra-prog" style="height:10px"><i style="width:${imp.tot ? pct : 4}%"></i></div><p class="vuoto" style="margin-top:8px">${esc(imp.msg)} ${imp.tot ? `${imp.fatti} di ${imp.tot}` : ""}</p>`
        : `<div style="display:flex;gap:10px;flex-wrap:wrap"><button class="btn primario" data-azione="importa">${I.su.replace("<svg", '<svg style="transform:rotate(180deg)"')}Importa foto e video</button><button class="btn" data-azione="apriCartella" data-q="foto">${I.cartella}Apri la cartella</button></div>
           ${imp.msg ? `<p class="vuoto" style="margin-top:10px">${esc(imp.msg)}</p>` : ""}`}</div>
      <div class="vetro card c4"><h2>File</h2><p class="vuoto" style="margin-bottom:14px">Sul Pixel seleziona USB › Trasferimento file. Poi apri il telefono in File. Le cartelle private delle app non sono accessibili.</p><button class="btn" data-azione="cavoApri">${I.cartella}Apri in File</button></div>
    </div>`;
  }
  set("#cavo", h);
}

function renderImpostazioni() {
  const c = S.conf;
  document.querySelectorAll("[data-conf]").forEach(i => { if (document.activeElement !== i) i.checked = !!c[i.dataset.conf]; });
  document.querySelectorAll("[data-conf-testo]").forEach(i => { if (document.activeElement !== i) i.value = c[i.dataset.confTesto]; });
  document.querySelectorAll("#segTema button").forEach(b => b.classList.toggle("on", b.dataset.v === c.tema));
  if (!applicaAspetto.trascina) $("#vetro").value = c.vetro;
  set("#impCartella", esc(c.cartella));
  set("#impFoto", esc(c.cartella_foto));
  set("#fidati", S.fidati.length ? `<div class="sezione">I miei dispositivi</div><div class="vetro lista">` + S.fidati.map(f =>
    `<div class="riga"><div class="ic" style="background:linear-gradient(160deg,#7aa2ff,#5e5ce6)">${I.telefono}</div><div class="testo"><b>${esc(f.alias)}</b><small>Accetto i file senza chiedere</small></div>
     <button class="btn piccolo" data-azione="togliFidato" data-fp="${esc(f.fp)}">Rimuovi</button></div>`).join("") + "</div>" : "");
  set("#componenti", S.componenti.map(k => `<div class="riga"><div class="ic" style="background:${k.ok ? "var(--verde)" : "var(--arancio)"}">${k.ok ? I.ok : I.attenzione}</div>
    <div class="testo"><b>${esc(k.nome)}</b><small>${esc(k.desc)}</small></div><span class="val">${k.ok ? "Pronto" : "Manca"}</span></div>`).join(""));
  const manca = S.componenti.some(k => !k.ok), ins = S.installazione;
  set("#installa", ins.attiva ? "Installo…" : manca ? "Installa mancanti" : esc(ins.msg || ""));
  $("#installa").dataset.azione = manca && !ins.attiva ? "installa" : "";
  const r = S.rete;
  set("#rete", `<div class="riga"><div class="ic" style="background:var(--blu)">${I.wifi}</div><div class="testo"><b>Connessione</b></div><span class="val">${esc(r.nome || "—")}${r.hotspot ? " · hotspot Pixel" : ""}</span></div>
    <div class="riga"><div class="ic" style="background:#8e8e93">${I.rete}</div><div class="testo"><b>Indirizzo del PC</b></div><span class="val" style="user-select:text">${esc(r.ip.join(", ") || "—")}</span></div>
    <div class="riga"><div class="ic" style="background:${S.ls.attivo ? "var(--verde)" : "var(--rosso)"}">${I.lucchetto}</div><div class="testo"><b>Ricezione LocalSend</b><small>${S.ls.protocollo === "https" ? "Connessione cifrata" : "Connessione non cifrata"} · porta 53317</small></div><span class="val">${S.ls.attivo ? "Attiva" : "Spenta"}</span></div>`);
  set("#versione", `Pixel Hub ${esc(S.versione)} · fatto con scrcpy, LocalSend e ADB`);
}

/* ------------------------------------------------------------ sheet richieste */
function renderSheet() {
  const r = S.ls.richieste.find(x => x.stato === "attesa");
  const velo = $("#velo");
  if (!r) { if (sheetId) { velo.classList.remove("on"); sheetId = null; } return; }
  if (sheetId === r.id) return;
  sheetId = r.id;
  const n = r.file.length;
  set("#sheet", `<div class="av">${esc(iniziale(r.alias))}</div>
    <h3>${esc(r.alias)}</h3><p>vuole inviarti ${n === 1 ? "un file" : n + " file"} · ${dim(r.totale)}</p>
    <div class="files">${r.file.slice(0, 50).map(f => `<div>${iconaTipo(f.tipo)}<span>${esc(f.nome)}</span><em>${dim(f.size)}</em></div>`).join("")}${n > 50 ? `<div><span>…e altri ${n - 50}</span></div>` : ""}</div>
    <div class="azioni"><button class="btn" data-azione="rifiuta">Rifiuta</button><button class="btn primario" data-azione="accetta">Accetta</button></div>
    <label><span class="sw" style="transform:scale(.8)"><input type="checkbox" id="fidatiCk"><span></span></span>È il mio Pixel: accetta sempre senza chiedere</label>`);
  velo.classList.add("on");
}

/* ------------------------------------------------------------ azioni */
const AZ = {
  async schermo() { const on = S.schermo.attivo; const r = await api("schermo", { azione: on ? "ferma" : "avvia" }); if (!on && !r.ok) toast("Non riesco ad avviare scrcpy", "warn"); aggiorna(); },
  vai(e) { vai(e.dataset.p); },
  async riprovaLS() { const r = await api("ls/riprova"); toast(r.ok ? "Ricezione attiva" : "Ancora occupata: chiudi LocalSend per PC", r.ok ? "ok" : "warn"); aggiorna(); },
  async accetta() { await api("richiesta", { id: sheetId, accetta: true, fidati: $("#fidatiCk")?.checked }); $("#velo").classList.remove("on"); toast("Ricevo i file…"); aggiorna(); },
  async rifiuta() { await api("richiesta", { id: sheetId, accetta: false }); $("#velo").classList.remove("on"); aggiorna(); },
  async copia(e) { const m = S.ls.messaggi.find(x => x.id === e.dataset.id); if (m) { await copiaTesto(m.testo); toast("Copiato negli appunti"); } },
  apriLink(e) { const m = S.ls.messaggi.find(x => x.id === e.dataset.id); if (m) location.href = m.testo.trim(); },
  async annullaInvio(e) { await api("invio/annulla", { id: e.dataset.id }); },
  togliCoda(e) { coda.splice(+e.dataset.i, 1); renderCondividi(); },
  async mostra(e, ev) { ev.stopPropagation(); await api("mostra", { path: e.dataset.path }); },
  async apriCartella(e) { await api("apri-cartella", { quale: e.dataset.q === "foto" ? "foto" : "file" }); },
  async scegliCartella(e) { const r = await api("scegli-cartella", { quale: e.dataset.q }); if (r.ok) { toast("Cartella aggiornata"); aggiorna(); } },
  async togliFidato(e) { await api("fidato/rimuovi", { fp: e.dataset.fp }); aggiorna(); },
  async installa() { await api("installa"); toast("Ti chiedo la password di Fedora…"); aggiorna(); },
  async importa() { const r = await api("cavo/importa"); if (!r.ok) toast("Non riesco: sblocca il Pixel e riprova", "warn"); aggiorna(); },
  async cavoApri() { await api("cavo/apri"); },
  async associa() { const r = await api("cavo/associa"); toast(r.ok ? "Autorizzato" : "Sblocca il Pixel e accetta «Consenti debug USB»", r.ok ? "ok" : "warn"); aggiorna(); },
};
document.addEventListener("click", ev => {
  const el = ev.target.closest("[data-azione]");
  if (el && el.dataset.azione && AZ[el.dataset.azione]) { AZ[el.dataset.azione](el, ev); return; }
  const d = ev.target.closest(".disp[data-fp]");
  if (d) { destSel = d.dataset.fp; render(); return; }
  const f = ev.target.closest("[data-file]");
  if (f) api("apri", { path: f.dataset.file });
});
async function copiaTesto(t) {
  try { await navigator.clipboard.writeText(t); return; } catch (e) {}
  const a = document.createElement("textarea"); a.value = t; document.body.appendChild(a); a.select(); document.execCommand("copy"); a.remove();
}

/* impostazioni */
document.querySelectorAll("[data-conf]").forEach(i => i.addEventListener("change", async () => {
  await api("conf", { [i.dataset.conf]: i.checked }); aggiorna();
}));
document.querySelectorAll("[data-conf-testo]").forEach(i => {
  const salva = async () => { const v = i.value.trim(); if (v && v !== S.conf[i.dataset.confTesto]) { await api("conf", { [i.dataset.confTesto]: v }); toast("Nome salvato"); aggiorna(); } };
  i.addEventListener("change", salva);
  i.addEventListener("keydown", e => { if (e.key === "Enter") i.blur(); });
});
document.querySelectorAll("#segTema button").forEach(b => b.onclick = async () => { S.conf.tema = b.dataset.v; render(); await api("conf", { tema: b.dataset.v }); });
const vetro = $("#vetro");
vetro.addEventListener("input", () => { applicaAspetto.trascina = true; document.documentElement.style.setProperty("--tint", (vetro.value / 100).toFixed(2)); });
vetro.addEventListener("change", async () => { await api("conf", { vetro: +vetro.value }); S.conf.vetro = +vetro.value; applicaAspetto.trascina = false; });

/* testo */
$("#homeIncolla").onclick = async () => {
  try { $("#homeTesto").value = await navigator.clipboard.readText(); } catch (e) { $("#homeTesto").focus(); toast("Premi Ctrl+V per incollare", "warn"); }
};
$("#homeInviaTesto").onclick = async () => {
  const t = $("#homeTesto").value; const d = destinazione();
  if (!t.trim()) { $("#homeTesto").focus(); return; }
  if (!d) { toast("Apri LocalSend sul Pixel", "warn"); return; }
  const r = await api("invia", { fp: d.fp, testo: t });
  if (r.ok) { toast("Testo inviato a " + d.alias); $("#homeTesto").value = ""; } else toast("Invio non riuscito", "warn");
  aggiorna();
};

/* cerca */
$("#cerca").onclick = async () => { toast("Cerco dispositivi…"); const r = await api("cerca"); aggiorna(); if (r.trovati) toast("Trovato!"); };
$("#ipVai").onclick = async () => {
  const ip = $("#ipManuale").value.trim();
  if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(ip)) { toast("Scrivi un indirizzo come 192.168.1.50", "warn"); return; }
  const r = await api("cerca", { ip }); toast(r.trovati ? "Collegato" : "Non risponde: LocalSend è aperto sul Pixel?", r.trovati ? "ok" : "warn"); aggiorna();
};

/* ------------------------------------------------------------ file: scegli e trascina */
let scopoScelta = "coda";
$("#homeDrop").onclick = () => { scopoScelta = "subito"; $("#sceltaFile").click(); };
$("#condDrop").onclick = () => { scopoScelta = "coda"; $("#sceltaFile").click(); };
$("#sceltaFile").onchange = e => { prendiFile([...e.target.files], scopoScelta === "subito" ? destinazione() : null, scopoScelta === "subito"); e.target.value = ""; };

async function prendiFile(files, dest, subito) {
  if (!files.length) return;
  if (subito && !dest) { toast("Nessun Pixel vicino: li metto in coda", "warn"); subito = false; }
  const pronti = [];
  for (const f of files) {
    if (!f.size && !f.type) { toast(`«${f.name}» è una cartella: comprimila e riprova`, "warn"); continue; }
    try {
      const r = await fetch(`/api/prepara?nome=${encodeURIComponent(f.name)}`, { method: "POST", headers: { "X-Token": T }, body: f });
      const j = await r.json();
      if (j.path) pronti.push({ nome: j.nome, size: j.size, path: j.path, tipo: tipoDa(j.nome) });
    } catch (e) { toast("Non riesco a leggere " + f.name, "warn"); }
  }
  if (!pronti.length) return;
  if (subito) {
    const r = await api("invia", { fp: dest.fp, percorsi: pronti.map(p => p.path) });
    toast(r.ok ? `Invio a ${dest.alias}: accetta sul Pixel` : "Invio non riuscito", r.ok ? "ok" : "warn");
    aggiorna();
  } else { coda.push(...pronti); if (pagina !== "condividi") vai("condividi"); else renderCondividi(); }
}
function tipoDa(n) {
  const e = (n.split(".").pop() || "").toLowerCase();
  if (["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp"].includes(e)) return "foto";
  if (["mov", "mp4", "m4v", "mkv", "webm", "avi"].includes(e)) return "video";
  if (["mp3", "m4a", "wav", "aac", "flac", "ogg"].includes(e)) return "audio";
  if (e === "pdf") return "pdf";
  if (["zip", "rar", "7z", "gz", "tar"].includes(e)) return "archivio";
  return "file";
}
$("#inviaCoda").onclick = async () => {
  const d = destinazione(); if (!d || !coda.length) return;
  const r = await api("invia", { fp: d.fp, percorsi: coda.map(c => c.path) });
  if (r.ok) { toast(`Invio a ${d.alias}: accetta sul Pixel`); coda = []; } else toast("Invio non riuscito", "warn");
  aggiorna();
};

// trascinamento dal desktop dentro l'app
let conta = 0;
const haFile = e => [...(e.dataTransfer?.types || [])].includes("Files");
document.addEventListener("dragenter", e => {
  if (!haFile(e)) return; e.preventDefault(); conta++;
  const d = destinazione();
  $("#rilascioT").textContent = d ? `Rilascia per inviare a ${d.alias}` : "Rilascia per aggiungerli";
  $("#rilascioS").textContent = d ? "oppure lasciali sul dispositivo che preferisci in Condividi" : "Apri LocalSend sul Pixel per inviarli";
  $("#rilascio").classList.add("on");
});
document.addEventListener("dragleave", e => { if (!haFile(e)) return; if (--conta <= 0) { conta = 0; $("#rilascio").classList.remove("on"); } });
document.addEventListener("dragover", e => {
  if (!haFile(e)) return; e.preventDefault();
  document.querySelectorAll(".disp.sopra").forEach(x => x.classList.remove("sopra"));
  const d = e.target.closest?.(".disp[data-fp]"); if (d) d.classList.add("sopra");
});
document.addEventListener("drop", e => {
  if (!haFile(e)) return; e.preventDefault(); conta = 0; $("#rilascio").classList.remove("on");
  const files = [...e.dataTransfer.files];
  const su = e.target.closest?.(".disp[data-fp]");
  document.querySelectorAll(".disp.sopra").forEach(x => x.classList.remove("sopra"));
  if (su) { const d = vicini().find(x => x.fp === su.dataset.fp); prendiFile(files, d, true); return; }
  if (pagina === "condividi") prendiFile(files, null, false);
  else prendiFile(files, destinazione(), true);
});
// trascinamento dall'app verso il desktop
document.addEventListener("dragstart", e => {
  const f = e.target.closest?.("[data-file]"); if (!f) return;
  const uri = "file://" + f.dataset.file.split("/").map(encodeURIComponent).join("/");
  e.dataTransfer.effectAllowed = "copy";
  e.dataTransfer.setData("text/uri-list", uri);
  e.dataTransfer.setData("text/plain", f.dataset.file);
});

/* ------------------------------------------------------------ ciclo */
async function aggiorna() {
  try {
    const r = await fetch("/api/stato", { headers: { "X-Token": T } });
    if (!r.ok) throw 0;
    S = await r.json();
    render();
  } catch (e) { /* il motore si sta riavviando */ }
}
icone();
try { const p = localStorage.getItem("hub.pagina"); if (p && document.querySelector(`.pagina[data-p="${p}"]`)) vai(p); } catch (e) {}
let wizardOpen = false, wizardBusy = false;
function wizardRender() {
  const show = !S.pairing.ready || wizardOpen;
  const ch = S.pairing.challenge || {};
  $('#wConfirm').disabled = wizardBusy || !ch.active;
  $('#wSend').disabled = wizardBusy;
  $('#wCountdown').textContent = ch.active ? `Codice per ${ch.alias}: ${Math.floor(ch.seconds / 60)}:${String(ch.seconds % 60).padStart(2, '0')} rimanenti · ${ch.attempts} tentativi.` : 'Premi Invia codice al Pixel selezionato per iniziare o riprovare.';
  $('#wizard').style.display = show ? 'block' : 'none';
  $('#main').inert = show;
  const paired = !!S.pairing.fp;
  $('#wizardPair').style.display = paired ? 'none' : 'block';
  $('#wizardADB').style.display = paired ? 'block' : 'none';
  $('#wPaired').textContent = paired ? `LocalSend associato: ${S.pairing.alias}. Impronta: ${S.pairing.fp}` : '';
}
function wizardOptions(id, list, key, label, placeholder) {
 const el = $(id), previous = el.value;
 el.replaceChildren(new Option(placeholder, ''));
 for (const item of list) el.add(new Option(label(item), item[key]));
 if ([...el.options].some(o => o.value === previous)) el.value = previous;
}
async function wizardCall(path, data) {
 $('#wStatus').textContent = 'Operazione in corso…';
 const r = await api('pairing/' + path, data);
 $('#wStatus').textContent = r.ok ? 'Operazione completata.' : (r.errore || 'Operazione non riuscita.');
 return r;
}
async function wizardRefresh() {
 const r = await wizardCall('lista');
 if (!r.ok) return;
 wizardOptions('#wBTChoice', r.bluetooth, 'mac', b => `${b.nome} — ${b.mac}`, 'Salta Bluetooth');
 wizardOptions('#wDevice', r.devices, 'fp', d => `${d.alias} — ${d.modello || 'telefono'} — ${d.ip} — ${d.fp.slice(0,12)}`, 'Seleziona esplicitamente il Pixel…');
 wizardOptions('#wADB', r.adb, 'serial', d => `${d.model} — ${d.serial} — identità ${d.hardware}`, 'Per ora usa solo LocalSend');
}
$('#wBT').onclick = () => wizardCall('bluetooth');
$('#wRefresh').onclick = wizardRefresh;
$('#wRefreshADB').onclick = wizardRefresh;
$('#wFind').onclick = async () => { await api('cerca', {ip: $('#wIP').value.trim()}); await wizardRefresh(); };
$('#wSend').onclick = async () => {
 if (wizardBusy) return;
 wizardBusy = true; wizardRender();
 try {
  const r = await wizardCall('inizia', {fp: $('#wDevice').value, bluetooth: $('#wBTChoice').value});
  if (r.ok) $('#wStatus').textContent = `Leggi in LocalSend sul Pixel (${r.alias}) il messaggio “Pixel Hub — codice di associazione”. Riporta quel codice nel primo campo e scegli tu un PIN nel secondo.`;
 } finally { wizardBusy = false; await aggiorna(); }
};
$('#wConfirm').onclick = async () => {
 if (wizardBusy) return;
 wizardBusy = true; wizardRender();
 try {
  const r = await wizardCall('conferma', {code: $('#wCode').value.trim(), pin: $('#wPIN').value});
  if (r.ok) { $('#wCode').value = ''; $('#wPIN').value = ''; await aggiorna(); await wizardRefresh(); }
 } finally { wizardBusy = false; await aggiorna(); }
};
$('#wWiFi').onclick = async () => {
 const r = await wizardCall('wifi', {pair: $('#wPairAddr').value.trim(), connect: $('#wConnectAddr').value.trim(), code: $('#wADBCode').value.trim()});
 $('#wADBCode').value = '';
 if (r.ok) await wizardRefresh();
};
$('#wFinish').onclick = async () => {
 const r = await wizardCall('fine', {serial: $('#wADB').value});
 if (r.ok) {wizardOpen = false; await aggiorna();}
};

$('#configurePixel').onclick = async () => { wizardOpen = true; wizardRender(); await wizardRefresh(); };
aggiorna().then(wizardRefresh);
(function ciclo() { setTimeout(() => aggiorna().finally(ciclo), document.hidden ? 4000 : 1000); })();
document.addEventListener("keydown", e => {
  if (e.ctrlKey && /^[1-5]$/.test(e.key)) vai(["home", "schermo", "condividi", "pixel", "impostazioni"][+e.key - 1]);
});
</script>
</body>
</html>
HTMLEOF
cat > "$DIR/icon.svg" <<'SVGEOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="g" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#5b8def"/><stop offset="1" stop-color="#8a5cf6"/>
    </linearGradient>
  </defs>
  <rect x="8" y="8" width="112" height="112" rx="26" fill="url(#g)"/>
  <rect x="36" y="22" width="40" height="76" rx="9" fill="none" stroke="#fff" stroke-width="6"/>
  <rect x="49" y="28" width="14" height="4" rx="2" fill="#fff"/>
  <path d="M84 52h22m-8-8 8 8-8 8" fill="none" stroke="#fff" stroke-width="6" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M106 76H84m8 8-8-8 8-8" fill="none" stroke="#fff" stroke-width="6" stroke-linecap="round" stroke-linejoin="round" opacity=".75"/>
</svg>
SVGEOF
chmod +x "$DIR/pixel_hub.py"
cp "$DIR/icon.svg" "$ICONS/$ID.svg"
cat > "$APPS/$ID.desktop" <<DEOF
[Desktop Entry]
Type=Application
Name=Pixel Hub
Comment=Pixel e Fedora: schermo, file, foto e testi
Exec=python3 "$DIR/pixel_hub.py"
Icon=$ID
Terminal=false
Categories=Utility;GTK;
Keywords=pixel;android;scrcpy;localsend;foto;
StartupWMClass=$ID
DEOF
update-desktop-database "$APPS" 2>/dev/null || true
python3 -m py_compile "$DIR/pixel_hub.py"
if ! python3 -c "import gi; gi.require_version('WebKit','6.0'); gi.require_version('Adw','1'); from gi.repository import WebKit, Adw"; then
  echo 'La finestra GTK non è disponibile: puoi usare il fallback browser.'
fi
echo '
Installato! Cerca Pixel Hub nel menu: al primo avvio appare il wizard.
Puoi iniziare da Bluetooth, poi confermare LocalSend sullo stesso Pixel.
Per schermo e DCIM: ADB via USB o Debug wireless, entrambi nel wizard.
Chiudi LocalSend desktop: usa la stessa porta del server integrato.
Se il firewall blocca LocalSend, usa il comando firewall ZONA sulla rete fidata.
La configurazione v3 è separata dalle vecchie versioni: nessun dispositivo
precedentemente fidato viene importato automaticamente.
'
cat > "$DIR/LEGGIMI.txt" <<'HELP'
PIXEL HUB 3.1 — FEDORA / PIXEL

Avvia Pixel Hub dal menu. Il wizard blocca le normali funzioni fino alla conferma.
1. Facoltativo: apri Bluetooth dal wizard, abbina il Pixel nelle impostazioni
   Fedora, confronta il codice su entrambi gli schermi. Aggiorna e selezionalo.
   Bluetooth non trasporta i file LocalSend o lo schermo scrcpy e non prova
   automaticamente che un dispositivo LocalSend sia lo stesso telefono.
2. Apri LocalSend sul Pixel con cifratura attiva e stessa rete Wi-Fi. Seleziona
   esplicitamente il telefono nel wizard, inviagli il codice e riportalo sul PC.
   Scegli un PIN di ricezione a 6 cifre: lo dovrai inserire in LocalSend sul Pixel
   quando invii a Fedora. Il sender deve supportare la richiesta PIN del protocollo.
3. Facoltativo: collega ADB via USB (Debug USB) oppure associa via Debug wireless
   con IP, porta di abbinamento e codice; poi IP e porta di connessione.
   Seleziona esplicitamente modello e seriale dello stesso Pixel.
4. Completa e apri l'app. Per aggiungere ADB in seguito: Impostazioni > Configura Pixel.

ISOLAMENTO
Nessuna selezione automatica del primo telefono in LAN. Invii solo al dispositivo
scelto, con controllo del suo certificato sulla connessione HTTPS effettiva.
Nessun fallback a HTTP. Ricezioni file e testi bloccate prima del wizard; dopo,
richiedono fingerprint configurata e PIN corretto. PIN memorizzato come hash con
salt, massimo 10 errori in 5 minuti. Il nome del telefono e il MAC Bluetooth
non sono credenziali LocalSend. Il controllo ricezioni usa PIN + identità dichiarata
dal protocollo: non è autenticazione reciproca mediante certificati client.
Non condividere il PIN con altri dispositivi. La protezione evita selezioni e
trasferimenti accidentali; non sostituisce la sicurezza della rete locale.
ADB accetta solo il seriale hardware selezionato. In presenza di più connessioni
ADB, lascia attiva quella del Pixel. Se il certificato LocalSend cambia, serve
un nuovo pairing esplicito. Foto e file vanno in sottocartelle Pixel-<impronta>.
Non importa né cancella vecchi file, vecchie impostazioni o vecchie chiavi.

DATI LOCALI
Config: ~/.config/pixel-hub-v3/config.json (permessi 600).
Certificato e chiave HTTPS generati sul PC in ~/.cache/pixel-hub-v3, chiave 600.
Token interfaccia e codici pairing generati a runtime. Codici transitori non salvati.
ADB gestisce autonomamente le proprie chiavi in ~/.android; BlueZ gestisce le
proprie associazioni di sistema. Nessuna chiave privata è inclusa nell'installer.

CAMBIA PIXEL / PIN
Chiudi l'app, poi:
  bash installa-pixel-hub-fedora.sh reset-pairing
Richiede conferma RESET; conserva i file. Per revocare anche i pairing di sistema,
dimentica Fedora nel Bluetooth e nel Debug wireless del Pixel.

FIREWALL
  firewall-cmd --get-active-zones
Individua la zona della rete fidata e usa:
  bash installa-pixel-hub-fedora.sh firewall NOME_ZONA
Apre solo 53317 TCP/UDP nella zona scelta. Non disabilita firewall o SELinux.
Se non trovi il Pixel verifica rete ospiti, isolamento Wi-Fi o VPN.

DIAGNOSTICA
  bash installa-pixel-hub-fedora.sh diagnostica
Il comando mostra identificativi locali: oscurali prima di condividere l'output.
  python3 ~/.local/share/pixel-hub/pixel_hub.py --browser
Per problemi ADB: controlla autorizzazione, cavo dati, porta Debug wireless o
regole udev se compare no permissions. Non avviare l'app come root.

IMPORTAZIONE FOTO
Copia /sdcard/DCIM locale, non le foto presenti solo sul cloud. Nessuna cancellazione
sul Pixel. Salta file con stesso percorso e dimensione; non confronta hash.
File con dimensione diversa sono conservati come copie separate.

DISINSTALLAZIONE
  bash installa-pixel-hub-fedora.sh rimuovi
Conserva dati, configurazioni, pacchetti, COPR e regole firewall eventualmente aggiunti.

LIMITI DI VALIDAZIONE
Test automatici con ADB simulato e protocollo HTTPS locale. Bluetooth, LocalSend
Android, finestre GTK e scrcpy devono essere collaudati sul PC Fedora e Pixel reali.
Fedora tradizionale con DNF; l'installer si ferma su Fedora Atomic.
HELP
echo "Guida completa: $DIR/LEGGIMI.txt"
