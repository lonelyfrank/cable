#!/usr/bin/env bash
# Script di sviluppo: non installa nulla e non modifica il sistema.
#   scripts/dev.sh run [FILE...]   avvia dal sorgente (usa la tua configurazione reale)
#   scripts/dev.sh sandbox         avvia con dati isolati in /tmp e senza multicast (non tocca ~/.config)
#   scripts/dev.sh test            test automatici (unitari + integrazione, senza grafica)
#   scripts/dev.sh ui-test         prova della finestra reale + screenshot (serve una sessione grafica)
set -euo pipefail
cd "$(dirname "$0")/.."
export PYTHONPATH="$PWD${PYTHONPATH:+:$PYTHONPATH}"
case "${1:-run}" in
  run)     shift || true; exec python3 -m pixelhub "$@" ;;
  sandbox) d=$(mktemp -d /tmp/pixelhub-sandbox.XXXXXX); echo "Dati isolati in $d"
           HOME="$d" HUB_NO_MULTICAST=1 HUB_DEBUG=1 exec python3 -m pixelhub ;;
  test)    exec python3 -m unittest discover -s tests -t . -p 'test_*.py' -v ;;
  ui-test) shift || true; HUB_DEBUG=1 exec python3 tests/ui_smoke.py "${1:-../../docs/screenshots}" ;;
  *) sed -n '2,7p' "$0"; exit 1 ;;
esac
