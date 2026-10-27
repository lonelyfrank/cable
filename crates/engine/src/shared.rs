//! Stato condiviso fra le componenti del motore (identità, impostazioni, registro, bus).
use crate::bus::{Bus, Notifier};
use crate::devices::Registry;
use crate::identity::Identity;
use crate::paths::Paths;
use crate::protocol::parse_peer_info;
use crate::settings::Settings;
use rustls::ClientConfig;
use serde_json::{json, Value};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub type ClientTls = (Arc<ClientConfig>, Arc<Mutex<Option<String>>>);

pub struct Shared {
    pub paths: Paths,
    pub settings: Arc<Settings>,
    pub identity: RwLock<Identity>,
    pub registry: Registry,
    pub bus: Bus,
    pub notifier: Arc<Notifier>,
    port: AtomicU16,
}

impl Shared {
    pub fn new(paths: Paths, port: u16) -> Arc<Shared> {
        let settings = Arc::new(Settings::load(&paths));
        let identity = Identity::load(&paths);
        let bus = Bus::new();
        let notifier = Arc::new(Notifier::default());
        notifier.enable(settings.bool("notifiche"));
        let registry = Registry::new(&identity.fingerprint, settings.clone(), bus.clone());
        Arc::new(Shared { paths, settings, identity: RwLock::new(identity), registry, bus, notifier, port: AtomicU16::new(port) })
    }

    pub fn port(&self) -> u16 { self.port.load(Ordering::Relaxed) }
    pub fn set_port(&self, p: u16) { self.port.store(p, Ordering::Relaxed); }
    pub fn fingerprint(&self) -> String { self.identity.read().unwrap().fingerprint.clone() }

    pub fn info(&self, with_port: bool) -> Value {
        let mut d = json!({"alias": self.settings.str("nome_pc"), "version": "2.0", "deviceModel": "Linux",
                           "deviceType": "desktop", "fingerprint": self.fingerprint(), "download": false});
        if with_port { d["port"] = json!(self.port()); d["protocol"] = json!("https"); }
        d
    }

    pub fn client_tls(&self) -> Result<ClientTls, String> { self.identity.read().unwrap().client_config() }

    pub fn on_register(&self, body: &Value, ip: IpAddr) {
        if let Some(p) = parse_peer_info(body) {
            if p.fp != self.fingerprint() { self.registry.upsert(body, &ip.to_string(), "incoming", None); }
        }
    }
}
