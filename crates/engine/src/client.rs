//! Client HTTPS verso altri dispositivi LocalSend, con verifica dell'impronta del certificato.
use crate::shared::ClientTls;
use crate::util::ct_eq;
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full, Limited};
use hyper::client::conn::http1;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use serde_json::Value;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

pub type Body = BoxBody<Bytes, std::io::Error>;

pub fn full(b: impl Into<Bytes>) -> Body { Full::new(b.into()).map_err(|never| match never {}).boxed() }

#[derive(Debug, Clone, PartialEq)]
pub enum ClientError { Identity, Timeout, Refused, Closed, Tls(String), Http(String), Io(String) }

impl ClientError {
    /// Messaggio comprensibile per l'utente.
    pub fn message(&self) -> String {
        match self {
            ClientError::Identity => "L'identità del dispositivo non corrisponde: aggiorna l'elenco e riprova".into(),
            ClientError::Timeout => "Il dispositivo non risponde (timeout)".into(),
            ClientError::Refused => "Connessione rifiutata: apri LocalSend sul destinatario".into(),
            ClientError::Closed | ClientError::Http(_) => "Connessione interrotta".into(),
            ClientError::Tls(_) => "Errore di cifratura nella connessione".into(),
            ClientError::Io(e) => format!("Non raggiungibile: {e}"),
        }
    }
    pub fn fatal_for_session(&self) -> bool { !matches!(self, ClientError::Identity) }
}

fn map_io(e: std::io::Error) -> ClientError {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => ClientError::Refused,
        ConnectionReset | ConnectionAborted | BrokenPipe | UnexpectedEof => ClientError::Closed,
        TimedOut => ClientError::Timeout,
        _ => ClientError::Io(e.to_string()),
    }
}

pub struct Client {
    pub tx: http1::SendRequest<Body>,
    /// impronta SHA-256 effettiva del certificato visto sulla connessione
    pub fp: String,
    driver: JoinHandle<()>,
}
impl Drop for Client { fn drop(&mut self) { self.driver.abort(); } }

/// Apre una connessione TLS e confronta l'impronta *effettiva* con quella attesa. Nessun ripiego su HTTP.
pub async fn connect(tls: ClientTls, host: &str, port: u16, t: Duration, expected: Option<&str>) -> Result<Client, ClientError> {
    let (cfg, seen) = tls;
    let tcp = timeout(t, TcpStream::connect((host, port))).await.map_err(|_| ClientError::Timeout)?.map_err(map_io)?;
    let _ = tcp.set_nodelay(true);
    let name = ServerName::try_from(host.to_string()).map_err(|e| ClientError::Tls(e.to_string()))?;
    let stream = timeout(t, TlsConnector::from(cfg).connect(name, tcp)).await.map_err(|_| ClientError::Timeout)?
        .map_err(|e| match e.kind() { std::io::ErrorKind::InvalidData | std::io::ErrorKind::Other => ClientError::Tls(e.to_string()), _ => map_io(e) })?;
    let actual = seen.lock().unwrap().clone().ok_or_else(|| ClientError::Tls("nessun certificato".into()))?;
    if let Some(e) = expected {
        if !ct_eq(actual.as_bytes(), e.to_uppercase().as_bytes()) { return Err(ClientError::Identity); }
    }
    let (tx, conn) = http1::handshake(TokioIo::new(stream)).await.map_err(|e| ClientError::Http(e.to_string()))?;
    let driver = tokio::spawn(async move { let _ = conn.await; });
    Ok(Client { tx, fp: actual, driver })
}

impl Client {
    pub async fn send(&mut self, req: Request<Body>) -> Result<(u16, Option<Value>), ClientError> {
        let resp = self.tx.send_request(req).await.map_err(|e| ClientError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = Limited::new(resp.into_body(), 8 * 1024 * 1024).collect().await
            .map_err(|e| ClientError::Http(e.to_string()))?.to_bytes();
        Ok((status, serde_json::from_slice(&bytes).ok()))
    }
}

pub fn json_req(host: &str, port: u16, method: Method, path: &str, body: Option<&Value>) -> Request<Body> {
    let data = body.map(|b| serde_json::to_vec(b).unwrap()).unwrap_or_default();
    let mut b = Request::builder().method(method).uri(path).header("Host", format!("{host}:{port}"));
    if body.is_some() { b = b.header("Content-Type", "application/json"); }
    b.header("Content-Length", data.len()).body(full(data)).unwrap()
}

/// Restituisce (stato, json, impronta effettiva).
pub async fn json_request(tls: ClientTls, host: &str, port: u16, method: Method, path: &str, body: Option<&Value>,
                          t: Duration, expected: Option<&str>) -> Result<(u16, Option<Value>, String), ClientError> {
    let mut c = connect(tls, host, port, t, expected).await?;
    let fp = c.fp.clone();
    let (st, v) = timeout(t, c.send(json_req(host, port, method, path, body))).await.map_err(|_| ClientError::Timeout)??;
    Ok((st, v, fp))
}
