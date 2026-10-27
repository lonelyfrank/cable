> **Aggiornamento del ciclo «affidabilità»** (branch `hardening/affidabilita`, base `70906ae`): vedi in fondo la sezione «Ciclo affidabilità».
> Le sezioni sotto descrivono la base di partenza e restano valide salvo dove indicato.

# Esiti dei test (27 ottobre 2026 · Fedora 44 · rustc/cargo 1.98 · Tauri 2 · webkit2gtk 4.1)

**Nessuna prova è stata fatta con smartphone reali né con le app LocalSend ufficiali (Android o iOS).** Tutto ciò che segue è
automatico, su loopback, fra implementazioni interne (motore Rust ↔ motore Rust, e Rust ↔ peer Python scritto dallo stesso autore).
Dimostra la coerenza del codice, **non** la compatibilità con i telefoni. Ciò che richiede hardware è in
[CHECKLIST-MANUALE.md](CHECKLIST-MANUALE.md).

Base di partenza (prima delle modifiche di Cable): 51 test del motore + 30 controlli della finestra, tutti verdi; nessuna regressione
preesistente da distinguere. `clippy` non è installato: non eseguito.

## 1. Test automatici — `scripts/dev.sh test`: **80 test, tutti passati** (5 esecuzioni consecutive senza instabilità dopo la correzione di due test)

| Suite | Test | Novità di Cable |
|---|---|---|
| unitari (`lib`) | 42 | **migrazione** da Pixel Hub (copia, impronta identica, chiave 0600, originali intatti, nome personalizzato conservato, idempotenza, nessuna sovrascrittura, dati vecchi+nuovi, impostazioni corrotte); **registro** (più indirizzi e cambio IP, limite, nomi uguali con etichette distinte, cambio d'identità senza trasferire la fiducia, avviso ignorabile, le quattro proprietà indipendenti, presenza che decade dopo rete/sospensione, stella = scorciatoia, nessun annuncio non provato riscrive il record, tetto dei ricordati); **discovery** (interfacce virtuali/VPN, gateway per metrico, sottoreti); **ETA** solo se attendibile; **messaggi** d'errore (fatti + cause, niente colpa al firewall) |
| `receiver` | 15 | invariati (sessioni, token, scadenze, annullamento, parziali, PIN, 507, TOCTOU…) |
| `e2e` | 23 | **riprova** (solo i non completati, mai quelli riusciti, nessun doppione, non due volte, non da completato, file spariti); **interruzione** a metà con esito per file; **indirizzo vecchio** che non rallenta né rompe l'invio (< 2,5 s); **aggiorna**: «vicino» ≠ «raggiungibile», messaggio con indirizzo e senza «firewall»; **evento di ripresa** accorpato che fa decadere la presenza; più tutti i 17 precedenti (rifiuto, parziale, PIN, annullamento da entrambi i lati, sorgente modificata, identità incoerente, nessuna auto-accettazione, wizard, memoria limitata, identità corrotta mai rigenerata) |

Limiti dei test: l'esito `parziale`/`interrotto` dopo un annullamento del destinatario dipende da chi vince una corsa (risposta d'errore o chiusura
del socket); i test accettano entrambi. I test con file enormi sono serializzati perché la memoria di picco è del processo.

## 2. Altre prove eseguite

* **Finestra reale** (`scripts/dev.sh ui-test`, sessione Wayland, dati isolati, motore peer in-process): **30 controlli su 30**. Copre wizard,
  accettazione parziale, PIN, rilascio sul dispositivo (consegna alla pagina dell'evento), diagnostica, zero errori JavaScript. **Non coperti**:
  scelta del destinatario per file da riga di comando, «Riprova», avvisi d'identità, «Apri»/«Mostra» (funzioni aggiunte dopo: il motore è
  provato, l'interfaccia solo per assenza di errori JS e a vista su uno screenshot di *Condividi*).
* **Interoperabilità Python ↔ Rust** (`scripts/dev.sh interop`): file con sha256 e testo in entrambe le direzioni, ECDSA contro RSA: OK.
* **Migrazione sui dati reali dell'utente, in copia** (HOME isolata, poi cancellata): impronta del certificato identica prima/dopo, preferiti
  e conferme uguali, nome predefinito sostituito, originali byte per byte uguali, seconda esecuzione senza effetti, chiave 0600.
* **Installer** in HOME isolata: installa, salva il lanciatore di Pixel Hub, `desktop-file-validate` OK, `rimuovi` ripristina. **Non** eseguito
  sul sistema reale (Pixel Hub era in esecuzione: non l'ho chiuso io).
* **Eventi D-Bus**: l'esempio `netwatch_probe` si collega al bus di sistema e riceve segnali **sintetici** (emessi con `busctl`: un segnale sotto
  `/org/freedesktop/NetworkManager` → *Network*, `PrepareForSleep(false)` → *Resume*). Eventi **reali** di NetworkManager/logind non provati
  (non ho spento la rete della macchina su cui lavoro).

## 3. Misure (loopback, stessa macchina — `cargo run --release -p pixelhub-engine --example bench`)

Intel i7-10510U, Fedora 44, due motori nello stesso processo, TLS reale, disco locale. **Non sono prestazioni su Wi-Fi né su telefono.**

| Misura | Valore |
|---|---|
| Latenza di avvio (invio → richiesta visibile sul destinatario), 1 file piccolo | mediana 3,1 ms (min 2,6 · max 4,0), 5 prove |
| File grande 1 GiB | 1,19 s ≈ 859 MiB/s |
| 2000 file da 4 KiB | preparazione 0,22 s, totale 7,3 s ≈ 274 file/s (una richiesta per file: i file piccoli sono il caso lento) |
| Memoria (due motori + 1 GiB trasferito) | RSS 7 MiB a riposo, **picco 28 MiB** → lo streaming non copia i file in memoria |
| App in background (binario `--background`, senza multicast, 20 s) | CPU 0,0 %, RSS 65 MiB, 18 thread (RSS include le librerie condivise: non confrontabile con le misure PSS di Pixel Hub) |

**Non misurati** (richiedono rete/telefoni o un ambiente non disruptivo): tempo di scoperta reale (multicast su Wi-Fi), latenza su Wi-Fi,
recupero dopo cambio rete o sospensione veri, memoria con finestra aperta, riapertura della finestra.

## 4. Prove con client ufficiali — **NON eseguite**
Compatibilità con LocalSend ufficiale su Android e iOS **non dimostrata**: certificato ECDSA accettato?, messaggio col codice del wizard mostrato?,
comportamento a PIN ripetuti e con `sha256`, multicast su hotspot. Vedi la checklist.


## 5. Ciclo affidabilità (27 ottobre 2026 · Fedora 44 · rustc/cargo 1.98)

Base prima delle modifiche: `cargo test -p pixelhub-engine` = 42 + 15 + 23 = **80 test verdi**; `clippy` e `rustfmt` non erano installati sull'host
(eseguiti poi in un contenitore Fedora 44 isolato, `podman`, immagine locale `cable-build:44`).

| Controllo | Esito osservato |
|---|---|
| `cargo test -p pixelhub-engine` | **46 lib + 23 e2e + 15 receiver + 4 server_body = 88 passati**, 0 falliti |
| `cargo test -p cable` | **7 passati** (coda di apertura) |
| Finestra reale (`ui-test`, Wayland, bus D-Bus isolato) | **54 controlli su 54** (erano 30): + apertura file (pronta, nascosta, distrutta, ricreata in ritardo, più invocazioni, riapertura), tastiera, nessun dispositivo, invio da tastiera, avviso d'identità |
| `server_body` sul vecchio `server.rs` | **3 su 4 falliscono** (regressione riprodotta); con la correzione passano in ~0,8 s |
| `cargo fmt --check` (contenitore) | prima: 602 differenze; dopo il commit di formattazione: pulito |
| `cargo clippy -D warnings` motore e desktop (contenitore) | prima: 35 + 3 avvisi di stile preesistenti, nessun errore; dopo: pulito, inclusa la feature `ui-test` |
| Installer utente in HOME temporanea | installa, `desktop-file-validate` ok, `rimuovi` ripristina il lanciatore di Pixel Hub; il Cable di sistema in esecuzione **non** viene terminato |
| RPM | vedi [PACCHETTO.md](PACCHETTO.md) |

**Preesistente, non regressione**: le 35+3 segnalazioni di clippy e le 602 differenze di formato erano già nel commit base. Nessun test esistente è stato modificato o indebolito.
**Non eseguito**: tutto ciò che richiede telefoni, VPN/hotspot/Wi-Fi reali, la consegna D-Bus fra due processi veri dell'istanza unica, la CI su GitHub (il workflow è validato come YAML; fmt, clippy e test sono stati eseguiti a mano nel contenitore Fedora (e build/test del desktop sull'host), ma non è mai girato su GitHub Actions: i nomi dei pacchetti Ubuntu 24.04 sono dedotti, non provati).
