"""Registro dei dispositivi LocalSend visti in rete e dei preferiti."""
import threading
import time

from .protocol import parse_peer_info

NEARBY_SECONDS = 120
FORGET_SECONDS = 900
MAX_DEVICES = 256


class Device:
    __slots__ = ("fp", "alias", "model", "type", "ip", "port", "seen", "source", "verified")

    def __init__(self, fp, alias, model, type_, ip, port, source):
        self.fp, self.alias, self.model, self.type = fp, alias, model, type_
        self.ip, self.port, self.source = ip, port, source
        self.seen = time.time()
        self.verified = False   # vero solo dopo aver visto un certificato TLS con questa impronta

    def target(self):
        return {"ip": self.ip, "port": self.port, "fp": self.fp, "alias": self.alias}


class Registry:
    def __init__(self, own_fp, settings, bus):
        self.own_fp = own_fp.upper()
        self.settings = settings
        self.bus = bus
        self.lock = threading.RLock()
        self.devices = {}

    def upsert(self, info, ip, source="multicast", port=None):
        p = parse_peer_info(info if port is None else dict(info, port=port))
        if not p or p["fp"] == self.own_fp:
            return None
        with self.lock:
            d = self.devices.get(p["fp"])
            if d is None:
                if len(self.devices) >= MAX_DEVICES:
                    self._evict()
                d = self.devices[p["fp"]] = Device(p["fp"], p["alias"], p["model"], p["type"], ip, p["port"], source)
            else:
                d.alias, d.model, d.type, d.ip, d.port = p["alias"], p["model"], p["type"], ip, p["port"]
                d.seen = time.time()
            fav = self.settings["preferiti"]
            if p["fp"] in fav:
                self.settings.preferito(p["fp"], True, {"alias": d.alias, "ip": ip, "port": d.port})
        self.bus.emit("dispositivi")
        return d

    def _evict(self):
        fav = self.settings["preferiti"]
        old = sorted((d for d in self.devices.values() if d.fp not in fav), key=lambda d: d.seen)
        for d in old[:16]:
            self.devices.pop(d.fp, None)

    def get(self, fp):
        with self.lock:
            return self.devices.get(str(fp).upper())

    def mark_verified(self, fp):
        with self.lock:
            d = self.devices.get(fp)
            if d:
                d.verified = True

    def remove(self, fp):
        with self.lock:
            self.devices.pop(str(fp).upper(), None)
        self.bus.emit("dispositivi")

    def clear(self):
        with self.lock:
            self.devices.clear()
        self.bus.emit("dispositivi")

    def prune(self):
        now = time.time()
        fav = self.settings["preferiti"]
        with self.lock:
            for fp in [k for k, d in self.devices.items() if now - d.seen > FORGET_SECONDS and k not in fav]:
                del self.devices[fp]

    def snapshot(self):
        """Elenco per la UI: dispositivi visti + preferiti attualmente non raggiungibili."""
        now = time.time()
        fav = self.settings["preferiti"]
        out, seen = [], set()
        with self.lock:
            for d in sorted(self.devices.values(), key=lambda d: -d.seen):
                seen.add(d.fp)
                out.append({"fp": d.fp, "alias": d.alias, "modello": d.model, "tipo": d.type, "ip": d.ip,
                            "port": d.port, "vicino": now - d.seen < NEARBY_SECONDS, "visto": d.seen,
                            "preferito": d.fp in fav, "verificato": d.verified, "origine": d.source})
        for fp, rec in fav.items():
            if fp not in seen:
                out.append({"fp": fp, "alias": rec["alias"], "modello": "", "tipo": "mobile", "ip": rec.get("ip", ""),
                            "port": rec.get("port", 53317), "vicino": False, "visto": 0,
                            "preferito": True, "verificato": False, "origine": "preferito"})
        out.sort(key=lambda d: (not d["vicino"], not d["preferito"], d["alias"].lower()))
        return out
