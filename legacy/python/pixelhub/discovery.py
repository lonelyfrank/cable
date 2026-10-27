"""Scoperta LocalSend: annunci multicast, registrazione HTTPS e inserimento manuale."""
import http.client
import ipaddress
import json
import socket
import ssl
import threading
import time

from . import net
from .bus import log
from .client import IdentityMismatch, json_request
from .protocol import API, parse_peer_info

MCAST = "224.0.0.167"


def in_thread(fn, *a):
    t = threading.Thread(target=fn, args=a, daemon=True)
    t.start()
    return t


class Discovery:
    def __init__(self, engine, multicast=True):
        self.e = engine
        self.multicast = multicast
        self.stop_event = threading.Event()
        self._sock = None
        self._groups = set()
        self._last_reply = {}
        self.lock = threading.Lock()
        self.errore = ""

    # ------------------------------------------------------------ ciclo di vita
    def start(self):
        if self.multicast:
            in_thread(self._listen)
        in_thread(self._periodic)

    def stop(self):
        self.stop_event.set()
        s = self._sock
        if s:
            try:
                s.close()
            except OSError:
                pass

    def rete_cambiata(self):
        """Dopo un cambio rete o la ripresa dalla sospensione."""
        self._groups.clear()
        self._join()
        in_thread(self.announce, True)

    # ------------------------------------------------------------ multicast
    def _mc_socket(self):
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        if hasattr(socket, "SO_REUSEPORT"):
            try:
                s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
            except OSError:
                pass
        return s

    def _join(self):
        s = self._sock
        if not s:
            return
        for _name, ip in net.local_ips() + [("any", "0.0.0.0")]:
            if ip in self._groups:
                continue
            try:
                s.setsockopt(socket.IPPROTO_IP, socket.IP_ADD_MEMBERSHIP,
                             socket.inet_aton(MCAST) + socket.inet_aton(ip))
                self._groups.add(ip)
            except OSError:
                pass

    def _listen(self):
        while not self.stop_event.is_set():
            s = self._mc_socket()
            try:
                s.bind(("", self.e.port))
            except OSError as e:
                self.errore = "Scoperta multicast non disponibile: %s" % e.strerror
                log("multicast bind", e)
                s.close()
                if self.stop_event.wait(15):
                    return
                continue
            self.errore = ""
            self._sock = s
            self._groups.clear()
            self._join()
            s.settimeout(10)
            while not self.stop_event.is_set():
                try:
                    data, (ip, _p) = s.recvfrom(8192)
                except socket.timeout:
                    self._join()       # interfacce nuove (hotspot acceso dopo)
                    continue
                except OSError:
                    break
                self._on_datagram(data, ip)
            s.close()

    def _on_datagram(self, data, ip):
        try:
            info = json.loads(data)
        except ValueError:
            return
        p = parse_peer_info(info)
        if not p or p["fp"] == self.e.identity.fingerprint:
            return
        self.e.registry.upsert(info, ip, "multicast")
        if info.get("announce") is True:
            now = time.monotonic()
            with self.lock:
                if now - self._last_reply.get(ip, 0) < 5:
                    return
                self._last_reply[ip] = now
            in_thread(self._reply_to_announce, p, ip)

    def _reply_to_announce(self, p, ip):
        try:
            st, resp, _ = json_request(ip, p["port"], "POST", API + "/register", self.e.identity,
                                       self.e.info(), 4, expected_fp=p["fp"])
            if st == 200:
                self.e.registry.mark_verified(p["fp"])
                return
        except IdentityMismatch:
            self.e.registry.remove(p["fp"])     # annuncio incoerente col certificato
            return
        except (OSError, http.client.HTTPException, ssl.SSLError):
            pass
        self.announce(False)                    # ripiego UDP previsto dalla specifica

    def announce(self, announce=True):
        if not self.multicast:
            return
        data = json.dumps(dict(self.e.info(), announce=announce)).encode()
        s = self._mc_socket()
        try:
            s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_TTL, 1)
            sent = 0
            for _name, ip in net.local_ips():
                try:
                    s.setsockopt(socket.IPPROTO_IP, socket.IP_MULTICAST_IF, socket.inet_aton(ip))
                    s.sendto(data, (MCAST, self.e.port))
                    sent += 1
                except OSError:
                    pass
            if not sent:
                try:
                    s.sendto(data, (MCAST, self.e.port))
                except OSError:
                    pass
        finally:
            s.close()

    def _periodic(self):
        self.announce(True)
        while not self.stop_event.wait(60):
            self.announce(True)
            self.e.registry.prune()
            self.e.bus.emit("dispositivi")

    # ------------------------------------------------------------ registrazione in ingresso
    def on_register(self, body, ip):
        p = parse_peer_info(body)
        if p and p["fp"] != self.e.identity.fingerprint:
            self.e.registry.upsert(body, ip, "incoming")

    # ------------------------------------------------------------ ricerca attiva e IP manuale
    def scan(self, ip=None, port=None):
        """Annuncia e prova direttamente un indirizzo (o il gateway, utile con l'hotspot)."""
        self.announce(True)
        targets = []
        if ip:
            try:
                targets.append(str(ipaddress.IPv4Address(ip)))
            except ValueError:
                return 0
        else:
            gw = net.gateway()
            if gw:
                targets.append(gw)
        found = 0
        for t in targets:
            for port in (port or self.e.port,):
                try:
                    st, resp, actual = json_request(t, port, "POST", API + "/register", self.e.identity,
                                                    self.e.info(), 3)
                except (OSError, http.client.HTTPException, ssl.SSLError):
                    continue
                if st == 200 and isinstance(resp, dict):
                    # in HTTPS l'identità è l'impronta del certificato appena visto, non il JSON
                    d = self.e.registry.upsert(dict(resp, fingerprint=actual), t, "manuale", port)
                    if d:
                        self.e.registry.mark_verified(d.fp)
                        found += 1
        return found
