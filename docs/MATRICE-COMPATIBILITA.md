# Matrice di collaudo con i client LocalSend ufficiali

**Stato: nessuna prova eseguita.** Ogni riga è `NON ESEGUITO` finché una persona con un telefono vero non la compila.
Non scrivere «OK» per deduzione: le prove automatiche del repository (loopback, motore ↔ motore e ↔ peer Python scritto dallo stesso autore) **non** sostituiscono queste.

## Come compilare ogni prova
Per ogni riga registra nel foglio (o in un issue) i campi dell'intestazione comune, poi: passi, atteso, osservato, log.

**Intestazione comune (una volta per sessione):**
| Campo | Come ottenerlo |
|---|---|
| Versione/commit di Cable | `cable --versione` e `git rev-parse --short HEAD` (o `rpm -q cable`) |
| Binario realmente avviato | `command -v cable`; con l'app aperta: `readlink /proc/$(pgrep -x cable)/exe` |
| Distribuzione, desktop, sessione | `cat /etc/os-release`; `echo $XDG_CURRENT_DESKTOP $XDG_SESSION_TYPE` |
| Telefono e OS | es. «Pixel 8 · Android 15», «iPhone 13 · iOS 18.x» (Impostazioni › Info) |
| Versione di LocalSend | schermata Info dell'app (versione e store di provenienza) |
| Rete | Wi-Fi 2,4/5 GHz del router · hotspot del PC · hotspot del telefono · cavo; VPN attiva sì/no |
| Log | `HUB_DEBUG=1 cable 2> cable.log`; «Diagnostica» da Impostazioni |

Prima di ogni sessione usa dati isolati se non vuoi toccare la tua configurazione: `scripts/dev.sh sandbox`.

## Scenari (ripeti per ciascuna riga della tabella delle direzioni)
| # | Scenario | Atteso |
|---|---|---|
| S1 | Testo breve; testo Unicode (accenti, emoji, `日本語`) | Arriva identico; in Messaggi; Copia funziona |
| S2 | Foto singola; 10 foto | Tutte integre (`sha256sum`); iOS: annota formato (HEIC/JPEG) |
| S3 | File grande (≥ 2 GB) | Avanzamento e ETA sensati; memoria stabile; hash uguale |
| S4 | 200 file piccoli | Nessuna perdita; annota il tempo totale |
| S5 | Cartella (se il client la supporta) | Struttura conservata |
| S6 | Nome lungo (~200 caratteri), spazi, accenti; due file omonimi | `nome (1).ext`, mai sovrascritto |
| S7 | Accettazione completa / parziale / rifiuto | Stato coerente su entrambi i lati; nessun «completato» per file non arrivati |
| S8 | PIN corretto, errato, 10 tentativi ripetuti | Errato rifiutato; ripetuti limitati (429) senza bloccare per sempre |
| S9 | Wizard: verifica con codice (giusto, sbagliato, telefono sbagliato) | Dopo il codice giusto «Confermato»; con quello sbagliato nulla viene salvato |
| S10 | Trasferimento interrotto (spegni Wi-Fi del telefono a metà) e «Riprova» | Stato «Interrotto»; Riprova invia solo i non completati; nota sul rischio di duplicati |

## Direzioni e dispositivi
| ID | Direzione | Telefono / OS | LocalSend (versione) | Rete | Scenari | Esito |
|---|---|---|---|---|---|---|
| A1 | Cable → Android | — | — | — | S1–S10 | **NON ESEGUITO** |
| A2 | Android → Cable | — | — | — | S1–S10 | **NON ESEGUITO** |
| I1 | Cable → iPhone/iPad | — | — | — | S1–S10 | **NON ESEGUITO** |
| I2 | iPhone/iPad → Cable | — | — | — | S1–S10 | **NON ESEGUITO** |

## Uso quotidiano (richiede telefono e rete reali)
| ID | Prova | Esito |
|---|---|---|
| Q1 | Finestra Cable chiusa, ricezione in background attiva: arriva la richiesta (notifica con Accetta/Rifiuta) | **NON ESEGUITO** |
| Q2 | Riapertura da launcher, da `cable file…` e da Nautilus › Scripts › Invia con Cable (Cable già aperto e in background) | **NON ESEGUITO** |
| Q3 | Telefono bloccato / app mobile in background: annota se l'invio verso il telefono e la ricezione funzionano (non è promesso) | **NON ESEGUITO** |
| Q4 | Cambio Wi-Fi e cambio IP del PC/telefono: presenza aggiornata entro ~5–10 s | **NON ESEGUITO** |
| Q5 | Sospensione e ripresa del PC | **NON ESEGUITO** |
| Q6 | Hotspot del telefono; hotspot del PC | **NON ESEGUITO** |
| Q7 | Multicast bloccato (AP con isolamento): collegamento con IP manuale | **NON ESEGUITO** |
| Q8 | Rete interrotta tra due file; interruzione dopo l'ultimo byte e prima della conferma (esito incerto segnalato) | **NON ESEGUITO** |
| Q9 | Spazio disco insufficiente sul PC (es. partizione piccola come destinazione) | **NON ESEGUITO** (provato solo con soglia artificiale in `receiver::insufficient_space_507`) |
| Q10 | LocalSend per PC aperto mentre parte Cable: «porta occupata» chiara, Riprova dopo la chiusura | **NON ESEGUITO** su applicazione reale (provato con socket in `e2e`) |
| Q11 | LAN + VPN reale (Tailscale/WireGuard): il telefono si trova ancora, `Diagnostica` elenca la VPN fra le escluse | **NON ESEGUITO** |

Vedi anche la lista più ampia in [CHECKLIST-MANUALE.md](CHECKLIST-MANUALE.md). Non cambiare firewall, VPN o rete del PC personale per queste prove senza saperlo.
