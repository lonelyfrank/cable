# Architettura, analisi e decisioni (Cable 6.0, già Pixel Hub 5.0)

## 0. Cable 6.0: cosa è cambiato rispetto a Pixel Hub 5.0

Analisi di partenza (stato reale al momento del lavoro): motore e GUI già separati, 51 test verdi, ma il progetto si presentava come
«Pixel Hub»/«Fedora» (nome, notifiche, lanciatore, testi della UI, nome predefinito, cartelle `Pixel`), il registro teneva **un solo
indirizzo per dispositivo** (l'ultimo annuncio, non provato, vinceva), la presenza non decadeva dopo cambio rete/sospensione,
`deviceType` mancante diventava «mobile», il gateway per l'hotspot era «il primo» di `/proc/net/route` (anche una VPN) e un annuncio
con certificato incoerente **rimuoveva l'intero dispositivo** (cancellabile da chiunque in rete). Nessuna restrizione a un solo
telefono/marchio nel codice di Rust (la 3.1 Bash sì, già superata); la scheda «Pixel» della UI era però un'assunzione di prodotto.

| Area | Modifica | Dove |
|---|---|---|
| Identità del prodotto | nome Cable ovunque; binario `cable`, ID `it.frank.Cable`, cartelle `cable`, icona nuova (PNG hicolor 16–512) | `paths`, `desktop`, `src-tauri`, `data/`, `packaging/`, UI |
| Migrazione | copia idempotente e non distruttiva di impostazioni + identità + avvio automatico; registro in `migrazione-pixel-hub.json` | `migrate.rs`, `desktop::migra_avvio_automatico` |
| Indipendenza dal marchio | modello/tipo sconosciuti restano vuoti; niente prefissi «Pixel/Android» nelle iniziali; schede ADB/scrcpy visibili solo se i programmi esistono | `protocol`, UI |
| Registro | più indirizzi per impronta (max 4) con *visto* e *confermato* separati; nessun annuncio non provato cambia il record salvato; etichette distinte per nomi uguali; avviso di cambio d'identità | `devices.rs` |
| Quattro proprietà | `ricordato`, `vicino`, `raggiungibile`, `confermato` indipendenti nello snapshot; la stella è solo `preferito` | `devices.rs`, `settings.rs` |
| Discovery | filtro interfacce virtuali (docker, virbr, veth…); gateway ordinati per metrico senza VPN; `refresh` parallelo con probe TLS (2 s); presenza che decade; salto d'orologio = ripresa; D-Bus NetworkManager/logind come acceleratore (raffiche accorpate, 1,5 s) | `discovery.rs`, `netwatch.rs` |
| Invio | indirizzi provati **in parallelo** (3 s, una configurazione TLS per tentativo) prima di calcolare gli hash; stati `scaduto` e `interrotto`; esito per file con byte inviati; ETA solo se attendibile; `riprova` | `sender.rs`, `client.rs`, `util.rs` |
| Messaggi | fatti osservati + cause possibili; nessun timeout attribuito al firewall | `client.rs` |
| UI | scelta del destinatario per file portati da fuori (CLI, file manager, rilascio sulla finestra); «Apri» e «Mostra nella cartella»; «Riprova»; avvisi di identità | `ui/` |

### Decisioni e identificatori storici

* **Nomi tecnici cambiati** (visibili all'utente/sistema, migrazione fornita): binario, ID applicazione, cartelle, avvio automatico, lanciatore.
* **Nomi tecnici mantenuti** (rischio senza beneficio): chiave `preferiti` in `settings.json` (ora *ricordati*; il campo `preferito` distingue
  la stella; i record vecchi senza campo valgono come stellati), crate `pixelhub-engine`, nomi italiani di stati/campi, `legacy/`, `reference/`,
  schema 4 delle impostazioni (compatibile all'indietro).
* **zbus** è già nell'albero (lo usa `notify-rust`): dichiararlo nel motore non compila crate in più. Serve per gli eventi D-Bus senza processi
  esterni né polling. Nessun'altra dipendenza nuova.
* **Un invio verso un indirizzo qualsiasi è sempre verificato dall'impronta**: perciò provare più indirizzi in parallelo è sicuro, ma ogni tentativo
  ha una configurazione TLS propria (una condivisa permetterebbe a una connessione di leggere l'impronta vista da un'altra).
* **Fiducia** (limiti espliciti): vedi §4. Il nome non è mai usato per decidere nulla, solo per *avvisare* (cambio d'identità) e per etichettare.

## 1. Analisi della base 3.1 (installer `reference/installa-pixel-hub-fedora.sh`)

Letto il codice senza eseguire l'installer (estratto in `reference/extracted/`).

**Confermato nel codice**

| Punto | Evidenza |
|---|---|
| Sorgenti incorporati in Bash | Python, HTML/CSS/JS e SVG sono heredoc dell'installer |
| WebKit in finestra GTK + server HTTP locale con token | `finestra()`, `ServerUI`/`GestoreUI` |
| File caricati prima nella cache | `/api/prepara` scrive in `~/.cache/pixel-hub-v3/uscita/` |
| Wizard obbligatorio, un solo telefono | `Hub.pronto()` blocca quasi tutte le API; `aggiungi_dispositivo` scarta ogni impronta diversa da quella associata |
| Certificato e chiave nella cache | `Certificato`: `CACHE/cert.pem`, `key.pem` |
| Controllo ricezione sull'impronta *dichiarata* | `prepara()` confronta `info.fingerprint` del JSON: chiunque può copiarla |
| Polling dell'intera app | la UI richiede `/api/stato` ogni secondo; ogni richiesta lancia `nmcli`/`ip` e `scandir` |
| Integrità/concorrenza | nessun controllo dei byte sulla dimensione dichiarata; `sha256` ignorato; temporaneo `nome.part` fisso; `libera()` poi `replace()` non atomici (può sovrascrivere); stesso file ricaricabile più volte |
| Annullamento/scadenze | ricezione non annullabile dall'utente; sessioni che non scadono mai; thread di connessione illimitati |

**Ipotesi non verificate** (`CHECKLIST-MANUALE.md`): come l'app ufficiale gestisca 401/PIN ripetuti e `sha256`/`metadata`;
se presenti un certificato client (la specifica non ne parla: per questo non ci si fa affidamento); multicast su hotspot e reti con isolamento.

## 2. Protocollo: fonti e scelte

Consultata la specifica ufficiale (`localsend/protocol`: discovery, prepare-upload, upload, cancel, fingerprint). **Non ho letto il codice
del client ufficiale.** Dalla specifica: multicast `224.0.0.167:53317`; in HTTPS l'impronta è lo SHA-256 del certificato e il campo
`fingerprint` della register è ignorato; `prepare-upload` → `sessionId` + token per file; PIN come `?pin=`; codici
204/400/401/403/409/429/500; upload con 422 se `sha256` non coincide; `cancel?sessionId=`.

Scelte mie dove la specifica tace: limiti (5000 file, 8 MiB di JSON, 5 richieste in attesa, 32 connessioni, 120 s di attesa decisione, 120 s
di inattività sessione); `507` per spazio insufficiente (non standard: i client lo mostrano come errore generico); invii di solo testo = file con
`preview` testuale, accettazione manuale, risposta `204` (se il ricevente risponde 200 si carica comunque il testo); versione annunciata `2.0`;
nessuna funzione di v3 (nonce) né Download API; solo IPv4.

## 3. Struttura del progetto

```
crates/engine/   libreria Rust del motore (nessuna dipendenza da GUI); test unitari, di ricevitore e end-to-end
src-tauri/       guscio Tauri 2: finestra, comandi, eventi, notifiche, istanza unica; banco di prova `ui-test`
ui/              interfaccia HTML/CSS/JS (index.html + app.js), invariata nell'aspetto rispetto alla 3.1; incorporata nel binario
legacy/python/   versione Python 3.1/4.0 (riferimento, ritorno indietro, peer per la prova di interoperabilità)
packaging/ data/ scripts/ docs/ tests/interop/ reference/
```

| Modulo del motore | Responsabilità |
|---|---|
| `engine` | facade: ciclo di vita, azioni, `snapshot()` per la UI, diagnostica, **wizard di verifica** |
| `receiver` | richieste, sessioni, token, upload in streaming, scadenze, annullamento, pulizia dei parziali |
| `server` | server HTTPS (TLS per connessione con timeout, limite connessioni, hyper) |
| `sender` | invio in streaming, PIN, accettazione parziale, annullamento = abbandono del future (i socket si chiudono subito) |
| `discovery`, `devices`, `client`, `shared` | multicast, IP manuale, registro, client HTTPS con verifica d'impronta, stato condiviso |
| `identity`, `settings`, `paths`, `protocol`, `util`, `bus` | identità TLS, impostazioni e migrazione, percorsi, validazione, utilità, eventi |
| `android`, `desktop` | scrcpy e ADB (opzionali, separati); cartelle, file manager, avvio automatico, istanze Python |
| `ui_api` | metodi che la pagina può chiamare (percorsi consentiti, validazione); testato senza Tauri |

### Decisioni rilevanti
* **Tauri 2 = GTK3 + webkit2gtk-4.1**, non GTK4/libadwaita: la finestra non è «libadwaita vera». Scelta voluta per mantenere l'interfaccia HTML;
  in cambio il drag-and-drop consegna percorsi reali (evento nativo di Tauri) e si hanno istanza unica e dialoghi pronti.
* **La finestra esiste solo quando serve.** Con la ricezione in background resta in vita solo il motore (la misura iniziale, con la finestra
  nascosta ma creata, dava 164 MB contro 38 MB di Python: era sbagliato e l'ho corretto; ora 28 MB). Riaprire crea una nuova WebView.
* **Handler annullabili.** In Rust un handler HTTP può essere abbandonato in qualunque `await` (il client chiude la connessione). Stato dei file,
  conteggio upload e temporanei sono perciò ripuliti da guardie `Drop` (verificato: richiesta in attesa → «annullata»; upload interrotto → nessun parziale).
* **Nessun server HTTP per la UI, nessun polling:** la pagina usa `invoke` verso il motore e riceve lo stato spinto con eventi accorpati (≤10/s,
  solo a finestra presente). Selezione e trascinamento passano percorsi reali; l'invio legge la sorgente in streaming (blocchi da 1 MiB).
* **Identità:** i nuovi certificati sono **ECDSA P-256** (ring non genera RSA); le chiavi RSA create dalle versioni Python vengono lette e usate
  (testato, e la 4.0 Python legge a sua volta quelle ECDSA). Non provato che ogni client ufficiale accetti un certificato ECDSA.
* **Notifiche** con `notify-rust` (D-Bus), perché il plugin Tauri non offre pulsanti d'azione su Linux. Avvio automatico gestito in proprio.
* **Rete:** eventi D-Bus (NetworkManager, logind `PrepareForSleep`) quando disponibili; sempre attivi come ripiego il controllo ogni 5 s delle
  interfacce e il confronto fra orologio di sistema e monotonico (rileva la sospensione anche senza logind). Dopo l'evento: la presenza dei
  dispositivi non locali decade, si riaggancia il multicast, si riannuncia e si riverificano i noti. Gli eventi D-Bus reali di NetworkManager/logind
  **non** sono stati provati (solo segnali sintetici sul bus di sistema, vedi TEST).

## 4. Identità e fiducia
* In invio: impronta del certificato visto == impronta annunciata, altrimenti errore. «Verificato» (UI) = *visto in questa sessione un certificato con questa impronta*.
* «Confermato» = il wizard ha verificato con un codice che quell'impronta è del telefono in mano all'utente (persistito, ma non concede nulla).
* In ricezione: nessuna prova possibile → conferma manuale sempre; **nessuna auto-accettazione**.
* Cambio d'identità del PC: solo su richiesta esplicita («Rigenera identità»), con backup dei vecchi file.

## 5. Migrazione (idempotente, non distruttiva)
Stesso `settings.json` (schema 4) della 4.0 Python. Dalla 3.x: copia di sicurezza in `~/.config/pixel-hub/backups/` (Pixel Hub 5.0; in Cable: `~/.config/cable/backups/`), il file 3.x non viene mai modificato;
cartelle, nome, tema, PIN (stesso hash) conservati; i «fidati» e il telefono associato diventano **preferiti non confermati**; l'auto-accettazione è scartata;
`wizard_fatto` è vero se c'era un telefono associato. Certificato/chiave 3.x **copiati** (non spostati) in `~/.local/share/cable/identity/`.
L'avvio automatico che puntava a script Python viene riscritto dopo averne salvato una copia. Se è in esecuzione una versione Python la 5.0 rifiuta di partire (codice 3).

## 6. Misure (stessa macchina, dati isolati, 8 s a riposo; PSS dell'intero albero di processi)

| | Python 4.0 (GTK4+WebKit6) | Rust 5.0 (Tauri, release) |
|---|---|---|
| Tempo fino alla ricezione attiva | 317–470 ms | **97 ms** |
| Finestra aperta, inattiva (PSS) | 259 MB (8 processi) | 227 MB (6 processi) |
| In background, nessuna finestra (PSS) | 38 MB | **28 MB** |
| Binario | script + dipendenze Python | 14,4 MB, unico |

Conclusione onesta: il guadagno è netto su avvio e su memoria in background; **con la finestra aperta la memoria è dominata da WebKit** e cambia poco
(−12%). Non ho misurato la velocità di trasferimento né la latenza di riapertura della finestra.

## 7. Limiti noti
* Compatibilità con i client ufficiali **non dimostrata** (vedi `TEST.md`).
* Solo IPv4; nessuna ripresa da punto interrotto (non prevista dal protocollo interoperabile).
* Sorgente modificata durante l'invio: controllo a ogni MiB e interruzione (il destinatario scarta il parziale); se la modifica cade nell'ultimo blocco
  il destinatario può aver già salvato una copia incoerente e il mittente lo segnala.
* Funzioni Android (ADB/scrcpy): portate dalla versione Python, non provate con un telefono reale; usano l'unico dispositivo ADB collegato.
* Nautilus: nessuna estensione; lo script *Scripts › Invia con Cable* e `cable FILE...` (non provati in Nautilus reale).
* Fedora Atomic, Flatpak non verificati. Il `.rpm` non è stato costruito (`rpmbuild` assente).
