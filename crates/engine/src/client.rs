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

pub fn full(b: impl Into<Bytes>) -> Body {
    Full::new(b.into()).map_err(|never| match never {}).boxed()
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientError {
    /// il certificato visto non ha l'impronta attesa
    Identity,
    /// nessuna risposta alla richiesta già inviata
    Timeout,
    /// la connessione TCP/TLS non si è stabilita entro il tempo concesso
    ConnectTimeout,
    Refused,
    /// «nessun percorso verso l'host/la rete»
    Unreachable,
    /// connessione chiusa o reset a metà
    Closed,
    Tls(String),
    Http(String),
    Io(String),
}

impl ClientError {
    /// Messaggio comprensibile per l'utente, senza indirizzo.
    pub fn message(&self) -> String {
        self.describe("", 0)
    }

    /// Messaggio per l'utente: prima il fatto osservato, poi le cause possibili (mai una sola causa presentata come certa).
    pub fn describe(&self, ip: &str, port: u16) -> String {
        let at = if ip.is_empty() { "il dispositivo".to_string() } else { format!("{ip}:{port}") };
        match self {
            ClientError::Identity => format!("Il dispositivo che risponde a {at} non ha l'identità attesa (certificato diverso). Può essere un altro dispositivo con quell'indirizzo o l'app reinstallata: aggiorna l'elenco e controlla il nome."),
            ClientError::Timeout => "Il dispositivo non ha risposto in tempo.".into(),
            ClientError::ConnectTimeout => format!("Nessuna risposta da {at}. Può essere spento o in standby, su un'altra rete, oppure la connessione è bloccata (isolamento Wi-Fi del router, VPN, firewall): controlla che sia sulla stessa rete con LocalSend aperto, poi premi «Aggiorna»."),
            ClientError::Refused => format!("{at} ha rifiutato la connessione: di solito LocalSend è chiuso sul dispositivo o usa un'altra porta. Aprilo e riprova."),
            ClientError::Unreachable => format!("Nessun percorso verso {at}: il dispositivo non è sulla stessa rete o la rete è cambiata. Controlla la Wi-Fi e premi «Aggiorna»."),
            ClientError::Closed | ClientError::Http(_) => "La connessione si è interrotta.".into(),
            ClientError::Tls(_) => "Errore di cifratura nella connessione (certificato o protocollo non accettati).".into(),
            ClientError::Io(e) => format!("Non raggiungibile: {e}"),
        }
    }
    pub fn fatal_for_session(&self) -> bool {
        !matches!(self, ClientError::Identity)
    }
    /// Rete caduta o connessione persa durante lo scambio (non: dispositivo mai raggiunto).
    pub fn is_interruption(&self) -> bool {
        matches!(self, ClientError::Closed | ClientError::Http(_) | ClientError::Io(_) | ClientError::Timeout)
    }
    /// Il dispositivo non si è proprio raggiunto.
    pub fn is_unreachable(&self) -> bool {
        matches!(self, ClientError::ConnectTimeout | ClientError::Refused | ClientError::Unreachable)
    }
}

fn map_io(e: std::io::Error) -> ClientError {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => ClientError::Refused,
        HostUnreachable | NetworkUnreachable | NetworkDown => ClientError::Unreachable,
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
impl Drop for Client {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Apre una connessione TLS e confronta l'impronta *effettiva* con quella attesa. Nessun ripiego su HTTP.
pub async fn connect(tls: ClientTls, host: &str, port: u16, t: Duration, expected: Option<&str>) -> Result<Client, ClientError> {
    let (cfg, seen) = tls;
    let tcp = timeout(t, TcpStream::connect((host, port))).await.map_err(|_| ClientError::ConnectTimeout)?.map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            ClientError::ConnectTimeout
        } else {
            map_io(e)
        }
    })?;
    let _ = tcp.set_nodelay(true);
    let name = ServerName::try_from(host.to_string()).map_err(|e| ClientError::Tls(e.to_string()))?;
    let stream = timeout(t, TlsConnector::from(cfg).connect(name, tcp)).await.map_err(|_| ClientError::ConnectTimeout)?.map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidData | std::io::ErrorKind::Other => ClientError::Tls(e.to_string()),
        _ => map_io(e),
    })?;
    let actual = seen.lock().unwrap().clone().ok_or_else(|| ClientError::Tls("nessun certificato".into()))?;
    if let Some(e) = expected {
        if !ct_eq(actual.as_bytes(), e.to_uppercase().as_bytes()) {
            return Err(ClientError::Identity);
        }
    }
    let (tx, conn) = http1::handshake(TokioIo::new(stream)).await.map_err(|e| ClientError::Http(e.to_string()))?;
    let driver = tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(Client { tx, fp: actual, driver })
}

impl Client {
    pub async fn send(&mut self, req: Request<Body>) -> Result<(u16, Option<Value>), ClientError> {
        let resp = self.tx.send_request(req).await.map_err(|e| ClientError::Http(e.to_string()))?;
        let status = resp.status().as_u16();
        let bytes = Limited::new(resp.into_body(), 8 * 1024 * 1024).collect().await.map_err(|e| ClientError::Http(e.to_string()))?.to_bytes();
        Ok((status, serde_json::from_slice(&bytes).ok()))
    }
}

pub fn json_req(host: &str, port: u16, method: Method, path: &str, body: Option<&Value>) -> Request<Body> {
    let data = body.map(|b| serde_json::to_vec(b).unwrap()).unwrap_or_default();
    let mut b = Request::builder().method(method).uri(path).header("Host", format!("{host}:{port}"));
    if body.is_some() {
        b = b.header("Content-Type", "application/json");
    }
    b.header("Content-Length", data.len()).body(full(data)).unwrap()
}

/// Restituisce (stato, json, impronta effettiva).
#[allow(clippy::too_many_arguments)]
pub async fn json_request(
    tls: ClientTls,
    host: &str,
    port: u16,
    method: Method,
    path: &str,
    body: Option<&Value>,
    t: Duration,
    expected: Option<&str>,
) -> Result<(u16, Option<Value>, String), ClientError> {
    let mut c = connect(tls, host, port, t, expected).await?;
    let fp = c.fp.clone();
    let (st, v) = timeout(t, c.send(json_req(host, port, method, path, body))).await.map_err(|_| ClientError::Timeout)??;
    Ok((st, v, fp))
}

/// Prova **in parallelo** tutti gli indirizzi candidati e restituisce la prima connessione la cui impronta TLS è quella attesa.
/// Così un indirizzo vecchio (DHCP cambiato, altra interfaccia) non fa aspettare quello giusto. Se nessuno funziona
/// restituisce l'errore più significativo: identità diversa > rifiuto > nessun percorso > nessuna risposta.
pub async fn connect_any(
    make_tls: &(dyn Fn() -> Result<ClientTls, String> + Sync),
    cands: &[(String, u16)],
    t: Duration,
    expected: &str,
) -> Result<(Client, String, u16), ClientError> {
    use futures_util::stream::{FuturesUnordered, StreamExt};
    if cands.is_empty() {
        return Err(ClientError::Unreachable);
    }
    // ogni tentativo ha la SUA configurazione TLS: l'impronta vista è registrata lì dentro e va letta
    // solo dalla connessione che l'ha prodotta (una configurazione condivisa permetterebbe di scambiare le identità)
    let mut set = FuturesUnordered::new();
    for (ip, port) in cands {
        let tls = make_tls().map_err(ClientError::Tls)?;
        let (ip, port) = (ip.clone(), *port);
        set.push(async move { connect(tls, &ip, port, t, Some(expected)).await.map(|c| (c, ip, port)) });
    }
    let mut worst: Option<ClientError> = None;
    while let Some(r) = set.next().await {
        match r {
            Ok(v) => return Ok(v),
            Err(e) => {
                let rank = |e: &ClientError| match e {
                    ClientError::Identity => 4,
                    ClientError::Refused => 3,
                    ClientError::Unreachable => 2,
                    _ => 1,
                };
                if worst.as_ref().is_none_or(|w| rank(&e) > rank(w)) {
                    worst = Some(e);
                }
            }
        }
    }
    Err(worst.unwrap_or(ClientError::Unreachable))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_state_facts_and_do_not_blame_a_single_cause() {
        let t = ClientError::ConnectTimeout.describe("192.168.1.9", 53317);
        assert!(t.contains("192.168.1.9:53317") && t.contains("Può essere"), "{t}");
        assert!(t.contains("firewall") && t.contains("isolamento") && t.contains("VPN"), "più cause possibili, non una sola: {t}");
        let r = ClientError::Refused.describe("192.168.1.9", 53317);
        assert!(!r.to_lowercase().contains("firewall"), "un rifiuto esplicito non è un timeout di firewall: {r}");
        assert!(ClientError::Identity.describe("1.2.3.4", 1).contains("identità"));
        assert!(ClientError::Closed.is_interruption() && !ClientError::Refused.is_interruption());
        assert!(ClientError::ConnectTimeout.is_unreachable() && !ClientError::Closed.is_unreachable());
    }
}
