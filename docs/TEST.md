# Esiti dei test (27 ottobre 2026 · Fedora 44 · rustc/cargo 1.98 · Tauri 2 · webkit2gtk 4.1)

## 1. Test automatici eseguiti

### Motore Rust — `scripts/dev.sh test`: **51 test, tutti passati**, ripetuti 5 volte senza instabilità
Senza grafica; i due motori dialogano su loopback con TLS reale. Durata ~18 s (la suite Python equivalente: ~31 s).

| Suite | Test | Cosa copre |
|---|---|---|
| unitari (`lib`) | 19 | nomi sicuri (`..`, separatori, controlli, Unicode, lunghezza), collegamenti simbolici, rinomina senza sovrascrittura, validazione metadati/dimensioni/sha256/anteprime/limiti, migrazione 3.x (conserva cartelle, declassa «fidati» a preferiti, scarta auto-accettazione, file 3.x intatto, idempotente, backup, PIN compatibile), lettura di un `settings.json` della 4.0 Python, file corrotto messo da parte, identità (creazione, permessi 0600, **import di una chiave RSA creata da Python**, nessuna rigenerazione automatica, rigenerazione esplicita con backup), percorsi consentiti e validazione dell'API UI |
| `receiver` | 15 | sessioni/token/IP, upload duplicato (409) e concorrente (uno solo vince), dimensione/Content-Length errati, sha256 errato (422) e ritentativo, rete caduta a metà, **handler abbandonato a metà upload e a metà richiesta** (parziali ripuliti, stato coerente), annullamento dal mittente e locale (esito *parziale*, mai «completata»), scadenze, accettazione parziale, rifiuto, timeout, 409/429, PIN (401, limite, 429), spazio insufficiente (507), testo con conferma |
| `e2e` | 17 | invio/ricezione di file, testo, cartelle; rifiuto; accettazione parziale; PIN (errato poi giusto, mai in chiaro); annullamento a metà di 600 MB da entrambi i lati; mittente che annulla mentre l'utente sta ancora decidendo; sorgente modificata/rimossa (prima e **durante** l'upload: interrotto a metà, nulla salvato); identità incoerente rifiutata; destinatario spento; client ostili (HTTP in chiaro, spazzatura, corpo da 9 MB → 413); nessuna auto-accettazione neppure con l'impronta di un preferito; 300 MB con memoria limitata (<60 MB); nomi duplicati; **wizard** (codice giusto/sbagliato/formato, 5 errori → blocco, il codice vale una sola volta, nulla salvato prima della conferma, prova inversa, invio del codice non visibile fra i trasferimenti); preferiti/dimentica/ripristino/rigenerazione/porta occupata e ripresa; identità corrotta; stato completo e senza segreti |

### Finestra reale (Tauri) — `scripts/dev.sh ui-test`: **30 controlli, tutti passati**
Sessione Wayland, dati isolati, secondo motore in-process come telefono. Verificati: caricamento e stato spinto dal motore, **tutto il wizard** (apertura al
primo avvio, controlli del passo 1, «Avanti» spento finché non scegli tu il telefono, codice sbagliato/giusto, conferma salvata, prova inversa con la richiesta
sopra il wizard, applicazione di nome e PIN come solo hash, chiusura definitiva, riapertura da Impostazioni, «Salta per ora»), poi accettazione **parziale**
dall'interfaccia, invio con richiesta PIN dalla UI, consegna alla pagina dell'evento di rilascio sul riquadro di un dispositivo (percorso reale, nessuna cache di
uscita), diagnostica, zero errori JavaScript. Gli screenshot si generano in locale in `docs/screenshots-v5/` e **non sono versionati** (mostrano il nome dell'utente di sistema).

### Altro eseguito
* **Interoperabilità Python ↔ Rust** (`scripts/dev.sh interop`): file da 5 MB con sha256 e testo in entrambe le direzioni; certificato ECDSA (Rust) contro RSA (Python). Sono due
  implementazioni **indipendenti**, ma scritte da me con la stessa lettura della specifica: non sostituisce la prova con l'app ufficiale.
* Istanza unica con inoltro dei file, riapertura da background (la finestra si crea e la seconda istanza esce in ~200 ms), guardia contro versioni Python in esecuzione,
  lettura dell'identità Rust da parte di Python, `installa-utente.sh` (install/rimuovi con ripristino, in HOME isolata), `desktop-file-validate`.
* Suite Python di riferimento (`scripts/dev.sh legacy-test`): 87 test passati.
* Bug trovati dai test e corretti in questa fase: tag `</style>` dimenticato nel patch del CSS (pagina vuota), feature `custom-protocol` mancante
  (Tauri in modalità sviluppo senza `devUrl`), finestra creata anche in background (memoria 164 MB invece di 28 MB).

## 2. Prove con client ufficiali — **NON eseguite**
Nessuna prova con LocalSend ufficiale su Android o iOS: la compatibilità è **non dimostrata**. Vedi `CHECKLIST-MANUALE.md` (sezione W per il wizard).

## 3. Verifiche ancora da fare
* Telefoni reali, multicast su Wi-Fi/hotspot, firewalld, VPN; che LocalSend ufficiale accetti il certificato ECDSA e mostri il messaggio col codice del wizard (se chieda *Accetta*).
* Drag-and-drop **reale** da Nautilus (provata solo la consegna alla pagina), selettori file/cartella nativi, aspetto e azioni delle notifiche in GNOME Shell,
  associazione icona/finestra (`StartupWMClass=pixel-hub`), avvio automatico al login.
* ADB e scrcpy con un telefono reale (codice portato, non eseguito contro un dispositivo).
* Costruzione dell'RPM (`rpmbuild` assente), Fedora Atomic.
* Non misurati: velocità di trasferimento, latenza di riapertura della finestra.
