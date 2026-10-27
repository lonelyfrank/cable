//! Impostazioni persistenti. Stesso file e stesso schema (4) della versione Python 4.0,
//! più la migrazione dalla 3.x. Il file 3.x non viene mai modificato.
use crate::paths::Paths;
use crate::util::{clean_label, ct_eq, random_hex};
use pbkdf2::pbkdf2_hmac;
use serde_json::{json, Map, Value};
use sha2::Sha256;
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const SCHEMA: u64 = 4;
pub const PIN_ITERATIONS: u32 = 200_000;

pub fn is_fp(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }

pub fn hash_pin(pin: &str, salt: Option<String>, it: u32) -> Value {
    let salt = salt.unwrap_or_else(|| random_hex(16));
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha256>(pin.as_bytes(), salt.as_bytes(), it, &mut out);
    json!({"salt": salt, "hash": hex::encode(out), "it": it})
}

pub fn verify_pin_record(rec: &Value, pin: &str) -> bool {
    let (Some(salt), Some(h), Some(it)) = (rec["salt"].as_str(), rec["hash"].as_str(), rec["it"].as_u64()) else { return false };
    let mut out = [0u8; 32];
    pbkdf2_hmac::<Sha256>(pin.as_bytes(), salt.as_bytes(), it as u32, &mut out);
    ct_eq(hex::encode(out).as_bytes(), h.as_bytes())
}

fn valid_pin_record(r: &Value) -> bool {
    r["salt"].is_string() && r["hash"].is_string() && r["it"].as_u64().map_or(false, |i| (1000..=10_000_000).contains(&i))
}

fn clean_favorites(v: &Value) -> Value {
    let mut out = Map::new();
    if let Some(o) = v.as_object() {
        for (fp, rec) in o {
            let fp = fp.to_uppercase();
            if !is_fp(&fp) { continue; }
            let Some(r) = rec.as_object() else { continue };
            let port = r.get("port").and_then(Value::as_u64).filter(|p| *p > 0 && *p < 65536).unwrap_or(53317);
            let alias = r.get("alias").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("Dispositivo");
            out.insert(fp, json!({
                "alias": alias.chars().take(60).collect::<String>(),
                "ip": r.get("ip").and_then(Value::as_str).unwrap_or("").chars().take(45).collect::<String>(),
                "port": port,
                "confermato": r.get("confermato").and_then(Value::as_bool).unwrap_or(false),
            }));
        }
    }
    Value::Object(out)
}

/// (valido, valore normalizzato)
pub fn validate(key: &str, v: &Value) -> Option<Value> {
    match key {
        "nome_pc" => v.as_str().filter(|s| !s.trim().is_empty()).map(|s| json!(clean_label(s, 40, "Fedora Pixel Hub"))),
        "cartella" | "cartella_foto" => v.as_str().filter(|s| s.starts_with('/')).map(|s| json!(s)),
        "vetro" => v.as_f64().map(|n| json!((n as i64).clamp(0, 100))),
        "tema" => v.as_str().filter(|s| ["auto", "chiaro", "scuro"].contains(s)).map(|s| json!(s)),
        "notifiche" | "schermo_intero" | "background" | "avvio_login" | "wizard_fatto" => v.as_bool().map(|b| json!(b)),
        "preferiti" => Some(clean_favorites(v)),
        "pin_ricezione" => if v.is_null() || valid_pin_record(v) { Some(v.clone()) } else { None },
        _ => None,
    }
}

pub fn defaults(p: &Paths) -> Map<String, Value> {
    let Value::Object(m) = json!({
        "schema": SCHEMA, "nome_pc": "Fedora Pixel Hub",
        "cartella": p.downloads.join("Pixel Hub").to_string_lossy(),
        "cartella_foto": p.pictures.join("Pixel").to_string_lossy(),
        "vetro": 45, "tema": "auto", "notifiche": true, "schermo_intero": false, "background": true,
        "avvio_login": false, "preferiti": {}, "pin_ricezione": null, "migrato_da": null, "wizard_fatto": false,
    }) else { unreachable!() };
    m
}

fn normalize(raw: &Value, p: &Paths) -> Map<String, Value> {
    let mut d = defaults(p);
    if let Some(o) = raw.as_object() {
        for (k, v) in o {
            if let Some(nv) = validate(k, v) { d.insert(k.clone(), nv); }
        }
        if o.get("migrato_da").map_or(false, Value::is_object) { d.insert("migrato_da".into(), o["migrato_da"].clone()); }
    }
    d
}

fn atomic_write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { fs::create_dir_all(dir)?; }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    {
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
}

pub fn backup_file(src: &Path, dir: &Path, label: &str) -> std::io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_else(|| ".json".into());
    let mut dest = dir.join(format!("{label}-{}{ext}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
    let mut n = 1;
    while dest.exists() {
        dest = dir.join(format!("{label}-{}-{n}{ext}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        n += 1;
    }
    fs::copy(src, &dest)?;
    fs::set_permissions(&dest, fs::Permissions::from_mode(0o600))?;
    Ok(dest)
}

/// Converte la configurazione 3.x. I "fidati" e il telefono associato diventano preferiti NON confermati;
/// l'auto-accettazione viene scartata; il PIN di ricezione (stesso hash) è conservato.
pub fn migrate_v3(legacy: &Path, p: &Paths) -> Result<Map<String, Value>, String> {
    let old: Value = serde_json::from_str(&fs::read_to_string(legacy).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut new = normalize(&old, p);
    let mut cand: Map<String, Value> = old["fidati"].as_object().cloned().unwrap_or_default();
    let pairing = old["pairing"].as_object().cloned().unwrap_or_default();
    if let Some(fp) = pairing.get("fp").and_then(Value::as_str) {
        cand.entry(fp.to_string()).or_insert_with(|| json!(pairing.get("alias").and_then(Value::as_str).unwrap_or("Pixel")));
    }
    let mut fav = Map::new();
    for (fp, alias) in cand {
        let fp = fp.to_uppercase();
        if is_fp(&fp) {
            fav.insert(fp, json!({"alias": alias.as_str().unwrap_or("Dispositivo").chars().take(60).collect::<String>(),
                                   "ip": "", "port": 53317, "confermato": false}));
        }
    }
    let n = fav.len();
    new.insert("preferiti".into(), Value::Object(fav));
    let pin = json!({"salt": pairing.get("pin_salt"), "hash": pairing.get("pin_hash"), "it": 100000});
    let keep = valid_pin_record(&pin);
    new.insert("pin_ricezione".into(), if keep { pin } else { Value::Null });
    new.insert("wizard_fatto".into(), json!(!pairing.is_empty()));      // aveva già collegato un telefono
    new.insert("migrato_da".into(), json!({"versione": 3, "quando": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                                           "preferiti_importati": n, "pin_conservato": keep}));
    Ok(new)
}

pub struct Settings {
    pub paths: Paths,
    d: Mutex<Map<String, Value>>,
    pub avvisi: Mutex<Vec<String>>,
}

impl Settings {
    pub fn load(paths: &Paths) -> Settings {
        let s = Settings { paths: paths.clone(), d: Mutex::new(defaults(paths)), avvisi: Mutex::new(vec![]) };
        s.load_inner();
        s
    }

    fn warn(&self, m: String) { self.avvisi.lock().unwrap().push(m); }

    fn load_inner(&self) {
        let file = self.paths.settings_file();
        if file.exists() {
            match fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object) {
                Some(raw) => {
                    let version = raw["schema"].as_u64().unwrap_or(0);
                    *self.d.lock().unwrap() = normalize(&raw, &self.paths);
                    if version != SCHEMA {
                        let _ = backup_file(&file, &self.paths.backups(), &format!("settings-v{version}"));
                        self.d.lock().unwrap().insert("schema".into(), json!(SCHEMA));
                        let _ = self.save();
                    }
                    return;
                }
                None => {
                    let bad = file.with_file_name(format!("settings.json.corrotto-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
                    let _ = fs::rename(&file, &bad);
                    self.warn(format!("Le impostazioni erano illeggibili: sono state messe da parte in {}.", bad.file_name().unwrap().to_string_lossy()));
                }
            }
        }
        let legacy = self.paths.legacy_config();
        if legacy.exists() {
            let res = backup_file(&legacy, &self.paths.backups(), "config-v3").map_err(|e| e.to_string())
                .and_then(|_| migrate_v3(&legacy, &self.paths));
            match res {
                Ok(d) => {
                    *self.d.lock().unwrap() = d;
                    let _ = self.save();
                    self.warn(format!("Impostazioni importate dalla versione 3.x (copia di sicurezza in {}).", self.paths.backups().display()));
                }
                Err(e) => self.warn(format!("Migrazione dalla 3.x non riuscita ({e}): partenza con valori predefiniti.")),
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(&Value::Object(self.d.lock().unwrap().clone()))?;
        atomic_write(&self.paths.settings_file(), &text)
    }

    pub fn get(&self, k: &str) -> Value { self.d.lock().unwrap().get(k).cloned().unwrap_or(Value::Null) }
    pub fn str(&self, k: &str) -> String { self.get(k).as_str().unwrap_or("").to_string() }
    pub fn bool(&self, k: &str) -> bool { self.get(k).as_bool().unwrap_or(false) }
    pub fn snapshot(&self) -> Map<String, Value> { self.d.lock().unwrap().clone() }

    /// Applica solo chiavi note e valide; restituisce quelle effettivamente cambiate.
    pub fn aggiorna(&self, patch: &Map<String, Value>) -> Map<String, Value> {
        let mut done = Map::new();
        {
            let mut d = self.d.lock().unwrap();
            for (k, v) in patch {
                if k == "schema" || k == "migrato_da" { continue; }
                if let Some(nv) = validate(k, v) { d.insert(k.clone(), nv.clone()); done.insert(k.clone(), nv); }
            }
        }
        if !done.is_empty() { let _ = self.save(); }
        done
    }

    /// pin None/"" = nessun PIN. Salva solo hash con sale; restituisce false se la lunghezza non è valida.
    pub fn imposta_pin(&self, pin: Option<&str>) -> bool {
        match pin.filter(|p| !p.is_empty()) {
            Some(p) => {
                if !(4..=32).contains(&p.chars().count()) { return false; }
                self.aggiorna(&[("pin_ricezione".to_string(), hash_pin(p, None, PIN_ITERATIONS))].into_iter().collect());
            }
            None => { self.aggiorna(&[("pin_ricezione".to_string(), Value::Null)].into_iter().collect()); }
        }
        true
    }

    pub fn preferito(&self, fp: &str, on: bool, info: Option<(&str, &str, u16)>) -> bool {
        let fp = fp.to_uppercase();
        if !is_fp(&fp) { return false; }
        {
            let mut d = self.d.lock().unwrap();
            let fav = d.entry("preferiti").or_insert_with(|| json!({}));
            let fav = fav.as_object_mut().unwrap();
            if on {
                let prev = fav.get(&fp).cloned().unwrap_or(json!({}));
                let (alias, ip, port) = info.map(|(a, i, p)| (a.to_string(), i.to_string(), p)).unwrap_or_else(|| (
                    prev["alias"].as_str().unwrap_or("Dispositivo").to_string(),
                    prev["ip"].as_str().unwrap_or("").to_string(), prev["port"].as_u64().unwrap_or(53317) as u16));
                fav.insert(fp, json!({"alias": alias.chars().take(60).collect::<String>(), "ip": ip, "port": port,
                                       "confermato": prev["confermato"].as_bool().unwrap_or(false)}));
            } else {
                fav.remove(&fp);
            }
        }
        let _ = self.save();
        true
    }

    /// Segna un dispositivo come confermato con il codice del wizard (e lo rende preferito).
    pub fn conferma_dispositivo(&self, fp: &str, alias: &str, ip: &str, port: u16) {
        let fp = fp.to_uppercase();
        if !is_fp(&fp) { return; }
        {
            let mut d = self.d.lock().unwrap();
            let fav = d.entry("preferiti").or_insert_with(|| json!({})).as_object_mut().unwrap();
            fav.insert(fp, json!({"alias": alias.chars().take(60).collect::<String>(), "ip": ip, "port": port, "confermato": true}));
        }
        let _ = self.save();
    }

    pub fn preferiti(&self) -> Map<String, Value> { self.get("preferiti").as_object().cloned().unwrap_or_default() }

    /// Riporta tutto ai valori predefiniti dopo un backup. Non tocca file ricevuti né identità.
    pub fn ripristina(&self) -> Option<PathBuf> {
        let file = self.paths.settings_file();
        let bk = if file.exists() { backup_file(&file, &self.paths.backups(), "settings-prima-del-ripristino").ok() } else { None };
        *self.d.lock().unwrap() = defaults(&self.paths);
        let _ = self.save();
        bk
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FP1: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const FP2: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";

    fn legacy(p: &Paths, extra: Value) {
        fs::create_dir_all(p.legacy_config().parent().unwrap()).unwrap();
        let mut d = json!({"nome_pc": "Il mio PC", "cartella": "/data/ricevuti", "cartella_foto": "/data/foto",
            "tema": "scuro", "vetro": 70, "auto_accetta": true,
            "fidati": {FP2.to_lowercase(): "Vecchio fidato", "non-valido": "x"},
            "pairing": {"fp": FP1, "alias": "Pixel di prova", "pin_salt": "s", "pin_hash": "h", "ready": true, "adb_serial": "XYZ"}});
        for (k, v) in extra.as_object().unwrap() { d[k] = v.clone(); }
        fs::write(p.legacy_config(), d.to_string()).unwrap();
    }

    #[test]
    fn v3_migration_preserves_and_demotes() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        legacy(&p, json!({}));
        let original = fs::read(p.legacy_config()).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.str("cartella"), "/data/ricevuti");
        assert_eq!(s.str("cartella_foto"), "/data/foto");
        assert_eq!(s.str("nome_pc"), "Il mio PC");
        assert_eq!(s.str("tema"), "scuro");
        assert!(!s.snapshot().contains_key("auto_accetta"));
        let fav = s.preferiti();
        assert_eq!(fav.keys().cloned().collect::<std::collections::BTreeSet<_>>(), [FP1.to_string(), FP2.to_string()].into());
        assert!(fav.values().all(|f| f["confermato"] == json!(false)));
        assert_eq!(s.get("migrato_da")["versione"], 3);
        assert!(s.bool("wizard_fatto"));
        assert_eq!(fs::read(p.legacy_config()).unwrap(), original);
        assert_eq!(fs::read_dir(p.backups()).unwrap().count(), 1);
    }

    #[test]
    fn migration_idempotent() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        legacy(&p, json!({}));
        Settings::load(&p);
        fs::write(p.legacy_config(), json!({"nome_pc": "Cambiato dopo"}).to_string()).unwrap();
        assert_eq!(Settings::load(&p).str("nome_pc"), "Il mio PC");
        assert_eq!(fs::read_dir(p.backups()).unwrap().count(), 1);
    }

    #[test]
    fn old_pin_hash_compatible() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let rec = hash_pin("123456", Some("abcd".into()), 100000);
        legacy(&p, json!({"pairing": {"fp": FP1, "alias": "P", "pin_salt": "abcd", "pin_hash": rec["hash"]}}));
        let s = Settings::load(&p);
        assert!(verify_pin_record(&s.get("pin_ricezione"), "123456"));
        assert!(!verify_pin_record(&s.get("pin_ricezione"), "654321"));
    }

    #[test]
    fn reads_python_4_0_file_and_corrupt_is_set_aside() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        fs::create_dir_all(&p.config).unwrap();
        fs::write(p.settings_file(), r#"{"schema":4,"nome_pc":"Da Python","preferiti":{},"pin_ricezione":null}"#).unwrap();
        assert_eq!(Settings::load(&p).str("nome_pc"), "Da Python");
        fs::write(p.settings_file(), "{non json").unwrap();
        let s = Settings::load(&p);
        assert!(!s.avvisi.lock().unwrap().is_empty());
        assert_eq!(s.str("nome_pc"), "Fedora Pixel Hub");
        assert!(fs::read_dir(&p.config).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("corrotto")));
    }

    #[test]
    fn validation_pin_favorites_reset() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let s = Settings::load(&p);
        let patch = json!({"tema": "rosa", "vetro": 999, "nome_pc": "  PC  ", "cartella": "relativa", "x": 1, "schema": 99, "background": "si"});
        let done = s.aggiorna(patch.as_object().unwrap());
        assert_eq!(Value::Object(done), json!({"vetro": 100, "nome_pc": "PC"}));
        assert!(s.imposta_pin(Some("123456")));
        assert!(!fs::read_to_string(p.settings_file()).unwrap().contains("123456"));
        assert!(!s.imposta_pin(Some("12")));
        assert!(s.imposta_pin(None) && s.get("pin_ricezione").is_null());
        assert!(!s.preferito("zz", true, None));
        assert!(s.preferito(&FP1.to_lowercase(), true, Some(("X", "1.2.3.4", 53317))));
        assert!(s.preferiti().contains_key(FP1));
        let bk = s.ripristina().unwrap();
        assert!(fs::read_to_string(bk).unwrap().contains("PC"));
        assert!(s.preferiti().is_empty());
        assert_eq!(s.str("nome_pc"), "Fedora Pixel Hub");
    }
}
