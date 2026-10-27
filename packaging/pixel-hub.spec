Name:           pixel-hub
Version:        5.0.0
Release:        1%{?dist}
Summary:        Trasferimento rapido di file e testi via LocalSend per Fedora GNOME
License:        LicenseRef-proprietary
URL:            https://example.invalid/pixel-hub
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
Pixel Hub invia e riceve file e testi fra Fedora e gli smartphone sulla rete
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
cargo build --release --offline --locked -p pixel-hub

%install
install -Dm755 target/release/pixel-hub %{buildroot}%{_bindir}/pixel-hub
install -Dm644 data/it.frank.PixelHub.desktop %{buildroot}%{_datadir}/applications/it.frank.PixelHub.desktop
install -Dm644 data/it.frank.PixelHub.metainfo.xml %{buildroot}%{_metainfodir}/it.frank.PixelHub.metainfo.xml
install -Dm644 data/icons/hicolor/scalable/apps/it.frank.PixelHub.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/it.frank.PixelHub.svg

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/it.frank.PixelHub.desktop
appstream-util validate-relax --nonet %{buildroot}%{_metainfodir}/it.frank.PixelHub.metainfo.xml
export CARGO_HOME=$PWD/.cargo-home
cargo test --release --offline --locked -p pixelhub-engine --lib --test receiver --test e2e

%files
%doc README.md docs/
%{_bindir}/pixel-hub
%{_datadir}/applications/it.frank.PixelHub.desktop
%{_metainfodir}/it.frank.PixelHub.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/it.frank.PixelHub.svg

%changelog
* Tue Oct 27 2026 Mantenitore - 5.0.0-1
- Riscrittura in Rust con Tauri; wizard di primo collegamento con verifica a codice
