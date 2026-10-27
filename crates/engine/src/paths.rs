//! Percorsi dell'applicazione. Gli stessi della versione Python 4.0 (migrazione trasparente).
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub downloads: PathBuf,
    pub pictures: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let pick = |d: Option<PathBuf>, fb: &str| d.filter(|p| *p != home).unwrap_or_else(|| home.join(fb));
        Paths {
            config: dirs::config_dir().unwrap_or_else(|| home.join(".config")).join("pixel-hub"),
            data: dirs::data_dir().unwrap_or_else(|| home.join(".local/share")).join("pixel-hub"),
            cache: dirs::cache_dir().unwrap_or_else(|| home.join(".cache")).join("pixel-hub"),
            downloads: pick(dirs::download_dir(), "Downloads"),
            pictures: pick(dirs::picture_dir(), "Pictures"),
            home,
        }
    }

    /// Percorsi isolati in una cartella (test e sandbox).
    pub fn under(base: &Path) -> Self {
        let home = base.join("home");
        Paths {
            config: base.join("config"),
            data: base.join("data"),
            cache: base.join("cache"),
            downloads: home.join("Downloads"),
            pictures: home.join("Pictures"),
            home,
        }
    }

    pub fn legacy_config(&self) -> PathBuf { self.home.join(".config/pixel-hub-v3/config.json") }
    pub fn legacy_cache(&self) -> PathBuf { self.home.join(".cache/pixel-hub-v3") }
    pub fn settings_file(&self) -> PathBuf { self.config.join("settings.json") }
    pub fn backups(&self) -> PathBuf { self.config.join("backups") }
    pub fn identity_dir(&self) -> PathBuf { self.data.join("identity") }
    pub fn autostart_file(&self) -> PathBuf { self.home.join(".config/autostart/it.frank.PixelHub.desktop") }
}
