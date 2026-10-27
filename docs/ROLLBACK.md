# Tornare a una versione precedente conservando i dati

Le versioni 3.x e 4.x (Python) e 5.0 (Rust) **non cancellano né modificano i dati le une delle altre**:
`~/.local/share/pixel-hub/` (script 3.x), `~/.config/pixel-hub-v3/` e `~/.cache/pixel-hub-v3/` (config e certificato 3.x),
cartelle di file e foto. Le cose della 4.x/5.0 stanno in `~/.config/pixel-hub/` (impostazioni e `backups/`) e
`~/.local/share/pixel-hub/identity/` (identità, **copiata** dalla 3.x).

La 5.0 legge e scrive lo stesso `settings.json` (schema 4) della 4.0 Python: si può passare dall'una all'altra.
Chiavi nuove della 5.0 (`wizard_fatto`, `preferiti[*].confermato`): la 4.0 Python le ignora e può scartarle se riscrive il file.
Le identità create dalla 5.0 sono ECDSA P-256, quelle delle versioni Python RSA 2048: entrambe sono lette da entrambe le versioni.

1. Chiudi Pixel Hub 5: *Impostazioni › Esci* (o `pkill -x pixel-hub`).
2. Installata con `scripts/installa-utente.sh`: `scripts/installa-utente.sh rimuovi` (rimuove il binario e il lanciatore 5.0,
   **ripristina** il lanciatore e l'avvio automatico precedenti dalle copie salvate). Installata via RPM: `sudo dnf remove pixel-hub`.
3. Per la 4.0 Python: `legacy/python/scripts/installa-utente.sh` (richiede le dipendenze Python/GTK4: vedi `legacy/python/`).
   Per la 3.x: `bash reference/installa-pixel-hub-fedora.sh` (ricrea script e lanciatore; riusa la configurazione `pixel-hub-v3`).
4. Avvia la versione scelta dal menu. Il telefono associato (3.x) o i preferiti (4.x/5.0) e le cartelle sono quelli di prima.

Cosa **non** torna indietro: preferiti confermati dal wizard, PIN e impostazioni cambiati *dopo* la migrazione (la 3.x non li conosce);
restano comunque in `~/.config/pixel-hub/` e nei suoi `backups/`. Non avviare due versioni insieme: usano la stessa porta
(la 5.0 e la 4.0 rifiutano di partire se vedono una versione Python in esecuzione).
Per ripartire da zero senza perdere nulla: *Impostazioni › Ripristina impostazioni* (backup automatico; file e identità intatti).
