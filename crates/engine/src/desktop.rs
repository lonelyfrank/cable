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
    ok || p.parent().is_some_and(apri)
}

pub fn has_command(cmd: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
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
        let is_py = args.first().is_some_and(|a| a.ends_with(b"python3") || a.ends_with(b"python") || a.ends_with(b"/env"));
        let hit = args.iter().take(5).any(|a| a.ends_with(b"/pixel-hub/pixel_hub.py") || *a == b"pixel_hub.py")
            || (args.windows(2).take(5).any(|w| w[0] == b"-m" && w[1] == b"pixelhub"));
        if is_py && hit { found.push(pid); }
    }
    found
}
