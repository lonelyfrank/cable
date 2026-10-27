# Migrazione da Pixel Hub e ritorno indietro

## Cosa fa Cable al primo avvio (e a ogni avvio, senza effetti se già fatto)

| Elemento | Origine (Pixel Hub 5.0) | Destinazione (Cable) | Regola |
|---|---|---|---|
| Impostazioni, dispositivi ricordati, conferme, PIN (hash) | `~/.config/pixel-hub/settings.json` | `~/.config/cable/settings.json` | copia; l'originale è salvato anche in `~/.config/cable/backups/settings-pixel-hub-*.json`; solo il nome predefinito «Fedora Pixel Hub» è sostituito |
| Identità TLS (certificato + chiave) | `~/.local/share/pixel-hub/identity/` | `~/.local/share/cable/identity/` | copia byte per byte (**stessa impronta**), chiave 0600; mai rigenerata in automatico |
| Avvio automatico | `~/.config/autostart/it.frank.PixelHub.desktop` | `~/.config/autostart/it.frank.Cable.desktop` | copia di sicurezza in `backups/autostart-pixel-hub-*.desktop`, poi si toglie il vecchio per non avviare due app |
| Lanciatore | `~/.local/share/applications/it.frank.PixelHub.desktop` | `…/it.frank.Cable.desktop` | lo fa `scripts/installa-utente.sh`: salva il vecchio in `~/.local/share/cable-launcher-backup/` |
| File ricevuti | cartella scelta in Pixel Hub | invariata | il percorso è nelle impostazioni, nulla si sposta |

* **Si copia, non si sposta:** le vecchie cartelle non vengono né modificate né cancellate.
* **Mai sovrascrivere:** se esiste già un file di Cable (impostazioni o identità), vince quello di Cable; la migrazione lo segnala
  una volta e non tocca nulla. Una seconda esecuzione non cambia niente (provato con test automatici e sui dati reali, in copia).
* **Impostazioni illeggibili** nel vecchio Pixel Hub: Cable parte con i predefiniti e lo dice; l'originale resta.
* Stato della migrazione: `~/.config/cable/migrazione-pixel-hub.json`. `cable --migra` la esegue/mostra senza aprire la finestra.

## Tornare a Pixel Hub

1. Chiudi Cable (*Impostazioni › Esci*).
2. `scripts/installa-utente.sh rimuovi`: toglie Cable, ripristina il lanciatore e l'avvio automatico di Pixel Hub dalle copie.
   Installato via RPM: `sudo dnf remove cable`.
3. Avvia Pixel Hub: le sue cartelle (`~/.config/pixel-hub`, `~/.local/share/pixel-hub`) sono come le hai lasciate, identità inclusa.

Cosa **non** torna indietro: i dispositivi ricordati, le conferme, il PIN e le impostazioni cambiati *dopo* la migrazione stanno in
`~/.config/cable/` e Pixel Hub non li vede. Per riportarli indietro a mano: copia `~/.config/cable/settings.json` su
`~/.config/pixel-hub/settings.json` (formato compatibile; le chiavi nuove vengono ignorate).
**Non avviare Cable e Pixel Hub insieme**: usano la stessa porta (Cable rifiuta di partire se vede Pixel Hub in esecuzione).
Se avevi lasciato che **entrambi** usassero la stessa identità, nulla si rompe: sono due copie dello stesso certificato.

## Cambiare identità apposta

*Impostazioni › Rigenera identità*: i telefoni vedranno un dispositivo nuovo. I vecchi file restano in `identity/backup-*`.
*Impostazioni › Ripristina impostazioni*: copia automatica in `backups/`, file ricevuti e identità intatti.

## Versioni più vecchie (3.x e 4.x Python)

Le versioni Python usano `~/.config/pixel-hub-v3/` e `~/.config/pixel-hub/`; Cable non le modifica. La 4.x Python legge ancora il
vecchio `settings.json` di Pixel Hub (schema 4). Per la 3.x: `bash reference/installa-pixel-hub-fedora.sh`.
