"""Bus di eventi minimale: il motore emette, la UI (o i test) ascolta."""
import sys
import threading
import os


def log(*a):
    if os.environ.get("HUB_DEBUG"):
        print("[hub]", *a, file=sys.stderr, flush=True)


class Bus:
    def __init__(self):
        self._lock = threading.Lock()
        self._listeners = []

    def subscribe(self, fn):
        with self._lock:
            self._listeners.append(fn)

    def emit(self, topic="stato", **data):
        with self._lock:
            listeners = list(self._listeners)
        for fn in listeners:
            try:
                fn(topic, data)
            except Exception as e:  # un ascoltatore difettoso non deve fermare il motore
                log("listener", repr(e))
