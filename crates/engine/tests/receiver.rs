use bytes::Bytes;
use futures_util::stream;
use pixelhub_engine::bus::{Bus, Notifier};
use pixelhub_engine::paths::Paths;
use pixelhub_engine::protocol::Limits;
use pixelhub_engine::receiver::{Receiver, RxError, StreamError};
use pixelhub_engine::settings::Settings;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const IP: &str = "10.0.0.5";
fn ip(s: &str) -> IpAddr { s.parse().unwrap() }

struct Env { _t: tempfile::TempDir, rx: Arc<Receiver>, settings: Arc<Settings>, inbox: PathBuf }

fn env_with(limits: Limits) -> Env {
    let t = tempfile::tempdir().unwrap();
    let paths = Paths::under(t.path());
    let settings = Arc::new(Settings::load(&paths));
    let inbox = t.path().join("in");
    settings.aggiorna(json!({"cartella": inbox.to_str().unwrap()}).as_object().unwrap());
    let rx = Receiver::new(settings.clone(), Bus::new(), limits, Arc::new(Notifier::default()));
    Env { _t: t, rx, settings, inbox }
}
fn env() -> Env {
    env_with(Limits { decision_timeout: Duration::from_secs(5), session_idle: Duration::from_millis(300), ..Limits::default() })
}

type Files<'a> = Vec<(&'a str, &'a str, &'a [u8], bool)>; // id, nome, dati, con sha256
fn body(files: &Files) -> Value {
    let mut m = serde_json::Map::new();
    for (id, name, data, sha) in files {
        let mut f = json!({"id": id, "fileName": name, "size": data.len(), "fileType": "application/octet-stream"});
        if *sha { f["sha256"] = json!(hex::encode(Sha256::digest(data))); }
        m.insert(id.to_string(), f);
    }
    json!({"info": {"alias": "Mittente", "fingerprint": "C".repeat(64)}, "files": m})
}

async fn first_pending(rx: &Receiver) -> String {
    for _ in 0..500 {
        let (r, _) = rx.snapshot();
        if let Some(i) = r.as_array().unwrap().iter().find(|i| i["stato"] == "attesa") { return i["id"].as_str().unwrap().to_string(); }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("nessuna richiesta in attesa");
}

/// prepare + decisione; restituisce (sessionId, token per file)
async fn session(e: &Env, files: Files<'_>, accept: Option<Vec<String>>) -> (String, HashMap<String, String>) {
    let rx = e.rx.clone();
    let b = body(&files);
    let h = tokio::spawn(async move { rx.prepare(ip(IP), None, b).await });
    let id = first_pending(&e.rx).await;
    assert!(e.rx.decide(&id, accept));
    let (st, resp) = h.await.unwrap().unwrap();
    assert_eq!(st, 200);
    let r = resp.unwrap();
    (r["sessionId"].as_str().unwrap().to_string(),
     r["files"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect())
}

fn q(sid: &str, fid: &str, tok: &str) -> HashMap<String, String> {
    [("sessionId", sid), ("fileId", fid), ("token", tok)].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}
type St = stream::Iter<std::vec::IntoIter<Result<Bytes, StreamError>>>;
fn chunks(data: &[u8]) -> St {
    stream::iter(data.chunks(4).map(|c| Ok(Bytes::copy_from_slice(c))).collect::<Vec<_>>())
}
fn status(r: Result<PathBuf, RxError>) -> u16 {
    match r { Err(RxError::Proto(p)) => p.status, Err(RxError::Stream) => 0, Ok(_) => 200 }
}
fn state(e: &Env) -> Value { e.rx.snapshot().0[0].clone() }
fn listing(p: &PathBuf) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(p).map(|r| r.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    v.sort();
    v
}

#[tokio::test]
async fn happy_path_atomic_and_complete() {
    let e = env();
    let data = b"ciao mondo!";
    let (sid, tok) = session(&e, vec![("f", "a.txt", data, true)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "f", &tok["f"]), Some(data.len() as u64), chunks(data)).await.unwrap();
    assert_eq!(std::fs::read(e.inbox.join("a.txt")).unwrap(), data);
    assert_eq!(state(&e)["stato"], "completata");
    assert_eq!(listing(&e.inbox), ["a.txt"]);
}

#[tokio::test]
async fn zero_byte_unicode_subdir_and_traversal() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "cartella/è già.txt", b"", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(0), chunks(b"")).await.unwrap();
    assert!(e.inbox.join("cartella/è già.txt").exists());
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "../../evil.txt", b"x", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"x")).await.unwrap();
    assert!(e.inbox.join("evil.txt").exists());
    assert!(!e.inbox.parent().unwrap().parent().unwrap().join("evil.txt").exists());
}

#[tokio::test]
async fn existing_file_never_overwritten_and_symlink_refused() {
    let e = env();
    std::fs::create_dir_all(&e.inbox).unwrap();
    std::fs::write(e.inbox.join("a.txt"), "originale").unwrap();
    let (sid, tok) = session(&e, vec![("a", "a.txt", b"nuovo", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(5), chunks(b"nuovo")).await.unwrap();
    assert_eq!(std::fs::read_to_string(e.inbox.join("a.txt")).unwrap(), "originale");
    assert_eq!(std::fs::read_to_string(e.inbox.join("a (1).txt")).unwrap(), "nuovo");

    let e = env();
    std::fs::create_dir_all(&e.inbox).unwrap();
    let out = e.inbox.parent().unwrap().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::os::unix::fs::symlink(&out, e.inbox.join("d")).unwrap();
    let (sid, tok) = session(&e, vec![("a", "d/x.txt", b"x", false)], None).await;
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"x")).await), 403);
    assert!(listing(&out).is_empty());
}

#[tokio::test]
async fn wrong_token_ip_session_and_params() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"x", false)], None).await;
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", "sbagliato"), Some(1), chunks(b"x")).await), 403);
    assert_eq!(status(e.rx.upload(ip("10.9.9.9"), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"x")).await), 403);
    assert_eq!(status(e.rx.upload(ip(IP), &q("nope", "a", &tok["a"]), Some(1), chunks(b"x")).await), 403);
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "zzz", &tok["a"]), Some(1), chunks(b"x")).await), 403);
    let mut only = HashMap::new();
    only.insert("sessionId".to_string(), sid.clone());
    assert_eq!(status(e.rx.upload(ip(IP), &only, Some(1), chunks(b"x")).await), 400);
}

#[tokio::test]
async fn duplicate_after_completion_409_and_session_stays_open() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"x", false), ("b", "b", b"y", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"x")).await.unwrap();
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"x")).await), 409);
    assert_eq!(state(&e)["stato"], "ricezione");        // b manca: non è completata
}

fn slow(data: Vec<u8>, ms: u64) -> impl futures_util::Stream<Item = Result<Bytes, StreamError>> + Unpin {
    Box::pin(stream::unfold((data, 0usize), move |(d, i)| async move {
        if i >= d.len() { return None; }
        tokio::time::sleep(Duration::from_millis(ms)).await;
        let end = (i + 4).min(d.len());
        let c = Bytes::copy_from_slice(&d[i..end]);
        Some((Ok(c), (d, end)))
    }))
}

#[tokio::test]
async fn concurrent_same_file_only_one_wins() {
    let e = env();
    let data = vec![7u8; 400];
    let (sid, tok) = session(&e, vec![("a", "a", &data, false)], None).await;
    let rx = e.rx.clone();
    let (s2, t2, d2) = (sid.clone(), tok["a"].clone(), data.clone());
    let first = tokio::spawn(async move { rx.upload(ip(IP), &q(&s2, "a", &t2), Some(400), slow(d2, 5)).await });
    for _ in 0..200 {
        if state(&e)["file"][0]["stato"] == "ricezione" { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(400), chunks(&data)).await), 409);
    assert_eq!(status(first.await.unwrap()), 200);
    assert_eq!(listing(&e.inbox), ["a"]);
}

#[tokio::test]
async fn size_mismatch_hash_mismatch_and_interruption_leave_nothing() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"xxx", false)], None).await;
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), None, chunks(b"troppo lungo")).await), 400);
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), None, chunks(b"xx")).await), 400);
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(99), chunks(b"xxx")).await), 400);
    assert!(listing(&e.inbox).is_empty());
    // sha256 errato → 422, poi ritentativo valido
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"giusto", true)], None).await;
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(6), chunks(b"sbagli")).await), 422);
    assert!(listing(&e.inbox).is_empty());
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(6), chunks(b"giusto")).await.unwrap();
    assert_eq!(state(&e)["stato"], "completata");
    // rete caduta a metà
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", &[1u8; 100], false)], None).await;
    let broken = stream::iter(vec![Ok(Bytes::from_static(b"1234")), Err(StreamError)]);
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(100), broken).await), 0);
    assert!(listing(&e.inbox).is_empty());
    assert_eq!(state(&e)["file"][0]["stato"], "attesa");
}

#[tokio::test]
async fn dropped_upload_future_cleans_temp_and_resets_state() {
    // il client chiude la connessione: hyper annulla l'handler, qui simulato con abort()
    let e = env();
    let data = vec![1u8; 4000];
    let (sid, tok) = session(&e, vec![("a", "a", &data, false)], None).await;
    let rx = e.rx.clone();
    let (s2, t2) = (sid.clone(), tok["a"].clone());
    let h = tokio::spawn(async move { rx.upload(ip(IP), &q(&s2, "a", &t2), Some(4000), slow(vec![1u8; 4000], 5)).await });
    for _ in 0..200 {
        if state(&e)["byte"].as_u64().unwrap() > 0 { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(listing(&e.inbox).iter().any(|n| n.starts_with(".pixelhub-")));
    h.abort();
    let _ = h.await;
    assert!(listing(&e.inbox).is_empty());
    assert_eq!(state(&e)["file"][0]["stato"], "attesa");
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(4000), chunks(&data)).await.unwrap();   // il ritentativo funziona
}

#[tokio::test]
async fn cancel_by_sender_and_locally_stop_inflight_io() {
    let e = env();
    let data = vec![1u8; 4000];
    let (sid, tok) = session(&e, vec![("a", "a", &data, false)], None).await;
    let rx = e.rx.clone();
    let (s2, t2) = (sid.clone(), tok["a"].clone());
    let h = tokio::spawn(async move { rx.upload(ip(IP), &q(&s2, "a", &t2), Some(4000), slow(vec![1u8; 4000], 5)).await });
    for _ in 0..200 {
        if state(&e)["byte"].as_u64().unwrap() > 0 { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(e.rx.cancel_from_sender(ip("10.9.9.9"), &sid).is_err());
    e.rx.cancel_from_sender(ip(IP), &sid).unwrap();
    assert_eq!(status(h.await.unwrap()), 409);
    assert!(listing(&e.inbox).is_empty());
    assert_eq!(state(&e)["stato"], "annullata");

    // annullamento locale dopo un file: esito PARZIALE, mai "completata"
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"1", false), ("b", "b", b"2", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"1")).await.unwrap();
    let id = state(&e)["id"].as_str().unwrap().to_string();
    assert!(e.rx.annulla(&id));
    let st = state(&e);
    assert_eq!((st["stato"].as_str().unwrap(), st["ricevuti"].as_u64().unwrap()), ("parziale", 1));
    assert_eq!(listing(&e.inbox), ["a"]);
}

#[tokio::test]
async fn idle_session_expires_to_partial_or_expired() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"1", false), ("b", "b", b"2", false)], None).await;
    e.rx.upload(ip(IP), &q(&sid, "a", &tok["a"]), Some(1), chunks(b"1")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(400)).await;
    e.rx.sweep();
    assert_eq!(state(&e)["stato"], "parziale");
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "b", &tok["b"]), Some(1), chunks(b"2")).await), 403);
    let e = env();
    session(&e, vec![("a", "a", b"1", false)], None).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    e.rx.sweep();
    assert_eq!(state(&e)["stato"], "scaduta");
}

#[tokio::test]
async fn partial_accept_reject_timeout_and_peer_gone() {
    let e = env();
    let (sid, tok) = session(&e, vec![("a", "a", b"1", false), ("b", "b", b"2", false), ("c", "c", b"3", false)], Some(vec!["a".into(), "c".into()])).await;
    assert_eq!(tok.keys().cloned().collect::<std::collections::BTreeSet<_>>(), ["a".to_string(), "c".to_string()].into());
    assert_eq!(status(e.rx.upload(ip(IP), &q(&sid, "b", &"x".repeat(32)), Some(1), chunks(b"2")).await), 403);
    for (k, d) in [("a", b"1"), ("c", b"3")] { e.rx.upload(ip(IP), &q(&sid, k, &tok[k]), Some(1), chunks(d)).await.unwrap(); }
    assert_eq!(state(&e)["stato"], "completata");
    assert_eq!(listing(&e.inbox), ["a", "c"]);
    // rifiuto
    let e = env();
    let rx = e.rx.clone();
    let b = body(&vec![("a", "a", b"1", false)]);
    let h = tokio::spawn(async move { rx.prepare(ip(IP), None, b).await });
    let id = first_pending(&e.rx).await;
    e.rx.decide(&id, Some(vec![]));
    assert_eq!(h.await.unwrap().unwrap_err().status, 403);
    assert_eq!(state(&e)["stato"], "rifiutata");
    // timeout
    let e = env_with(Limits { decision_timeout: Duration::from_millis(300), ..Limits::default() });
    assert_eq!(e.rx.prepare(ip(IP), None, body(&vec![("a", "a", b"1", false)])).await.unwrap_err().status, 403);
    assert_eq!(state(&e)["stato"], "scaduta");
    // il mittente chiude mentre attende: l'handler viene annullato
    let e = env();
    let rx = e.rx.clone();
    let b = body(&vec![("a", "a", b"1", false)]);
    let h = tokio::spawn(async move { rx.prepare(ip(IP), None, b).await });
    first_pending(&e.rx).await;
    h.abort();
    let _ = h.await;
    assert_eq!(state(&e)["stato"], "annullata");
}

#[tokio::test]
async fn busy_pending_limit_and_invalid_body() {
    let e = env();
    session(&e, vec![("a", "a", b"1", false)], None).await;
    assert_eq!(e.rx.prepare(ip("10.0.0.9"), None, body(&vec![("z", "z", b"1", false)])).await.unwrap_err().status, 409);
    let e = env_with(Limits { max_pending: 2, decision_timeout: Duration::from_secs(3), ..Limits::default() });
    let mut hs = vec![];
    for i in 0..2 {
        let rx = e.rx.clone();
        let b = body(&vec![("a", "a", b"1", false)]);
        hs.push(tokio::spawn(async move { let _ = rx.prepare(ip(&format!("10.0.1.{i}")), None, b).await; }));
    }
    for _ in 0..300 {
        if e.rx.snapshot().0.as_array().unwrap().iter().filter(|i| i["stato"] == "attesa").count() == 2 { break; }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(e.rx.prepare(ip("10.0.1.99"), None, body(&vec![("z", "z", b"1", false)])).await.unwrap_err().status, 429);
    for i in e.rx.snapshot().0.as_array().unwrap() { e.rx.decide(i["id"].as_str().unwrap(), Some(vec![])); }
    for h in hs { let _ = h.await; }
    for b in [Value::Null, json!({"info": {}, "files": {}}), json!({"files": {"a": {}}, "info": {}})] {
        assert_eq!(e.rx.prepare(ip(IP), None, b).await.unwrap_err().status, 400);
    }
}

#[tokio::test]
async fn text_message_needs_accept_and_returns_204() {
    let e = env();
    let b = json!({"info": {"alias": "Tel", "fingerprint": "C".repeat(64)},
        "files": {"t": {"id": "t", "fileName": "t.txt", "size": 4, "fileType": "text/plain", "preview": "ciao"}}});
    let rx = e.rx.clone();
    let h = tokio::spawn(async move { rx.prepare(ip(IP), None, b).await });
    let id = first_pending(&e.rx).await;
    assert!(e.rx.snapshot().1.as_array().unwrap().is_empty());     // nulla consegnato prima dell'ok
    e.rx.decide(&id, None);
    assert_eq!(h.await.unwrap().unwrap().0, 204);
    assert_eq!(e.rx.snapshot().1[0]["testo"], "ciao");
}

#[tokio::test]
async fn pin_required_wrong_rate_limited_and_correct() {
    let e = env();
    e.settings.imposta_pin(Some("123456"));
    let b = || body(&vec![("a", "a", b"1", false)]);
    assert_eq!(e.rx.prepare(ip(IP), None, b()).await.unwrap_err().status, 401);
    for _ in 0..10 { assert_eq!(e.rx.prepare(ip(IP), Some("000000".into()), b()).await.unwrap_err().status, 401); }
    assert_eq!(e.rx.prepare(ip(IP), Some("123456".into()), b()).await.unwrap_err().status, 429);
    let e = env();
    e.settings.imposta_pin(Some("123456"));
    let rx = e.rx.clone();
    let bb = b();
    let h = tokio::spawn(async move { rx.prepare(ip(IP), Some("123456".into()), bb).await });
    let id = first_pending(&e.rx).await;
    e.rx.decide(&id, None);
    assert_eq!(h.await.unwrap().unwrap().0, 200);
}

#[tokio::test]
async fn insufficient_space_507() {
    let e = env_with(Limits { disk_margin: 1 << 60, decision_timeout: Duration::from_secs(5), ..Limits::default() });
    let rx = e.rx.clone();
    let b = body(&vec![("a", "a", b"1", false)]);
    let h = tokio::spawn(async move { rx.prepare(ip(IP), None, b).await });
    let id = first_pending(&e.rx).await;
    e.rx.decide(&id, None);
    assert_eq!(h.await.unwrap().unwrap_err().status, 507);
    let st = state(&e);
    assert_eq!(st["stato"], "errore");
    assert!(st["errore"].as_str().unwrap().contains("Spazio"));
}
