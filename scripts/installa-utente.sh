#!/usr/bin/env bash
# Installazione per l'utente corrente (senza RPM, senza sudo, senza dnf): tutto sotto ~/.local.
#   scripts/installa-utente.sh           compila (se serve) e installa binario, icone, lanciatore e voce per Nautilus
#   scripts/installa-utente.sh rimuovi   rimuove Cable e ripristina lanciatore/avvio automatico di Pixel Hub (se salvati)
# Per compilare servono (da installare tu, una volta):
#   sudo dnf install cargo rust gcc gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel
# Per eseguire: gtk3, webkit2gtk4.1, libsoup3 (già presenti su Fedora Workstation). Facoltativi: android-tools, scrcpy (solo Android).
set -euo pipefail
ID=it.frank.Cable
OLD_ID=it.frank.PixelHub
SRC="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HOME/.local/bin/cable"
OLD_BIN="$HOME/.local/bin/pixel-hub"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor"
BAK="$HOME/.local/share/cable-launcher-backup"
NAUT="$HOME/.local/share/nautilus/scripts/Invia con Cable"
AUTO="$HOME/.config/autostart/$ID.desktop"
OLD_AUTO="$HOME/.config/autostart/$OLD_ID.desktop"

case "${1:-installa}" in
  installa)
    if (( EUID == 0 )); then echo "Esegui come utente normale, senza sudo."; exit 1; fi
    if pgrep -x pixel-hub >/dev/null; then echo "Pixel Hub è ancora aperto: chiudilo prima (Impostazioni › Esci)."; exit 1; fi
    if pgrep -f '^python3 .*(pixel_hub\.py|-m pixelhub)' >/dev/null; then
      echo "Una versione Python di Pixel Hub è in esecuzione: chiudila prima."; exit 1
    fi
    # ricompila sempre (incrementale: è rapido se nulla è cambiato), così non si installa un binario vecchio di un altro commit
    if command -v cargo >/dev/null; then (cd "$SRC" && cargo build --release -p cable)
    elif [[ ! -x "$SRC/target/release/cable" ]]; then echo "cargo non trovato: vedi le dipendenze in cima a questo script."; exit 1
    else echo "Attenzione: cargo non trovato, installo il binario già presente in target/release (potrebbe non corrispondere ai sorgenti)."; fi
    mkdir -p "$APPS" "$BAK" "$(dirname "$BIN")"
    # il lanciatore di Pixel Hub si salva (non si cancella) e poi si toglie, per non avere due icone
    if [[ -f "$APPS/$OLD_ID.desktop" ]]; then
      cp -p "$APPS/$OLD_ID.desktop" "$BAK/$OLD_ID.desktop"; rm -f "$APPS/$OLD_ID.desktop"
      echo "Lanciatore di Pixel Hub salvato in $BAK e tolto dal menu."
    fi
    if [[ -x /usr/bin/cable ]]; then
      echo "Attenzione: esiste anche il pacchetto di sistema (/usr/bin/cable). Questa installazione in ~/.local/bin ha la precedenza nel PATH"
      echo "e nel menu: per usare il pacchetto, esegui «$0 rimuovi». Controlla quale parte: «cable --versione» e «command -v cable»."
    fi
    install -m755 "$SRC/target/release/cable" "$BIN"
    for d in "$SRC"/data/icons/hicolor/*/apps; do
      s="$(basename "$(dirname "$d")")"
      install -Dm644 "$d/$ID.png" "$ICONS/$s/apps/$ID.png"
    done
    sed "s|^Exec=cable %F|Exec=$BIN %F|" "$SRC/data/$ID.desktop" > "$APPS/$ID.desktop"
    # voce «Scripts › Invia con Cable» nel menu contestuale di Nautilus (non richiede estensioni né pacchetti)
    mkdir -p "$(dirname "$NAUT")"
    cat > "$NAUT" <<SH
#!/usr/bin/env bash
# Passa a Cable i file selezionati: si sceglie il destinatario nella finestra.
[[ -n "\${NAUTILUS_SCRIPT_SELECTED_FILE_PATHS:-}" ]] || exit 0
mapfile -t F <<<"\$NAUTILUS_SCRIPT_SELECTED_FILE_PATHS"
exec "$BIN" "\${F[@]}"
SH
    chmod +x "$NAUT"
    update-desktop-database "$APPS" 2>/dev/null || true
    gtk-update-icon-cache -q "$ICONS" 2>/dev/null || true
    echo "Installato in $BIN. Cerca «Cable» nel menu delle applicazioni."
    echo "In Nautilus: tasto destro su file › Scripts › Invia con Cable."
    echo "Le impostazioni e l'identità di Pixel Hub (~/.config/pixel-hub, ~/.local/share/pixel-hub) non vengono modificate:"
    echo "al primo avvio Cable ne importa una copia." ;;
  rimuovi)
    # ferma solo il Cable avviato da QUESTA installazione (non un eventuale pacchetto di sistema in /usr/bin)
    for pid in $(pgrep -x cable 2>/dev/null || true); do
      [[ "$(readlink "/proc/$pid/exe" 2>/dev/null || true)" == "$BIN" ]] && kill "$pid" 2>/dev/null || true
    done
    rm -f "$BIN" "$APPS/$ID.desktop" "$NAUT"
    for d in "$ICONS"/*/apps; do rm -f "$d/$ID.png"; done
    [[ -f "$BAK/$OLD_ID.desktop" ]] && cp -p "$BAK/$OLD_ID.desktop" "$APPS/$OLD_ID.desktop" && echo "Ripristinato il lanciatore di Pixel Hub."
    last=$(ls -1t "$HOME"/.config/cable/backups/autostart-pixel-hub-*.desktop 2>/dev/null | head -1 || true)
    if [[ -n "$last" ]]; then cp -p "$last" "$OLD_AUTO"; rm -f "$AUTO"; echo "Ripristinato l'avvio automatico di Pixel Hub."
    elif [[ -f "$AUTO" ]]; then rm -f "$AUTO"; echo "Rimosso l'avvio automatico di Cable."; fi
    update-desktop-database "$APPS" 2>/dev/null || true
    gtk-update-icon-cache -q "$ICONS" 2>/dev/null || true
    echo "Rimosso. Impostazioni (~/.config/cable), identità (~/.local/share/cable), copie di sicurezza e file ricevuti sono rimasti." ;;
  *) sed -n '2,4p' "$0"; exit 1 ;;
esac
