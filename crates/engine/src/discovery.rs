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

/// Come un'interfaccia partecipa alla rete di Cable.
///
/// Politica (una sola, usata da join multicast, annunci, sottoreti locali, gateway e diagnostica):
/// * **Scoperta automatica** (multicast): solo interfacce `Lan` (Wi-Fi, Ethernet, hotspot, USB tethering).
///   Tunnel VPN e interfacce virtuali sono escluse: non portano telefoni e gli annunci non devono uscire in una VPN.
/// * **Connessioni esplicite** (IP digitato, dispositivo ricordato, invio): nessun filtro per interfaccia.
///   Il percorso lo sceglie il sistema operativo; Cable non blocca un indirizzo perché passa da un tunnel.
/// * Nessun ripiego sull'interfaccia predefinita: con una VPN potrebbe essere proprio il tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfaceKind { Lan, Virtual, Tunnel }

/// Interfacce di contenitori e macchine virtuali: non portano dispositivi LocalSend e sporcano elenco e annunci.
pub fn is_virtual_iface(name: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "vnet", "macvtap", "vboxnet", "vmnet", "lxcbr", "lxdbr", "incusbr", "podman", "cni", "flannel", "cali", "kube", "cilium", "weave", "waydroid", "dummy"]
        .iter().any(|p| name.starts_with(p))
}
/// Tunnel/VPN: nessuna scoperta multicast e nessun «gateway del telefono» da cercare lì.
pub fn is_tunnel_iface(name: &str) -> bool {
    ["tun", "tap", "wg", "ppp", "vpn", "ipsec", "zt", "tailscale", "nordlynx", "proton", "mullvad", "utun", "gre", "ipip", "sit"].iter().any(|p| name.starts_with(p))
}
pub fn classify_iface(name: &str) -> IfaceKind {
    if is_tunnel_iface(name) { IfaceKind::Tunnel } else if is_virtual_iface(name) { IfaceKind::Virtual } else { IfaceKind::Lan }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalNet { pub name: String, pub ip: Ipv4Addr, pub mask: Ipv4Addr }

/// Un indirizzo IPv4 locale con l'esito della politica e la ragione, per la diagnostica.
#[derive(Clone, Debug, PartialEq)]
pub struct IfaceInfo { pub name: String, pub ip: Ipv4Addr, pub mask: Ipv4Addr, pub usata: bool, pub motivo: &'static str }

/// Applica la politica a un elenco di (nome, ip, maschera, loopback). Pura: testabile senza toccare la rete.
pub fn classify_addrs(addrs: impl IntoIterator<Item = (String, Ipv4Addr, Ipv4Addr, bool)>) -> Vec<IfaceInfo> {
    addrs.into_iter().map(|(name, ip, mask, lo)| {
        let (usata, motivo) = if lo || ip.is_loopback() { (false, "indirizzo interno del computer") }
            else if ip.is_link_local() { (false, "indirizzo 169.254.x.x: la rete non ha assegnato un indirizzo") }
            else { match classify_iface(&name) {
                IfaceKind::Lan => (true, "rete locale (Wi-Fi, Ethernet o hotspot)"),
                IfaceKind::Tunnel => (false, "tunnel VPN: la scoperta automatica non lo attraversa (l'IP a mano funziona comunque)"),
                IfaceKind::Virtual => (false, "interfaccia virtuale di contenitori o macchine virtuali"),
            } };
        IfaceInfo { name, ip, mask, usata, motivo }
    }).collect()
}

/// Tutti gli indirizzi IPv4 del sistema, ciascuno con l'esito della politica.
pub fn interfaces() -> Vec<IfaceInfo> {
    classify_addrs(if_addrs::get_if_addrs().unwrap_or_default().into_iter().filter_map(|i| {
        let lo = i.is_loopback();
        match i.addr { if_addrs::IfAddr::V4(a) => Some((i.name.clone(), a.ip, a.netmask, lo)), _ => None }
    }))
}

/// Reti candidate alla scoperta automatica (solo `IfaceKind::Lan`).
pub fn local_nets() -> Vec<LocalNet> {
    interfaces().into_iter().filter(|i| i.usata).map(|i| LocalNet { name: i.name, ip: i.ip, mask: i.mask }).collect()
}

pub fn local_ipv4() -> Vec<Ipv4Addr> { local_nets().into_iter().map(|n| n.ip).collect() }

/// Righe di diagnostica: interfacce usate ed escluse, con la ragione.
pub fn interfaces_report(all: &[IfaceInfo]) -> Vec<String> {
    let usate: Vec<String> = all.iter().filter(|i| i.usata).map(|i| format!("{} {}", i.name, i.ip)).collect();
    let mut l = vec![format!("Interfacce usate per la scoperta: {}", if usate.is_empty() { "nessuna".to_string() } else { usate.join(", ") })];
    let escluse: Vec<String> = all.iter().filter(|i| !i.usata && !i.ip.is_loopback()).map(|i| format!("{} {} ({})", i.name, i.ip, i.motivo)).collect();
    if !escluse.is_empty() { l.push(format!("Interfacce escluse: {}", escluse.join("; "))); }
    l
}

/// Quali membership multicast lasciare e quali prendere per passare da `joined` a `desired`.
/// Con `force` si lascia e si riprende tutto: dopo un cambio rete o una sospensione le membership possono essere
/// decadute senza che il processo se ne accorga.
pub fn plan_membership(joined: &BTreeSet<Ipv4Addr>, desired: &BTreeSet<Ipv4Addr>, force: bool) -> (Vec<Ipv4Addr>, Vec<Ipv4Addr>) {
    if force { return (joined.iter().copied().collect(), desired.iter().copied().collect()); }
    (joined.difference(desired).copied().collect(), desired.difference(joined).copied().collect())
}

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
        self.sync_membership(true);
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

    /// Allinea le membership multicast alle interfacce LAN attuali: lascia quelle sparite, prende le nuove.
    /// Nessun ripiego su 0.0.0.0 (lascerebbe scegliere al sistema l'interfaccia predefinita, magari una VPN).
    fn join(&self) { self.sync_membership(false); }

    fn sync_membership(&self, force: bool) {
        let Some(s) = self.sock.lock().unwrap().clone() else { return };
        let desired: BTreeSet<Ipv4Addr> = local_ipv4().into_iter().collect();
        let mut j = self.joined.lock().unwrap();
        let (leave, join) = plan_membership(&j, &desired, force);
        for ip in leave { let _ = s.leave_multicast_v4(MCAST, ip); j.remove(&ip); }
        for ip in join { if s.join_multicast_v4(MCAST, ip).is_ok() { j.insert(ip); } }
    }

    /// Interfacce sulle quali si è effettivamente in ascolto (diagnostica e test).
    pub fn membership(&self) -> Vec<Ipv4Addr> { self.joined.lock().unwrap().iter().copied().collect() }

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
        for ip in &ips {
            let Ok(s) = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP)) else { continue };
            let ok = s.set_multicast_if_v4(ip).is_ok() && s.set_multicast_ttl_v4(1).is_ok() && s.set_nonblocking(true).is_ok()
                && s.bind(&SocketAddr::from(SocketAddrV4::new(*ip, 0)).into()).is_ok();
            if !ok { continue; }
            if let Ok(u) = UdpSocket::from_std(s.into()) { let _ = u.send_to(&data, dest).await; }
        }
        // nessun ripiego sull'interfaccia predefinita: senza rete locale non si annuncia nulla (e mai dentro una VPN)
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
        for n in ["docker0", "br-1a2b", "veth123", "virbr0", "vnet3", "podman1", "cni0", "lxdbr0"] { assert!(is_virtual_iface(n), "{n}"); }
        for n in ["wlp2s0", "enp3s0", "eth0", "wlan0", "enx0011", "usb0", "br0", "tun0", "wg0"] { assert!(!is_virtual_iface(n), "{n}"); }
        for n in ["tun0", "wg0", "ppp0", "tap1", "tailscale0", "zt5u4abcde", "nordlynx", "proton0"] { assert!(is_tunnel_iface(n), "{n}"); }
        for n in ["wlp2s0", "enp3s0", "br0", "usb0", "docker0"] { assert!(!is_tunnel_iface(n), "{n}"); }
    }

    fn net(name: &str, ip: &str) -> (String, Ipv4Addr, Ipv4Addr, bool) { (name.into(), ip.parse().unwrap(), "255.255.255.0".parse().unwrap(), name == "lo") }
    fn used(v: &[IfaceInfo]) -> Vec<&str> { v.iter().filter(|i| i.usata).map(|i| i.name.as_str()).collect() }

    #[test]
    fn policy_keeps_lan_and_excludes_tunnels_and_virtual_ifaces() {
        // Wi-Fi, Ethernet e hotspot restano
        assert_eq!(used(&classify_addrs([net("wlp2s0", "192.168.1.5")])), ["wlp2s0"]);
        assert_eq!(used(&classify_addrs([net("enp3s0", "10.0.0.5")])), ["enp3s0"]);
        assert_eq!(used(&classify_addrs([net("wlan0", "10.42.0.1")])), ["wlan0"], "hotspot del PC (NetworkManager)");
        assert_eq!(used(&classify_addrs([net("enp0s20f0u1", "192.168.42.129")])), ["enp0s20f0u1"], "USB tethering del telefono");
        // Docker/Podman/bridge virtuali/tun/tap/Tailscale/WireGuard: esclusi
        for n in ["docker0", "br-9f3a", "podman1", "virbr0", "veth1a", "cni-podman0", "tun0", "tap0", "tailscale0", "wg0", "nordlynx"] {
            let r = classify_addrs([net(n, "172.17.0.1")]);
            assert!(used(&r).is_empty(), "{n} non deve essere usata");
            assert!(!r[0].motivo.is_empty());
        }
        // loopback e link-local non sono mai candidati
        let r = classify_addrs([net("lo", "127.0.0.1"), net("wlp2s0", "169.254.3.4")]);
        assert!(used(&r).is_empty());
    }

    #[test]
    fn lan_plus_vpn_keeps_the_lan_working() {
        let r = classify_addrs([net("lo", "127.0.0.1"), net("wlp2s0", "192.168.1.5"), net("tun0", "10.8.0.2"), net("tailscale0", "100.64.0.9"), net("wg0", "10.66.0.2"), net("docker0", "172.17.0.1")]);
        assert_eq!(used(&r), ["wlp2s0"], "la VPN attiva non spegne la LAN e non entra nella scoperta");
        let rep = interfaces_report(&r);
        assert!(rep[0].contains("wlp2s0 192.168.1.5") && !rep[0].contains("tun0"), "{rep:?}");
        assert!(rep[1].contains("tun0 10.8.0.2") && rep[1].contains("tunnel VPN") && rep[1].contains("docker0") && rep[1].contains("virtuale"), "{rep:?}");
        assert!(!rep[1].contains("127.0.0.1"));
        // solo VPN: nessuna interfaccia di scoperta, ma lo si dice (l'IP a mano resta possibile)
        let rep = interfaces_report(&classify_addrs([net("wg0", "10.66.0.2")]));
        assert!(rep[0].ends_with("nessuna"));
    }

    #[test]
    fn explicit_addresses_are_not_filtered_by_interface_policy() {
        // la politica riguarda solo l'elenco delle interfacce di scoperta: un IP digitato (anche dietro un tunnel) si prova sempre.
        // `scan(Some(ip))` accetta qualunque IPv4 e `connect_any` non consulta mai `local_nets()` (provato in tests/e2e.rs con 127.0.0.1,
        // che non è una rete locale per la politica).
        let r = classify_addrs([net("tun0", "10.8.0.2")]);
        assert!(used(&r).is_empty());
        assert!(!in_local_subnet("10.8.0.9", &[]), "in_local_subnet serve solo a far decadere la presenza, non a vietare connessioni");
    }

    #[test]
    fn membership_plan_follows_interface_changes() {
        let set = |v: &[&str]| v.iter().map(|s| s.parse::<Ipv4Addr>().unwrap()).collect::<BTreeSet<_>>();
        let (wifi, eth, vpn) = ("192.168.1.5", "10.0.0.5", "10.8.0.2");
        // avvio
        assert_eq!(plan_membership(&set(&[]), &set(&[wifi]), false), (vec![], vec![wifi.parse().unwrap()]));
        // compare la VPN: non è nei desiderata, nessun cambiamento
        assert_eq!(plan_membership(&set(&[wifi]), &set(&[wifi]), false), (vec![], vec![]));
        // cambio rete: Wi-Fi sparisce, Ethernet arriva → si lascia la vecchia e si prende la nuova
        assert_eq!(plan_membership(&set(&[wifi]), &set(&[eth]), false), (vec![wifi.parse().unwrap()], vec![eth.parse().unwrap()]));
        // rimozione di tutte le interfacce
        assert_eq!(plan_membership(&set(&[wifi, eth]), &set(&[]), false), (vec![eth.parse().unwrap(), wifi.parse().unwrap()], vec![]));
        // dopo sospensione/cambio: tutto lasciato e ripreso, anche se l'indirizzo è lo stesso
        assert_eq!(plan_membership(&set(&[wifi]), &set(&[wifi]), true), (vec![wifi.parse().unwrap()], vec![wifi.parse().unwrap()]));
        // mai 0.0.0.0 e mai una VPN: il pianificatore prende solo ciò che gli si chiede
        assert!(!plan_membership(&set(&[]), &set(&[wifi]), true).1.contains(&vpn.parse().unwrap()));
        assert!(!plan_membership(&set(&[]), &set(&[wifi]), true).1.contains(&Ipv4Addr::UNSPECIFIED));
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
        let vpn = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT\n\
                   wg0\t00000000\t0100000A\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
                   tailscale0\t00000000\t0100000A\t0003\t0\t0\t1\t00000000\t0\t0\t0\n";
        assert_eq!(vpn.lines().skip(1).map(|l| l.split_whitespace().count()).collect::<Vec<_>>(), [11, 11], "righe ben formate: solo il nome dell'interfaccia le fa scartare");
        assert!(parse_gateways(vpn).is_empty(), "WireGuard e Tailscale non sono gateway di un telefono");
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
