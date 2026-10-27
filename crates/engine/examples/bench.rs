//! Misure sul loopback fra due motori nello stesso processo (NON sono misure su rete reale né su telefoni):
//! latenza di avvio dell'invio, velocità su un file grande, molti file piccoli, memoria di picco.
//! Uso: cargo run --release -p pixelhub-engine --example bench
use pixelhub_engine::paths::Paths;
use pixelhub_engine::Engine;
use serde_json::Value;
use std::time::{Duration, Instant};

fn rss_kb(key: &str) -> u64 {
    std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with(key)).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse().ok()).unwrap_or(0)
}

async fn until<F: FnMut() -> Option<T>, T>(mut f: F, secs: u64) -> T {
    let end = Instant::now() + Duration::from_secs(secs);
    loop { if let Some(v) = f() { return v; } assert!(Instant::now() < end, "timeout"); tokio::time::sleep(Duration::from_millis(2)).await; }
}

async fn send(a: &std::sync::Arc<Engine>, b: &std::sync::Arc<Engine>, paths: Vec<String>) -> (Duration, Duration) {
    let t0 = Instant::now();
    let jid = a.invia(&b.sh.fingerprint(), &paths, None).unwrap();
    let req = until(|| b.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa").cloned(), 30).await;
    let ask = t0.elapsed();                                    // dall'invio alla richiesta visibile sul destinatario
    b.decidi(req["id"].as_str().unwrap(), None);
    until(|| a.sender.snapshot().as_array().unwrap().iter().find(|j| j["id"] == Value::from(jid.clone()) && ["completato", "errore", "parziale"].contains(&j["stato"].as_str().unwrap())).cloned(), 600).await;
    (ask, t0.elapsed())
}

#[tokio::main]
async fn main() {
    let dir = tempfile::tempdir().unwrap();
    let mk = |n: &str, port: u16| Engine::new(Paths::under(&dir.path().join(n)), port, false);
    let (a, b) = (mk("a", 45101), mk("b", 45102));
    assert!(a.start().await && b.start().await);
    assert_eq!(a.cerca(Some("127.0.0.1"), Some(45102)).await, 1);
    let base = rss_kb("VmRSS:");

    let one = dir.path().join("uno.txt"); std::fs::write(&one, b"x").unwrap();
    let mut asks = vec![];
    for _ in 0..5 { asks.push(send(&a, &b, vec![one.to_string_lossy().into()]).await.0); }
    asks.sort();
    println!("latenza avvio (invio → richiesta visibile sul destinatario), 1 file piccolo: mediana {:?}, min {:?}, max {:?}", asks[2], asks[0], asks[4]);

    let big = dir.path().join("grande.bin");
    { use std::io::Write; let mut f = std::fs::File::create(&big).unwrap(); let chunk = vec![0xA5u8; 1 << 20]; for _ in 0..1024 { f.write_all(&chunk).unwrap(); } }
    let (_, total) = send(&a, &b, vec![big.to_string_lossy().into()]).await;
    println!("file grande 1 GiB: {:?} → {:.0} MiB/s (loopback, TLS, scrittura su disco)", total, 1024.0 / total.as_secs_f64());

    let many = dir.path().join("molti"); std::fs::create_dir_all(&many).unwrap();
    for i in 0..2000 { std::fs::write(many.join(format!("f{i:04}.dat")), vec![i as u8; 4096]).unwrap(); }
    let (ask, total) = send(&a, &b, vec![many.to_string_lossy().into()]).await;
    println!("2000 file da 4 KiB: preparazione {:?}, totale {:?} → {:.0} file/s", ask, total, 2000.0 / total.as_secs_f64());

    println!("memoria: RSS a riposo {} MiB, picco (VmHWM) {} MiB per due motori + trasferimento di 1 GiB", base / 1024, rss_kb("VmHWM:") / 1024);
    a.stop().await; b.stop().await;
}
