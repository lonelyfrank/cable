# Affidabilità: correzioni di questo ciclo, copertura dei casi d'uso quotidiani, limiti

Branch `hardening/affidabilita` (base: `70906ae`). Ogni punto è stato **riprodotto o confermato leggendo il codice prima di correggerlo**,
come indicato. Nessuna prova qui coinvolge telefoni, VPN, hotspot o Wi-Fi reali: vedi [MATRICE-COMPATIBILITA.md](MATRICE-COMPATIBILITA.md).

## 1. Corpi JSON senza scadenza (`crates/engine/src/server.rs`) — CONFERMATO e corretto
* **Problema**: `register` e `prepare-upload` limitavano la dimensione del corpo ma non il tempo. `header_read_timeout` copre solo gli header:
  un client con header completi e corpo mai finito (o a gocce) teneva occupato uno slot (32) per sempre.
  Riprodotto: sul vecchio `server.rs` 3 dei 4 test di `tests/server_body.rs` falliscono.
* **Correzione**: lettura con scadenza **complessiva** `Limits::json_body_timeout` (30 s). Esiti distinti: `408` scadenza, `413` troppo grande
  (per `Content-Length` o in streaming a blocchi), `400` JSON malformato / corpo vuoto / corpo interrotto. Gli errori ≥ 400 chiudono la connessione e lo slot
  si libera a ogni uscita. `header_timeout` e `tls_timeout` sono ora in `Limits` (configurabili nei test).
* **Upload dei file invariato**: nessun timeout totale; resta l'inattività per chunk (`READ_TIMEOUT` in `receiver.rs`) e la scadenza di sessione.
* **Nota**: la richiesta `prepare-upload` attende poi la decisione dell'utente (fino a 120 s) *dopo* aver letto il corpo: quell'attesa non è coperta dalla scadenza del corpo, di proposito.

## 2. File persi riaprendo Cable dal background (`src-tauri/src/apertura.rs`, `main.rs`, `ui/app.js`) — CONFERMATO dal codice, corretto
* **Problema**: il gestore della seconda istanza chiamava `mostra()` (che può *creare* la finestra) ed emetteva subito `hub-file`: se la pagina non aveva ancora
  registrato il listener, l'evento andava perso. Il `cwd` del chiamante veniva ignorato. Non l'ho riprodotto dinamicamente sul codice vecchio (dipenderebbe dalla
  temporizzazione di WebKit); la dinamica è evidente dal codice e lo scenario «pagina ricreata in ritardo» dimostra che la nuova via la gestisce.
* **Protocollo**: coda nel backend (`Coda`). L'evento `hub-apertura` non porta dati, è solo una sveglia. La pagina ritira con `richieste_apertura` (non consuma),
  mostra, poi conferma per `id` con `conferma_apertura`. Non confermata ⇒ riproposta alla pagina successiva; confermata ⇒ mai più; id già visti dalla pagina
  non si ripresentano anche con conferme in volo. Più richieste ⇒ ognuna col proprio id, nessuna sovrascritta; i percorsi uguali si fondono nella scelta.
* **Percorsi**: argv della seconda istanza risolti rispetto al suo `cwd`; spazi e Unicode ok; URI `file://` decodificati; `--` accettato; file inesistenti segnalati
  («File non trovato») e mai inviati. Nulla parte da solo: si arriva sempre alla scelta del destinatario.
* **Prove**: 7 test unitari (`cargo test -p cable`) + scenario nella finestra reale (pronta, nascosta, distrutta, ricreata in ritardo, più invocazioni, riapertura).
  **Non provata** la consegna D-Bus del plugin single-instance con due processi veri (avrebbe inoltrato al Cable in esecuzione sul sistema di sviluppo);
  il gestore del plugin è un passa-carte di `(argv, cwd)` verso `apri_con_file`, che è ciò che i test esercitano.

## 3. Interfacce VPN e discovery (`crates/engine/src/discovery.rs`) — CONFERMATO e corretto
* **Problema**: `local_nets()` escludeva solo le interfacce *virtuali*; `is_tunnel_iface()` serviva solo alla ricerca dei gateway. Quindi `tun0`/`wg0`/`tailscale` (e `zt*`)
  entravano in join multicast, annunci e sottoreti locali. In più `join()` aggiungeva sempre `0.0.0.0` e `announce()` ripiegava su `0.0.0.0`: il sistema sceglie l'interfaccia
  predefinita, che con una VPN può essere il tunnel (le esclusioni venivano aggirate). Le membership non venivano mai lasciate.
* **Politica** (un solo `classify_iface`): scoperta automatica ⇒ solo LAN (Wi-Fi, Ethernet, hotspot, USB tethering). Tunnel e virtuali ⇒ esclusi con motivo.
  **Connessioni esplicite (IP digitato, ricordato, invio) non sono filtrate** dalla politica. Nessun ripiego su `0.0.0.0`. Se non c'è una LAN non si annuncia nulla.
  Le membership si allineano alle interfacce (lascia le sparite; dopo cambio rete/sospensione lascia e riprende tutto).
* **Diagnostica**: «Interfacce usate per la scoperta», «Interfacce escluse … (motivo)», «Ascolto multicast su». Verificata sul PC di sviluppo (wlp0s20f3 usata).
* **Prove**: test sintetici su `classify_addrs` (Wi-Fi, Ethernet, hotspot, USB tethering, Docker/Podman, bridge, tun/tap, Tailscale, WireGuard, LAN+VPN) e sul piano delle membership.
  **Sono sintetici**: nessuna prova con una VPN o un hotspot reali; il vero `join_multicast_v4` non è coperto da test automatici.
* **Rischio residuo**: un'interfaccia LAN reale con un nome che inizia per un prefisso escluso (es. `tun*`, `br-*`) verrebbe esclusa dalla scoperta; l'IP manuale resta possibile e la diagnostica lo dice.

## 4. Casi d'uso quotidiani: cosa è coperto da test automatici
Tutti i test sono su loopback con due motori reali. «Lacuna» = non provato automaticamente.

| Caso | Copertura |
|---|---|
| Nessun falso «completato»; accettazione parziale | `e2e::partial_selection_…`, `receiver` (selezione, conteggi) |
| Nessuna sovrascrittura; omonimi `nome (1)` | `e2e`, `receiver` (nomi, TOCTOU) |
| Pulizia temporanei | `receiver::size_mismatch_hash_mismatch_and_interruption_leave_nothing`, `cancel_*` |
| Annullamento da mittente e da destinatario | `e2e::cancel_during_large_transfer…`, `receiver_cancel_stops_sender`, `sender_cancel_while_receiver_is_still_asking…` |
| Rete interrotta durante un file | `e2e::network_drop_mid_transfer_is_an_interruption_with_per_file_outcome` |
| Retry dei soli non completati; esito incerto/duplicati | `e2e::retry_resends_only_unfinished…`, `retry_skips_files_that_vanished…` |
| Spazio insufficiente | `receiver::insufficient_space_507` |
| Porta occupata da altra applicazione | `e2e::favorites_reset_identity_and_port_handling` (errore «porta» e `riprova_server`) |
| Sospensione/ripresa, presenza | `e2e::resume_event_expires_presence_and_is_debounced` (evento **sintetico**) |
| IP manuale con multicast assente | `e2e::pair` usa solo IP manuale (multicast disattivato) |
| Client lenti / incompleti | `server_body` (nuovo) |
| Rete interrotta *tra* due file; interruzione dopo i byte ma prima della conferma | **Lacuna parziale**: coperte dall'esito per file e dalla nota di esito incerto (`e2e`), non da un test che tagli proprio in quel punto |
| Finestra chiusa con ricezione in background; riapertura da launcher/CLI/file manager | Finestra distrutta + riapertura: scenario UI (nuovo). Launcher e file manager reali: **NON ESEGUITO** |
| Telefono bloccato / app mobile in background; cambio Wi-Fi reale; hotspot reale; sospensione reale | **NON ESEGUITO** (hardware). Non si promette ricezione mobile a schermo bloccato |
