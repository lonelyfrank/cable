# Checklist manuale (richiede hardware esterno)

Spunta e annota data, modello, versione dell'app. **Nulla qui è stato eseguito**: sono le prove che i test automatici non possono sostituire.
Parti da zero con `scripts/dev.sh sandbox` (dati isolati) o con una HOME pulita. Per leggere cosa succede: `HUB_DEBUG=1`.

## 1. Linux ↔ Android con LocalSend ufficiale
- [ ] Lo smartphone compare in *Condividi* entro 10 s dall'apertura di LocalSend; il PC compare nell'app col nome scelto (cambiandolo si aggiorna).
- [ ] Il certificato ECDSA di Cable è accettato dall'app (primo invio in entrambe le direzioni).
- [ ] Android → PC, 1 foto: richiesta con mittente/dimensione corrette, Accetta, file integro (`sha256sum`).
- [ ] Android → PC, 5 file: accettazione **parziale** (togli 2): arrivano 3; l'app non dà «completato» per i 5.
- [ ] Android → PC, testo: richiesta «messaggio», compare in Messaggi, Copia funziona.
- [ ] PC → Android: file, testo, cartella (struttura conservata); il telefono chiede conferma; file integro.
- [ ] A fine ricezione sul PC: **Apri** apre il file, **Mostra nella cartella** lo evidenzia in Nautilus; anche dai pulsanti della notifica.
- [ ] Nomi con accenti, emoji, spazi, caratteri speciali, file da 0 byte; due volte lo stesso file → `nome (1).ext`, nessun sovrascritto.
- [ ] File ≥ 2 GB nei due versi: avanzamento, velocità e **tempo residuo** sensati (compare solo dopo qualche secondo), memoria stabile (`ps -o rss`).
- [ ] PIN sul telefono (invio dal PC) e PIN in Cable (ricezione): errato e poi giusto.

## 2. Linux ↔ iPhone/iPad con LocalSend ufficiale
- [ ] Scoperta, invio e ricezione di foto/video (anche HEIC; 1 e 10 elementi), PIN, rifiuto.
- [ ] Con l'app in background o lo schermo spento: annota il comportamento (iOS sospende la ricezione).
- [ ] **Senza adb e scrcpy installati** l'onboarding completo funziona senza errori e senza schede Android.

## 3. Due smartphone contemporaneamente
- [ ] Android + iPhone (o due Android) in elenco: nessun invio parte «da solo» (si sceglie); un rilascio sulla finestra apre la scelta del destinatario.
- [ ] Rilascio **su un dispositivo**: l'invio parte verso quello giusto.
- [ ] Richiesta contemporanea da due mittenti: la seconda attende o riceve 409; nessuna mescolanza di file.

## 4. Dispositivi con nomi identici
- [ ] Due telefoni chiamati uguale (es. «iPhone»): in elenco hanno etichette diverse con le prime cifre dell'impronta; l'invio va a quello scelto
      (verifica sul telefono). Nessun avviso di «cambio d'identità» (sono contemporanei).
- [ ] Reinstalla l'app su un telefono confermato (nuovo certificato, stesso nome): compare l'avviso di identità diversa, il vecchio record non cambia,
      il nuovo non è «confermato»; «Ignora» lo nasconde, «Sostituisci» sposta il record senza confermarlo.

## 5. Cambio IP, rete e sospensione
- [ ] Cambia IP del telefono (riconnetti il Wi-Fi): dopo pochi secondi è ancora un solo dispositivo; l'invio funziona subito.
- [ ] Wi-Fi → cavo (o hotspot del telefono): entro ~5 s il PC si riannuncia; i dispositivi della vecchia rete non sono più «vicini».
- [ ] Sospendi il PC (chiudi il coperchio) e riprendi: entro ~5 s i telefoni riappaiono e la ricezione funziona. Con `HUB_DEBUG=1` verifica che compaia
      «eventi D-Bus: in ascolto» (altrimenti Diagnostica deve dire che resta il controllo periodico).
- [ ] Con una VPN attiva e/o Docker/libvirt: elenco e «Indirizzi del PC» non mostrano interfacce virtuali; l'hotspot si trova.
- [ ] Un'altra distribuzione/desktop (se ti serve): annota requisiti mancanti e se funzionano notifiche, «Apri», tray/background.

## 6. Destinatario offline
- [ ] Chiudi l'app sul telefono e prova a inviare: errore **entro pochi secondi** (non dopo gli hash), con indirizzo, fatto osservato e più cause
      possibili; **non** attribuisce il problema al solo firewall. *Aggiorna* mostra «visto, ma non risponde» (vicino ≠ raggiungibile).

## 7. Rifiuto e annullamento
- [ ] Rifiuto sul telefono: il PC mostra «Rifiutato dal destinatario» (≠ «Annullato»).
- [ ] Rifiuto sul PC (ricezione): il telefono mostra rifiutato.
- [ ] Annulla durante l'invio dal PC; annulla dal telefono; annulla la ricezione dal PC: nessun `.cable-*.part` resta nella cartella.
- [ ] Lascia scadere la decisione (non accettare): il PC mostra «Scaduto», non un errore generico.

## 8. Trasferimento parzialmente completato
- [ ] Invia ~10 file e spegni il Wi-Fi del telefono a metà: stato **Interrotto**, esito per file, nessun parziale sul PC.
- [ ] «**Riprova i file non completati**»: partono solo i non completati, i riusciti non si rimandano; se il telefono aveva già ricevuto l'ultimo file
      compare la nota di esito incerto e, riprovando, un eventuale doppione si chiama `nome (1).ext` (mai sovrascritto).

## 9. Porta occupata da un'altra istanza
- [ ] Con LocalSend per PC aperto: Cable segnala «porta occupata», dopo averlo chiuso *Riprova* funziona.
- [ ] Con **Pixel Hub 5.0** aperto: `cable` rifiuta di partire con un messaggio chiaro (codice 3).
- [ ] Due volte `cable file1`: una sola istanza; la seconda inoltra i file e si apre la scelta del destinatario (senza tornare alla Home).
- [ ] Con Cable **in background e senza finestra** (`cable --background`), da una cartella diversa: `cd ~/Documenti && cable "foto 1.jpg" altro.pdf` → la finestra si ricrea e la scelta del
      destinatario mostra i due file (percorsi relativi alla cartella da cui hai lanciato, non a quella della prima istanza); riaprendo la finestra non ricompaiono.
- [ ] Doppio clic sull'icona mentre Cable è già aperto e minimizzato: torna in primo piano senza file fantasma.

## 10. Migrazione da Pixel Hub (sul tuo sistema reale)
- [ ] Chiudi Pixel Hub, `scripts/installa-utente.sh`: «Cable» e l'icona compaiono nel menu; il vecchio lanciatore non c'è più.
- [ ] Primo avvio: avviso di importazione; il telefono già collegato **non** chiede un nuovo collegamento e il fingerprint mostrato è lo stesso di prima.
- [ ] Con *Avvia all'accensione* attivo in Pixel Hub: dopo il login parte Cable (non Pixel Hub); un solo `cable` in esecuzione.
- [ ] `scripts/installa-utente.sh rimuovi` → Pixel Hub di nuovo avviabile con i suoi dati.
- [ ] In Nautilus: tasto destro su un file › Scripts › Invia con Cable → si apre la scelta del destinatario.

## 11. Wizard e sicurezza
- [ ] Il wizard si apre da solo al primo avvio; *Salta per ora* lo chiude e non ricompare; da Impostazioni si riapre.
- [ ] Passo 3: il messaggio «Cable — codice di verifica» arriva (annota se serve toccare *Accetta*); codice sbagliato scala i tentativi; dopo 5 errori
      o 5 minuti serve un nuovo codice; scegliendo il telefono **sbagliato** il codice non compare sul telefono in mano (è il punto).
- [ ] Dopo il wizard il dispositivo è «Confermato»; **un preferito che invia non viene mai accettato in automatico**.
- [ ] Il PIN e il codice non compaiono in `~/.config/cable/settings.json` (solo hash), né in *Trasferimenti*, né nei log.

## 12. Desktop (GNOME)
- [ ] L'icona (la «C» blu col cavo) è quella in overview/dock/finestra (`StartupWMClass=cable`).
- [ ] Notifica con **Accetta/Rifiuta** (finestra chiusa e non in primo piano); notifica di fine con **Mostra nella cartella** e **Apri** (un solo file).
- [ ] `--background`: nessuna finestra, ma si riceve e arriva la notifica. Chiudi finestra ≠ esci.
- [ ] Trascina file da Nautilus sulla finestra e su un dispositivo (il gesto reale; l'evento è provato solo a livello di pagina).
- [ ] Tema chiaro/scuro seguito in Automatico.

## 13. Strumenti Android facoltativi (cavo)
- [ ] ADB: batteria/spazio, importazione DCIM idempotente; con due dispositivi l'app si ferma con un messaggio chiaro. scrcpy: avvio/arresto.
- [ ] Senza ADB/scrcpy l'app parte, i trasferimenti Wi-Fi funzionano e le schede «Schermo»/«Android USB» non compaiono.
