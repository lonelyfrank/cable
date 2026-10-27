//! Server HTTPS del protocollo LocalSend v2 (ricezione e registrazione dispositivi).
use crate::protocol::{ProtocolError, API};
use crate::receiver::{Receiver, RxError, StreamError};
use crate::shared::Shared;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::{Method, Request, Response};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;
use tokio_util::sync::CancellationToken;

pub struct ServerHandle {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl ServerHandle {
    pub async fn stop(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

type Resp = Response<Full<Bytes>>;

fn reply(status: u16, obj: Option<Value>) -> Resp {
    let data = obj.map(|o| serde_json::to_vec(&o).unwrap()).unwrap_or_default();
    let mut b = Response::builder().status(status);
    if !data.is_empty() { b = b.header("Content-Type", "application/json"); }
    if status >= 400 { b = b.header("Connection", "close"); }
    b.body(Full::new(Bytes::from(data))).unwrap()
}
fn err(e: ProtocolError) -> Resp { reply(e.status, if e.message.is_empty() { None } else { Some(json!({"message": e.message})) }) }

pub async fn start(shared: Arc<Shared>, receiver: Arc<Receiver>, port: u16) -> std::io::Result<ServerHandle> {
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;
    let acceptor = TlsAcceptor::from(shared.identity.read().unwrap().server_config()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?);
    let slots = Arc::new(Semaphore::new(receiver.limits.max_connections));
    let cancel = CancellationToken::new();
    let c2 = cancel.clone();
    let task = tokio::spawn(async move {
        loop {
            let (sock, peer) = tokio::select! {
                _ = c2.cancelled() => break,
                r = listener.accept() => match r { Ok(x) => x, Err(_) => { tokio::time::sleep(Duration::from_millis(50)).await; continue; } },
            };
            let Ok(permit) = slots.clone().try_acquire_owned() else { continue };   // troppe connessioni: scartata
            let (acceptor, shared, receiver, c3) = (acceptor.clone(), shared.clone(), receiver.clone(), c2.clone());
            let (tls_timeout, header_timeout) = (receiver.limits.tls_timeout, receiver.limits.header_timeout);
            tokio::spawn(async move {
                let _permit = permit;      // rilasciato a ogni uscita: errore, scadenza o chiusura
                let _ = sock.set_nodelay(true);
                let Ok(Ok(tls)) = tokio::time::timeout(tls_timeout, acceptor.accept(sock)).await else { return };
                let ip = peer.ip();
                let svc = hyper::service::service_fn(move |req| {
                    let (shared, receiver) = (shared.clone(), receiver.clone());
                    async move { handle(shared, receiver, ip, req).await }
                });
                let conn = http1::Builder::new().timer(TokioTimer::new()).header_read_timeout(header_timeout)
                    .serve_connection(TokioIo::new(tls), svc);
                tokio::pin!(conn);
                tokio::select! { _ = &mut conn => {}, _ = c3.cancelled() => {} }
            });
        }
    });
    Ok(ServerHandle { cancel, task })
}

/// Legge il corpo JSON con limite di dimensione **e** scadenza complessiva: un client che invia
/// lentamente (o smette di inviare) non può trattenere lo slot della connessione. Vale solo per i
/// messaggi JSON; `upload` ha una politica di inattività propria (vedi `Receiver::upload`).
async fn json_body(req: Request<Incoming>, limit: usize, deadline: Duration) -> Result<Value, ProtocolError> {
    if let Some(l) = req.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok()) {
        if l == 0 { return Err(ProtocolError::new(400, "corpo vuoto")); }
        if l > limit { return Err(ProtocolError::new(413, "corpo troppo grande")); }
    }
    let read = Limited::new(req.into_body(), limit).collect();
    let bytes = match tokio::time::timeout(deadline, read).await {
        Err(_) => return Err(ProtocolError::new(408, "corpo non completato in tempo")),
        Ok(Ok(c)) => c.to_bytes(),
        Ok(Err(e)) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => return Err(ProtocolError::new(413, "corpo troppo grande")),
        Ok(Err(_)) => return Err(ProtocolError::new(400, "corpo interrotto")),
    };
    serde_json::from_slice(&bytes).map_err(|_| ProtocolError::new(400, "JSON non valido"))
}

async fn handle(shared: Arc<Shared>, rx: Arc<Receiver>, ip: IpAddr, req: Request<Incoming>) -> Result<Resp, Abort> {
    let path = req.uri().path().to_string();
    let q: HashMap<String, String> = req.uri().query().map(url_pairs).unwrap_or_default();
    match (req.method().clone(), path.as_str()) {
        (Method::GET, p) if p == format!("{API}/info") => Ok(reply(200, Some(shared.info(false)))),
        (Method::POST, p) if p == format!("{API}/register") => match json_body(req, 64 * 1024, rx.limits.json_body_timeout).await {
            Ok(b) => { shared.on_register(&b, ip); Ok(reply(200, Some(shared.info(false)))) }
            Err(e) => Ok(err(e)),
        },
        (Method::POST, p) if p == format!("{API}/prepare-upload") => {
            let body = match json_body(req, rx.limits.max_json_body, rx.limits.json_body_timeout).await { Ok(b) => b, Err(e) => return Ok(err(e)) };
            match rx.prepare(ip, q.get("pin").cloned(), body).await {
                Ok((st, obj)) => Ok(reply(st, obj)),
                Err(e) => Ok(err(e)),
            }
        }
        (Method::POST, p) if p == format!("{API}/upload") => {
            let h = req.headers();
            let chunked = h.get("transfer-encoding").and_then(|v| v.to_str().ok()).is_some_and(|v| v.to_lowercase().contains("chunked"));
            let cl = h.get("content-length").and_then(|v| v.to_str().ok());
            if !chunked && cl.is_none() { return Ok(err(ProtocolError::new(411, "Content-Length mancante"))); }
            let length = if chunked { None } else { match cl.and_then(|v| v.parse::<u64>().ok()) { Some(l) => Some(l), None => return Ok(err(ProtocolError::new(400, "Content-Length non valido"))) } };
            let stream = Box::pin(req.into_body().into_data_stream().map(|r| r.map_err(|_| StreamError)));
            match rx.upload(ip, &q, length, stream).await {
                Ok(_) => Ok(reply(200, None)),
                Err(RxError::Proto(e)) => Ok(err(e)),
                Err(RxError::Stream) => Err(Abort),     // chiude la connessione senza risposta
            }
        }
        (Method::POST, p) if p == format!("{API}/cancel") => match rx.cancel_from_sender(ip, q.get("sessionId").map(String::as_str).unwrap_or("")) {
            Ok(()) => Ok(reply(200, None)),
            Err(e) => Ok(err(e)),
        },
        _ => Ok(reply(404, None)),
    }
}

fn url_pairs(q: &str) -> HashMap<String, String> {
    q.split('&').filter_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        let dec = |s: &str| percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned();
        (!k.is_empty()).then(|| (dec(k), dec(v)))
    }).collect()
}

/// Chiude la connessione senza rispondere (il mittente è già scomparso).
#[derive(Debug)]
struct Abort;
impl std::fmt::Display for Abort { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "connessione interrotta") } }
impl std::error::Error for Abort {}
