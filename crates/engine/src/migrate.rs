//! Migrazione da Pixel Hub 5.0 (cartelle `pixel-hub`) a Cable (cartelle `cable`).
//!
//! Principi: si **copia**, non si sposta (le vecchie cartelle restano intatte e permettono di tornare indietro);
//! non si sovrascrive mai un file di Cable già esistente; l'identità TLS viene copiata byte per byte, quindi
//! l'impronta e le associazioni degli altri dispositivi non cambiano; ripetere la funzione non ha effetti.
use crate::paths::Paths;
use crate::settings::{backup_file, default_name, OLD_DEFAULT_NAME};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    /// Messaggi per l'utente (mostrati una volta nella pagina iniziale)
    pub avvisi: Vec<String>,
    pub impostazioni_copiate: bool,
    pub identita_copiata: bool,
}

fn write_atomic(dest: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    let tmp = dest.with_extension(format!("migrazione-{}.tmp", std::process::id()));
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, dest)
}

fn migrate_settings(p: &Paths, r: &mut Report) {
    let (old, new) = (p.old_settings_file(), p.settings_file());
    if !old.is_file() {
        return;
    }
    if new.exists() {
        // dati vecchi e nuovi: vincono quelli di Cable, senza fonderli alla cieca
        if !p.migration_record().exists() && fs::read(&old).ok() != fs::read(&new).ok() {
            r.avvisi.push(format!(
                "Esistono impostazioni sia di Pixel Hub sia di Cable: uso quelle di Cable. Le vecchie sono rimaste in {}.",
                p.old_config.display()
            ));
        }
        return;
    }
    let result = (|| -> Result<(), String> {
        let text = fs::read_to_string(&old).map_err(|e| e.to_string())?;
        let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        let obj = v.as_object_mut().ok_or("formato non valido")?;
        if obj.get("nome_pc").and_then(Value::as_str) == Some(OLD_DEFAULT_NAME) {
            obj.insert("nome_pc".into(), json!(default_name())); // solo il nome predefinito, mai uno scelto dall'utente
        }
        if !obj.get("migrato_da").is_some_and(Value::is_object) {
            obj.insert("migrato_da".into(), json!({"versione": "pixel-hub-5", "quando": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()}));
        }
        // copia di sicurezza dell'originale accanto alle impostazioni di Cable, poi il file nuovo
        backup_file(&old, &p.backups(), "settings-pixel-hub").map_err(|e| e.to_string())?;
        write_atomic(&new, serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?.as_bytes(), 0o600).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => {
            r.impostazioni_copiate = true;
        }
        Err(e) => r.avvisi.push(format!(
            "Non sono riuscito a importare le impostazioni di Pixel Hub ({e}): Cable parte con i valori predefiniti. I file originali non sono stati toccati."
        )),
    }
}

fn migrate_identity(p: &Paths, r: &mut Report) {
    let old = p.old_identity_dir();
    let (oc, ok) = (old.join("cert.pem"), old.join("key.pem"));
    if !oc.is_file() || !ok.is_file() {
        return;
    }
    let dir = p.identity_dir();
    let (nc, nk) = (dir.join("cert.pem"), dir.join("key.pem"));
    match (nc.exists(), nk.exists()) {
        (false, false) => {
            let res = (|| -> std::io::Result<()> {
                fs::create_dir_all(&dir)?;
                fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                // prima la chiave (0600), poi il certificato: un'interruzione non lascia mai un certificato senza chiave
                write_atomic(&nk, &fs::read(&ok)?, 0o600)?;
                write_atomic(&nc, &fs::read(&oc)?, 0o644)
            })();
            match res {
                Ok(()) => r.identita_copiata = true,
                Err(e) => {
                    let _ = fs::remove_file(&nk);
                    let _ = fs::remove_file(&nc);
                    r.avvisi.push(format!("Non sono riuscito a copiare l'identità di Pixel Hub ({e}). Cable ne creerà una nuova: i dispositivi dovranno ricollegarsi. L'identità originale è intatta in {}.", old.display()));
                }
            }
        }
        _ => {
            // identità di Cable già presente (anche incompleta): non si tocca
            if !p.migration_record().exists() {
                r.avvisi.push(format!("Esiste già un'identità di Cable: non ho copiato quella di Pixel Hub (rimasta in {}).", old.display()));
            }
        }
    }
}

/// Esegue la migrazione. Va chiamata **prima** di caricare impostazioni e identità.
pub fn run(p: &Paths) -> Report {
    let mut r = Report::default();
    migrate_settings(p, &mut r);
    migrate_identity(p, &mut r);
    if r.impostazioni_copiate || r.identita_copiata {
        let mut parts = vec![];
        if r.impostazioni_copiate {
            parts.push("impostazioni, dispositivi ricordati e preferenze");
        }
        if r.identita_copiata {
            parts.push("identità del PC (stessa impronta: i telefoni già collegati ti riconoscono)");
        }
        r.avvisi.push(format!(
            "Dati importati da Pixel Hub: {}. Le cartelle originali ({}, {}) non sono state modificate: restano come copia di ripristino.",
            parts.join(" e "),
            p.old_config.display(),
            p.old_data.display()
        ));
        let rec = json!({"quando": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                         "impostazioni": r.impostazioni_copiate, "identita": r.identita_copiata,
                         "origine_config": p.old_config.to_string_lossy(), "origine_dati": p.old_data.to_string_lossy()});
        let _ = write_atomic(&p.migration_record(), serde_json::to_string_pretty(&rec).unwrap().as_bytes(), 0o600);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use crate::settings::Settings;

    fn old_install(p: &Paths, name: &str) -> String {
        // un'identità "vecchia": creata con un Paths il cui data è la vecchia cartella
        let mut q = p.clone();
        q.data = p.old_data.clone();
        let id = Identity::load(&q);
        assert_eq!(id.stato, "ok");
        fs::create_dir_all(&p.old_config).unwrap();
        let fav = "A".repeat(64);
        fs::write(
            p.old_settings_file(),
            json!({"schema": 4, "nome_pc": name, "tema": "scuro", "wizard_fatto": true,
            "preferiti": {fav: {"alias": "Telefono di Anna", "ip": "192.168.1.9", "port": 53317, "confermato": true}}})
            .to_string(),
        )
        .unwrap();
        id.fingerprint.clone()
    }

    #[test]
    fn copies_settings_and_identity_keeping_originals_and_fingerprint() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let fp = old_install(&p, OLD_DEFAULT_NAME);
        let before = fs::read(p.old_settings_file()).unwrap();
        let key_before = fs::read(p.old_identity_dir().join("key.pem")).unwrap();
        let r = run(&p);
        assert!(r.impostazioni_copiate && r.identita_copiata, "{r:?}");
        let s = Settings::load(&p);
        assert_eq!(s.str("nome_pc"), default_name(), "il nome predefinito diventa quello di Cable");
        assert_eq!(s.str("tema"), "scuro");
        assert!(s.bool("wizard_fatto"));
        assert_eq!(s.preferiti()[&"A".repeat(64)]["confermato"], json!(true), "associazioni conservate");
        assert_eq!(s.get("migrato_da")["versione"], "pixel-hub-5");
        let id = Identity::load(&p);
        assert_eq!((id.stato.as_str(), id.origine.as_str(), id.fingerprint.as_str()), ("ok", "esistente", fp.as_str()));
        assert_eq!(fs::metadata(p.identity_dir().join("key.pem")).unwrap().permissions().mode() & 0o777, 0o600);
        // originali intatti, copia di sicurezza presente
        assert_eq!(fs::read(p.old_settings_file()).unwrap(), before);
        assert_eq!(fs::read(p.old_identity_dir().join("key.pem")).unwrap(), key_before);
        assert!(fs::read_dir(p.backups()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("settings-pixel-hub")));
        assert!(p.migration_record().exists());
    }

    #[test]
    fn custom_device_name_is_kept() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        old_install(&p, "Il PC di casa");
        run(&p);
        assert_eq!(Settings::load(&p).str("nome_pc"), "Il PC di casa");
    }

    #[test]
    fn idempotent_and_never_overwrites_new_data() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let fp = old_install(&p, "Vecchio");
        assert!(run(&p).identita_copiata);
        // l'utente cambia qualcosa in Cable; il vecchio file cambia pure
        let s = Settings::load(&p);
        s.aggiorna(json!({"nome_pc": "Nuovo in Cable"}).as_object().unwrap());
        fs::write(p.old_settings_file(), json!({"schema": 4, "nome_pc": "Cambiato nel vecchio"}).to_string()).unwrap();
        let again = run(&p);
        assert!(!again.impostazioni_copiate && !again.identita_copiata);
        assert!(again.avvisi.is_empty(), "nessun avviso ripetuto: {:?}", again.avvisi);
        assert_eq!(Settings::load(&p).str("nome_pc"), "Nuovo in Cable");
        assert_eq!(Identity::load(&p).fingerprint, fp);
    }

    #[test]
    fn existing_new_identity_wins_and_is_reported_once() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        old_install(&p, "Vecchio");
        let nuova = Identity::load(&p).fingerprint; // Cable crea la propria identità prima della migrazione
        let r = run(&p);
        assert!(!r.identita_copiata);
        assert!(r.avvisi.iter().any(|a| a.contains("identità di Cable")), "{:?}", r.avvisi);
        assert_eq!(Identity::load(&p).fingerprint, nuova);
    }

    #[test]
    fn corrupt_old_settings_do_not_break_and_originals_stay() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        fs::create_dir_all(&p.old_config).unwrap();
        fs::write(p.old_settings_file(), "{non json").unwrap();
        let r = run(&p);
        assert!(!r.impostazioni_copiate);
        assert!(r.avvisi.iter().any(|a| a.contains("Non sono riuscito")));
        assert_eq!(fs::read_to_string(p.old_settings_file()).unwrap(), "{non json");
        assert!(!p.settings_file().exists());
    }

    #[test]
    fn nothing_to_do_without_old_data() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        assert_eq!(run(&p), Report::default());
        assert!(!p.migration_record().exists());
    }
}
