//! Scadenze e limiti del server sui corpi JSON (`register`, `prepare-upload`). Durate ridotte tramite `Limits`:
//! nessuna attesa arbitraria, ogni controllo ha un margine ampio rispetto alla scadenza configurata.
mod common;
use common::*;
use pixelhub_engine::protocol::Limits;
use pixelhub_engine::Engine;
use rustls::pki_types::ServerName;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

const DEADLINE: Duration = Duration::from_millis(400);

fn limits(max_connections: usize) -> Limits {
    Limits { json_body_timeout: DEADLINE, max_connections, max_json_body: 4096, ..Limits::default() }
}

async fn raw(e: &Arc<Engine>) -> TlsStream<TcpStream> {
    let (cfg, _) = e.sh.client_tls().unwrap();
    let tcp = TcpStream::connect(("127.0.0.1", e.port())).await.unwrap();
    TlsConnector::from(cfg).connect(ServerName::try_from("127.0.0.1").unwrap(), tcp).await.unwrap()
}

fn head(path: &str, extra: &str) -> Vec<u8> {
    format!("POST /api/localsend/v2/{path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\n{extra}\r\n").into_bytes()
}

/// Legge fino alla chiusura (o al massimo `max`): restituisce (stato HTTP se presente, connessione chiusa dal server?).
async fn outcome(s: &mut TlsStream<TcpStream>, max: Duration) -> (Option<u16>, bool) {
    let mut buf = Vec::new();
    let closed = tokio::time::timeout(max, async {
        let mut tmp = [0u8; 1024];
        loop {
            match s.read(&mut tmp).await { Ok(0) | Err(_) => break, Ok(n) => buf.extend_from_slice(&tmp[..n]) }
        }
    }).await.is_ok();
    let status = std::str::from_utf8(&buf).ok().and_then(|t| t.split_whitespace().nth(1)).and_then(|c| c.parse().ok());
    (status, closed)
}

async fn one_shot(e: &Arc<Engine>, path: &str, extra: &str, body: &[u8]) -> (Option<u16>, bool) {
    let mut s = raw(e).await;
    s.write_all(&head(path, &format!("Content-Length: {}\r\nConnection: close\r\n{extra}", body.len()))).await.unwrap();
    s.write_all(body).await.unwrap();
    outcome(&mut s, Duration::from_secs(5)).await
}

#[tokio::test(flavor = "multi_thread")]
async fn body_never_completed_gets_408_and_connection_closed() {
    let mut r = Rig::new();
    let e = r.engine_with_limits("s", limits(8)).await;
    for path in ["register", "prepare-upload"] {
        let mut s = raw(&e).await;
        s.write_all(&head(path, "Content-Length: 100\r\n")).await.unwrap();      // header completi, corpo assente
        let t = Instant::now();
        let (st, closed) = outcome(&mut s, Duration::from_secs(5)).await;
        assert_eq!(st, Some(408), "{path}");
        assert!(closed, "{path}: il server deve chiudere la connessione");
        assert!(t.elapsed() >= DEADLINE && t.elapsed() < Duration::from_secs(3), "{path}: {:?}", t.elapsed());
    }
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn slow_drip_body_is_cut_at_the_deadline() {
    let mut r = Rig::new();
    let e = r.engine_with_limits("s", limits(8)).await;
    let s = raw(&e).await;
    let (mut rd, mut wr) = tokio::io::split(s);
    wr.write_all(&head("register", "Content-Length: 200\r\n")).await.unwrap();
    let t = Instant::now();
    // un byte ogni 60 ms: ogni singolo intervallo è breve, il totale supera la scadenza
    let drip = tokio::spawn(async move { while wr.write_all(b" ").await.is_ok() { tokio::time::sleep(Duration::from_millis(60)).await; } });
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), async { let mut tmp = [0u8; 512]; loop { match rd.read(&mut tmp).await { Ok(0) | Err(_) => break, Ok(n) => buf.extend_from_slice(&tmp[..n]) } } }).await.expect("il server doveva chiudere");
    assert!(String::from_utf8_lossy(&buf).starts_with("HTTP/1.1 408"), "{}", String::from_utf8_lossy(&buf));
    assert!(t.elapsed() >= DEADLINE && t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    drip.abort();
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oversize_malformed_and_valid_bodies_are_told_apart() {
    let mut r = Rig::new();
    let e = r.engine_with_limits("s", limits(8)).await;
    // oltre il limite dichiarato da Content-Length
    let big = vec![b' '; 5000];
    assert_eq!(one_shot(&e, "prepare-upload", "", &big).await.0, Some(413));
    // oltre il limite con trasferimento a blocchi (nessun Content-Length)
    let mut s = raw(&e).await;
    s.write_all(&head("prepare-upload", "Transfer-Encoding: chunked\r\n")).await.unwrap();
    for _ in 0..3 { let _ = s.write_all(format!("{:x}\r\n", 2000).as_bytes()).await; let _ = s.write_all(&vec![b' '; 2000]).await; let _ = s.write_all(b"\r\n").await; }
    assert_eq!(outcome(&mut s, Duration::from_secs(5)).await.0, Some(413));
    // JSON malformato (per entrambi gli endpoint) e corpo vuoto
    assert_eq!(one_shot(&e, "register", "", b"{non json").await.0, Some(400));
    assert_eq!(one_shot(&e, "prepare-upload", "", b"[1,").await.0, Some(400));
    assert_eq!(one_shot(&e, "register", "", b"").await.0, Some(400));
    // JSON valido ma privo di campi: risposta protocollare (400 da prepare), non un errore di lettura né un timeout
    let st = one_shot(&e, "prepare-upload", "", b"{}").await.0;
    assert_eq!(st, Some(400));
    // register valido: 200
    let ok = br#"{"alias":"x","version":"2.0","deviceModel":"m","deviceType":"desktop","fingerprint":"AAAA","port":53317,"protocol":"https","download":false}"#;
    assert_eq!(one_shot(&e, "register", "", ok).await.0, Some(200));
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stalled_connections_up_to_the_limit_do_not_block_the_server_afterwards() {
    const N: usize = 4;
    let mut r = Rig::new();
    let e = r.engine_with_limits("s", limits(N)).await;
    let mut stalled = Vec::new();
    for i in 0..N {
        let mut s = raw(&e).await;
        s.write_all(&head(if i % 2 == 0 { "register" } else { "prepare-upload" }, "Content-Length: 100\r\n")).await.unwrap();
        stalled.push(s);
    }
    // slot esauriti: una richiesta legittima ora non viene servita (connessione scartata)...
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ok = br#"{"alias":"x","version":"2.0","deviceModel":"m","deviceType":"desktop","fingerprint":"BBBB","port":53317,"protocol":"https","download":false}"#;
    let busy = tokio::time::timeout(Duration::from_millis(250), async {
        let (cfg, _) = e.sh.client_tls().unwrap();
        let tcp = TcpStream::connect(("127.0.0.1", e.port())).await.unwrap();
        let mut s = TlsConnector::from(cfg).connect(ServerName::try_from("127.0.0.1").unwrap(), tcp).await.map_err(|_| ())?;
        s.write_all(&head("register", &format!("Content-Length: {}\r\n", ok.len()))).await.map_err(|_| ())?;
        s.write_all(ok).await.map_err(|_| ())?;
        Ok::<_, ()>(outcome(&mut s, Duration::from_millis(200)).await)
    }).await;
    assert!(!matches!(busy, Ok(Ok((Some(200), _)))), "con tutti gli slot occupati non doveva essere servita: {busy:?}");
    // ...ma ognuna delle incomplete riceve 408 e libera lo slot
    for s in stalled.iter_mut() { assert_eq!(outcome(s, Duration::from_secs(5)).await, (Some(408), true)); }
    // dopo la scadenza il server serve di nuovo richieste legittime, più volte
    for _ in 0..N + 1 { assert_eq!(one_shot(&e, "register", "", ok).await.0, Some(200)); }
    r.stop_all().await;
}
