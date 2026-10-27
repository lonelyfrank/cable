//! Integrazione con il desktop: cartelle, file manager, avvio automatico, istanze della versione Python.
use crate::paths::Paths;
use crate::settings::backup_file;
use std::path::Path;
use std::process::{Command, Stdio};

pub const APP_ID: &str = "it.frank.Cable";
/// Identificatore usato fino a Pixel Hub 5.0 (lanciatore e avvio automatico precedenti)
pub const OLD_APP_ID: &str = "it.frank.PixelHub";

fn spawn(argv: &[&str]) -> bool {
    Command::new(argv[0]).args(&argv[1..]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok()
}

pub fn apri(p: &Path) -> bool { spawn(&["gio", "open", &p.to_string_lossy()]) }

/// Evidenzia il file nel file manager; ripiega sull'apertura della cartella.
pub fn mostra(p: &Path) -> bool {
    let uri = format!("['file://{}']", p.to_string_lossy());
    let ok = Command::new("gdbus").args(["call", "--session", "--dest", "org.freedesktop.FileManager1", "--object-path",
        "/org/freedesktop/FileManager1", "--method", "org.freedesktop.FileManager1.ShowItems", &uri, ""])
        .stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false);
    ok || p.parent().map_or(false, apri)
}

pub fn has_command(cmd: &str) -> bool {
    std::env::var_os("PATH").map_or(false, |p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
}

fn exec_line() -> String {
    let exe = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|_| "cable".into());
    format!("\"{}\" --background", exe.replace('"', "\\\""))
}

pub fn imposta_avvio(paths: &Paths, attivo: bool) -> bool {
    let f = paths.autostart_file();
    if attivo {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        std::fs::write(&f, format!("[Desktop Entry]\nType=Application\nName=Cable\nExec={}\nIcon={APP_ID}\n\
                                    X-GNOME-Autostart-enabled=true\nNoDisplay=true\n", exec_line())).is_ok()
    } else {
        let _ = std::fs::remove_file(paths.old_autostart_file());       // anche quello di Pixel Hub, se ancora presente
        !f.exists() || std::fs::remove_file(&f).is_ok()
    }
}

/// Porta l'avvio automatico precedente (Pixel Hub 5.0 o script Python) verso Cable: prima ne salva una copia in
/// `backups/` (serve per il ritorno indietro), poi scrive il nuovo e toglie il vecchio per non avviare due app
/// sulla stessa porta. Non fa nulla se l'utente non aveva l'avvio automatico.
pub fn migra_avvio_automatico(paths: &Paths) {
    let old = paths.old_autostart_file();
    if old.is_file() {
        let _ = backup_file(&old, &paths.backups(), "autostart-pixel-hub");
        // se Cable ha già il proprio avvio automatico lo si lascia com'è
        if paths.autostart_file().exists() || imposta_avvio(paths, true) { let _ = std::fs::remove_file(&old); }
    }
    let f = paths.autostart_file();
    let Ok(t) = std::fs::read_to_string(&f) else { return };
    if t.contains("pixel_hub.py") || t.contains("-m pixelhub") || t.contains("/pixel-hub") {
        let _ = backup_file(&f, &paths.backups(), "autostart-precedente");
        imposta_avvio(paths, true);
    }
}

/// PID di processi `pixel-hub` (Pixel Hub 5.0): usano la stessa porta di Cable.
pub fn processi_pixel_hub() -> Vec<u32> {
    let me = std::process::id();
    let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
    rd.flatten().filter_map(|e| {
        let pid: u32 = e.file_name().to_string_lossy().parse().ok()?;
        if pid == me { return None; }
        let comm = std::fs::read_to_string(e.path().join("comm")).ok()?;
        (comm.trim() == "pixel-hub").then_some(pid)
    }).collect()
}

/// PID di istanze della versione Python (3.x o 4.0): usano le stesse porte e lo stesso APP_ID.
pub fn processi_python() -> Vec<u32> {
    let me = std::process::id();
    let mut found = vec![];
    let Ok(rd) = std::fs::read_dir("/proc") else { return found };
    for e in rd.flatten() {
        let Ok(pid) = e.file_name().to_string_lossy().parse::<u32>() else { continue };
        if pid == me { continue; }
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        let args: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
        let is_py = args.first().map_or(false, |a| a.ends_with(b"python3") || a.ends_with(b"python") || a.ends_with(b"/env"));
        let hit = args.iter().take(5).any(|a| a.ends_with(b"/pixel-hub/pixel_hub.py") || *a == b"pixel_hub.py")
            || (args.windows(2).take(5).any(|w| w[0] == b"-m" && w[1] == b"pixelhub"));
        if is_py && hit { found.push(pid); }
    }
    found
}

// ---------------------------------------------------------------------------------------- distribuzione e firewall
/// Famiglia della distribuzione: decide quale gestore di pacchetti suggerire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Famiglia { Debian, Fedora, Altra }

/// Ricava la famiglia dal contenuto di `/etc/os-release` (`ID` e `ID_LIKE`: Ubuntu, Mint, Pop!_OS… dichiarano `debian`).
pub fn famiglia_da(os_release: &str) -> Famiglia {
    let ids: Vec<String> = os_release.lines()
        .filter_map(|l| l.strip_prefix("ID=").or_else(|| l.strip_prefix("ID_LIKE=")))
        .flat_map(|v| v.trim_matches('"').split_whitespace().map(str::to_lowercase).collect::<Vec<_>>())
        .collect();
    let ha = |n: &str| ids.iter().any(|i| i == n);
    if ha("debian") || ha("ubuntu") { Famiglia::Debian }
    else if ha("fedora") || ha("rhel") || ha("centos") { Famiglia::Fedora }
    else { Famiglia::Altra }
}

fn leggi_os_release() -> String { std::fs::read_to_string("/etc/os-release").unwrap_or_default() }

pub fn famiglia() -> Famiglia { famiglia_da(&leggi_os_release()) }

/// «Ubuntu 26.04.1 LTS», «Fedora Linux 44 (Workstation Edition)»… oppure vuoto se non dichiarata.
pub fn nome_sistema() -> String {
    leggi_os_release().lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string()).unwrap_or_default()
}

/// Comando da mostrare per installare un componente facoltativo (`adb`, `scrcpy`).
pub fn comando_installazione(f: Famiglia, componente: &str) -> String {
    match (f, componente) {
        (Famiglia::Debian, "adb") => "sudo apt install adb".into(),
        (Famiglia::Debian, "scrcpy") => "sudo apt install scrcpy".into(),
        (Famiglia::Debian, "uxplay") => "sudo apt install uxplay gstreamer1.0-plugins-bad gstreamer1.0-libav avahi-daemon".into(),
        (Famiglia::Fedora, "uxplay") => "sudo dnf install uxplay".into(),
        (Famiglia::Fedora, "adb") => "sudo dnf install android-tools".into(),
        (Famiglia::Fedora, "scrcpy") => "sudo dnf copr enable zeno/scrcpy && sudo dnf install scrcpy".into(),
        (_, "adb") => "installa «adb» (Android platform tools) con il gestore di pacchetti della tua distribuzione".into(),
        _ => format!("installa «{componente}» con il gestore di pacchetti della tua distribuzione"),
    }
}

/// `ENABLED=yes|no` in `/etc/ufw/ufw.conf` (leggibile senza privilegi, a differenza di `ufw status`).
pub fn ufw_attivo_da(conf: &str) -> Option<bool> {
    conf.lines().find_map(|l| l.trim().strip_prefix("ENABLED=")).map(|v| v.trim().eq_ignore_ascii_case("yes"))
}

/// Descrizione del firewall per la diagnostica e, se ne conosciamo uno attivo, il comando per aprire la porta di Cable.
/// Le regole di ufw non si leggono senza sudo: si dice solo se è attivo, non se la porta è aperta.
pub fn firewall(porta: u16) -> (String, Option<String>) {
    if let Some(z) = Command::new("firewall-cmd").arg("--get-active-zones").output().ok().filter(|o| o.status.success()) {
        return (format!("firewalld: {}", String::from_utf8_lossy(&z.stdout).replace('\n', " | ")), None);
    }
    match std::fs::read_to_string("/etc/ufw/ufw.conf").ok().and_then(|c| ufw_attivo_da(&c)) {
        Some(true) => ("ufw attivo (le regole non si leggono senza sudo: controlla con «sudo ufw status»)".into(),
                       Some(format!("sudo ufw allow {porta}/tcp && sudo ufw allow {porta}/udp"))),
        Some(false) => ("ufw installato ma disattivato".into(), None),
        None => ("nessun firewall rilevato (firewalld o ufw)".into(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn famiglia_dalle_voci_di_os_release() {
        assert_eq!(famiglia_da("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n"), Famiglia::Debian);
        assert_eq!(famiglia_da("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), Famiglia::Debian);
        assert_eq!(famiglia_da("ID=fedora\n"), Famiglia::Fedora);
        assert_eq!(famiglia_da("ID=rocky\nID_LIKE=\"rhel centos fedora\"\n"), Famiglia::Fedora);
        assert_eq!(famiglia_da("ID=arch\n"), Famiglia::Altra);
        assert_eq!(famiglia_da(""), Famiglia::Altra);
    }

    #[test]
    fn comandi_per_distribuzione() {
        assert_eq!(comando_installazione(Famiglia::Debian, "adb"), "sudo apt install adb");
        assert!(comando_installazione(Famiglia::Fedora, "scrcpy").contains("dnf"));
        assert!(!comando_installazione(Famiglia::Debian, "scrcpy").contains("dnf"));
        assert!(!comando_installazione(Famiglia::Altra, "adb").starts_with("sudo"));
    }

    #[test]
    fn ufw_attivo_o_no() {
        assert_eq!(ufw_attivo_da("# c\nENABLED=yes\nLOGLEVEL=low\n"), Some(true));
        assert_eq!(ufw_attivo_da("\nENABLED=no\n"), Some(false));
        assert_eq!(ufw_attivo_da("LOGLEVEL=low\n"), None);
    }
}
