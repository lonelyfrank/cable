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
    /// Cartelle usate fino a Pixel Hub 5.0 (solo lettura: la migrazione le copia, non le sposta)
    pub old_config: PathBuf,
    pub old_data: PathBuf,
}

impl Paths {
    pub fn system() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let config_base = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let data_base = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let pick = |d: Option<PathBuf>, fb: &str| d.filter(|p| *p != home).unwrap_or_else(|| home.join(fb));
        Paths {
            config: config_base.join("cable"),
            data: data_base.join("cable"),
            cache: dirs::cache_dir().unwrap_or_else(|| home.join(".cache")).join("cable"),
            old_config: config_base.join("pixel-hub"),
            old_data: data_base.join("pixel-hub"),
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
            old_config: base.join("old-config"),
            old_data: base.join("old-data"),
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
    pub fn autostart_file(&self) -> PathBuf { self.home.join(".config/autostart/it.frank.Cable.desktop") }
    /// Avvio automatico creato da Pixel Hub 5.0
    pub fn old_autostart_file(&self) -> PathBuf { self.home.join(".config/autostart/it.frank.PixelHub.desktop") }
    pub fn old_settings_file(&self) -> PathBuf { self.old_config.join("settings.json") }
    pub fn old_identity_dir(&self) -> PathBuf { self.old_data.join("identity") }
    /// Registro della migrazione da Pixel Hub (presenza = già eseguita almeno una volta)
    pub fn migration_record(&self) -> PathBuf { self.config.join("migrazione-pixel-hub.json") }
}
