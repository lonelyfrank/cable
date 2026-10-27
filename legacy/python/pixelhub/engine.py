"""Motore di Pixel Hub: unisce impostazioni, identità, rete, ricezione e invio.

Non importa GTK: la UI si limita a chiamare questi metodi e ad ascoltare `bus`.
"""
import os
import platform
import pwd
import socket
import threading
import time
from pathlib import Path

from . import LS_PORT_DEFAULT, VERSION, android as android_mod, desktop, net
from .bus import Bus, log
from .devices import Registry
from .discovery import Discovery
from .identity import Identity
from .paths import Paths
from .receiver import Receiver
from .sender import Sender
from .server import LocalSendServer
from .settings import Settings, backup_file
from .util import tipo_file

CONF_KEYS = ("nome_pc", "cartella", "cartella_foto", "vetro", "tema", "notifiche",
             "schermo_intero", "background", "avvio_login")


def nome_utente():
    try:
        g = pwd.getpwuid(os.getuid()).pw_gecos.split(",")[0].strip()
        if g:
            return g.split()[0]
    except (KeyError, IndexError):
        pass
    return ""


class Engine:
    def __init__(self, paths=None, port=None, multicast=None):
        if multicast is None:
            multicast = not os.environ.get("HUB_NO_MULTICAST")
        self.paths = paths or Paths.default()
        self.bus = Bus()
        self.settings = Settings(self.paths)
        self.identity = Identity(self.paths)
        self.port = int(port or os.environ.get("HUB_LS_PORT") or LS_PORT_DEFAULT)
        self.registry = Registry(self.identity.fingerprint, self.settings, self.bus)
        self.receiver = Receiver(self.settings, self.bus, notify=self.notify)
        self.sender = Sender(self)
        self.discovery = Discovery(self, multicast)
        self.android = android_mod.Android(self.settings, self.bus)
        self.server = None
        self.server_error = ""
        self.notifier = None
        self._stop = threading.Event()
        self._cache = {}

    # ------------------------------------------------------------------ identità e notifiche
    def info(self, with_port=True):
        d = {"alias": self.settings["nome_pc"], "version": "2.0", "deviceModel": "Linux",
             "deviceType": "desktop", "fingerprint": self.identity.fingerprint, "download": False}
        if with_port:
            d.update(port=self.port, protocol="https")
        return d

    def notify(self, kind, **kw):
        if self.notifier and self.settings["notifiche"]:
            try:
                self.notifier(kind, kw)
            except Exception as e:  # noqa: BLE001
                log("notifier", repr(e))

    # ------------------------------------------------------------------ ciclo di vita
    def avvia(self):
        for k in ("cartella", "cartella_foto"):
            try:
                Path(self.settings[k]).mkdir(parents=True, exist_ok=True)
            except OSError:
                pass
        self.receiver.pulisci_residui()
        self._migra_avvio_automatico()
        ok = self._avvia_server()
        self.discovery.start()
        self.android.start()
        threading.Thread(target=self._timer, daemon=True).start()
        return ok

    def _migra_avvio_automatico(self):
        """Se l'avvio al login puntava ancora allo script 3.x, lo riscrive verso questa versione."""
        f = desktop.autostart_file(self.paths)
        try:
            if f.exists() and "pixel_hub.py" in f.read_text():
                backup_file(f, self.paths.backups, "autostart-v3")      # serve al ritorno alla 3.x
                desktop.imposta_avvio(self.paths, True)
        except OSError:
            pass

    def _avvia_server(self):
        if self.server:
            return True
        if self.identity.stato != "ok":
            self.server_error = "identita"
            return False
        try:
            self.server = LocalSendServer(("0.0.0.0", self.port), self)
        except OSError as e:
            self.server_error = "porta" if e.errno in (98, 13) else (e.strerror or "errore")
            log("server non avviato", e)
            return False
        self.server_error = ""
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.bus.emit("server")
        return True

    def riprova_server(self):
        ok = self._avvia_server()
        self.bus.emit("server")
        return ok

    def _timer(self):
        while not self._stop.wait(5):
            self.receiver.sweep()
            self.sender._prune()

    def ferma(self):
        self._stop.set()
        self.discovery.stop()
        self.android.stop()
        if self.server:
            self.server.shutdown()
            self.server.server_close()
            self.server = None

    def rete_cambiata(self):
        """Cambio rete o ripresa dalla sospensione: riannuncia e, se serve, riavvia il server."""
        self._cache.pop("rete", None)
        self.discovery.rete_cambiata()
        if not self.server:
            self._avvia_server()
        self.bus.emit("server")

    # ------------------------------------------------------------------ azioni
    def cerca(self, ip=None, port=None):
        n = self.discovery.scan(ip, port)
        self.bus.emit("dispositivi")
        return n

    def invia(self, fp, percorsi=None, testo=None):
        dev = self.registry.get(fp)
        if not dev:
            return None, "Dispositivo non raggiungibile: apri LocalSend sul destinatario."
        if self.identity.stato != "ok":
            return None, "Identità non valida: rigenerala dalle Impostazioni."
        if testo is not None and not str(testo).strip():
            testo = None
        return self.sender.invia(dev, [str(p) for p in (percorsi or [])], testo)

    def decidi(self, rid, accetta_ids=None):
        return self.receiver.decide(rid, accetta_ids)

    def preferito(self, fp, on):
        d = self.registry.get(fp)
        ok = self.settings.preferito(fp, on, {"alias": d.alias, "ip": d.ip, "port": d.port} if d else None)
        self.bus.emit("dispositivi")
        return ok

    def dimentica(self, fp):
        """Toglie il dispositivo dai preferiti e dall'elenco (ricomparirà se annuncia di nuovo)."""
        self.settings.preferito(fp, False)
        self.registry.remove(fp)
        return True

    def aggiorna_impostazioni(self, patch):
        done = self.settings.aggiorna({k: v for k, v in patch.items() if k in CONF_KEYS})
        for k in ("cartella", "cartella_foto"):
            if k in done:
                try:
                    Path(done[k]).mkdir(parents=True, exist_ok=True)
                except OSError:
                    pass
        if "avvio_login" in done:
            desktop.imposta_avvio(self.paths, done["avvio_login"])
        if "nome_pc" in done:
            threading.Thread(target=self.discovery.announce, args=(True,), daemon=True).start()
        self.bus.emit("impostazioni")
        return list(done)

    def imposta_pin(self, pin):
        ok = self.settings.imposta_pin(pin)
        self.bus.emit("impostazioni")
        return ok

    def ripristina_impostazioni(self):
        bk = self.settings.ripristina()
        desktop.imposta_avvio(self.paths, False)
        self.registry.clear()
        self.bus.emit("impostazioni")
        return str(bk) if bk else ""

    def rigenera_identita(self):
        self.identity.rigenera()
        self.registry.own_fp = self.identity.fingerprint
        self.registry.clear()
        ok = self._avvia_server()
        self.discovery.rete_cambiata()
        self.bus.emit("server")
        return ok

    # ------------------------------------------------------------------ stato per la UI
    def _cached(self, key, ttl, fn):
        t, v = self._cache.get(key, (0, None))
        if time.monotonic() - t > ttl:
            v = fn()
            self._cache[key] = (time.monotonic(), v)
        return v

    def recenti(self, n=40):
        try:
            with os.scandir(self.settings["cartella"]) as it:
                voci = [e for e in it if not e.name.startswith(".")]
        except OSError:
            return []
        out = []
        for e in sorted(voci, key=lambda e: e.stat().st_mtime, reverse=True)[:n]:
            try:
                st = e.stat()
            except OSError:
                continue
            out.append({"nome": e.name, "path": e.path, "size": st.st_size, "mtime": st.st_mtime,
                        "dir": e.is_dir(), "tipo": "cartella" if e.is_dir() else tipo_file(e.name)})
        return out

    def componenti(self):
        ha = android_mod.ha
        return [
            {"id": "adb", "nome": "ADB", "ok": ha("adb"), "desc": "Facoltativo: batteria, spazio e importazione DCIM via USB",
             "comando": "sudo dnf install android-tools"},
            {"id": "scrcpy", "nome": "scrcpy", "ok": ha("scrcpy"), "desc": "Facoltativo: schermo e controllo del telefono",
             "comando": "sudo dnf copr enable zeno/scrcpy && sudo dnf install scrcpy"},
        ]

    def snapshot(self):
        s = self.settings.snapshot()
        rcv = self.receiver.snapshot()
        ips = self._cached("rete_ip", 15, net.local_ips)
        return {
            "versione": VERSION, "utente": nome_utente(),
            "conf": {k: s[k] for k in CONF_KEYS}, "pin_impostato": bool(s["pin_ricezione"]),
            "dispositivi": self.registry.snapshot(),
            "ricezioni": rcv["ricezioni"], "messaggi": rcv["messaggi"],
            "invii": self.sender.snapshot(),
            "server": {"attivo": self.server is not None, "errore": self.server_error,
                       "porta": self.port, "protocollo": "https"},
            "identita": {"stato": self.identity.stato, "errore": self.identity.errore,
                         "impronta": self.identity.fingerprint, "origine": self.identity.origine},
            "rete": {"nome": self._cached("rete_nome", 15, net.network_name), "ip": [ip for _n, ip in ips],
                     "multicast": self.discovery.errore},
            "recenti": self._cached("recenti", 2, self.recenti),
            "componenti": self._cached("componenti", 30, self.componenti),
            "avvisi": list(self.settings.avvisi),
            "migrazione": s.get("migrato_da"),
            **self.android.snapshot(),
        }

    # ------------------------------------------------------------------ diagnostica
    def diagnostica(self):
        """Testo per la sezione Diagnostica, con ipotesi distinte e non assolute."""
        rc, zones = net.run(["firewall-cmd", "--get-active-zones"])
        lines = ["Pixel Hub %s · Python %s" % (VERSION, platform.python_version())]
        try:
            lines.append(Path("/etc/fedora-release").read_text().strip())
        except OSError:
            pass
        lines.append("Server HTTPS: %s (porta %d)" % ("attivo" if self.server else "SPENTO: " + (self.server_error or "?"), self.port))
        lines.append("Identità: %s %s" % (self.identity.stato, self.identity.fingerprint[:16] + "…" if self.identity.fingerprint else ""))
        lines.append("Indirizzi: " + (", ".join(ip for _n, ip in net.local_ips()) or "nessuno"))
        lines.append("Multicast: " + (self.discovery.errore or "ok"))
        lines.append("Dispositivi visti: %d" % len(self.registry.devices))
        lines.append("Firewall: " + (zones.replace("\n", " | ") if rc == 0 else "firewalld non interrogabile"))
        hints = []
        if not net.local_ips():
            hints.append("Nessuna rete attiva: connettiti al Wi-Fi o all'hotspot del telefono.")
        if self.server_error == "porta":
            hints.append("La porta %d è occupata, di solito da LocalSend per PC: chiudilo e premi Riprova." % self.port)
        if self.discovery.errore:
            hints.append("La scoperta automatica non parte (%s): puoi comunque inserire l'IP a mano." % self.discovery.errore)
        if self.server and not self.registry.devices:
            hints.append("Nessun dispositivo visto. Cause possibili: LocalSend chiuso sul telefono, reti diverse, "
                         "isolamento client del router, VPN, oppure firewall del PC (porta %d TCP/UDP): "
                         "verifica una per volta, il firewall è solo una delle ipotesi." % self.port)
        lines.append("")
        lines += ["• " + h for h in hints] or ["Nessun problema evidente."]
        return "\n".join(lines)
