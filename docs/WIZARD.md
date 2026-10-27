# Procedura guidata di primo collegamento

Si apre da sola al primo avvio (`wizard_fatto = false`) e si riapre da *Impostazioni › Collega un telefono*.
È **sempre saltabile**: i trasferimenti funzionano anche senza, e la verifica con codice (passo 3) è facoltativa per l'uso:
serve solo a confermare che l'impronta appartiene allo smartphone che hai in mano. Non usa ADB né scrcpy, quindi vale
uguale per Android e iPhone/iPad. Cinque passi.

| # | Passo | Cosa fa il programma | Cosa fa l'utente |
|---|---|---|---|
| 1 | **Prepara** | Controlla rete, ricezione attiva (porta 53317, TLS) e identità valida; se la porta è occupata lo dice e offre *Riprova* | Apre LocalSend (o un'app compatibile) sullo smartphone, stessa Wi-Fi, cifratura attiva, scheda «Ricevi» |
| 2 | **Trova** | Mostra i dispositivi LocalSend vicini (multicast) + ricerca per IP | **Sceglie lui** il telefono. Nessuna scelta automatica; «Avanti» resta spento finché non sceglie |
| 3 | **Verifica** | Invia al telefono scelto, via LocalSend e con **impronta del certificato controllata**, un codice a 6 cifre (valido 5 minuti, 5 tentativi) | Legge il codice sul telefono e lo digita sul PC |
| 4 | **Prova** | Si mette in ascolto: segna riuscita la prova quando dal dispositivo arriva qualcosa e **l'utente lo accetta** | Dal telefono invia un testo al PC e accetta la richiesta |
| 5 | **Fine** | Applica nome PC, cartella, PIN di ricezione (facoltativo), avvio automatico | Rivede le impostazioni |

## Perché così (e non altrimenti)

* **Verifica con codice.** Con LocalSend v2 un'impronta vista in rete non dimostra *chi* sia il dispositivo (può annunciare
  qualunque nome). Il codice viaggia solo sulla connessione TLS verso *quell'impronta*: se l'utente lo legge sul telefono
  che ha in mano, quell'impronta appartiene a quel telefono. Al successo il dispositivo diventa **preferito confermato**
  (`preferiti[fp].confermato = true`) e, fino a un cambio di certificato, ogni invio verso di lui controlla di nuovo l'impronta.
* **Niente QR code:** l'app ufficiale LocalSend non può scansionarlo. **Niente Bluetooth:** non autentica LocalSend
  (la versione 3.1 lo mostrava e creava confusione). **Niente obbligo:** la 3.1 bloccava tutto fino alla fine.
* **Prova inversa:** serve a scoprire subito problemi di firewall/rete in ricezione, senza dare per scontato che sia il firewall
  (la schermata rimanda alla Diagnostica, che distingue le cause).

## Limiti dichiarati

* La verifica prova l'identità del telefono **per l'invio**. In **ricezione** LocalSend v2 non permette di verificare il mittente
  (nessun certificato client): ogni richiesta richiede sempre conferma manuale e il PIN è un'ulteriore protezione, non un'identità.
  La «prova inversa» riconosce il dispositivo dall'indirizzo IP: dimostra che la ricezione funziona, non chi sia il mittente.
* Il messaggio col codice compare sul telefono come testo ricevuto; alcune configurazioni di LocalSend chiedono di toccare
  *Accetta*. Se non compare: *Invia di nuovo*, oppure controlla la Diagnostica.
* Non provato con LocalSend ufficiale (vedi `CHECKLIST-MANUALE.md`, sezione W).

## Stato nel motore

`Engine::verifica_inizia(fp)`, `verifica_conferma(codice)`, `wizard_prova(fp)`, `wizard_completa()`; stato in `snapshot()["wizard"]`.
L'invio del codice è un trasferimento *silenzioso* (non compare in «Trasferimenti»). Il codice non viene mai scritto su disco né nei log.
