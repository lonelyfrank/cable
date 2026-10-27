#!/usr/bin/env bash
# Installazione per l'utente corrente (senza RPM, senza sudo, senza dnf): tutto sotto ~/.local.
#   scripts/installa-utente.sh           compila (se serve) e installa binario, icona e lanciatore
#   scripts/installa-utente.sh rimuovi   rimuove tutto e ripristina il lanciatore/avvio automatico precedenti (se salvati)
# Per compilare servono (da installare tu, una volta):
#   sudo dnf install cargo rust gcc gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel
# Per eseguire: gtk3, webkit2gtk4.1, libsoup3 (già presenti su Fedora Workstation). Facoltativi: android-tools, scrcpy.
set -euo pipefail
ID=it.frank.PixelHub
SRC="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$HOME/.local/bin/pixel-hub"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"
BAK="$HOME/.local/share/pixel-hub-launcher-backup"
AUTO="$HOME/.config/autostart/$ID.desktop"
CFGBK="$HOME/.config/pixel-hub/backups"

case "${1:-installa}" in
  installa)
    if (( EUID == 0 )); then echo "Esegui come utente normale, senza sudo."; exit 1; fi
    if pgrep -f '^python3 .*(pixel_hub\.py|-m pixelhub)' >/dev/null; then
      echo "Una versione Python di Pixel Hub è in esecuzione: chiudila prima (pkill -f '^python3 .*(pixel_hub.py|-m pixelhub)')."; exit 1
    fi
    if [[ ! -x "$SRC/target/release/pixel-hub" ]]; then
      command -v cargo >/dev/null || { echo "cargo non trovato: vedi le dipendenze in cima a questo script."; exit 1; }
      (cd "$SRC" && cargo build --release -p pixel-hub)
    fi
    mkdir -p "$APPS" "$ICONS" "$BAK" "$(dirname "$BIN")"
    if [[ -f "$APPS/$ID.desktop" ]] && ! grep -q "$BIN" "$APPS/$ID.desktop" && [[ ! -f "$BAK/$ID.desktop" ]]; then
      cp -p "$APPS/$ID.desktop" "$BAK/$ID.desktop"       # lanciatore precedente (3.x o 4.x Python)
      echo "Salvato il lanciatore precedente in $BAK"
    fi
    install -m755 "$SRC/target/release/pixel-hub" "$BIN"
    install -m644 "$SRC/data/icons/hicolor/scalable/apps/$ID.svg" "$ICONS/$ID.svg"
    cat > "$APPS/$ID.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pixel Hub
GenericName=Trasferimento file
Comment=Invia e ricevi file e testi con i tuoi dispositivi via LocalSend
Exec=$BIN %F
Icon=$ID
Terminal=false
Categories=Network;FileTransfer;GTK;
Keywords=localsend;airdrop;pixel;android;file;condividi;trasferimento;scrcpy;
StartupWMClass=pixel-hub
StartupNotify=true
EOF
    update-desktop-database "$APPS" 2>/dev/null || true
    gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    echo "Installato in $BIN. Cerca «Pixel Hub» nel menu."
    echo "Le impostazioni precedenti (~/.config/pixel-hub-v3 e ~/.config/pixel-hub) non sono state modificate: al primo avvio vengono"
    echo "importate/lette con copia di sicurezza in $CFGBK." ;;
  rimuovi)
    pkill -x pixel-hub 2>/dev/null || true
    rm -f "$BIN" "$APPS/$ID.desktop" "$ICONS/$ID.svg"
    [[ -f "$BAK/$ID.desktop" ]] && cp -p "$BAK/$ID.desktop" "$APPS/$ID.desktop" && echo "Ripristinato il lanciatore precedente."
    last=$(ls -1t "$CFGBK"/autostart-*.desktop 2>/dev/null | head -1 || true)
    if [[ -n "$last" ]]; then cp -p "$last" "$AUTO" && echo "Ripristinato l'avvio automatico precedente."
    elif [[ -f "$AUTO" ]] && grep -q "$BIN" "$AUTO"; then rm -f "$AUTO"; echo "Rimosso l'avvio automatico di questa versione."; fi
    update-desktop-database "$APPS" 2>/dev/null || true
    echo "Rimosso. Impostazioni (~/.config/pixel-hub), backup, identità (~/.local/share/pixel-hub) e file ricevuti sono rimasti." ;;
  *) sed -n '2,8p' "$0"; exit 1 ;;
esac
