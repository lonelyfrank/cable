#!/usr/bin/env bash
# Script di sviluppo: non installa nulla e non modifica il sistema (salvo la cartella target/).
#   scripts/dev.sh run [FILE...]    avvia (debug) con la tua configurazione reale
#   scripts/dev.sh sandbox          avvia con dati isolati in /tmp e senza multicast: non tocca ~/.config
#   scripts/dev.sh test             test del motore (unitari + integrazione end-to-end su TLS)
#   scripts/dev.sh ui-test          prova della finestra reale + screenshot (serve una sessione grafica)
#   scripts/dev.sh interop          prova Python <-> Rust fra le due implementazioni indipendenti
#   scripts/dev.sh release          compila il binario ottimizzato in target/release/cable
#   scripts/dev.sh vendor           crea packaging/cable-VERSIONE-vendor.tar.xz per l'RPM (richiede rete)
#   scripts/dev.sh legacy-test      test della vecchia versione Python (legacy/python)
set -euo pipefail
cd "$(dirname "$0")/.."
case "${1:-run}" in
  run)       shift || true; exec cargo run -q -p cable -- "$@" ;;
  sandbox)   d=$(mktemp -d /tmp/cable-sandbox.XXXXXX); echo "Dati isolati in $d"
             HOME="$d" HUB_NO_MULTICAST=1 HUB_DEBUG=1 exec cargo run -q -p cable ;;
  test)      exec cargo test --workspace ;;
  ui-test)   shift || true; d=$(mktemp -d /tmp/cable-uitest.XXXXXX)
             cargo build -q -p cable --features ui-test
             port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
             HOME="$d" HUB_NO_MULTICAST=1 HUB_LS_PORT=$port HUB_UI_SHOTS="${1:-$PWD/docs/screenshots-v5}" exec dbus-run-session -- target/debug/cable ;;   # bus isolato: l'istanza unica non deve inoltrare a un Cable già aperto
  interop)   exec tests/interop/run.sh ;;
  release)   exec cargo build --release -p cable ;;
  vendor)    v=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2); rm -rf vendor
             cargo vendor --locked vendor >/dev/null && tar cJf "packaging/cable-$v-vendor.tar.xz" vendor && rm -rf vendor
             echo "Creato packaging/cable-$v-vendor.tar.xz" ;;
  legacy-test) cd legacy/python && PYTHONPATH=. exec python3 -m unittest discover -s tests -t . -p 'test_*.py' ;;
  *) sed -n '2,11p' "$0"; exit 1 ;;
esac
