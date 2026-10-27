#!/usr/bin/env bash
# Prova di interoperabilità fra le due implementazioni indipendenti (Python <-> Rust), su loopback.
# NON sostituisce la prova con LocalSend ufficiale (docs/CHECKLIST-MANUALE.md).
set -euo pipefail
cd "$(dirname "$0")/../.."
tmp=$(mktemp -d /tmp/pixelhub-interop.XXXXXX); trap 'kill $(jobs -p) 2>/dev/null || true' EXIT
port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
python3 tests/interop/py_peer.py "$port" "$tmp/py" > "$tmp/py.log" 2>&1 &
for _ in $(seq 50); do grep -q READY "$tmp/py.log" 2>/dev/null && break; sleep 0.2; done
cargo run -q -p pixelhub-engine --example interop -- "$port" | tee "$tmp/rs.log"
sleep 1; cat "$tmp/py.log"
ok=1
for k in "RS_SENT_FILE \"completato\"" "RS_SENT_TESTO \"completato\"" "RS_RECEIVED_FILE_OK=true" ; do grep -q "$k" "$tmp/rs.log" || { echo "MANCA: $k"; ok=0; }; done
for k in "PY_SENT_FILE completato" "PY_SENT_TESTO completato" "PY_RECEIVED_FILE_OK=True" "ciao dal peer rust"; do grep -q "$k" "$tmp/py.log" || { echo "MANCA: $k"; ok=0; }; done
[[ $ok == 1 ]] && echo "INTEROP OK" || { echo "INTEROP FALLITO"; exit 1; }
