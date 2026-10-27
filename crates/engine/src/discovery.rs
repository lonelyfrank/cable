//! Scoperta LocalSend: annunci multicast, registrazione HTTPS e inserimento manuale.
use crate::client::{connect_any, json_req, json_request, ClientError};
use crate::protocol::{parse_peer_info, API};
use crate::shared::Shared;
use hyper::Method;
use serde_json::{json, Value};
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const MCAST: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

/// Eventi che rendono sospetta la presenza dei dispositivi: arrivano da D-Bus (`netwatch`), dal controllo periodico
/// delle interfacce o dal rilevamento del salto d'orologio dopo una sospensione.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetEvent { Network, Resume }

/// Interfacce di contenitori e macchine virtuali: non portano dispositivi LocalSend e sporcano elenco e annunci.
pub fn is_virtual_iface(name: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "vboxnet", "vmnet", "lxcbr", "lxdbr", "podman", "cni", "flannel", "cali", "kube", "tailscale"]
        .iter().any(|p| name.starts_with(p))
}
/// Tunnel/VPN: nessuna scoperta multicast e nessun «gateway del telefono» da cercare lì.
pub fn is_tunnel_iface(name: &str) -> bool { ["tun", "tap", "wg", "ppp", "vpn", "ipsec", "zt"].iter().any(|p| name.starts_with(p)) }

#[derive(Clone, Debug, PartialEq)]
pub struct LocalNet { pub name: String, pub ip: Ipv4Addr, pub mask: Ipv4Addr }

pub fn local_nets() -> Vec<LocalNet> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter(|i| !i.is_loopback()).filter(|i| !is_virtual_iface(&i.name))
        .filter_map(|i| match i.addr { if_addrs::IfAddr::V4(a) if !a.ip.is_link_local() => Some(LocalNet { name: i.name.clone(), ip: a.ip, mask: a.netmask }), _ => None }).collect()
}

pub fn local_ipv4() -> Vec<Ipv4Addr> { local_nets().into_iter().map(|n| n.ip).collect() }

pub fn in_local_subnet(ip: &str, nets: &[LocalNet]) -> bool {
    let Ok(ip) = ip.parse::<Ipv4Addr>() else { return false };
    let (x, m) = (u32::from(ip), |n: &LocalNet| u32::from(n.mask));
    nets.iter().any(|n| x & m(n) == u32::from(n.ip) & m(n))
}

/// Gateway predefiniti da `/proc/net/route`, dal metrico più basso; esclude tunnel, contenitori e VM.
/// (Con l'hotspot del telefono, il gateway è il telefono.)
pub fn parse_gateways(route_table: &str) -> Vec<String> {
    let mut v: Vec<(u32, String)> = vec![];
    for l in route_table.lines().skip(1) {
        let f: Vec<&str> = l.split_whitespace().collect();
        if f.len() < 8 || f[1] != "00000000" || is_virtual_iface(f[0]) || is_tunnel_iface(f[0]) { continue; }
        let (Ok(g), Ok(metric)) = (u32::from_str_radix(f[2], 16), f[6].parse::<u32>()) else { continue };
        if g == 0 { continue; }
        let gw = Ipv4Addr::from(g.to_le_bytes()).to_string();
        if !v.iter().any(|(_, x)| *x == gw) { v.push((metric, gw)); }
    }
    v.sort();
    v.into_iter().map(|x| x.1).collect()
}

fn gateways() -> Vec<String> { parse_gateways(&std::fs::read_to_string("/proc/net/route").unwrap_or_default()) }

pub struct Discovery {
    sh: Arc<Shared>,
    multicast: bool,
    cancel: CancellationToken,
    pub errore: Mutex<String>,
    last_reply: Mutex<HashMap<IpAddr, Instant>>,
    joined: Mutex<BTreeSet<Ipv4Addr>>,
    sock: Mutex<Option<Arc<UdpSocket>>>,
    ev_tx: mpsc::UnboundedSender<NetEvent>,
    ev_rx: Mutex<Option<mpsc::UnboundedReceiver<NetEvent>>>,
    /// quante volte la rete è stata ritenuta cambiata/ripresa (diagnostica e test)
    pub cambi: AtomicU32,
    /// eventi D-Bus: 0 = non noto, 1 = in ascolto, 2 = non disponibili (resta il controllo periodico)
    pub dbus: crate::netwatch::Status,
}

impl Discovery {
    pub fn new(sh: Arc<Shared>, multicast: bool) -> Arc<Discovery> {
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        Arc::new(Discovery { sh, multicast, cancel: CancellationToken::new(), errore: Mutex::new(String::new()),
                             last_reply: Mutex::new(HashMap::new()), joined: Mutex::new(BTreeSet::new()), sock: Mutex::new(None),
                             ev_tx, ev_rx: Mutex::new(Some(ev_rx)), cambi: AtomicU32::new(0), dbus: Default::default() })
    }

    pub fn start(self: &Arc<Self>) {
        if self.multicast { let me = self.clone(); tokio::spawn(async move { me.listen().await }); }
        if let Some(rx) = self.ev_rx.lock().unwrap().take() {
            let me = self.clone();
            tokio::spawn(async move { me.periodic(rx).await });
        }
        // D-Bus (NetworkManager, logind) come acceleratore; senza, restano il controllo periodico e il salto d'orologio
        if self.multicast { crate::netwatch::spawn(self.ev_tx.clone(), self.cancel.clone(), self.dbus.clone()); }
    }

    pub fn stop(&self) { self.cancel.cancel(); }

    /// Segnala un cambio di rete o una ripresa (da D-Bus, dall'utente o dai test). Gli eventi ravvicinati si accorpano.
    pub fn notifica(&self, ev: NetEvent) { let _ = self.ev_tx.send(ev); }

    /// Dopo un cambio rete o la ripresa dalla sospensione: la presenza dei dispositivi non è più attendibile,
    /// si riaggancia il multicast, si riannuncia e si riverificano i dispositivi ricordati.
    pub fn rete_cambiata(self: &Arc<Self>, ev: NetEvent) {
        self.cambi.fetch_add(1, Ordering::Relaxed);
        match ev {
            NetEvent::Resume => self.sh.registry.expire_presence(None),
            NetEvent::Network => {
                let nets = local_nets();
                self.sh.registry.expire_presence(Some(&|ip: &str| in_local_subnet(ip, &nets)));
            }
        }
        self.joined.lock().unwrap().clear();
        self.join();
        self.sh.bus.emit("rete");
        let me = self.clone();
        tokio::spawn(async move { me.refresh().await; });
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
            Ok((200, _, _)) => self.sh.registry.mark_verified(&fp, Some(&ip.to_string())),
            // annuncio incoerente col certificato: quell'indirizzo non è di questo dispositivo (il dispositivo vero resta in elenco)
            Err(ClientError::Identity) => self.sh.registry.drop_addr(&fp, &ip.to_string()),
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

    async fn periodic(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<NetEvent>) {
        self.announce(true).await;
        let mut nets: BTreeSet<Ipv4Addr> = local_ipv4().into_iter().collect();
        let mut tick = 0u32;
        // la sospensione ferma l'orologio monotonico ma non quello di sistema: la differenza rivela la ripresa
        let (mut wall, mut mono) = (SystemTime::now(), Instant::now());
        loop {
            let mut ev: Option<NetEvent> = None;
            tokio::select! {
                _ = self.cancel.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                e = rx.recv() => match e {
                    None => return,
                    Some(e) => {
                        ev = Some(e);
                        // accorpa la raffica: si agisce quando è passato un istante senza nuovi eventi
                        loop {
                            match tokio::time::timeout(Duration::from_millis(1500), rx.recv()).await {
                                Ok(Some(NetEvent::Resume)) => ev = Some(NetEvent::Resume),
                                Ok(Some(_)) => {}
                                _ => break,
                            }
                        }
                    }
                }
            }
            tick += 1;
            let slept = SystemTime::now().duration_since(wall).unwrap_or_default().saturating_sub(mono.elapsed());
            (wall, mono) = (SystemTime::now(), Instant::now());
            if slept > Duration::from_secs(20) { ev = Some(NetEvent::Resume); }
            let now: BTreeSet<Ipv4Addr> = local_ipv4().into_iter().collect();
            match ev {
                Some(NetEvent::Resume) => { nets = now; self.rete_cambiata(NetEvent::Resume); }
                // un evento di rete senza cambio di indirizzi (es. proprietà di NetworkManager) non richiede nulla
                Some(NetEvent::Network) if now == nets => {}
                _ if now != nets => { nets = now; self.rete_cambiata(NetEvent::Network); }
                _ => {}
            }
            if tick % 12 == 0 {                    // ogni minuto
                self.announce(true).await;
                self.sh.registry.prune();
                self.sh.bus.emit("dispositivi");
            }
        }
    }

    /// Prova a raggiungere un dispositivo noto su tutti i suoi indirizzi (in parallelo, tempo breve). Se risponde con
    /// l'impronta giusta è «raggiungibile» e «visto ora»; altrimenti è segnato come non raggiungibile. Non cambia mai
    /// la fiducia: conferma solo che *questo* certificato risponde a *quell'*indirizzo.
    pub async fn probe(&self, fp: &str) -> bool {
        let Some(dev) = self.sh.registry.get(fp) else { return false };
        let cands: Vec<(String, u16)> = dev.candidates().into_iter().take(3).map(|a| (a.ip, a.port)).collect();
        let make = || self.sh.client_tls();
        match connect_any(&make, &cands, Duration::from_secs(2), &dev.fp).await {
            Ok((mut c, ip, port)) => {
                self.sh.registry.mark_verified(&dev.fp, Some(&ip));
                let req = json_req(&ip, port, Method::POST, &format!("{API}/register"), Some(&self.sh.info(true)));
                if let Ok(Ok((200, Some(mut info)))) = tokio::time::timeout(Duration::from_secs(2), c.send(req)).await {
                    info["fingerprint"] = json!(dev.fp);          // in HTTPS l'identità è il certificato appena visto, non il JSON
                    self.sh.registry.upsert(&info, &ip, "probe", Some(port));
                }
                true
            }
            Err(_) => { self.sh.registry.mark_unreachable(&dev.fp); false }
        }
    }

    /// «Aggiorna»: riannuncia e riverifica fino a 16 dispositivi (vicini e ricordati), in parallelo.
    pub async fn refresh(self: &Arc<Self>) -> usize {
        self.announce(true).await;
        let mut fps: Vec<String> = self.sh.registry.snapshot().as_array().cloned().unwrap_or_default().iter()
            .filter(|d| d["indirizzi"].as_u64().unwrap_or(0) > 0 || !d["ip"].as_str().unwrap_or("").is_empty())
            .filter_map(|d| d["fp"].as_str().map(String::from)).collect();
        fps.truncate(16);
        self.sh.registry.seed_remembered();
        let mut set = FuturesUnordered::new();
        for fp in fps { let me = self.clone(); set.push(async move { me.probe(&fp).await }); }
        let mut ok = 0;
        while let Some(r) = set.next().await { if r { ok += 1; } }
        self.sh.bus.emit("dispositivi");
        ok
    }

    /// Annuncia e prova direttamente un indirizzo (o, senza indirizzo, i gateway delle reti locali: utile con
    /// l'hotspot del telefono). Con più reti attive (Wi-Fi, Ethernet…) si provano tutti i gateway, non il primo.
    pub async fn scan(&self, ip: Option<&str>, port: Option<u16>) -> usize {
        self.announce(true).await;
        let mut targets = vec![];
        match ip {
            Some(i) => match i.parse::<Ipv4Addr>() { Ok(a) => targets.push(a.to_string()), Err(_) => return 0 },
            None => targets.extend(gateways().into_iter().take(4)),
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
                self.sh.registry.mark_verified(&d.fp, Some(&t));
                found += 1;
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_and_tunnel_interfaces() {
        for n in ["docker0", "br-1a2b", "veth123", "virbr0", "podman1", "cni0", "tailscale0"] { assert!(is_virtual_iface(n), "{n}"); }
        for n in ["wlp2s0", "enp3s0", "eth0", "wlan0", "enx0011", "usb0", "tun0", "wg0"] { assert!(!is_virtual_iface(n), "{n}"); }
        for n in ["tun0", "wg0", "ppp0", "tap1"] { assert!(is_tunnel_iface(n), "{n}"); }
        assert!(!is_tunnel_iface("wlp2s0") && !is_tunnel_iface("enp3s0"));
    }

    #[test]
    fn gateways_are_sorted_by_metric_and_skip_vpn_and_containers() {
        let t = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                 tun0\t00000000\t0100000A\t0003\t0\t0\t50\t00000000\t0\t0\t0\n\
                 wlp2s0\t00000000\t0100A8C0\t0003\t0\t0\t600\t00000000\t0\t0\t0\n\
                 enp3s0\t00000000\t0200A8C0\t0003\t0\t0\t100\t00000000\t0\t0\t0\n\
                 docker0\t00000000\t010011AC\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
                 wlp2s0\t0000A8C0\t00000000\t0001\t0\t0\t600\t00FFFFFF\t0\t0\t0\n";
        assert_eq!(parse_gateways(t), ["192.168.0.2", "192.168.0.1"], "Ethernet (metrico 100) prima del Wi-Fi (600); niente VPN né Docker");
        assert!(parse_gateways("").is_empty());
        assert!(parse_gateways("Iface\tDestination\nbroken line").is_empty());
    }

    #[test]
    fn subnet_membership() {
        let nets = vec![LocalNet { name: "wlp2s0".into(), ip: "192.168.1.20".parse().unwrap(), mask: "255.255.255.0".parse().unwrap() }];
        assert!(in_local_subnet("192.168.1.77", &nets));
        assert!(!in_local_subnet("192.168.2.77", &nets));
        assert!(!in_local_subnet("non-un-ip", &nets));
    }
}
