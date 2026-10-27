#!/usr/bin/env bash
# Installazione per l'utente corrente (senza RPM/DEB, senza sudo): tutto sotto ~/.local.
#   scripts/installa-utente.sh           compila e installa binario, icone, lanciatore e voce per Nautilus
#   scripts/installa-utente.sh aggiorna  scarica le novità del repository (git pull) e reinstalla
#   … aggiorna --apri                     come sopra, poi riapre Cable con la finestra (lo usa il pulsante «Aggiorna e riavvia»)
# Se Cable era aperto, dopo un'installazione riuscita viene riavviato da solo; se la compilazione fallisce resta quello di prima.
#   scripts/installa-utente.sh rimuovi   rimuove Cable e ripristina lanciatore/avvio automatico di Pixel Hub (se salvati)
# Per compilare servono (da installare tu, una volta; lo script ricorda quelli giusti se la compilazione fallisce):
#   Ubuntu/Debian: sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev \
#                    libjavascriptcoregtk-4.1-dev librsvg2-dev libdbus-1-dev libxdo-dev libssl-dev   + Rust (https://rustup.rs)
#   Fedora:        sudo dnf install cargo rust gcc dbus-devel gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel
# Per eseguire: GTK 3, WebKitGTK 4.1, libsoup 3 (già presenti su Fedora Workstation e Ubuntu desktop). Facoltativi: adb, scrcpy (solo Android).
set -euo pipefail
# Rust installato con rustup: dal menu delle applicazioni ~/.cargo/bin non è nel PATH
[[ -d "$HOME/.cargo/bin" ]] && PATH="$HOME/.cargo/bin:$PATH"

dipendenze() {
  local id=""; [[ -r /etc/os-release ]] && id="$(. /etc/os-release; echo "${ID:-} ${ID_LIKE:-}")"
  echo "Per compilare servono Rust e le librerie di sviluppo. Installale una volta, poi riprova:"
  case " $id " in
    *" debian "*|*" ubuntu "*)
      echo "  sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev librsvg2-dev libdbus-1-dev libxdo-dev libssl-dev"
      echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh        # Rust aggiornato (poi riapri il terminale)" ;;
    *" fedora "*|*" rhel "*)
      echo "  sudo dnf install cargo rust gcc dbus-devel gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel" ;;
    *) echo "  GTK 3, WebKitGTK 4.1, libsoup 3, librsvg, libdbus (pacchetti -dev/-devel), pkg-config, un compilatore C e Rust (https://rustup.rs)" ;;
  esac
}
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

MODO="${1:-installa}"
APRI=""; [[ "${2:-}" == --apri ]] && APRI=1
if [[ "$MODO" == aggiorna ]]; then
  (( EUID == 0 )) && { echo "Esegui come utente normale, senza sudo."; exit 1; }
  [[ -d "$SRC/.git" ]] || { echo "Questa copia non è un clone git: non posso aggiornarla da qui."; exit 1; }
  echo "Scarico le novità…"
  git -C "$SRC" pull --ff-only || { echo "git pull non è riuscito (modifiche locali o rami divergenti): risolvi in $SRC e riprova."; exit 1; }
  MODO=installa
fi
case "$MODO" in
  installa)
    if (( EUID == 0 )); then echo "Esegui come utente normale, senza sudo."; exit 1; fi
    if pgrep -x pixel-hub >/dev/null; then echo "Pixel Hub è ancora aperto: chiudilo prima (Impostazioni › Esci)."; exit 1; fi
    if pgrep -f '^python3 .*(pixel_hub\.py|-m pixelhub)' >/dev/null; then
      echo "Una versione Python di Pixel Hub è in esecuzione: chiudila prima."; exit 1
    fi
    if pgrep -f 'iphone_hub\.py' >/dev/null; then
      echo "iPhone Hub è aperto e usa la stessa porta di Cable: chiudilo prima (pkill -f iphone_hub.py)."; exit 1
    fi
    # ricompila sempre: se non cambia nulla, cargo non rifà niente; così dopo un git pull il binario è quello nuovo
    if command -v cargo >/dev/null; then
      (cd "$SRC" && cargo build --release -p cable) || { echo; dipendenze; exit 1; }
    elif [[ ! -x "$SRC/target/release/cable" ]]; then
      echo "cargo non trovato."; dipendenze; exit 1
    fi
    # come riavviare dopo: con la finestra se richiesto o se era aperto così, altrimenti di nuovo in background
    RIAVVIA=""
    if pid=$(pgrep -xo cable); then
      RIAVVIA=finestra
      [[ -z "$APRI" ]] && tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | grep -q -- --background && RIAVVIA=background
    fi
    mkdir -p "$APPS" "$BAK" "$(dirname "$BIN")"
    # il lanciatore di Pixel Hub si salva (non si cancella) e poi si toglie, per non avere due icone
    if [[ -f "$APPS/$OLD_ID.desktop" ]]; then
      cp -p "$APPS/$OLD_ID.desktop" "$BAK/$OLD_ID.desktop"; rm -f "$APPS/$OLD_ID.desktop"
      echo "Lanciatore di Pixel Hub salvato in $BAK e tolto dal menu."
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
    if [[ -n "$RIAVVIA" ]]; then
      pkill -x cable || true
      for _ in $(seq 40); do pgrep -x cable >/dev/null || break; sleep 0.25; done
      pkill -9 -x cable 2>/dev/null || true
      if [[ "$RIAVVIA" == background ]]; then setsid "$BIN" --background >/dev/null 2>&1 < /dev/null &
      else setsid "$BIN" >/dev/null 2>&1 < /dev/null & fi
      disown || true
      echo "Cable riavviato con la nuova versione."
    fi
    echo "In Nautilus: tasto destro su file › Scripts › Invia con Cable."
    echo "Le impostazioni e l'identità di Pixel Hub (~/.config/pixel-hub, ~/.local/share/pixel-hub) non vengono modificate:"
    echo "al primo avvio Cable ne importa una copia." ;;
  rimuovi)
    pkill -x cable 2>/dev/null || true
    rm -f "$BIN" "$APPS/$ID.desktop" "$NAUT"
    for d in "$ICONS"/*/apps; do rm -f "$d/$ID.png"; done
    [[ -f "$BAK/$OLD_ID.desktop" ]] && cp -p "$BAK/$OLD_ID.desktop" "$APPS/$OLD_ID.desktop" && echo "Ripristinato il lanciatore di Pixel Hub."
    last=$(ls -1t "$HOME"/.config/cable/backups/autostart-pixel-hub-*.desktop 2>/dev/null | head -1 || true)
    if [[ -n "$last" ]]; then cp -p "$last" "$OLD_AUTO"; rm -f "$AUTO"; echo "Ripristinato l'avvio automatico di Pixel Hub."
    elif [[ -f "$AUTO" ]]; then rm -f "$AUTO"; echo "Rimosso l'avvio automatico di Cable."; fi
    update-desktop-database "$APPS" 2>/dev/null || true
    gtk-update-icon-cache -q "$ICONS" 2>/dev/null || true
    echo "Rimosso. Impostazioni (~/.config/cable), identità (~/.local/share/cable), copie di sicurezza e file ricevuti sono rimasti." ;;
  *) sed -n '2,5p' "$0"; exit 1 ;;
esac
