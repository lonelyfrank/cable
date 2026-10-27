# Cable

Condivisione di file e testi fra **Linux** e **smartphone Android o iPhone/iPad**, sulla rete locale, con il protocollo
**LocalSend v2**. Cable non è legato a un marchio di telefono: parla con qualunque app che implementi LocalSend
(l'app ufficiale LocalSend esiste per Android e iOS). Nessun account, cloud o relay; non serve LocalSend per PC e non va
tenuto aperto insieme a Cable (usa la stessa porta).

> Cable è il nuovo nome di *Pixel Hub*. Impostazioni, dispositivi ricordati e identità si importano da soli: vedi
> [Migrazione da Pixel Hub](#migrazione-da-pixel-hub).

**Stack:** motore in **Rust** (tokio, hyper, rustls) + guscio **Tauri 2** con interfaccia HTML/CSS/JS. Un solo binario, nessun Python
a runtime. La vecchia versione Python è in `legacy/python/` (riferimento e prove di interoperabilità).

## Requisiti e compatibilità

| | |
|---|---|
| **PC** | Linux con GTK 3 e WebKitGTK 4.1 (Fedora Workstation li ha già). Verificato **solo su Fedora 44 GNOME (Wayland)**. Altre distribuzioni e desktop: i requisiti sono gli stessi (`gtk3`, `webkit2gtk-4.1`, `libsoup3`), ma **non sono stati provati**. |
| **Smartphone** | Un'app compatibile con LocalSend v2 **con cifratura attiva (HTTPS)**: serve sempre, Cable non è un'app mobile. Android e iOS sono trattati allo stesso modo; **non è stata fatta alcuna prova con telefoni reali** (vedi [TEST](docs/TEST.md)). |
| **Rete** | Stessa rete locale (Wi-Fi, Ethernet, hotspot). Il multicast può essere bloccato da router/AP con isolamento dei client: in quel caso resta l'inserimento dell'IP. Solo IPv4. |
| **Facoltativi (solo Android)** | `adb` (batteria, spazio, importazione foto via USB) e `scrcpy` (schermo). Sono indipendenti dai trasferimenti: se mancano, le loro schede non compaiono e nulla segnala errore (neppure con un iPhone). |
| **Facoltativi (Linux)** | NetworkManager e logind (D-Bus) per accorgersi subito di cambi di rete e ripresa dalla sospensione; senza, funziona lo stesso con un controllo ogni 5 s. `gio`/`gdbus` per «Apri» e «Mostra nella cartella». |

Non si dichiara compatibilità universale: ogni app/versione di terze parti va verificata con la
[checklist manuale](docs/CHECKLIST-MANUALE.md).

## Come si usa

1. Al primo avvio parte la **procedura guidata** (saltabile, riapribile da Impostazioni): trova lo smartphone e, se vuoi, lo
   verifica con un codice. Vedi [docs/WIZARD.md](docs/WIZARD.md). La verifica è facoltativa e non blocca l'uso.
2. **Inviare:** trascina file/cartelle **su un dispositivo** (parte la preparazione dell'invio a lui) oppure **sulla finestra**
   (scegli il destinatario). Lo stesso vale aprendo Cable con dei file (`cable foto.jpg`, o *Scripts › Invia con Cable* in Nautilus).
   Se il destinatario ha un PIN, Cable lo chiede al momento e non lo salva.
3. **Ricevere:** arriva una richiesta con mittente, numero e dimensione dei file; accetti tutto, **solo alcuni file**, o rifiuti.
   A fine ricezione: **Apri** e **Mostra nella cartella**. Le notifiche hanno i pulsanti anche a finestra chiusa.
4. **Se qualcosa non va:** *Aggiorna* (riannuncia e riverifica i dispositivi), IP manuale in *Condividi*, poi
   *Impostazioni › Diagnostica*. Un invio non riuscito mostra l'esito per file e **«Riprova i file non completati»**.
5. Chiudere la finestra lascia attiva la ricezione (se abilitato); per uscire: *Impostazioni › Esci*.

### Stato dei dispositivi (quattro cose diverse)

* **Ricordato**: c'è un record salvato (nome, ultimo indirizzo). La stella ★ è solo una scorciatoia per tenerlo in cima.
* **Vicino**: annunciato o visto negli ultimi 2 minuti (presenza). Dopo un cambio di rete o la sospensione decade finché non si riannuncia.
* **Raggiungibile**: l'ultima connessione TLS verso uno dei suoi indirizzi è riuscita (o fallita: «visto, ma non risponde»).
* **Confermato**: hai verificato con il codice del wizard che l'impronta è del dispositivo che hai in mano.

## Sviluppo, installazione, avvio

Dipendenze di compilazione (da installare una volta, non le installo io):
```
sudo dnf install cargo rust gcc gtk3-devel webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel
```

| Comando | Effetto |
|---|---|
| `scripts/installa-utente.sh` | compila e installa in `~/.local`: binario `cable`, icone, voce **Cable** nel menu, script per Nautilus. Senza sudo né dnf. Salva il lanciatore di Pixel Hub, se c'è |
| `scripts/installa-utente.sh rimuovi` | rimuove Cable e ripristina lanciatore/avvio automatico di Pixel Hub |
| `scripts/dev.sh run [FILE...]` | avvia (debug) con la tua configurazione reale |
| `scripts/dev.sh sandbox` | avvia con dati isolati in `/tmp`, senza multicast |
| `scripts/dev.sh test` | test del motore: unitari + integrazione end-to-end su TLS reale |
| `scripts/dev.sh ui-test` | prova la finestra reale (30 controlli) e salva gli screenshot |
| `scripts/dev.sh interop` | prova Python ↔ Rust fra le due implementazioni indipendenti |
| `scripts/dev.sh release` | binario ottimizzato in `target/release/cable` |
| `cargo run --release -p pixelhub-engine --example bench` | misure sul loopback (latenza, velocità, memoria) |

Da riga di comando: `cable [--background] [FILE...]`, `--migra`, `--diagnostica`, `--versione`.
RPM: `packaging/cable.spec` (usa `scripts/dev.sh vendor`) — **non costruito** (`rpmbuild` assente).
Fedora Atomic, Flatpak: **non verificati**. Se Pixel Hub 5.0 è ancora aperto, Cable non parte (stessa porta): chiudilo prima.

## Migrazione da Pixel Hub

Al primo avvio Cable **copia** (non sposta) da `~/.config/pixel-hub` e `~/.local/share/pixel-hub` a `~/.config/cable` e
`~/.local/share/cable`: impostazioni, dispositivi ricordati (con le conferme) e **identità TLS** (stessa impronta: i telefoni già
collegati continuano a riconoscere il PC). Il nome predefinito «Fedora Pixel Hub» diventa «Cable <host>»; un nome scelto da te resta.
È idempotente e non sovrascrive: se esistono dati di Cable, vincono quelli di Cable e lo dice. Le vecchie cartelle restano intatte
come copia di ripristino; l'avvio automatico e il lanciatore si riportano a Cable con copia di sicurezza.
Dettagli e ritorno indietro: [docs/ROLLBACK.md](docs/ROLLBACK.md). Cartelle dei file ricevuti: restano dove erano.

**Identificatori storici mantenuti** (cambiarli non porterebbe benefici e solo rischi): chiave `preferiti` in `settings.json`
(contiene i dispositivi *ricordati*), crate interno `pixelhub-engine`, nomi italiani degli stati e dei campi dell'interfaccia,
`legacy/` e `reference/`. Nuovi: binario `cable`, ID `it.frank.Cable`, cartelle `cable`, temporanei `.cable-*.part` (i vecchi
`.pixelhub-*.part` si ripuliscono comunque).

## Modello di sicurezza (in breve)

* Solo HTTPS, nessun ripiego su HTTP; in invio si verifica l'impronta SHA-256 del certificato *effettivamente visto* sulla connessione.
* **Limiti dell'identità.** L'identità di un dispositivo è l'impronta del suo certificato autofirmato, **non** il nome, il modello o
  l'indirizzo IP (dichiarati da lui o dalla rete). Con LocalSend v2 chi *ti invia* qualcosa non presenta un certificato verificabile:
  l'identità del mittente **non è dimostrabile**. Perciò ogni richiesta richiede conferma manuale e **non esiste auto-accettazione**,
  nemmeno per i preferiti. Nessuna fiducia al primo contatto salvo la conferma con codice, che vale solo per l'invio.
* Se un dispositivo ricordato si ripresenta con un'impronta diversa, Cable lo segnala e **non** trasferisce la fiducia: il vecchio
  record non cambia, il nuovo non è confermato; sostituire è un'azione esplicita.
* PIN di ricezione opzionale (PBKDF2 con sale; limite ai tentativi). Il PIN, il codice del wizard e i token non vanno nei log.
* Scritture solo nella cartella scelta: componenti sanificati, nessun collegamento simbolico seguito, temporanei univoci,
  rinomina atomica, mai sovrascrivere (`nome (1).ext`); i parziali si scartano anche se la connessione cade.
* Identità TLS in `~/.local/share/cable/identity/` (chiave 0600); un'identità illeggibile **non** viene rigenerata da sola.

## Limitazioni note

* «Riprova» **non** riprende dal byte interrotto (il protocollo interoperabile non lo prevede): rimanda da capo solo i file non
  completati. Se la connessione cade dopo l'ultimo byte, l'esito sul destinatario è **incerto** e Cable lo dice: riprovando può
  nascere una copia doppia (mai una sovrascrittura).
* Nessuna prova con telefoni reali né con le app ufficiali; multicast su Wi-Fi/hotspot reali, firewalld e VPN non provati.
* Solo IPv4; i file speciali e i collegamenti simbolici nelle cartelle inviate sono ignorati.
* L'estensione per Nautilus non c'è: c'è lo script *Scripts › Invia con Cable* (nessun pacchetto in più). Altri file manager:
  `cable FILE...` da terminale o dal menu «Apri con».

## Documentazione

[Architettura e decisioni](docs/ARCHITETTURA.md) · [Wizard](docs/WIZARD.md) · [Esiti dei test](docs/TEST.md) ·
[Checklist manuale](docs/CHECKLIST-MANUALE.md) · [Migrazione e ritorno indietro](docs/ROLLBACK.md).
I file originali sono in `reference/` (l'installer 3.1 non è mai stato eseguito).
