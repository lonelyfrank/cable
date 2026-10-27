Name:           cable
Version:        6.0.0
Release:        1%{?dist}
Summary:        Trasferimento rapido di file e testi via LocalSend per Fedora GNOME
License:        LicenseRef-proprietary
URL:            https://example.invalid/cable
Source0:        %{name}-%{version}.tar.gz
# Crate delle dipendenze (RPM compila offline): si crea con `scripts/dev.sh vendor`
Source1:        %{name}-%{version}-vendor.tar.xz

BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  pkgconfig(gtk+-3.0)
BuildRequires:  pkgconfig(webkit2gtk-4.1)
BuildRequires:  pkgconfig(javascriptcoregtk-4.1)
BuildRequires:  pkgconfig(libsoup-3.0)
BuildRequires:  pkgconfig(librsvg-2.0)
BuildRequires:  desktop-file-utils
BuildRequires:  libappstream-glib

# Le librerie condivise (gtk3, webkit2gtk4.1, libsoup3...) sono dedotte automaticamente dal binario.
Requires:       xdg-utils
Requires:       glib2
Recommends:     NetworkManager
Recommends:     android-tools
Suggests:       scrcpy

%description
Cable invia e riceve file e testi fra Fedora e gli smartphone sulla rete
locale con il protocollo LocalSend (non serve LocalSend per PC). Ogni ricezione
richiede conferma. Procedura guidata di primo collegamento con verifica del
telefono tramite codice. Funzioni Android opzionali: schermo (scrcpy) e
importazione foto via ADB. Fedora tradizionale con DNF; Fedora Atomic non è
verificato.

%prep
%autosetup
tar xf %{SOURCE1}
mkdir -p .cargo
cat > .cargo/config.toml <<'EOF'
[source.crates-io]
replace-with = "vendored-sources"
[source.vendored-sources]
directory = "vendor"
EOF

%build
export CARGO_HOME=$PWD/.cargo-home
cargo build --release --offline --locked -p cable

%install
install -Dm755 target/release/cable %{buildroot}%{_bindir}/cable
install -Dm644 data/it.frank.Cable.desktop %{buildroot}%{_datadir}/applications/it.frank.Cable.desktop
install -Dm644 data/it.frank.Cable.metainfo.xml %{buildroot}%{_metainfodir}/it.frank.Cable.metainfo.xml
for n in 16 24 32 48 64 128 256 512; do
    install -Dm644 data/icons/hicolor/${n}x${n}/apps/it.frank.Cable.png \
        %{buildroot}%{_datadir}/icons/hicolor/${n}x${n}/apps/it.frank.Cable.png
done

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/it.frank.Cable.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/it.frank.Cable.metainfo.xml
export CARGO_HOME=$PWD/.cargo-home
cargo test --release --offline --locked -p pixelhub-engine --lib --test receiver --test e2e

%files
%doc README.md docs/
%{_bindir}/cable
%{_datadir}/applications/it.frank.Cable.desktop
%{_metainfodir}/it.frank.Cable.metainfo.xml
%{_datadir}/icons/hicolor/*/apps/it.frank.Cable.png

%changelog
* Tue Oct 27 2026 Mantenitore - 6.0.0-1
- Il progetto diventa Cable; migrazione da Pixel Hub; nuova icona
