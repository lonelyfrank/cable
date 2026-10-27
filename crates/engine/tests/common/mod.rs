#![allow(dead_code)]
use pixelhub_engine::paths::Paths;
use pixelhub_engine::Engine;
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

pub async fn wait_for<T, F: FnMut() -> Fut, Fut: Future<Output = Option<T>>>(mut f: F, secs: f64) -> T {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    loop {
        if let Some(v) = f().await { return v; }
        assert!(Instant::now() < end, "timeout in attesa della condizione");
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

pub struct Rig { pub dir: tempfile::TempDir, pub engines: Vec<Arc<Engine>> }

impl Rig {
    pub fn new() -> Rig { Rig { dir: tempfile::tempdir().unwrap(), engines: vec![] } }
    pub fn path(&self, n: &str) -> PathBuf { self.dir.path().join(n) }

    pub async fn engine(&mut self, name: &str) -> Arc<Engine> { self.engine_with_limits(name, Default::default()).await }

    pub async fn engine_with_limits(&mut self, name: &str, limits: pixelhub_engine::protocol::Limits) -> Arc<Engine> {
        let e = Engine::with_limits(Paths::under(&self.path(name)), free_port(), false, limits);
        e.sh.settings.aggiorna(serde_json::json!({"nome_pc": name}).as_object().unwrap());
        assert!(e.start().await, "server non avviato");
        self.engines.push(e.clone());
        e
    }

    pub async fn stop_all(&self) { for e in &self.engines { e.stop().await; } }
}

/// a scopre b tramite IP manuale (b conosce a grazie alla register).
pub async fn pair(a: &Arc<Engine>, b: &Arc<Engine>) {
    assert_eq!(a.cerca(Some("127.0.0.1"), Some(b.port())).await, 1);
}

pub fn inbox(e: &Engine) -> PathBuf { PathBuf::from(e.sh.settings.str("cartella")) }

pub fn listing(p: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(p).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

pub async fn accept_when_pending(e: &Engine, ids: Option<Vec<String>>) -> Value {
    let r = wait_for(|| async { e.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa").cloned() }, 10.0).await;
    assert!(e.decidi(r["id"].as_str().unwrap(), ids));
    r
}

pub fn job(e: &Engine, jid: &str) -> Value {
    e.sender.snapshot().as_array().unwrap().iter().find(|j| j["id"] == jid).cloned().unwrap_or(Value::Null)
}

pub async fn wait_job(e: &Engine, jid: &str, secs: f64) -> Value {
    wait_for(|| async { let j = job(e, jid); (j != Value::Null && ["completato", "parziale", "rifiutato", "annullato", "scaduto", "interrotto", "errore"].contains(&j["stato"].as_str().unwrap())).then_some(j) }, secs).await
}

pub fn sparse(path: &Path, size: u64) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::File::create(path).unwrap().set_len(size).unwrap();
}

pub fn make(rig: &Rig, name: &str, data: &[u8]) -> PathBuf {
    let p = rig.path("src").join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, data).unwrap();
    p
}

pub fn rss_hwm_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("VmHWM")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap()
}

/// I test con file enormi non girano insieme: la memoria di picco (VmHWM) è del processo, non del singolo test.
pub static BIG: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
