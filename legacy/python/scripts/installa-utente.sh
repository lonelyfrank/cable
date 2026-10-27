#!/usr/bin/env bash
# Installazione per l'utente corrente dal sorgente (senza RPM, senza sudo, senza dnf).
#   scripts/installa-utente.sh           crea il lanciatore nel menu che punta a questa cartella
#   scripts/installa-utente.sh rimuovi   rimuove il lanciatore e ripristina quello della 3.x (se salvato)
# Dipendenze (da installare tu, una volta):
#   sudo dnf install python3-gobject python3-cryptography gtk4 libadwaita webkitgtk6.0 iproute xdg-utils
# Facoltative: android-tools, scrcpy (vedi README).
set -euo pipefail
ID=it.frank.PixelHub
SRC="$(cd "$(dirname "$0")/.." && pwd)"
APPS="$HOME/.local/share/applications"
ICONS="$HOME/.local/share/icons/hicolor/scalable/apps"
BAK="$HOME/.local/share/pixel-hub-v3-backup"
AUTO="$HOME/.config/autostart/$ID.desktop"
CFGBK="$HOME/.config/pixel-hub/backups"

case "${1:-installa}" in
  installa)
    if (( EUID == 0 )); then echo "Esegui come utente normale, senza sudo."; exit 1; fi
    python3 - <<'PY' || { echo "Mancano dipendenze: vedi l'elenco in cima a questo script."; exit 1; }
import gi
gi.require_version("Gtk", "4.0"); gi.require_version("Adw", "1"); gi.require_version("WebKit", "6.0")
import cryptography
PY
    if pgrep -f '^python3 [^ ]*/pixel-hub/pixel_hub\.py' >/dev/null; then
      echo "La versione 3.x è in esecuzione: chiudila prima (pkill -f '^python3 .*pixel_hub.py')."; exit 1
    fi
    mkdir -p "$APPS" "$ICONS" "$BAK"
    if [[ -f "$APPS/$ID.desktop" ]] && ! grep -q 'pixelhub' "$APPS/$ID.desktop" && [[ ! -f "$BAK/$ID.desktop" ]]; then
      cp -p "$APPS/$ID.desktop" "$BAK/$ID.desktop"      # lanciatore 3.x, per il ritorno indietro
      echo "Salvato il lanciatore 3.x in $BAK"
    fi
    cp "$SRC/../../data/icons/hicolor/scalable/apps/$ID.svg" "$ICONS/$ID.svg"
    cat > "$APPS/$ID.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Pixel Hub
GenericName=Trasferimento file
Comment=Invia e ricevi file e testi con i tuoi dispositivi via LocalSend
Exec=env PYTHONPATH=$SRC python3 -m pixelhub %F
Icon=$ID
Terminal=false
Categories=Network;FileTransfer;GTK;
StartupWMClass=$ID
EOF
    update-desktop-database "$APPS" 2>/dev/null || true
    gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    echo "Installato. Cerca «Pixel Hub» nel menu. I dati 3.x (~/.config/pixel-hub-v3, ~/.local/share/pixel-hub) non sono stati toccati."
    echo "Al primo avvio le impostazioni 3.x vengono importate (con copia di sicurezza in $CFGBK)." ;;
  rimuovi)
    pkill -f '^python3 -m pixelhub' 2>/dev/null || true
    rm -f "$APPS/$ID.desktop" "$ICONS/$ID.svg"
    [[ -f "$BAK/$ID.desktop" ]] && cp -p "$BAK/$ID.desktop" "$APPS/$ID.desktop" && echo "Ripristinato il lanciatore 3.x."
    last=$(ls -1t "$CFGBK"/autostart-v3-*.desktop 2>/dev/null | head -1 || true)
    if [[ -n "$last" ]]; then cp -p "$last" "$AUTO" && echo "Ripristinato l'avvio automatico 3.x."
    elif [[ -f "$AUTO" ]] && grep -q pixelhub "$AUTO"; then rm -f "$AUTO"; echo "Rimosso l'avvio automatico di questa versione."; fi
    update-desktop-database "$APPS" 2>/dev/null || true
    echo "Rimosso. Impostazioni (~/.config/pixel-hub), backup, identità (~/.local/share/pixel-hub) e file ricevuti sono rimasti." ;;
  *) sed -n '2,8p' "$0"; exit 1 ;;
esac
