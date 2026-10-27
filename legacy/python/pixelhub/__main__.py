"""Punto d'ingresso: python -m pixelhub  /  pixel-hub"""
import argparse
import sys

from . import VERSION, desktop
from .paths import Paths

USO = """Pixel Hub %s

  pixel-hub [FILE...]       apre l'app (e prepara i file indicati per l'invio)
  pixel-hub --background    parte nascosta, pronta a ricevere
  pixel-hub --migra         importa le impostazioni della versione 3.x e mostra l'esito
  pixel-hub --diagnostica   stampa la diagnostica di rete, senza aprire la finestra
  pixel-hub --versione
"""


def main(argv=None):
    ap = argparse.ArgumentParser(add_help=False)
    ap.add_argument("--background", action="store_true")
    ap.add_argument("--migra", action="store_true")
    ap.add_argument("--diagnostica", action="store_true")
    ap.add_argument("--versione", action="store_true")
    ap.add_argument("-h", "--help", "--aiuto", action="store_true")
    ap.add_argument("files", nargs="*")
    a = ap.parse_args(argv if argv is not None else sys.argv[1:])
    if a.help:
        print(USO % VERSION)
        return 0
    if a.versione:
        print(VERSION)
        return 0
    paths = Paths.default()
    if a.migra:
        from .settings import Settings
        s = Settings(paths)
        print("\n".join(s.avvisi) or "Nessuna migrazione necessaria: impostazioni già nel formato corrente.")
        print("Migrazione registrata:", s["migrato_da"] or "nessuna (nessuna configurazione 3.x trovata)")
        return 0
    if a.diagnostica:
        from .engine import Engine
        print(Engine(paths).diagnostica())
        return 0
    vecchi = desktop.processi_vecchia_versione()
    if vecchi:
        print("La versione 3.x di Pixel Hub è ancora in esecuzione (PID %s). Chiudila prima di avviare questa:\n"
              "  pkill -f '^python3 .*pixel_hub.py'" % ", ".join(map(str, vecchi)), file=sys.stderr)
        return 3
    try:
        from .ui.shell import run
    except (ImportError, ValueError) as e:
        print("Interfaccia non disponibile (%s). Servono GTK4, libadwaita e WebKitGTK 6." % e, file=sys.stderr)
        return 2
    return run(a.files, a.background)


if __name__ == "__main__":
    raise SystemExit(main())
