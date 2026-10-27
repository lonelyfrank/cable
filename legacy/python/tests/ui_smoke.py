"""Prova manuale/automatica della finestra reale (serve una sessione grafica).

  HUB_DEBUG=1 python3 tests/ui_smoke.py [cartella-screenshot]

Avvia l'app con dati isolati, collega un secondo motore e verifica richiesta, accettazione,
invio dall'interfaccia, PIN e screenshot. Non tocca ~/.config né la rete locale.
"""
import json
import os
import sys
import tempfile
import threading
import time
from pathlib import Path

os.environ["HUB_NO_MULTICAST"] = "1"
tmp = Path(tempfile.mkdtemp(prefix="pixelhub-ui-"))
os.environ["HOME"] = str(tmp / "home")
(tmp / "home").mkdir()
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tests.helpers import free_port, wait_for  # noqa: E402
os.environ["HUB_LS_PORT"] = str(free_port())

import gi  # noqa: E402
from gi.repository import GLib  # noqa: E402
from pixelhub.engine import Engine  # noqa: E402
from pixelhub.paths import Paths  # noqa: E402
from pixelhub.ui.shell import HubApp  # noqa: E402

shots = Path(sys.argv[1]) if len(sys.argv) > 1 else tmp / "shots"
shots.mkdir(parents=True, exist_ok=True)
paths = Paths.sotto(tmp / "app")
app = HubApp(background=False, paths=paths)
results, errors = [], []


def check(name, cond, detail=""):
    results.append((name, bool(cond)))
    print(("OK   " if cond else "FAIL ") + name + (" — " + str(detail) if detail and not cond else ""), flush=True)


def js(script, timeout=10):
    """Valuta JS nella pagina da un thread di lavoro e restituisce il risultato come stringa."""
    done, out = threading.Event(), {}

    def run():
        def cb(web, res):
            try:
                out["v"] = web.evaluate_javascript_finish(res).to_string()
            except GLib.Error as e:
                out["e"] = str(e)
            done.set()
        app.web.evaluate_javascript(script, -1, None, None, None, cb)
        return False
    GLib.idle_add(run)
    if not done.wait(timeout):
        raise TimeoutError(script)
    if "e" in out:
        raise RuntimeError(out["e"])
    return out["v"]


def act(script):
    """Esegue uno statement (anche asincrono) senza restituirne il valore."""
    return js("(() => { %s; return 'ok'; })()" % script)


def shot(name):
    time.sleep(0.6)       # lascia finire le animazioni
    done = threading.Event()

    def run():
        def cb(web, res):
            try:
                web.get_snapshot_finish(res).save_to_png(str(shots / (name + ".png")))
            except Exception as e:  # noqa: BLE001
                errors.append("snapshot " + repr(e))
            done.set()
        from gi.repository import WebKit
        app.web.get_snapshot(WebKit.SnapshotRegion.VISIBLE, WebKit.SnapshotOptions.NONE, None, cb)
        return False
    GLib.idle_add(run)
    done.wait(10)


def scenario():
    try:
        wait_for(lambda: app.loaded and app.engine, 20)
        wait_for(lambda: js("typeof S !== 'undefined' && S ? 'si' : 'no'") == "si", 15)
        check("pagina caricata e stato ricevuto dal motore", True)
        check("nessun wizard", js("String(!document.getElementById('wizard'))") == "true")
        check("5 schede presenti", js("String(document.querySelectorAll('.tab').length)") == "5")
        shot("01-home-vuota")

        peer = Engine(Paths.sotto(tmp / "peer"), port=free_port(), multicast=False)
        peer.settings.aggiorna({"nome_pc": "Pixel di prova"})
        peer.avvia()
        check("scoperta manuale dal motore dell'app", app.engine.cerca("127.0.0.1", peer.port) == 1)
        wait_for(lambda: js("String(vicini().length)") == "1", 10)
        check("dispositivo mostrato nell'interfaccia", True)
        js("vai('condividi')")
        time.sleep(0.4)
        shot("02-condividi-dispositivo")

        # ---- ricezione: il peer invia, l'interfaccia mostra la richiesta
        src = tmp / "dati"
        src.mkdir()
        for n in ("foto1.jpg", "relazione.pdf", "note.txt"):
            (src / n).write_bytes(os.urandom(200_000))
        jid, err = peer.invia(app.engine.identity.fingerprint, [src / n for n in ("foto1.jpg", "relazione.pdf", "note.txt")])
        check("peer avvia l'invio", jid, err)
        peer.registry.upsert(app.engine.info(), "127.0.0.1", "manuale")
        wait_for(lambda: js("String(document.getElementById('velo').classList.contains('on'))") == "true", 10)
        check("sheet di richiesta visibile con 3 file selezionabili",
              js("String(document.querySelectorAll('#sheet input[data-fid]').length)") == "3")
        shot("03-richiesta-in-arrivo")
        js("document.querySelector('#sheet input[data-fid]:last-of-type').checked = false")
        act("AZ.accetta()")
        j = wait_for(lambda: (x := next((i for i in peer.sender.snapshot() if i["id"] == jid), None)) and x["stato"] in ("completato", "parziale", "errore") and x, 20)
        check("accettazione parziale dall'interfaccia → invio parziale", j["stato"] == "parziale", j)
        got = sorted(p.name for p in Path(app.engine.settings["cartella"]).iterdir())
        check("arrivati solo i 2 file scelti", len(got) == 2, got)
        time.sleep(0.5)
        shot("04-dopo-ricezione")

        # ---- rifiuto
        jid2, _ = peer.invia(app.engine.identity.fingerprint, testo="ciao")
        wait_for(lambda: js("String(document.getElementById('velo').classList.contains('on'))") == "true", 10)
        act("AZ.rifiuta()")
        wait_for(lambda: (x := next((i for i in peer.sender.snapshot() if i["id"] == jid2), None)) and x["stato"] == "rifiutato", 10)
        check("rifiuto dall'interfaccia", True)

        # ---- invio dall'interfaccia, con PIN richiesto dal peer
        peer.imposta_pin("2468")
        f = tmp / "uscita.bin"
        f.write_bytes(os.urandom(300_000))
        js("destSel = null")
        check("destinatario implicito = unico dispositivo vicino", js("destinazione() ? destinazione().alias : ''") == "Pixel di prova")
        act("inviaA(destinazione(), [%s])" % json.dumps(str(f)))
        wait_for(lambda: js("String(document.getElementById('veloPin').classList.contains('on'))") == "true", 10)
        check("l'interfaccia chiede il PIN", True)
        shot("05-richiesta-pin")
        act("document.getElementById('pinCampo').value='2468'; document.querySelector('[data-azione=pinSi]').click()")
        r = wait_for(lambda: next((x for x in peer.receiver.snapshot()["ricezioni"] if x["stato"] == "attesa"), None), 10)
        peer.decidi(r["id"])
        wait_for(lambda: (Path(peer.settings["cartella"]) / "uscita.bin").exists(), 10)
        check("invio con PIN dall'interfaccia completato", (Path(peer.settings["cartella"]) / "uscita.bin").stat().st_size == 300_000)

        # ---- rilascio di un file sul riquadro del dispositivo (percorso reale, nessuna copia in cache)
        peer.imposta_pin(None)
        g = tmp / "trascinato.bin"
        g.write_bytes(os.urandom(100_000))
        act("vai('condividi')")
        time.sleep(0.4)
        act("window.__r = document.querySelector('.disp[data-fp]').getBoundingClientRect()")
        act("hubDrop([%s], __r.x + __r.width/2, __r.y + __r.height/2)" % json.dumps(str(g)))
        r = wait_for(lambda: next((x for x in peer.receiver.snapshot()["ricezioni"] if x["stato"] == "attesa"), None), 10)
        peer.decidi(r["id"])
        wait_for(lambda: (Path(peer.settings["cartella"]) / "trascinato.bin").exists(), 10)
        check("rilascio sul dispositivo invia il file reale", (Path(peer.settings["cartella"]) / "trascinato.bin").stat().st_size == 100_000)
        check("nessuna copia nella cache di uscita", not list((paths.cache).rglob("uscita")))

        # ---- altre pagine, impostazioni, nessun errore JS
        for p in ("schermo", "pixel", "impostazioni"):
            js("vai('%s')" % p)
            time.sleep(0.4)
            shot("06-" + p)
        act("api('diagnostica').then(r => { window.__d = r.testo })")
        r = wait_for(lambda: js("window.__d ? window.__d.split('\\n')[0] : ''"), 10)
        check("diagnostica raggiungibile dalla pagina", "Pixel Hub" in r, r)
        # notifiche native con azioni (finestra nascosta: altrimenti non vengono inviate)
        err = []

        def notif():
            try:
                app.window.set_visible(False)
                app._notify("richiesta", {"id": "prova1", "alias": "Pixel di prova", "n": 2, "size": 1234, "tipo": "file"})
                app._notify("fine", {"id": "prova1", "alias": "Pixel di prova", "stato": "completata", "ricevuti": 2, "totale": 2, "errore": ""})
                app._notify("messaggio", {"alias": "Pixel di prova", "testo": "ciao"})
                app.withdraw_notification("req-prova1")
                app.window.set_visible(True)
            except Exception as e:  # noqa: BLE001
                err.append(repr(e))
        GLib.idle_add(lambda: (notif(), False)[1])
        time.sleep(1)
        check("notifiche con azioni costruite senza errori", not err, err)
        check("finestra nascosta e riaperta senza perdere lo stato", js("typeof S !== 'undefined' && S ? 'si' : 'no'") == "si")
        check("nessun errore JavaScript", js("String(window.__errs||0)") == "0")
        peer.ferma()
    except Exception as e:  # noqa: BLE001
        import traceback
        traceback.print_exc()
        results.append(("eccezione " + repr(e), False))
    finally:
        GLib.idle_add(app.esci)


GLib.timeout_add(500, lambda: (threading.Thread(target=scenario, daemon=True).start(), False)[1])
rc = app.run(["pixel-hub"])
bad = [n for n, ok in results if not ok]
print("\n%d controlli, %d falliti. Screenshot in %s" % (len(results), len(bad), shots))
sys.exit(1 if bad else 0)
