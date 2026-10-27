//! Scoperta LocalSend: annunci multicast, registrazione HTTPS e inserimento manuale.
use crate::client::{json_request, ClientError};
use crate::protocol::{parse_peer_info, API};
use crate::shared::Shared;
use hyper::Method;
use serde_json::{json, Value};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;

pub const MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

pub fn local_ipv4() -> Vec<Ipv4Addr> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|i| !i.is_loopback())
        .filter_map(|i| match i.ip() { IpAddr::V4(v) if !v.is_link_local() => Some(v), _ => None }).collect()
}

pub struct Discovery {
    sh: Arc<Shared>,
    multicast: bool,
    cancel: CancellationToken,
    pub errore: Mutex<String>,
    last_reply: Mutex<HashMap<IpAddr, Instant>>,
    joined: Mutex<BTreeSet<Ipv4Addr>>,
    sock: Mutex<Option<Arc<UdpSocket>>>,
}

impl Discovery {
    pub fn new(sh: Arc<Shared>, multicast: bool) -> Arc<Discovery> {
        Arc::new(Discovery { sh, multicast, cancel: CancellationToken::new(), errore: Mutex::new(String::new()),
                             last_reply: Mutex::new(HashMap::new()), joined: Mutex::new(BTreeSet::new()), sock: Mutex::new(None) })
    }

    pub fn start(self: &Arc<Self>) {
        if self.multicast { let me = self.clone(); tokio::spawn(async move { me.listen().await }); }
        let me = self.clone();
        tokio::spawn(async move { me.periodic().await });
    }

    pub fn stop(&self) { self.cancel.cancel(); }

    /// Dopo un cambio rete o la ripresa dalla sospensione.
    pub fn rete_cambiata(self: &Arc<Self>) {
        self.joined.lock().unwrap().clear();
        self.join();
        let me = self.clone();
        tokio::spawn(async move { me.announce(true).await });
    }

    fn bind(&self) -> std::io::Result<UdpSocket> {
        let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
        s.set_reuse_address(true)?;
        s.set_reuse_port(true)?;
        s.set_nonblocking(true)?;
        s.bind(&SocketAddr::from(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, self.sh.port())).into())?;
        UdpSocket::from_std(s.into())
    }

    fn join(&self) {
        let Some(s) = self.sock.lock().unwrap().clone() else { return };
        let mut j = self.joined.lock().unwrap();
        for ip in local_ipv4().into_iter().chain([Ipv4Addr::UNSPECIFIED]) {
            if j.contains(&ip) { continue; }
            if s.join_multicast_v4(MCAST, ip).is_ok() { j.insert(ip); }
        }
    }

    async fn listen(self: Arc<Self>) {
        while !self.cancel.is_cancelled() {
            let sock = match self.bind() {
                Ok(s) => Arc::new(s),
                Err(e) => {
                    *self.errore.lock().unwrap() = format!("Scoperta multicast non disponibile: {e}");
                    tokio::select! { _ = self.cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(15)) => continue }
                }
            };
            self.errore.lock().unwrap().clear();
            *self.sock.lock().unwrap() = Some(sock.clone());
            self.joined.lock().unwrap().clear();
            self.join();
            let mut buf = vec![0u8; 8192];
            loop {
                tokio::select! {
                    _ = self.cancel.cancelled() => return,
                    r = tokio::time::timeout(Duration::from_secs(10), sock.recv_from(&mut buf)) => match r {
                        Err(_) => self.join(),                       // interfacce nuove (hotspot acceso dopo)
                        Ok(Err(_)) => break,
                        Ok(Ok((n, from))) => self.on_datagram(&buf[..n], from.ip()),
                    }
                }
            }
        }
    }

    fn on_datagram(self: &Arc<Self>, data: &[u8], ip: IpAddr) {
        let Ok(info) = serde_json::from_slice::<Value>(data) else { return };
        let Some(p) = parse_peer_info(&info) else { return };
        if p.fp == self.sh.fingerprint() { return; }
        self.sh.registry.upsert(&info, &ip.to_string(), "multicast", None);
        if info["announce"] == json!(true) {
            {
                let mut lr = self.last_reply.lock().unwrap();
                if lr.get(&ip).map_or(false, |t| t.elapsed() < Duration::from_secs(5)) { return; }
                lr.insert(ip, Instant::now());
                if lr.len() > 512 { lr.retain(|_, t| t.elapsed() < Duration::from_secs(5)); }
            }
            let me = self.clone();
            tokio::spawn(async move { me.reply_to_announce(p.fp, p.port, ip).await });
        }
    }

    async fn reply_to_announce(self: Arc<Self>, fp: String, port: u16, ip: IpAddr) {
        let Ok(tls) = self.sh.client_tls() else { return };
        match json_request(tls, &ip.to_string(), port, Method::POST, &format!("{API}/register"), Some(&self.sh.info(true)), Duration::from_secs(4), Some(&fp)).await {
            Ok((200, _, _)) => self.sh.registry.mark_verified(&fp),
            Err(ClientError::Identity) => self.sh.registry.remove(&fp),          // annuncio incoerente col certificato
            _ => self.announce(false).await,                                       // ripiego UDP previsto dalla specifica
        }
    }

    pub async fn announce(&self, announce: bool) {
        if !self.multicast { return; }
        let mut msg = self.sh.info(true);
        msg["announce"] = json!(announce);
        let data = serde_json::to_vec(&msg).unwrap();
        let dest = SocketAddr::from(SocketAddrV4::new(MCAST, self.sh.port()));
        let ips = local_ipv4();
        let mut sent = false;
        for ip in &ips {
            let Ok(s) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) else { continue };
            let ok = s.set_multicast_if_v4(ip).is_ok() && s.set_multicast_ttl_v4(1).is_ok() && s.set_nonblocking(true).is_ok()
                && s.bind(&SocketAddr::from(SocketAddrV4::new(*ip, 0)).into()).is_ok();
            if !ok { continue; }
            if let Ok(u) = UdpSocket::from_std(s.into()) { if u.send_to(&data, dest).await.is_ok() { sent = true; } }
        }
        if !sent {
            if let Ok(u) = UdpSocket::bind("0.0.0.0:0").await { let _ = u.send_to(&data, dest).await; }
        }
    }

    async fn periodic(self: Arc<Self>) {
        self.announce(true).await;
        let mut ips: BTreeSet<Ipv4Addr> = local_ipv4().into_iter().collect();
        let mut tick = 0u32;
        loop {
            tokio::select! { _ = self.cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
            tick += 1;
            let now: BTreeSet<Ipv4Addr> = local_ipv4().into_iter().collect();
            if now != ips {                        // cambio di rete: riannuncia e riaggancia i gruppi
                ips = now;
                self.rete_cambiata();
                self.sh.bus.emit("rete");
            }
            if tick % 12 == 0 {                    // ogni minuto
                self.announce(true).await;
                self.sh.registry.prune();
                self.sh.bus.emit("dispositivi");
            }
        }
    }

    /// Annuncia e prova direttamente un indirizzo (o il gateway, utile con l'hotspot).
    pub async fn scan(&self, ip: Option<&str>, port: Option<u16>) -> usize {
        self.announce(true).await;
        let mut targets = vec![];
        match ip {
            Some(i) => match i.parse::<Ipv4Addr>() { Ok(a) => targets.push(a.to_string()), Err(_) => return 0 },
            None => if let Some(g) = gateway() { targets.push(g) },
        }
        let mut found = 0;
        for t in targets {
            let Ok(tls) = self.sh.client_tls() else { continue };
            let port = port.unwrap_or(self.sh.port());
            let Ok((200, Some(resp), actual)) = json_request(tls, &t, port, Method::POST, &format!("{API}/register"), Some(&self.sh.info(true)), Duration::from_secs(3), None).await else { continue };
            // in HTTPS l'identità è l'impronta del certificato appena visto, non il JSON
            let mut info = resp.clone();
            info["fingerprint"] = json!(actual);
            if let Some(d) = self.sh.registry.upsert(&info, &t, "manuale", Some(port)) {
                self.sh.registry.mark_verified(&d.fp);
                found += 1;
            }
        }
        found
    }
}

fn gateway() -> Option<String> {
    let t = std::fs::read_to_string("/proc/net/route").ok()?;
    for l in t.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() > 2 && f[1] == "00000000" {
            let g = u32::from_str_radix(f[2], 16).ok()?;
            return Some(Ipv4Addr::from(g.to_le_bytes()).to_string());
        }
    }
    None
}
