//! Integrazione con il desktop: cartelle, file manager, avvio automatico, istanze della versione Python.
use crate::paths::Paths;
use crate::settings::backup_file;
use std::path::Path;
use std::process::{Command, Stdio};

pub const APP_ID: &str = "it.frank.PixelHub";

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
    let exe = std::env::current_exe().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|_| "pixel-hub".into());
    format!("\"{}\" --background", exe.replace('"', "\\\""))
}

pub fn imposta_avvio(paths: &Paths, attivo: bool) -> bool {
    let f = paths.autostart_file();
    if attivo {
        if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
        std::fs::write(&f, format!("[Desktop Entry]\nType=Application\nName=Pixel Hub\nExec={}\nIcon={APP_ID}\n\
                                    X-GNOME-Autostart-enabled=true\nNoDisplay=true\n", exec_line())).is_ok()
    } else {
        !f.exists() || std::fs::remove_file(&f).is_ok()
    }
}

/// Se l'avvio al login puntava ancora a uno script Python di Pixel Hub, lo riscrive verso questa
/// versione dopo averne salvato una copia (serve per il ritorno indietro).
pub fn migra_avvio_automatico(paths: &Paths) {
    let f = paths.autostart_file();
    let Ok(t) = std::fs::read_to_string(&f) else { return };
    if t.contains("pixel_hub.py") || t.contains("-m pixelhub") {
        let _ = backup_file(&f, &paths.backups(), "autostart-precedente");
        imposta_avvio(paths, true);
    }
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
