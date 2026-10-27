"""Peer Python per la prova di interoperabilità con il motore Rust (uso: py_peer.py PORTA DIR)."""
import hashlib, os, sys, threading, time
os.environ["HUB_NO_MULTICAST"] = "1"
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "legacy", "python"))
from pathlib import Path
from pixelhub.engine import Engine
from pixelhub.paths import Paths

port, base = int(sys.argv[1]), Path(sys.argv[2])
data = bytes((i * 7 + 3) % 256 for i in range(5 * 1024 * 1024 + 123))
e = Engine(Paths.sotto(base), port=port, multicast=False)
e.settings.aggiorna({"nome_pc": "peer-python"})
assert e.avvia()
print("READY %d %s" % (e.port, e.identity.fingerprint), flush=True)

def acceptor():                       # accetta tutto, come farebbe un utente
    while True:
        for r in e.receiver.snapshot()["ricezioni"]:
            if r["stato"] == "attesa":
                e.decidi(r["id"])
        time.sleep(0.05)
threading.Thread(target=acceptor, daemon=True).start()

while not e.registry.devices:         # il Rust si registra presso di noi con POST /register
    time.sleep(0.1)
dev = next(iter(e.registry.devices.values()))
print("PY_SEES_RUST alias=%s" % dev.alias, flush=True)
src = base / "uscita.bin"
src.write_bytes(data)
for label, kw in (("FILE", dict(percorsi=[str(src)])), ("TESTO", dict(testo="ciao dal peer python"))):
    jid, err = e.invia(dev.fp, **kw)
    t0 = time.time()
    while time.time() - t0 < 60:
        j = next((x for x in e.sender.snapshot() if x["id"] == jid), None)
        if j and j["stato"] in ("completato", "parziale", "errore", "rifiutato", "annullato"):
            break
        time.sleep(0.05)
    print("PY_SENT_%s %s %s" % (label, j["stato"], j["errore"]), flush=True)

# verifica quanto ha mandato il Rust
want = hashlib.sha256(data).hexdigest()
for _ in range(400):
    got = [p for p in Path(e.settings["cartella"]).glob("rust-*.bin")]
    if got and hashlib.sha256(got[0].read_bytes()).hexdigest() == want and e.receiver.snapshot()["messaggi"]:
        break
    time.sleep(0.1)
ok_file = bool(got) and hashlib.sha256(got[0].read_bytes()).hexdigest() == want
msgs = [m["testo"] for m in e.receiver.snapshot()["messaggi"]]
print("PY_RECEIVED_FILE_OK=%s" % ok_file, flush=True)
print("PY_RECEIVED_TEXT=%r" % msgs, flush=True)
print("PY_DONE", flush=True)
time.sleep(3)
e.ferma()
