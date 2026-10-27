import hashlib
import os
import socket
import tempfile
import threading
import time
import unittest
from pathlib import Path

from pixelhub.engine import Engine
from pixelhub.paths import Paths


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    p = s.getsockname()[1]
    s.close()
    return p


def wait_for(cond, timeout=10.0, step=0.02):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        v = cond()
        if v:
            return v
        time.sleep(step)
    raise AssertionError("timeout in attesa della condizione")


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


class TmpCase(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = Path(self._tmp.name)
        self.engines = []

    def tearDown(self):
        for e in self.engines:
            e.ferma()
        self._tmp.cleanup()

    def engine(self, name, start=True):
        e = Engine(Paths.sotto(self.tmp / name), port=free_port(), multicast=False)
        e.settings.aggiorna({"nome_pc": name})
        self.engines.append(e)
        if start:
            self.assertTrue(e.avvia())
        return e

    def pair(self, a, b):
        """a scopre b tramite IP manuale (b conosce a grazie alla register)."""
        self.assertEqual(a.cerca("127.0.0.1", b.port), 1)
        return a.registry.get(b.identity.fingerprint)

    def accept_when_pending(self, eng, ids=None, timeout=10):
        def pend():
            for r in eng.receiver.snapshot()["ricezioni"]:
                if r["stato"] == "attesa":
                    return r
        r = wait_for(pend, timeout)
        self.assertTrue(eng.decidi(r["id"], ids))
        return r

    def job(self, eng, jid):
        for j in eng.sender.snapshot():
            if j["id"] == jid:
                return j

    def wait_job(self, eng, jid, states=("completato", "parziale", "rifiutato", "annullato", "errore"), timeout=20):
        return wait_for(lambda: (j := self.job(eng, jid)) and j["stato"] in states and j, timeout)
