//! Funzioni Android opzionali (scrcpy e importazione da USB/ADB).
//! Indipendenti da LocalSend: non influiscono su scoperta, identità o autorizzazioni dei trasferimenti Wi-Fi.
use crate::bus::Bus;
use crate::desktop::has_command;
use crate::settings::Settings;
use crate::util::{commit_unique, now_secs, random_hex};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Notify;

async fn run(argv: &[&str], t: Duration) -> (i32, String) {
    let mut c = Command::new(argv[0]);
    c.args(&argv[1..]).stdin(Stdio::null()).kill_on_drop(true);
    match tokio::time::timeout(t, c.output()).await {
        Ok(Ok(o)) => (o.status.code().unwrap_or(1), format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string()),
        Ok(Err(e)) => (1, e.to_string()),
        Err(_) => (1, "timeout".into()),
    }
}

struct AState {
    stato: Value, serial: Option<String>, imp: Value, scrcpy: Option<Child>, connesso: bool,
    log: VecDeque<String>, dal: f64,
}

pub struct Android {
    settings: Arc<Settings>,
    bus: Bus,
    st: Mutex<AState>,
    ui: AtomicBool,
    wake: Notify,
    stop: AtomicBool,
}

impl Android {
    pub fn new(settings: Arc<Settings>, bus: Bus) -> Arc<Android> {
        Arc::new(Android { settings, bus, ui: AtomicBool::new(false), wake: Notify::new(), stop: AtomicBool::new(false),
            st: Mutex::new(AState { stato: json!({"stato": "assente", "info": {}, "errore": ""}), serial: None,
                imp: json!({"attivo": false, "fatti": 0, "tot": 0, "msg": ""}), scrcpy: None, connesso: false, log: VecDeque::new(), dal: 0.0 }) })
    }

    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        tokio::spawn(async move {
            while !me.stop.load(Ordering::Relaxed) {
                if me.ui.load(Ordering::Relaxed) && !me.st.lock().unwrap().imp["attivo"].as_bool().unwrap_or(false) {
                    let old = me.st.lock().unwrap().stato.clone();
                    me.controlla().await;
                    if old != me.st.lock().unwrap().stato { me.bus.emit("android"); }
                }
                let _ = tokio::time::timeout(Duration::from_secs(5), me.wake.notified()).await;
            }
        });
    }

    pub fn stop(&self) { self.stop.store(true, Ordering::Relaxed); self.wake.notify_one(); self.ferma_schermo(); }

    /// Il monitor ADB gira solo se la finestra è visibile.
    pub fn ui_attiva(&self, on: bool) { self.ui.store(on, Ordering::Relaxed); if on { self.wake.notify_one(); } }

    fn set_stato(&self, serial: Option<String>, v: Value) { let mut s = self.st.lock().unwrap(); s.serial = serial; s.stato = v; }

    pub async fn controlla(&self) {
        if !has_command("adb") { return self.set_stato(None, json!({"stato": "mancante", "info": {}, "errore": ""})); }
        let (rc, out) = run(&["adb", "devices"], Duration::from_secs(10)).await;
        let devs: Vec<Vec<&str>> = out.lines().filter(|l| !l.starts_with("List ") && !l.starts_with('*') && !l.starts_with("adb:"))
            .map(|l| l.split_whitespace().collect::<Vec<_>>()).filter(|d| d.len() >= 2 && ["device", "unauthorized", "offline", "no"].contains(&d[1])).collect();
        if rc != 0 || devs.is_empty() { return self.set_stato(None, json!({"stato": "assente", "info": {}, "errore": if rc != 0 { out.clone() } else { String::new() }})); }
        if devs.len() != 1 {
            return self.set_stato(None, json!({"stato": "errore", "info": {}, "errore": "Sono collegati più dispositivi ADB: lascia collegato solo quello che vuoi usare."}));
        }
        let (serial, status) = (devs[0][0].to_string(), devs[0][1]);
        if status != "device" {
            let (stato, err) = if status == "unauthorized" { ("fiducia", "Autorizza Debug USB sul telefono.".to_string()) }
                               else { ("errore", format!("ADB: {status}. Ricollega il cavo e controlla i permessi USB.")) };
            return self.set_stato(None, json!({"stato": stato, "info": {}, "errore": err}));
        }
        let sh = |args: &'static [&'static str]| { let serial = serial.clone(); async move {
            let mut a = vec!["adb", "-s", &serial, "shell"]; a.extend_from_slice(args);
            let (rc, v) = run(&a, Duration::from_secs(10)).await; if rc == 0 { v } else { String::new() } } };
        let model = sh(&["getprop", "ro.product.model"]).await;
        let mut info = json!({"DeviceName": if model.is_empty() { "Android".to_string() } else { model.clone() }, "ProductType": model,
                              "ProductVersion": sh(&["getprop", "ro.build.version.release"]).await});
        let battery: std::collections::HashMap<String, String> = sh(&["dumpsys", "battery"]).await.lines()
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))).collect();
        info["BatteryCurrentCapacity"] = json!(battery.get("level").cloned().unwrap_or_default());
        info["BatteryIsCharging"] = json!(matches!(battery.get("status").map(String::as_str), Some("2" | "5")).to_string());
        if let Some(l) = sh(&["df", "-k", "/sdcard"]).await.lines().last() {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() > 3 {
                if let (Ok(t), Ok(a)) = (f[1].parse::<u64>(), f[3].parse::<u64>()) {
                    info["TotalDiskCapacity"] = json!((t * 1024).to_string());
                    info["AmountDataAvailable"] = json!((a * 1024).to_string());
                }
            }
        }
        self.set_stato(Some(serial), json!({"stato": "collegato", "info": info, "errore": ""}));
    }

    pub async fn riprova(&self) -> bool {
        self.controlla().await;
        self.bus.emit("android");
        self.st.lock().unwrap().stato["stato"] == "collegato"
    }

    // ------------------------------------------------------------------ importazione DCIM
    pub fn importa(self: &Arc<Self>) -> bool {
        let serial = {
            let mut s = self.st.lock().unwrap();
            if s.imp["attivo"] == json!(true) || s.stato["stato"] != "collegato" || s.serial.is_none() { return false; }
            s.imp = json!({"attivo": true, "fatti": 0, "tot": 0, "msg": "Leggo DCIM sul telefono…"});
            s.serial.clone().unwrap()
        };
        let me = self.clone();
        tokio::spawn(async move { me.importa_task(serial).await });
        true
    }

    fn imp_set(&self, k: &str, v: Value) { self.st.lock().unwrap().imp[k] = v; self.bus.emit("android"); }

    async fn importa_task(&self, serial: String) {
        let res = self.importa_inner(&serial).await;
        let msg = match res { Ok(m) => m, Err(e) => format!("Importazione interrotta: {}", e.chars().take(300).collect::<String>()) };
        { let mut s = self.st.lock().unwrap(); s.imp["attivo"] = json!(false); s.imp["msg"] = json!(msg); }
        self.bus.emit("android");
    }

    async fn importa_inner(&self, serial: &str) -> Result<String, String> {
        let dest = PathBuf::from(self.settings.str("cartella_foto"));
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let dest = std::fs::canonicalize(&dest).map_err(|e| e.to_string())?;
        let (rc, listing) = run(&["adb", "-s", serial, "shell", "find /sdcard/DCIM -type f -print0"], Duration::from_secs(60)).await;
        if rc != 0 { return Err(listing); }
        let paths: Vec<&str> = listing.split('\0').filter(|s| s.starts_with("/sdcard/DCIM/")).collect();
        self.imp_set("tot", json!(paths.len()));
        self.imp_set("msg", json!("Controllo e copio foto e video…"));
        let (mut copied, mut skipped) = (0, 0);
        for remote in &paths {
            let rel = Path::new(remote).strip_prefix("/sdcard/DCIM").map_err(|e| e.to_string())?;
            if rel.components().any(|c| matches!(c, std::path::Component::ParentDir)) { return Err("Percorso DCIM non valido".into()); }
            let q = dest.join(rel);
            let quoted = format!("stat -c %s '{}'", remote.replace('\'', "'\\''"));
            let (rc, size) = run(&["adb", "-s", serial, "shell", &quoted], Duration::from_secs(15)).await;
            let size: u64 = size.trim().parse().map_err(|_| format!("Impossibile leggere {remote}"))?;
            if rc != 0 { return Err(format!("Impossibile leggere {remote}")); }
            if q.metadata().map(|m| m.len() == size).unwrap_or(false) { skipped += 1; }
            else {
                std::fs::create_dir_all(q.parent().unwrap()).map_err(|e| e.to_string())?;
                let tmp = q.with_file_name(format!(".{}.{}.part", q.file_name().unwrap().to_string_lossy(), random_hex(4)));
                let (rc, out) = run(&["adb", "-s", serial, "pull", remote, &tmp.to_string_lossy()], Duration::from_secs(3600)).await;
                let ok = rc == 0 && tmp.metadata().map(|m| m.len() == size).unwrap_or(false);
                let r = if ok { commit_unique(&tmp, &q).map(|_| ()).map_err(|e| e.to_string()) } else { Err(if out.is_empty() { "Copia incompleta".into() } else { out }) };
                let _ = std::fs::remove_file(&tmp);
                r?;
                copied += 1;
            }
            let fatti = self.st.lock().unwrap().imp["fatti"].as_u64().unwrap_or(0) + 1;
            self.imp_set("fatti", json!(fatti));
        }
        Ok(if paths.is_empty() { "DCIM vuota: si importano solo i file locali, non quelli presenti solo su Google Foto.".into() }
           else { format!("Importati {copied}; già presenti {skipped}.") })
    }

    pub fn apri_file_manager(&self) -> bool { crate::desktop::apri(Path::new("computer:///")) }

    // ------------------------------------------------------------------ scrcpy
    pub async fn avvia_schermo(self: &Arc<Self>) -> bool {
        if self.attivo() { return true; }
        if !has_command("scrcpy") { return false; }
        self.controlla().await;
        let serial = {
            let s = self.st.lock().unwrap();
            if s.stato["stato"] != "collegato" { None } else { s.serial.clone() }
        };
        let Some(serial) = serial else {
            let e = self.st.lock().unwrap().stato["errore"].as_str().unwrap_or("").to_string();
            self.log_line(if e.is_empty() { "Collega un telefono e autorizza il debug USB.".into() } else { e });
            return false;
        };
        let mut argv = vec!["--serial".to_string(), serial, "--max-size=1920".into(), "--max-fps=60".into()];
        if self.settings.bool("schermo_intero") { argv.push("--fullscreen".into()); }
        let mut child = match Command::new("scrcpy").args(&argv).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn() {
            Ok(c) => c,
            Err(e) => { self.log_line(e.to_string()); return false; }
        };
        self.log_line(format!("$ scrcpy {}", argv.join(" ")));
        let (out, err) = (child.stdout.take(), child.stderr.take());
        { let mut s = self.st.lock().unwrap(); s.scrcpy = Some(child); s.connesso = false; s.dal = now_secs(); }
        for r in [out.map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Send + Unpin>), err.map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)].into_iter().flatten() {
            let me = self.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let low = l.to_lowercase();
                    if low.contains("texture:") || low.contains("renderer:") { me.st.lock().unwrap().connesso = true; }
                    else if ["connection closed", "teardown", "client disconnected"].iter().any(|k| low.contains(k)) { me.st.lock().unwrap().connesso = false; }
                    me.log_line(l);
                }
            });
        }
        self.bus.emit("android");
        true
    }

    fn log_line(&self, l: String) {
        let mut s = self.st.lock().unwrap();
        s.log.push_back(l);
        while s.log.len() > 120 { s.log.pop_front(); }
        drop(s);
        self.bus.emit("android");
    }

    fn attivo(&self) -> bool { self.st.lock().unwrap().scrcpy.as_mut().map_or(false, |c| c.try_wait().ok().flatten().is_none()) }

    pub fn ferma_schermo(&self) {
        let mut s = self.st.lock().unwrap();
        if let Some(c) = s.scrcpy.as_mut() { let _ = c.start_kill(); }
        s.scrcpy = None; s.connesso = false;
        drop(s);
        self.bus.emit("android");
    }

    pub fn snapshot(&self) -> (Value, Value) {
        let attivo = self.attivo();
        let s = self.st.lock().unwrap();
        let mut adb = s.stato.clone();
        adb["importa"] = s.imp.clone();
        adb["destinazione"] = json!(self.settings.str("cartella_foto"));
        let log: Vec<&String> = s.log.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        (adb, json!({"installato": has_command("scrcpy"), "attivo": attivo, "connesso": attivo && s.connesso,
                     "dal": if attivo { s.dal } else { 0.0 }, "log": log}))
    }
}
