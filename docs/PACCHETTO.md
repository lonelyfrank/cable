# Pacchetto RPM e installazione: cosa è stato verificato

Costruito il 27 ottobre 2026 in un **contenitore Fedora 44 isolato** (`podman`, nessuna installazione sul sistema dell'autore; `mock` non disponibile).
Questo documento dice solo ciò che è stato eseguito; il resto è elencato come non verificato.

## Riprodurre la costruzione
```bash
# 1. immagine con gli strumenti (una volta)
cat > Containerfile <<'X'
FROM registry.fedoraproject.org/fedora:44
RUN dnf -y --setopt=install_weak_deps=False install cargo rust clippy rustfmt gcc pkgconf-pkg-config gtk3-devel \
    webkit2gtk4.1-devel libsoup3-devel javascriptcoregtk4.1-devel librsvg2-devel rpm-build desktop-file-utils \
    libappstream-glib python3 git tar xz findutils which
X
podman build -t cable-build:44 .

# 2. sorgenti e RPM (dalla radice del repository, dopo il commit)
mkdir -p /tmp/rpm/out && git archive --prefix=cable-6.0.0/ HEAD | gzip > /tmp/rpm/cable-6.0.0.tar.gz && cp packaging/cable.spec /tmp/rpm/
cat > /tmp/rpm/build.sh <<'X'
set -euo pipefail
mkdir -p ~/rpmbuild/SOURCES ~/rpmbuild/SPECS && cp /rpm/cable-6.0.0.tar.gz ~/rpmbuild/SOURCES/
mkdir /tmp/v && cd /tmp/v && tar xf /rpm/cable-6.0.0.tar.gz && cd cable-6.0.0
cargo vendor --locked vendor >/dev/null && tar cJf ~/rpmbuild/SOURCES/cable-6.0.0-vendor.tar.xz vendor
cp /rpm/cable.spec ~/rpmbuild/SPECS/ && rpmbuild -ba ~/rpmbuild/SPECS/cable.spec
cp ~/rpmbuild/RPMS/*/*.rpm ~/rpmbuild/SRPMS/*.rpm /rpm/out/
X
podman run --rm -v /tmp/rpm:/rpm:Z cable-build:44 bash /rpm/build.sh
```
Il vendoring richiede rete (crates.io); la compilazione dell'RPM è poi offline (`--offline --locked`).

## Esiti osservati (`cable-6.0.0-2.fc44`)
| Verifica | Esito |
|---|---|
| `rpmbuild -ba` | **riuscito**: `cable-6.0.0-2.fc44.x86_64.rpm` (4,4 MiB) e `.src.rpm` |
| `%check` nello spec | `desktop-file-validate` e `appstream-util validate-relax` ok; test motore **46 + 23 + 15 + 4** passati |
| Dipendenze runtime dedotte | `libgtk-3`, `libwebkit2gtk-4.1`, `libjavascriptcoregtk-4.1`, `libsoup-3.0`, `libdbus-1`, `libcairo`, `libgdk_pixbuf`, `glib2`, `xdg-utils`; consigliati NetworkManager, android-tools; suggerito scrcpy |
| Contenuto | `/usr/bin/cable`, desktop entry, metainfo, icone hicolor 16–512 px, documentazione |
| Installazione in Fedora 44 pulita (`dnf install`) | riuscita (161 pacchetti, dipendenze risolte); `cable --versione` = 6.0.0; nessuna libreria mancante (`ldd`); `rpm -V` pulito; desktop entry valida |
| «Aggiornamento» | **solo reinstallazione della stessa versione** (`dnf reinstall`): impostazioni, chiave dell'identità TLS e file ricevuti identici (sha256) e presenti. Un vero aggiornamento da `6.0.0-1` non è stato provato (nessun RPM precedente disponibile) |
| Disinstallazione (`dnf remove`) | riuscita: binario e desktop entry rimossi; configurazione, identità e file ricevuti restano |

## Non verificato
* Avvio grafico dell'RPM installato (il contenitore non ha display): provato solo `--versione`. L'avvio con finestra è provato dal binario di sviluppo (ui-test), non da quello del pacchetto.
* Fedora Atomic, altre distribuzioni, `mock` con repository ufficiali, firma del pacchetto (`License:` e `URL:` dello spec sono segnaposto).
* Migrazione da Pixel Hub *tramite il pacchetto*: la migrazione è nel codice (test unitari in `migrate.rs`), non è stata rieseguita dopo l'installazione dell'RPM.

## Installazione utente (senza RPM)
`scripts/installa-utente.sh` in una **HOME temporanea** (provata): ricompila (o avvisa se usa un binario già presente), installa binario, icone e voce di menu, salva il lanciatore di
Pixel Hub, `desktop-file-validate` ok; `rimuovi` ripristina il lanciatore. Con il pacchetto di sistema già installato, lo script avvisa: la copia in `~/.local/bin` ha la precedenza.
Dopo la correzione di questo ciclo `rimuovi` ferma solo il Cable il cui eseguibile è quello utente (provato con un Cable di sistema in esecuzione, rimasto attivo).

## Quale Cable sto usando?
```bash
command -v cable                                   # percorso risolto nel PATH (~/.local/bin ha la precedenza su /usr/bin)
cable --versione                                   # versione del binario nel PATH
readlink /proc/$(pgrep -x cable)/exe               # binario realmente in esecuzione
rpm -q cable; rpm -qf /usr/bin/cable               # pacchetto di sistema, se installato
grep ^Exec= ~/.local/share/applications/it.frank.Cable.desktop /usr/share/applications/it.frank.Cable.desktop
```
Rollback e migrazione: [ROLLBACK.md](ROLLBACK.md).
