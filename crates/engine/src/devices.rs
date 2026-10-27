//! Registro dei dispositivi LocalSend visti in rete e dei preferiti.
use crate::bus::Bus;
use crate::protocol::parse_peer_info;
use crate::settings::Settings;
use crate::util::now_secs;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const NEARBY_SECONDS: f64 = 120.0;
pub const FORGET_SECONDS: f64 = 900.0;
pub const MAX_DEVICES: usize = 256;

#[derive(Clone, Debug)]
pub struct Device {
    pub fp: String, pub alias: String, pub model: String, pub kind: String,
    pub ip: String, pub port: u16, pub seen: f64, pub source: String,
    /// vero solo dopo aver visto un certificato TLS con questa impronta in questa sessione
    pub verified: bool,
}

pub struct Registry {
    own_fp: Mutex<String>,
    settings: Arc<Settings>,
    bus: Bus,
    devices: Mutex<HashMap<String, Device>>,
}

impl Registry {
    pub fn new(own_fp: &str, settings: Arc<Settings>, bus: Bus) -> Registry {
        Registry { own_fp: Mutex::new(own_fp.to_uppercase()), settings, bus, devices: Mutex::new(HashMap::new()) }
    }
    pub fn set_own_fp(&self, fp: &str) { *self.own_fp.lock().unwrap() = fp.to_uppercase(); }

    pub fn upsert(&self, info: &Value, ip: &str, source: &str, port: Option<u16>) -> Option<Device> {
        let mut info = info.clone();
        if let Some(p) = port { info["port"] = json!(p); }
        let p = parse_peer_info(&info)?;
        if p.fp == *self.own_fp.lock().unwrap() { return None; }
        let fav = self.settings.preferiti();
        let d = {
            let mut m = self.devices.lock().unwrap();
            if !m.contains_key(&p.fp) && m.len() >= MAX_DEVICES {
                let mut old: Vec<(String, f64)> = m.values().filter(|d| !fav.contains_key(&d.fp)).map(|d| (d.fp.clone(), d.seen)).collect();
                old.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                for (fp, _) in old.into_iter().take(16) { m.remove(&fp); }
            }
            let e = m.entry(p.fp.clone()).or_insert_with(|| Device {
                fp: p.fp.clone(), alias: String::new(), model: String::new(), kind: String::new(), ip: String::new(),
                port: p.port, seen: 0.0, source: source.into(), verified: false });
            e.alias = p.alias; e.model = p.model; e.kind = p.kind; e.ip = ip.into(); e.port = p.port; e.seen = now_secs();
            e.clone()
        };
        if fav.contains_key(&d.fp) { self.settings.preferito(&d.fp, true, Some((&d.alias, &d.ip, d.port))); }
        self.bus.emit("dispositivi");
        Some(d)
    }

    pub fn get(&self, fp: &str) -> Option<Device> { self.devices.lock().unwrap().get(&fp.to_uppercase()).cloned() }
    pub fn len(&self) -> usize { self.devices.lock().unwrap().len() }
    pub fn is_empty(&self) -> bool { self.len() == 0 }

    pub fn mark_verified(&self, fp: &str) {
        if let Some(d) = self.devices.lock().unwrap().get_mut(&fp.to_uppercase()) { d.verified = true; }
    }
    pub fn remove(&self, fp: &str) { self.devices.lock().unwrap().remove(&fp.to_uppercase()); self.bus.emit("dispositivi"); }
    pub fn clear(&self) { self.devices.lock().unwrap().clear(); self.bus.emit("dispositivi"); }

    pub fn prune(&self) {
        let fav = self.settings.preferiti();
        let now = now_secs();
        self.devices.lock().unwrap().retain(|fp, d| now - d.seen <= FORGET_SECONDS || fav.contains_key(fp));
    }

    /// Elenco per la UI: dispositivi visti + preferiti attualmente non raggiungibili.
    pub fn snapshot(&self) -> Value {
        let now = now_secs();
        let fav = self.settings.preferiti();
        let mut out: Vec<Value> = vec![];
        let mut seen = std::collections::HashSet::new();
        {
            let m = self.devices.lock().unwrap();
            let mut v: Vec<&Device> = m.values().collect();
            v.sort_by(|a, b| b.seen.partial_cmp(&a.seen).unwrap());
            for d in v {
                seen.insert(d.fp.clone());
                out.push(json!({"fp": d.fp, "alias": d.alias, "modello": d.model, "tipo": d.kind, "ip": d.ip, "port": d.port,
                    "vicino": now - d.seen < NEARBY_SECONDS, "visto": d.seen, "preferito": fav.contains_key(&d.fp),
                    "verificato": d.verified, "confermato": fav.get(&d.fp).map_or(false, |f| f["confermato"] == json!(true)),
                    "origine": d.source}));
            }
        }
        for (fp, rec) in &fav {
            if !seen.contains(fp) {
                out.push(json!({"fp": fp, "alias": rec["alias"], "modello": "", "tipo": "mobile", "ip": rec["ip"], "port": rec["port"],
                    "vicino": false, "visto": 0, "preferito": true, "verificato": false,
                    "confermato": rec["confermato"] == json!(true), "origine": "preferito"}));
            }
        }
        out.sort_by(|a, b| {
            let k = |v: &Value| (!v["vicino"].as_bool().unwrap(), !v["preferito"].as_bool().unwrap(), v["alias"].as_str().unwrap().to_lowercase());
            k(a).cmp(&k(b))
        });
        Value::Array(out)
    }
}
