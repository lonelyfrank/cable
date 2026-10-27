# Pixel Hub 5.0

Trasferimento rapido di file e testi fra Fedora GNOME e smartphone con il protocollo **LocalSend v2**.
Sul telefono basta l'app ufficiale LocalSend; su Fedora Pixel Hub implementa direttamente il protocollo
(non serve LocalSend per PC e non va tenuto aperto: userebbe la stessa porta). Nessun account, cloud o relay.

**Stack:** motore in **Rust** (tokio, hyper, rustls) + guscio **Tauri 2** con l'interfaccia HTML/CSS/JS di sempre.
Un unico binario, nessun Python a runtime. La vecchia versione Python è in `legacy/python/` (riferimento e ritorno indietro).

## Come si usa

1. Al primo avvio parte la **procedura guidata** (saltabile, riapribile da Impostazioni): trova il telefono, lo verifica con
   un codice, prova la ricezione. Vedi [docs/WIZARD.md](docs/WIZARD.md).
2. **Inviare:** scegli o trascina file/cartelle sul dispositivo, oppure scrivi un testo in Home. Se il destinatario ha un PIN,
   Pixel Hub lo chiede al momento e non lo salva mai.
3. **Ricevere:** arriva una richiesta con mittente, numero e dimensione dei file; accetti tutto, **solo alcuni file**, o rifiuti.
   Le notifiche hanno i pulsanti Accetta/Rifiuta anche a finestra chiusa.
4. Se un dispositivo non compare: IP manuale in *Condividi*, poi *Impostazioni › Diagnostica*.
5. Chiudere la finestra lascia attiva la ricezione (se abilitato); per uscire: *Impostazioni › Esci*.

## Sviluppo, installazione, avvio

Dipendenze di compilazione (da installare una volta, non le installo io):
```
sudo dnf install cargo rust gcc gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel
```
Facoltative a runtime: `android-tools` (ADB), `scrcpy` (COPR `zeno/scrcpy`). Fedora tradizionale; Fedora Atomic **non verificato**.

| Comando | Effetto |
|---|---|
| `scripts/dev.sh run [FILE...]` | avvia (debug) con la tua configurazione reale |
| `scripts/dev.sh sandbox` | avvia con dati isolati in `/tmp`, senza multicast: non tocca `~/.config` |
| `scripts/dev.sh test` | test del motore: unitari + integrazione end-to-end su TLS reale |
| `scripts/dev.sh ui-test` | prova la finestra reale (30 controlli) e salva gli screenshot |
| `scripts/dev.sh interop` | prova Python ↔ Rust fra le due implementazioni indipendenti |
| `scripts/dev.sh release` | binario ottimizzato in `target/release/pixel-hub` |
| `scripts/installa-utente.sh` | installa in `~/.local` (binario, icona, lanciatore), senza sudo né dnf |
| `scripts/installa-utente.sh rimuovi` | rimuove e ripristina lanciatore/avvio automatico precedenti |

Da riga di comando: `pixel-hub [--background] [FILE...]`, `--migra`, `--diagnostica`, `--versione`.
RPM: `packaging/pixel-hub.spec` (usa `scripts/dev.sh vendor`) — **non costruito in questa sessione** (`rpmbuild` assente).

## Modello di sicurezza (in breve)

* Solo HTTPS, nessun ripiego su HTTP; in invio si verifica l'impronta SHA-256 del certificato *effettivamente visto*.
* In ricezione l'identità del mittente **non è verificabile** con LocalSend v2: ogni richiesta richiede conferma manuale,
  **non esiste auto-accettazione**. «Preferito» è una scorciatoia; «confermato» (dal wizard) prova solo l'identità del telefono per l'invio.
* PIN di ricezione opzionale (PBKDF2 con sale; max 10 errori ogni 5 minuti).
* Scritture solo nella cartella scelta: componenti sanificati, nessun collegamento simbolico seguito, temporanei univoci,
  rinomina atomica, mai sovrascrivere (`nome (1).ext`); i parziali si scartano anche se la connessione cade.
* Identità TLS in `~/.local/share/pixel-hub/identity/` (chiave 0600); un'identità illeggibile **non** viene rigenerata da sola.

## Documentazione

[Architettura e decisioni](docs/ARCHITETTURA.md) · [Wizard](docs/WIZARD.md) · [Esiti dei test](docs/TEST.md) ·
[Checklist manuale](docs/CHECKLIST-MANUALE.md) · [Tornare indietro](docs/ROLLBACK.md).
I file originali sono in `reference/` (l'installer 3.1 non è mai stato eseguito).
