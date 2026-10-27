# Checklist manuale con client ufficiali

Nessuna di queste prove è stata eseguita: servono un Pixel/Android reale, un iPhone e la rete Wi-Fi di casa.
Segna ogni riga con data, versione dell'app LocalSend sul telefono e esito. Con `HUB_DEBUG=1` Pixel Hub
stampa su stderr le richieste (mai PIN né token).

Preparazione: Pixel Hub 5 su Fedora (`scripts/dev.sh run`), LocalSend **ufficiale** sul telefono, stessa rete,
LocalSend per PC chiuso, cifratura HTTPS attiva sul telefono.

## W. Wizard di primo collegamento (nuovo nella 5.0)
Parti da zero: `scripts/dev.sh sandbox` (dati isolati) oppure rinomina temporaneamente `~/.config/pixel-hub`.
- [ ] Il wizard si apre da solo al primo avvio; *Salta per ora* lo chiude e non ricompare; da Impostazioni si riapre.
- [ ] Passo 1: i controlli (rete/ricezione/identità) sono veritieri; con LocalSend per PC aperto segnala la porta occupata.
- [ ] Passo 2: il telefono reale compare entro ~10 s; con due telefoni vicini «Avanti» resta spento finché non ne scegli uno.
- [ ] Passo 3: il messaggio «Pixel Hub — codice di verifica» arriva sul telefono (annota se serve toccare *Accetta* in LocalSend);
      un codice sbagliato scala i tentativi; il codice giusto conferma; dopo 5 errori serve un nuovo codice; dopo 5 minuti scade.
- [ ] Passo 3 con il telefono **sbagliato** scelto: il codice non compare sul telefono che hai in mano → la verifica non si può completare (è il punto).
- [ ] Passo 4: dal telefono, *Invia › Testo* al PC; la richiesta compare sopra il wizard; dopo *Accetta* il passo risulta riuscito.
- [ ] Passo 5: nome, cartella, PIN e avvio automatico applicati (verifica `~/.config/pixel-hub/settings.json`: il PIN deve essere solo un hash).
- [ ] Dopo il wizard il telefono è «Preferito» e «Identità verificata»; riavviando l'app resta preferito ma «Identità verificata» riparte da no.
- [ ] Il codice non compare in *Trasferimenti*, in `settings.json` né nei log (`HUB_DEBUG=1`).

## A. Android (LocalSend ufficiale)
- [ ] Il telefono compare in *Condividi* entro 10 s dall'apertura di LocalSend (multicast).
- [ ] Il PC compare nel telefono con il nome scelto in Impostazioni; cambiando nome si aggiorna.
- [ ] Telefono → PC, 1 foto: richiesta con mittente/dimensione corrette, Accetta, file integro (confronta `sha256sum`).
- [ ] Telefono → PC, 5 file misti: accettazione **parziale** (togli 2): arrivano solo i 3; il telefono non mostra «completato» per i 5.
- [ ] Telefono → PC, testo: richiesta «messaggio», dopo Accetta compare in Messaggi; Copia funziona.
- [ ] PC → telefono, file e testo: il telefono chiede conferma; il file è integro.
- [ ] PC → telefono, cartella: struttura delle sottocartelle conservata.
- [ ] Rifiuto: sul PC (Rifiuta) il telefono mostra «rifiutato»; sul telefono (Rifiuta) il PC mostra «Rifiutato».
- [ ] PIN: imposta un PIN sul **telefono** (LocalSend › Impostazioni › PIN), invia dal PC: compare la richiesta PIN; PIN errato → richiesta di nuovo; PIN giusto → prosegue. Verifica che il PIN non compaia in `~/.config/pixel-hub/settings.json`.
- [ ] PIN in ricezione: imposta un PIN in Pixel Hub, invia dal telefono: LocalSend chiede il PIN; errato e poi giusto.
- [ ] File grande (≥ 2 GB) in entrambe le direzioni: avanzamento e velocità sensati, memoria di Pixel Hub stabile (`ps -o rss`), file integro.
- [ ] Annulla durante l'invio dal PC; annulla dal telefono; annulla la ricezione dal PC: nessun `.pixelhub-*.part` resta nella cartella.
- [ ] Rete interrotta a metà (spegni il Wi-Fi del telefono): errore comprensibile, nessun file parziale, nuovo invio possibile.
- [ ] Sospensione/ripresa del PC durante l'attesa: dopo ~5 s il telefono riricompare e la ricezione funziona.
- [ ] Riavvio: chiudi con *Esci*, riavvia, ripeti un invio. Con *Avvia all'accensione* attivo: dopo il login Pixel Hub è pronto a ricevere a finestra chiusa e arriva la notifica con Accetta/Rifiuta.
- [ ] Nome duplicato: invia due volte lo stesso file: nascono `nome.ext` e `nome (1).ext`, nessuno sovrascritto.
- [ ] Nomi con accenti, emoji, spazi e file vuoto (0 byte).

## B. iOS (LocalSend ufficiale)
- [ ] Scoperta, invio e ricezione di foto/video (anche HEIC, 1 e 10 elementi), PIN, rifiuto.
- [ ] iPhone con app in background/schermo spento: comportamento documentato (iOS sospende la ricezione).

## C. Più dispositivi e preferiti
- [ ] Due telefoni insieme: nessun invio parte «da solo» verso uno dei due (serve sceglierne uno in *Condividi*).
- [ ] Stella: il preferito resta in elenco, grigio, quando il telefono è spento; *Dimentica* lo toglie.
- [ ] Richiesta contemporanea da due mittenti: la seconda riceve 409 o attende; nessuna mescolanza di file.
- [ ] Un preferito che invia **non** viene accettato in automatico.

## D. Rete e firewall
- [ ] Con LocalSend per PC aperto: Pixel Hub segnala «porta occupata» e dopo averlo chiuso *Riprova* funziona.
- [ ] Con firewalld attivo: se il telefono non compare, *Diagnostica* distingue le cause; applica `firewall-cmd` solo alla zona della rete fidata (porte 53317 TCP/UDP) e verifica.
- [ ] Hotspot del telefono; Wi-Fi ospiti con isolamento client (atteso: non trovato → IP manuale non funziona neppure).
- [ ] Cambio rete (Wi-Fi → cavo): riannuncio automatico entro ~3 s.

## E. Desktop (GNOME) — Tauri/GTK3
- [ ] L'icona nel dock/overview è quella di Pixel Hub (la finestra ha classe `pixel-hub`, il lanciatore `StartupWMClass=pixel-hub`).
- [ ] Notifica con **Accetta/Rifiuta** (finestra chiusa e con finestra non in primo piano): i pulsanti agiscono; il click sul corpo apre l'app.
- [ ] Notifica «Apri cartella» a ricezione completata.
- [ ] Avvio con `--background`: nessuna finestra, ma si riceve e arriva la notifica.
- [ ] `pixel-hub file1 file2` con l'app già aperta → una sola istanza, file in coda in *Condividi*.
- [ ] Trascina file da Nautilus sulla finestra e **su un dispositivo** (evento nativo di Tauri con percorsi reali: la consegna alla pagina è provata, il gesto reale da Nautilus no).
- [ ] `pixel-hub file1 file2` con app aperta: file in coda in *Condividi*, nessuna seconda istanza.
- [ ] Notifica con *Accetta/Rifiuta* e *Apri cartella* (anche con finestra chiusa).
- [ ] Chiudi finestra ≠ esci; *Esci* e Ctrl+Q chiudono davvero; con *Resta pronta* disattivato la X esce.
- [ ] Tema chiaro/scuro di GNOME seguito in modalità Automatico.

## F. Android opzionale (cavo)
- [ ] ADB: batteria/spazio, importazione DCIM (idempotente), con un solo dispositivo collegato; con due dispositivi l'app si ferma con un messaggio chiaro.
- [ ] scrcpy: avvio/arresto dal pulsante.
- [ ] Senza ADB/scrcpy installati: l'app parte e i trasferimenti Wi-Fi funzionano.
