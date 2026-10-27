//! Prova di interoperabilità con il motore Python (tests/interop/py_peer.py). Uso: interop PORTA_PY
use pixelhub_engine::paths::Paths;
use pixelhub_engine::Engine;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let py_port: u16 = std::env::args().nth(1).expect("porta del peer Python").parse().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    let e = Engine::new(Paths::under(dir.path()), port, false);
    e.sh.settings.aggiorna(serde_json::json!({"nome_pc": "peer-rust"}).as_object().unwrap());
    assert!(e.start().await);
    // accetta tutto, come farebbe un utente
    let e2 = e.clone();
    tokio::spawn(async move { loop {
        for r in e2.receiver.snapshot().0.as_array().unwrap() { if r["stato"] == "attesa" { e2.decidi(r["id"].as_str().unwrap(), None); } }
        tokio::time::sleep(Duration::from_millis(50)).await;
    } });
    assert_eq!(e.cerca(Some("127.0.0.1"), Some(py_port)).await, 1, "il peer Python non risponde");
    let py = e.snapshot()["dispositivi"].as_array().unwrap().iter().find(|d| d["alias"] == "peer-python").cloned().expect("peer-python non trovato");
    let fp = py["fp"].as_str().unwrap().to_string();
    println!("RS_SEES_PYTHON verificato={}", py["verificato"]);
    let data: Vec<u8> = (0..5 * 1024 * 1024 + 123u32).map(|i| ((i as u64 * 7 + 3) % 256) as u8).collect();
    let src = dir.path().join("rust-uscita.bin");
    std::fs::write(&src, &data).unwrap();
    for (label, paths, text) in [("FILE", vec![src.to_string_lossy().into_owned()], None), ("TESTO", vec![], Some("ciao dal peer rust".to_string()))] {
        let jid = e.invia(&fp, &paths, text).unwrap();
        let t0 = Instant::now();
        let st = loop {
            let j = e.sender.snapshot().as_array().unwrap().iter().find(|j| j["id"] == jid.as_str()).cloned().unwrap();
            if ["completato", "parziale", "errore", "rifiutato", "annullato"].contains(&j["stato"].as_str().unwrap()) { break (j["stato"].clone(), j["errore"].clone()); }
            assert!(t0.elapsed() < Duration::from_secs(60));
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        println!("RS_SENT_{label} {} {}", st.0, st.1);
    }
    // quanto ha mandato Python
    let want = hex::encode(Sha256::digest(&data));
    let inbox = std::path::PathBuf::from(e.sh.settings.str("cartella"));
    let t0 = Instant::now();
    loop {
        let ok_file = std::fs::read_dir(&inbox).map(|r| r.flatten().any(|f| f.file_name() == "uscita.bin" && std::fs::read(f.path()).map(|b| hex::encode(Sha256::digest(&b)) == want).unwrap_or(false))).unwrap_or(false);
        let msgs = e.receiver.snapshot().1;
        if ok_file && !msgs.as_array().unwrap().is_empty() {
            println!("RS_RECEIVED_FILE_OK=true");
            println!("RS_RECEIVED_TEXT={}", msgs[0]["testo"]);
            break;
        }
        if t0.elapsed() > Duration::from_secs(40) { println!("RS_RECEIVED_FILE_OK=false (timeout) msgs={msgs}"); break; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    tokio::time::sleep(Duration::from_secs(4)).await;       // lascia a Python il tempo di verificare
    e.stop().await;
}
