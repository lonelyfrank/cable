//! Registro dei dispositivi LocalSend visti in rete e di quelli ricordati.
//!
//! Quattro proprietà **indipendenti**, che possono coesistere in qualunque combinazione:
//! * `ricordato`: c'è un record persistente in `settings.json` (chiave storica `preferiti`);
//! * `vicino`: annunciato o visto di recente (`NEARBY_SECONDS`), cioè presenza, non raggiungibilità;
//! * `raggiungibile`: l'ultimo tentativo di connessione TLS verso uno dei suoi indirizzi è riuscito (o fallito);
//! * `confermato`: l'utente ha verificato con il codice del wizard che l'impronta è del dispositivo che ha in mano.
//!
//! La stella (`preferito`) è una scorciatoia grafica: non implica nessuna delle quattro e non autorizza nulla.
//!
//! L'identità di un dispositivo è l'impronta SHA-256 del suo certificato. Nome, modello e indirizzo IP sono
//! dichiarati dal dispositivo (o dalla rete) e non provano nulla: servono solo a mostrarlo e a raggiungerlo.
use crate::bus::Bus;
use crate::protocol::parse_peer_info;
use crate::settings::Settings;
use crate::util::now_secs;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub const NEARBY_SECONDS: f64 = 120.0;
pub const FORGET_SECONDS: f64 = 900.0;
pub const MAX_DEVICES: usize = 256;
pub const MAX_ADDRS: usize = 4;
/// Un esito di raggiungibilità più vecchio di così non viene più mostrato come fatto
pub const REACH_VALID_SECONDS: f64 = 300.0;
const MAX_CHANGES: usize = 16;

/// Un indirizzo a cui il dispositivo è stato visto. Un dispositivo ne può avere più d'uno (Wi-Fi + Ethernet,
/// cambio di DHCP, VPN): l'identità è l'impronta, non l'indirizzo.
#[derive(Clone, Debug, PartialEq)]
pub struct Addr {
    pub ip: String,
    pub port: u16,
    /// ultimo annuncio/richiesta ricevuti da questo indirizzo (0 = presenza scaduta)
    pub seen: f64,
    /// ultima volta che una connessione TLS a questo indirizzo ha mostrato l'impronta giusta (0 = mai)
    pub confirmed: f64,
    pub manual: bool,
}

#[derive(Clone, Debug)]
pub struct Device {
    pub fp: String,
    pub alias: String,
    pub model: String,
    pub kind: String,
    /// indirizzo migliore al momento (derivato da `addrs`); tenuto per comodità dei chiamanti
    pub ip: String,
    pub port: u16,
    pub addrs: Vec<Addr>,
    pub seen: f64,
    pub source: String,
    /// vero solo dopo aver visto un certificato TLS con questa impronta in questa sessione
    pub verified: bool,
    /// (esito, quando) dell'ultimo tentativo di connessione
    pub reach: Option<(bool, f64)>,
}

impl Device {
    /// Ordine dei candidati per connettersi: prima gli indirizzi **confermati** di recente, poi i più recenti
    /// per presenza; gli indirizzi con presenza scaduta e mai confermati vanno per ultimi.
    pub fn candidates(&self) -> Vec<Addr> {
        let mut v = self.addrs.clone();
        v.sort_by(|a, b| {
            let score = |x: &Addr| (x.confirmed.max(x.seen), x.confirmed);
            score(b).partial_cmp(&score(a)).unwrap()
        });
        v
    }
    fn refresh_best(&mut self) {
        if let Some(a) = self.candidates().first() {
            self.ip = a.ip.clone();
            self.port = a.port;
        }
        self.seen = self.addrs.iter().map(|a| a.seen).fold(0.0, f64::max);
    }
}

/// Un dispositivo ricordato (con una certa impronta) è stato sostituito, in apparenza, da uno con lo stesso nome
/// ma impronta diversa: reinstallazione dell'app oppure un altro dispositivo. Non si decide al posto dell'utente.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityChange {
    pub old_fp: String,
    pub new_fp: String,
    pub alias: String,
    pub ip: String,
    pub when: f64,
    pub dismissed: bool,
}

pub struct Registry {
    own_fp: Mutex<String>,
    settings: Arc<Settings>,
    bus: Bus,
    devices: Mutex<HashMap<String, Device>>,
    changes: Mutex<Vec<IdentityChange>>,
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

impl Registry {
    pub fn new(own_fp: &str, settings: Arc<Settings>, bus: Bus) -> Registry {
        Registry { own_fp: Mutex::new(own_fp.to_uppercase()), settings, bus, devices: Mutex::new(HashMap::new()), changes: Mutex::new(vec![]) }
    }
    pub fn set_own_fp(&self, fp: &str) {
        *self.own_fp.lock().unwrap() = fp.to_uppercase();
    }

    /// Registra un annuncio o una richiesta di registrazione. `ip` è l'indirizzo da cui è arrivato
    /// (non provato); `source`: "multicast" | "incoming" | "manuale".
    pub fn upsert(&self, info: &Value, ip: &str, source: &str, port: Option<u16>) -> Option<Device> {
        let mut info = info.clone();
        if let Some(p) = port {
            info["port"] = json!(p);
        }
        let p = parse_peer_info(&info)?;
        if p.fp == *self.own_fp.lock().unwrap() {
            return None;
        }
        let fav = self.settings.preferiti();
        let now = now_secs();
        let mut new_change = None;
        let d = {
            let mut m = self.devices.lock().unwrap();
            if !m.contains_key(&p.fp) && m.len() >= MAX_DEVICES {
                let mut old: Vec<(String, f64)> = m.values().filter(|d| !fav.contains_key(&d.fp)).map(|d| (d.fp.clone(), d.seen)).collect();
                old.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                for (fp, _) in old.into_iter().take(16) {
                    m.remove(&fp);
                }
            }
            let is_new = !m.contains_key(&p.fp);
            if is_new && !fav.contains_key(&p.fp) {
                // impronta mai vista né ricordata, ma con il nome di un dispositivo ricordato che oggi non si vede
                for (ofp, rec) in &fav {
                    if !same_name(rec["alias"].as_str().unwrap_or(""), &p.alias) {
                        continue;
                    }
                    let old_near = m.get(ofp).is_some_and(|o| now - o.seen < NEARBY_SECONDS);
                    if !old_near {
                        new_change = Some((ofp.clone(), rec["confermato"] == json!(true)));
                        break;
                    }
                }
            }
            let e = m.entry(p.fp.clone()).or_insert_with(|| Device {
                fp: p.fp.clone(),
                alias: String::new(),
                model: String::new(),
                kind: String::new(),
                ip: String::new(),
                port: p.port,
                addrs: vec![],
                seen: 0.0,
                source: source.into(),
                verified: false,
                reach: None,
            });
            e.alias = p.alias.clone();
            e.model = p.model;
            e.kind = p.kind;
            let manual = source == "manuale";
            match e.addrs.iter_mut().find(|a| a.ip == ip && a.port == p.port) {
                Some(a) => {
                    a.seen = now;
                    a.manual |= manual;
                }
                None => {
                    e.addrs.push(Addr { ip: ip.into(), port: p.port, seen: now, confirmed: 0.0, manual });
                    if e.addrs.len() > MAX_ADDRS {
                        // via il più vecchio non confermato
                        let drop = e
                            .addrs
                            .iter()
                            .enumerate()
                            .min_by(|(_, a), (_, b)| (a.confirmed.max(a.seen)).partial_cmp(&b.confirmed.max(b.seen)).unwrap())
                            .map(|(i, _)| i)
                            .unwrap();
                        e.addrs.remove(drop);
                    }
                }
            }
            if manual {
                e.source = source.into();
            }
            e.refresh_best();
            e.clone()
        };
        if let Some((old_fp, _)) = new_change {
            let mut c = self.changes.lock().unwrap();
            if !c.iter().any(|x| x.old_fp == old_fp && x.new_fp == d.fp) {
                c.push(IdentityChange { old_fp, new_fp: d.fp.clone(), alias: d.alias.clone(), ip: ip.into(), when: now, dismissed: false });
                if c.len() > MAX_CHANGES {
                    c.remove(0);
                }
            }
        }
        self.bus.emit("dispositivi");
        Some(d)
    }

    /// I dispositivi ricordati ma non ancora visti in questa sessione entrano nel registro con la presenza a zero:
    /// si possono riverificare (`probe`) o raggiungere per indirizzo, senza apparire «vicini» finché non rispondono.
    pub fn seed_remembered(&self) {
        let mut m = self.devices.lock().unwrap();
        for (fp, rec) in self.settings.preferiti() {
            let ip = rec["ip"].as_str().unwrap_or("");
            if m.contains_key(&fp) || ip.is_empty() {
                continue;
            }
            let port = rec["port"].as_u64().unwrap_or(53317) as u16;
            let mut d = Device {
                fp: fp.clone(),
                alias: rec["alias"].as_str().unwrap_or("Dispositivo").into(),
                model: String::new(),
                kind: String::new(),
                ip: ip.into(),
                port,
                addrs: vec![Addr { ip: ip.into(), port, seen: 0.0, confirmed: 0.0, manual: false }],
                seen: 0.0,
                source: "ricordato".into(),
                verified: false,
                reach: None,
            };
            d.refresh_best();
            m.insert(fp, d);
        }
    }

    /// Dispositivo visto di recente a questo indirizzo (solo indicativo: gli indirizzi cambiano e non provano l'identità).
    pub fn find_by_ip(&self, ip: &str) -> Option<Device> {
        let now = now_secs();
        self.devices
            .lock()
            .unwrap()
            .values()
            .filter(|d| d.addrs.iter().any(|a| a.ip == ip && now - a.seen < NEARBY_SECONDS))
            .max_by(|a, b| a.seen.partial_cmp(&b.seen).unwrap())
            .cloned()
    }

    pub fn get(&self, fp: &str) -> Option<Device> {
        self.devices.lock().unwrap().get(&fp.to_uppercase()).cloned()
    }
    pub fn len(&self) -> usize {
        self.devices.lock().unwrap().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Una connessione TLS verso `ip` ha mostrato davvero l'impronta di `fp`: l'indirizzo è confermato e,
    /// se il dispositivo è ricordato, nome e indirizzo del record si aggiornano (solo ora, non su un annuncio non provato).
    pub fn mark_verified(&self, fp: &str, ip: Option<&str>) {
        let fp = fp.to_uppercase();
        let now = now_secs();
        let dev = {
            let mut m = self.devices.lock().unwrap();
            let Some(d) = m.get_mut(&fp) else { return };
            d.verified = true;
            d.reach = Some((true, now));
            if let Some(ip) = ip {
                for a in d.addrs.iter_mut().filter(|a| a.ip == ip) {
                    a.confirmed = now;
                }
            }
            d.refresh_best();
            d.clone()
        };
        if self.settings.preferiti().contains_key(&fp) {
            if let Some(a) = ip.and_then(|ip| dev.addrs.iter().find(|a| a.ip == ip)) {
                if self.settings.ricorda(&fp, &dev.alias, &a.ip, a.port) {
                    self.bus.emit("dispositivi");
                }
            }
        }
    }

    /// Ricorda (senza stella né conferma) un dispositivo con cui si è appena scambiato qualcosa: l'ultimo indirizzo
    /// confermato e il nome diventano il suo record, così resta in elenco anche quando non è vicino.
    pub fn remember(&self, fp: &str) {
        let Some(d) = self.get(fp) else { return };
        let Some(a) = d.candidates().into_iter().find(|a| a.confirmed > 0.0) else { return };
        if self.settings.ricorda(&d.fp, &d.alias, &a.ip, a.port) {
            self.bus.emit("dispositivi");
        }
    }

    /// Un tentativo di connessione a questo dispositivo è fallito (nessun indirizzo ha risposto con l'impronta giusta).
    pub fn mark_unreachable(&self, fp: &str) {
        if let Some(d) = self.devices.lock().unwrap().get_mut(&fp.to_uppercase()) {
            d.reach = Some((false, now_secs()));
        }
        self.bus.emit("dispositivi");
    }

    /// Un annuncio da `ip` ha dichiarato questa impronta ma il certificato non corrisponde: l'indirizzo non è di questo dispositivo.
    pub fn drop_addr(&self, fp: &str, ip: &str) {
        let mut m = self.devices.lock().unwrap();
        if let Some(d) = m.get_mut(&fp.to_uppercase()) {
            d.addrs.retain(|a| a.ip != ip);
            if d.addrs.is_empty() {
                m.remove(&fp.to_uppercase());
            } else if let Some(d) = m.get_mut(&fp.to_uppercase()) {
                d.refresh_best();
            }
        }
        drop(m);
        self.bus.emit("dispositivi");
    }

    pub fn remove(&self, fp: &str) {
        self.devices.lock().unwrap().remove(&fp.to_uppercase());
        self.bus.emit("dispositivi");
    }
    pub fn clear(&self) {
        self.devices.lock().unwrap().clear();
        self.changes.lock().unwrap().clear();
        self.bus.emit("dispositivi");
    }

    /// Dopo un cambio di rete o la ripresa dalla sospensione la presenza non è più attendibile: finché il dispositivo
    /// non si annuncia di nuovo non è «vicino». Gli indirizzi inseriti a mano restano (possono essere su reti instradate).
    /// `still_local`, se presente, salva gli indirizzi che stanno ancora in una sottorete locale attuale.
    pub fn expire_presence(&self, still_local: Option<&dyn Fn(&str) -> bool>) {
        let mut m = self.devices.lock().unwrap();
        for d in m.values_mut() {
            for a in d.addrs.iter_mut() {
                if a.manual {
                    continue;
                }
                if still_local.is_some_and(|f| f(&a.ip)) {
                    continue;
                }
                a.seen = 0.0;
                a.confirmed = 0.0;
            }
            if d.reach.is_some_and(|(ok, _)| !ok) || still_local.is_none() {
                d.reach = None;
            }
            d.refresh_best();
        }
        drop(m);
        self.bus.emit("dispositivi");
    }

    pub fn prune(&self) {
        let fav = self.settings.preferiti();
        let now = now_secs();
        self.devices
            .lock()
            .unwrap()
            .retain(|fp, d| now - d.seen <= FORGET_SECONDS || fav.contains_key(fp) || d.addrs.iter().any(|a| a.manual && now - a.seen <= FORGET_SECONDS));
    }

    // ------------------------------------------------------------------ cambi d'identità
    pub fn identity_changes(&self) -> Vec<IdentityChange> {
        let fav = self.settings.preferiti();
        self.changes.lock().unwrap().iter().filter(|c| !c.dismissed && fav.contains_key(&c.old_fp)).cloned().collect()
    }
    pub fn dismiss_change(&self, new_fp: &str) -> bool {
        let new_fp = new_fp.to_uppercase();
        let mut c = self.changes.lock().unwrap();
        let mut any = false;
        for x in c.iter_mut().filter(|x| x.new_fp == new_fp) {
            x.dismissed = true;
            any = true;
        }
        drop(c);
        self.bus.emit("dispositivi");
        any
    }
    /// Esito di `sostituisci_ricordato` per la UI: il vecchio record sparisce, il nuovo non eredita la conferma.
    pub fn replace_remembered(&self, old_fp: &str, new_fp: &str) -> bool {
        let Some(d) = self.get(new_fp) else { return false };
        let ok = self.settings.sostituisci_ricordato(old_fp, &d.fp, &d.alias, &d.ip, d.port);
        if ok {
            self.changes.lock().unwrap().retain(|c| c.old_fp != old_fp.to_uppercase());
            self.bus.emit("dispositivi");
        }
        ok
    }

    /// Elenco per la UI: dispositivi visti + ricordati non visti.
    pub fn snapshot(&self) -> Value {
        let now = now_secs();
        let fav = self.settings.preferiti();
        let mut out: Vec<Value> = vec![];
        let mut seen = HashSet::new();
        let reach = |r: Option<(bool, f64)>| match r {
            Some((ok, t)) if now - t < REACH_VALID_SECONDS => json!(ok),
            _ => Value::Null,
        };
        {
            let m = self.devices.lock().unwrap();
            let mut v: Vec<&Device> = m.values().collect();
            v.sort_by(|a, b| b.seen.partial_cmp(&a.seen).unwrap());
            for d in v {
                seen.insert(d.fp.clone());
                let rec = fav.get(&d.fp);
                out.push(json!({"fp": d.fp, "alias": d.alias, "modello": d.model, "tipo": d.kind, "ip": d.ip, "port": d.port,
                    "indirizzi": d.addrs.len(),
                    "vicino": now - d.seen < NEARBY_SECONDS, "visto": d.seen,
                    "ricordato": rec.is_some(), "preferito": rec.is_some_and(|r| r["preferito"] != json!(false)),
                    "raggiungibile": reach(d.reach), "verificato": d.verified,
                    "confermato": rec.is_some_and(|f| f["confermato"] == json!(true)),
                    "origine": d.source}));
            }
        }
        for (fp, rec) in &fav {
            if !seen.contains(fp) {
                out.push(json!({"fp": fp, "alias": rec["alias"], "modello": "", "tipo": "", "ip": rec["ip"], "port": rec["port"],
                    "indirizzi": 0, "vicino": false, "visto": rec["visto"], "ricordato": true, "preferito": rec["preferito"] != json!(false),
                    "raggiungibile": Value::Null, "verificato": false,
                    "confermato": rec["confermato"] == json!(true), "origine": "ricordato"}));
            }
        }
        out.sort_by(|a, b| {
            let k = |v: &Value| (!v["vicino"].as_bool().unwrap(), !v["preferito"].as_bool().unwrap(), v["alias"].as_str().unwrap().to_lowercase());
            k(a).cmp(&k(b))
        });
        label_duplicates(&mut out);
        Value::Array(out)
    }
}

/// Aggiunge `etichetta`: il nome, oppure — se più dispositivi lo condividono — il nome con le prime cifre
/// dell'impronta (quante bastano a distinguerli). Il nome da solo non identifica nessuno.
pub fn label_duplicates(list: &mut [Value]) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, d) in list.iter().enumerate() {
        groups.entry(d["alias"].as_str().unwrap_or("").trim().to_lowercase()).or_default().push(i);
    }
    for (i, d) in list.iter_mut().enumerate() {
        let _ = i;
        d["etichetta"] = d["alias"].clone();
    }
    for idx in groups.values().filter(|g| g.len() > 1) {
        let fps: Vec<String> = idx.iter().map(|i| list[*i]["fp"].as_str().unwrap_or("").to_string()).collect();
        let mut n = 4;
        while n < 12 && fps.iter().map(|f| f.chars().take(n).collect::<String>()).collect::<HashSet<_>>().len() < fps.len() {
            n += 2;
        }
        for (k, i) in idx.iter().enumerate() {
            let alias = list[*i]["alias"].as_str().unwrap_or("").to_string();
            list[*i]["etichetta"] = json!(format!("{alias} · {}", fps[k].chars().take(n).collect::<String>()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    const A: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const B: &str = "BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    const C: &str = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";

    fn reg() -> (tempfile::TempDir, Arc<Settings>, Registry) {
        let t = tempfile::tempdir().unwrap();
        let s = Arc::new(Settings::load(&Paths::under(t.path())));
        let r = Registry::new(&"F".repeat(64), s.clone(), Bus::new());
        (t, s, r)
    }
    fn info(fp: &str, alias: &str) -> Value {
        json!({"alias": alias, "fingerprint": fp, "port": 53317, "protocol": "https", "deviceModel": "Qualunque", "deviceType": "mobile"})
    }

    #[test]
    fn unknown_model_and_type_stay_unknown() {
        let (_t, _s, r) = reg();
        let d = r.upsert(&json!({"alias": "Tel", "fingerprint": A}), "10.0.0.2", "multicast", None).unwrap();
        assert_eq!((d.model.as_str(), d.kind.as_str()), ("", ""));
        let v = r.snapshot();
        assert_eq!(v[0]["tipo"], "");
    }

    #[test]
    fn ip_change_keeps_one_device_and_prefers_confirmed_address() {
        let (_t, _s, r) = reg();
        r.upsert(&info(A, "Tel"), "192.168.1.5", "multicast", None);
        r.mark_verified(A, Some("192.168.1.5"));
        r.upsert(&info(A, "Tel"), "192.168.1.77", "multicast", None); // nuovo DHCP: stesso dispositivo
        let d = r.get(A).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(d.addrs.len(), 2);
        // un annuncio non provato non scalza un indirizzo confermato di recente solo perché più recente
        assert_eq!(d.candidates()[0].ip, "192.168.1.77", "la presenza più recente vince finché non c'è conferma più recente");
        r.mark_verified(A, Some("192.168.1.77"));
        assert_eq!(r.get(A).unwrap().ip, "192.168.1.77");
        r.drop_addr(A, "192.168.1.5");
        assert_eq!(r.get(A).unwrap().addrs.len(), 1);
    }

    #[test]
    fn address_list_is_bounded() {
        let (_t, _s, r) = reg();
        for i in 0..10 {
            r.upsert(&info(A, "Tel"), &format!("10.0.0.{i}"), "multicast", None);
        }
        assert_eq!(r.get(A).unwrap().addrs.len(), MAX_ADDRS);
    }

    #[test]
    fn same_name_devices_get_distinct_labels_and_identities() {
        let (_t, _s, r) = reg();
        r.upsert(&info(A, "iPhone"), "10.0.0.2", "multicast", None);
        r.upsert(&info(B, "iPhone"), "10.0.0.3", "multicast", None);
        r.upsert(&info(C, "Altro"), "10.0.0.4", "multicast", None);
        assert_eq!(r.len(), 3);
        let v = r.snapshot();
        let l: Vec<&str> = v.as_array().unwrap().iter().map(|d| d["etichetta"].as_str().unwrap()).collect();
        let iph: Vec<&&str> = l.iter().filter(|e| e.starts_with("iPhone")).collect();
        assert_eq!(iph.len(), 2);
        assert_ne!(iph[0], iph[1]);
        assert!(l.contains(&"Altro"));
        // nessun cambio d'identità segnalato: i due sono presenti insieme (omonimi), non uno al posto dell'altro
        assert!(r.identity_changes().is_empty());
    }

    #[test]
    fn label_grows_when_prefixes_collide() {
        let mut v = vec![json!({"fp": "ABCD0000", "alias": "X"}), json!({"fp": "ABCD1111", "alias": "x"})];
        label_duplicates(&mut v);
        assert_ne!(v[0]["etichetta"], v[1]["etichetta"]);
    }

    #[test]
    fn remembered_stale_device_with_new_fingerprint_raises_a_change_without_touching_trust() {
        let (_t, s, r) = reg();
        s.conferma_dispositivo(A, "Telefono di Anna", "10.0.0.2", 53317);
        r.upsert(&info(B, "telefono di anna "), "10.0.0.9", "multicast", None);
        let c = r.identity_changes();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].old_fp.as_str(), c[0].new_fp.as_str()), (A, B));
        // la fiducia precedente non è stata toccata né trasferita
        let fav = s.preferiti();
        assert_eq!(fav[A]["confermato"], json!(true));
        assert!(!fav.contains_key(B));
        let snap = r.snapshot();
        let nuovo = snap.as_array().unwrap().iter().find(|d| d["fp"] == B).unwrap().clone();
        assert_eq!((nuovo["confermato"].clone(), nuovo["ricordato"].clone()), (json!(false), json!(false)));
        // ripetere l'annuncio non duplica l'avviso
        r.upsert(&info(B, "Telefono di Anna"), "10.0.0.9", "multicast", None);
        assert_eq!(r.identity_changes().len(), 1);
        // sostituzione solo su azione esplicita: il nuovo NON eredita la conferma
        assert!(r.replace_remembered(A, B));
        let fav = s.preferiti();
        assert!(!fav.contains_key(A));
        assert_eq!(fav[B]["confermato"], json!(false));
        assert!(r.identity_changes().is_empty());
    }

    #[test]
    fn dismissing_a_change_hides_it() {
        let (_t, s, r) = reg();
        s.ricorda(A, "Tel", "10.0.0.2", 53317);
        r.upsert(&info(B, "Tel"), "10.0.0.3", "multicast", None);
        assert_eq!(r.identity_changes().len(), 1);
        assert!(r.dismiss_change(B));
        assert!(r.identity_changes().is_empty());
    }

    #[test]
    fn four_properties_are_independent() {
        let (_t, s, r) = reg();
        // ricordato ma non vicino, non confermato, non raggiunto
        s.ricorda(A, "Ricordato", "10.0.0.2", 53317);
        // vicino ma non ricordato
        r.upsert(&info(B, "Vicino"), "10.0.0.3", "multicast", None);
        // vicino + raggiunto + ricordato + confermato
        r.upsert(&info(C, "Tutto"), "10.0.0.4", "multicast", None);
        s.conferma_dispositivo(C, "Tutto", "10.0.0.4", 53317);
        r.mark_verified(C, Some("10.0.0.4"));
        let v = r.snapshot();
        let get = |fp: &str| v.as_array().unwrap().iter().find(|d| d["fp"] == fp).unwrap().clone();
        let (a, b, c) = (get(A), get(B), get(C));
        assert_eq!(
            (a["ricordato"].clone(), a["vicino"].clone(), a["raggiungibile"].clone(), a["confermato"].clone(), a["preferito"].clone()),
            (json!(true), json!(false), Value::Null, json!(false), json!(false))
        );
        assert_eq!(
            (b["ricordato"].clone(), b["vicino"].clone(), b["raggiungibile"].clone(), b["confermato"].clone()),
            (json!(false), json!(true), Value::Null, json!(false))
        );
        assert_eq!(
            (c["ricordato"].clone(), c["vicino"].clone(), c["raggiungibile"].clone(), c["confermato"].clone()),
            (json!(true), json!(true), json!(true), json!(true))
        );
        r.mark_unreachable(B);
        let b2 = r.snapshot().as_array().unwrap().iter().find(|d| d["fp"] == B).unwrap().clone();
        assert_eq!((b2["vicino"].clone(), b2["raggiungibile"].clone()), (json!(true), json!(false)), "visto di recente ma non raggiungibile");
    }

    #[test]
    fn presence_expires_after_network_change_except_manual_and_local() {
        let (_t, _s, r) = reg();
        r.upsert(&info(A, "Wifi vecchio"), "192.168.1.5", "multicast", None);
        r.upsert(&info(B, "Manuale"), "10.9.9.9", "manuale", None);
        r.upsert(&info(C, "Ancora locale"), "10.0.0.4", "multicast", None);
        r.expire_presence(Some(&|ip: &str| ip.starts_with("10.0.0.")));
        let v = r.snapshot();
        let near = |fp: &str| v.as_array().unwrap().iter().find(|d| d["fp"] == fp).unwrap()["vicino"].clone();
        assert_eq!((near(A), near(B), near(C)), (json!(false), json!(true), json!(true)));
        // dopo la ripresa dalla sospensione nessuno è attendibile finché non si annuncia di nuovo
        r.expire_presence(None);
        let v = r.snapshot();
        assert_eq!(v.as_array().unwrap().iter().filter(|d| d["vicino"] == json!(true)).count(), 1, "resta solo l'indirizzo manuale");
        r.upsert(&info(A, "Wifi vecchio"), "192.168.1.5", "multicast", None);
        assert!(r.get(A).unwrap().seen > 0.0);
    }

    #[test]
    fn star_is_only_a_shortcut() {
        let (_t, s, r) = reg();
        r.upsert(&info(A, "Tel"), "10.0.0.2", "multicast", None);
        assert!(s.preferito(A, true, Some(("Tel", "10.0.0.2", 53317))));
        let d = r.snapshot()[0].clone();
        assert_eq!((d["preferito"].clone(), d["confermato"].clone(), d["verificato"].clone()), (json!(true), json!(false), json!(false)));
        assert!(s.preferito(A, false, None));
        let d = r.snapshot()[0].clone();
        assert_eq!((d["preferito"].clone(), d["ricordato"].clone()), (json!(false), json!(true)), "senza stella resta ricordato");
    }

    #[test]
    fn unverified_announce_does_not_rewrite_remembered_record() {
        let (_t, s, r) = reg();
        s.ricorda(A, "Tel", "10.0.0.2", 53317);
        r.upsert(&info(A, "Rinominato da un annuncio"), "10.0.0.66", "multicast", None);
        assert_eq!(s.preferiti()[A]["ip"], json!("10.0.0.2"));
        assert_eq!(s.preferiti()[A]["alias"], json!("Tel"));
        r.mark_verified(A, Some("10.0.0.66"));
        assert_eq!(s.preferiti()[A]["ip"], json!("10.0.0.66"), "dopo una connessione TLS verificata sì");
    }

    #[test]
    fn remembered_devices_are_capped() {
        let (_t, s, _r) = reg();
        for i in 0..(crate::settings::MAX_RICORDATI + 8) {
            s.ricorda(&format!("{:064X}", i + 1), "x", "1.1.1.1", 53317);
        }
        assert_eq!(s.preferiti().len(), crate::settings::MAX_RICORDATI);
        s.preferito(&format!("{:064X}", 1), true, None); // con la stella non viene scartato
        for i in 100..140 {
            s.ricorda(&format!("{:064X}", i), "x", "1.1.1.1", 53317);
        }
        assert!(s.preferiti().contains_key(&format!("{:064X}", 1)));
    }
}
