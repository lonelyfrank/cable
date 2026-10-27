//! Bus di eventi: il motore segnala "qualcosa è cambiato", la UI ricostruisce lo stato.
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct Bus {
    tx: broadcast::Sender<&'static str>,
}

impl Default for Bus { fn default() -> Self { Self::new() } }

impl Bus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Bus { tx }
    }
    pub fn emit(&self, topic: &'static str) { let _ = self.tx.send(topic); }
    pub fn subscribe(&self) -> broadcast::Receiver<&'static str> { self.tx.subscribe() }
}

pub fn debug_enabled() -> bool { std::env::var_os("HUB_DEBUG").is_some() }

#[macro_export]
macro_rules! dlog {
    ($($a:tt)*) => { if $crate::bus::debug_enabled() { eprintln!("[hub] {}", format!($($a)*)); } };
}

/// Canale verso la shell per le notifiche native (richiesta, fine ricezione, messaggio, esito invio).
pub type NotifyFn = std::sync::Arc<dyn Fn(&str, serde_json::Value) + Send + Sync>;

#[derive(Default)]
pub struct Notifier {
    f: std::sync::RwLock<Option<NotifyFn>>,
    enabled: std::sync::atomic::AtomicBool,
}

impl Notifier {
    pub fn set(&self, f: NotifyFn) { *self.f.write().unwrap() = Some(f); }
    pub fn enable(&self, on: bool) { self.enabled.store(on, std::sync::atomic::Ordering::Relaxed); }
    pub fn send(&self, kind: &str, v: serde_json::Value) {
        if !self.enabled.load(std::sync::atomic::Ordering::Relaxed) { return; }
        if let Some(f) = self.f.read().unwrap().as_ref() { f(kind, v); }
    }
}
