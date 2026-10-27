//! Facade del motore: unisce impostazioni, identità, rete, ricezione, invio e wizard.
//! Non dipende da GUI. `snapshot()` fa piccole letture bloccanti: dalla UI chiamarlo in un thread di lavoro.
use crate::airplay::AirPlay;
use crate::android::Android;
use crate::bus::{Bus, Notifier};
use crate::desktop;
use crate::discovery::{local_ipv4, Discovery};
use crate::paths::Paths;
use crate::protocol::Limits;
use crate::receiver::Receiver;
use crate::sender::Sender;
use crate::server::{self, ServerHandle};
use crate::shared::Shared;
use crate::util::{now_secs, tipo_file};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CONF_KEYS: [&str; 10] = ["nome_pc", "cartella", "cartella_foto", "vetro", "tema", "notifiche", "schermo_intero", "background", "avvio_login", "controlla_aggiornamenti"];

struct Challenge { fp: String, alias: String, ip: String, port: u16, code: String, created: Instant, tries: u8 }

#[derive(Default)]
struct Wizard {
    pending: Option<Challenge>,
    job: Option<String>,
    confermato_fp: Option<String>,
    prova: Option<(String, f64)>,        // (ip del dispositivo, istante di inizio)
}

pub struct Engine {
    pub sh: Arc<Shared>,
    pub receiver: Arc<Receiver>,
    pub sender: Arc<Sender>,
    pub discovery: Arc<Discovery>,
    pub android: Arc<Android>,
    pub airplay: Arc<AirPlay>,
    server: tokio::sync::Mutex<Option<ServerHandle>>,
    server_error: Mutex<String>,
    pub(crate) cancel: CancellationToken,
    pub(crate) agg: Mutex<crate::aggiornamenti::Stato>,
    /// senza rete verso l'esterno (prove, sandbox): niente controllo degli aggiornamenti
    pub(crate) isolato: bool,
    cache: Mutex<HashMap<&'static str, (Instant, Value)>>,
    wizard: Mutex<Wizard>,
}

impl Engine {
    pub fn new(paths: Paths, port: u16, multicast: bool) -> Arc<Engine> {
        Engine::with_limits(paths, port, multicast, Limits::default())
    }

    pub fn with_limits(paths: Paths, port: u16, multicast: bool, limits: Limits) -> Arc<Engine> {
        let sh = Shared::new(paths, port);
        let receiver = Receiver::new(sh.settings.clone(), sh.bus.clone(), limits, sh.notifier.clone());
        Arc::new(Engine {
            receiver, sender: Sender::new(sh.clone()), discovery: Discovery::new(sh.clone(), multicast),
            android: Android::new(sh.settings.clone(), sh.bus.clone()), airplay: AirPlay::new(sh.settings.clone(), sh.bus.clone()),
            server: tokio::sync::Mutex::new(None), server_error: Mutex::new(String::new()), cancel: CancellationToken::new(),
            agg: Mutex::new(Default::default()), isolato: !multicast,
            cache: Mutex::new(HashMap::new()), wizard: Mutex::new(Wizard::default()), sh,
        })
    }

    pub fn bus(&self) -> &Bus { &self.sh.bus }
    pub fn notifier(&self) -> &Arc<Notifier> { &self.sh.notifier }
    pub fn port(&self) -> u16 { self.sh.port() }

    // ------------------------------------------------------------------ ciclo di vita
    pub async fn start(self: &Arc<Self>) -> bool {
        for k in ["cartella", "cartella_foto"] { let _ = std::fs::create_dir_all(self.sh.settings.str(k)); }
        self.receiver.pulisci_residui();
        desktop::migra_avvio_automatico(&self.sh.paths);
        let ok = self.avvia_server().await;
        self.sh.registry.seed_remembered();
        self.discovery.start();
        self.android.start();
        self.avvia_controllo_aggiornamenti();
        // dopo ogni cambio di rete/ripresa: svuota le cache di rete, riavvia il server se era caduto, riverifica i dispositivi
        let me = self.clone();
        tokio::spawn(async move {
            let mut rx = me.sh.bus.subscribe();
            loop {
                let t = tokio::select! { _ = me.cancel.cancelled() => return, t = rx.recv() => t };
                match t {
                    Ok("rete") | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        { let mut c = me.cache.lock().unwrap(); c.remove("rete_ip"); c.remove("rete_nome"); }
                        if !me.server_attivo() { me.avvia_server().await; me.sh.bus.emit("server"); }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    _ => {}
                }
            }
        });
        let d = self.discovery.clone();
        tokio::spawn(async move { d.refresh().await; });
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! { _ = me.cancel.cancelled() => return, _ = tokio::time::sleep(Duration::from_secs(5)) => {} }
                me.receiver.sweep();
                me.sender.prune();
            }
        });
        ok
    }

    async fn avvia_server(&self) -> bool {
        let mut g = self.server.lock().await;
        if g.is_some() { return true; }
        if self.sh.identity.read().unwrap().stato != "ok" { *self.server_error.lock().unwrap() = "identita".into(); return false; }
        match server::start(self.sh.clone(), self.receiver.clone(), self.sh.port()).await {
            Ok(h) => { *g = Some(h); self.server_error.lock().unwrap().clear(); self.sh.bus.emit("server"); true }
            Err(e) => {
                *self.server_error.lock().unwrap() = if e.kind() == std::io::ErrorKind::AddrInUse || e.kind() == std::io::ErrorKind::PermissionDenied { "porta".into() } else { e.to_string() };
                crate::dlog!("server non avviato: {e}");
                false
            }
        }
    }

    pub async fn riprova_server(&self) -> bool { let ok = self.avvia_server().await; self.sh.bus.emit("server"); ok }

    pub async fn stop(&self) {
        self.cancel.cancel();
        self.discovery.stop();
        self.android.stop();
        self.airplay.ferma();
        if let Some(h) = self.server.lock().await.take() { h.stop().await; }
    }

    fn server_attivo(&self) -> bool { self.server.try_lock().map(|g| g.is_some()).unwrap_or(true) }

    /// «Aggiorna» (manuale): riannuncia, riverifica i dispositivi noti su tutti i loro indirizzi e, se serve, riavvia il server.
    /// Restituisce quanti dispositivi hanno risposto.
    pub async fn aggiorna(self: &Arc<Self>) -> usize {
        { let mut c = self.cache.lock().unwrap(); c.remove("rete_ip"); c.remove("rete_nome"); }
        if !self.server_attivo() { self.avvia_server().await; }
        let n = self.discovery.refresh().await;
        self.sh.bus.emit("server");
        n
    }

    // ------------------------------------------------------------------ azioni
    pub async fn cerca(&self, ip: Option<&str>, port: Option<u16>) -> usize {
        let n = self.discovery.scan(ip, port).await;
        self.sh.bus.emit("dispositivi");
        n
    }

    pub fn invia(self: &Arc<Self>, fp: &str, percorsi: &[String], testo: Option<String>) -> Result<String, String> {
        let dev = self.sh.registry.get(fp).ok_or("Dispositivo non trovato: apri l'app compatibile (ad es. LocalSend) sul destinatario e premi «Aggiorna».")?;
        if self.sh.identity.read().unwrap().stato != "ok" { return Err("Identità non valida: rigenerala dalle Impostazioni.".into()); }
        let testo = testo.filter(|t| !t.trim().is_empty());
        self.sender.invia(&dev, percorsi, testo, false)
    }

    /// «Riprova i file non completati» di un invio terminato (nuovo invio verso lo stesso dispositivo).
    pub fn riprova(self: &Arc<Self>, jid: &str) -> Result<(String, Vec<String>), String> {
        if self.sh.identity.read().unwrap().stato != "ok" { return Err("Identità non valida: rigenerala dalle Impostazioni.".into()); }
        let fp = self.sender.fp_di(jid).ok_or("Trasferimento non trovato")?;
        self.sender.riprova(jid, self.sh.registry.get(&fp))
    }

    /// Un dispositivo ricordato si ripresenta con un'impronta diversa e l'utente decide che è lo stesso: il nuovo prende il
    /// posto del vecchio nell'elenco, senza ereditare la conferma.
    pub fn identita_sostituisci(&self, old_fp: &str, new_fp: &str) -> bool { self.sh.registry.replace_remembered(old_fp, new_fp) }
    /// L'utente ha letto l'avviso di cambio d'identità e non vuole altro: il vecchio record resta com'è.
    pub fn identita_ignora(&self, new_fp: &str) -> bool { self.sh.registry.dismiss_change(new_fp) }

    pub fn decidi(&self, rid: &str, accetta: Option<Vec<String>>) -> bool { self.receiver.decide(rid, accetta) }

    /// La stella è solo una scorciatoia (ordine e accesso rapido): non autorizza nulla, non conferma l'identità.
    pub fn preferito(&self, fp: &str, on: bool) -> bool {
        let d = self.sh.registry.get(fp);
        let ok = self.sh.settings.preferito(fp, on, d.as_ref().map(|d| (d.alias.as_str(), d.ip.as_str(), d.port)));
        self.sh.bus.emit("dispositivi");
        ok
    }

    /// Toglie il dispositivo dai preferiti e dall'elenco (ricomparirà se annuncia di nuovo).
    pub fn dimentica(&self, fp: &str) -> bool {
        self.sh.settings.dimentica(fp);
        self.sh.registry.remove(fp);
        true
    }

    pub fn aggiorna_impostazioni(self: &Arc<Self>, patch: &Map<String, Value>) -> Vec<String> {
        let patch: Map<String, Value> = patch.iter().filter(|(k, _)| CONF_KEYS.contains(&k.as_str()) || *k == "wizard_fatto").map(|(k, v)| (k.clone(), v.clone())).collect();
        let done = self.sh.settings.aggiorna(&patch);
        for k in ["cartella", "cartella_foto"] { if let Some(v) = done.get(k).and_then(Value::as_str) { let _ = std::fs::create_dir_all(v); } }
        if let Some(v) = done.get("avvio_login").and_then(Value::as_bool) { desktop::imposta_avvio(&self.sh.paths, v); }
        if let Some(v) = done.get("notifiche").and_then(Value::as_bool) { self.sh.notifier.enable(v); }
        if done.get("controlla_aggiornamenti").and_then(Value::as_bool) == Some(true) && !self.isolato {
            let me = self.clone(); tokio::spawn(async move { me.controlla_aggiornamenti().await });
        }
        if done.contains_key("nome_pc") { let d = self.discovery.clone(); tokio::spawn(async move { d.announce(true).await }); }
        self.sh.bus.emit("impostazioni");
        done.keys().cloned().collect()
    }

    pub fn imposta_pin(&self, pin: Option<&str>) -> bool { let ok = self.sh.settings.imposta_pin(pin); self.sh.bus.emit("impostazioni"); ok }

    pub fn ripristina_impostazioni(&self) -> String {
        let bk = self.sh.settings.ripristina();
        desktop::imposta_avvio(&self.sh.paths, false);
        self.sh.notifier.enable(self.sh.settings.bool("notifiche"));
        self.sh.registry.clear();
        self.sh.bus.emit("impostazioni");
        bk.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    }

    pub async fn rigenera_identita(self: &Arc<Self>) -> bool {
        if let Some(h) = self.server.lock().await.take() { h.stop().await; }     // il vecchio certificato non va più usato
        let r = self.sh.identity.write().unwrap().rigenera();
        if r.is_err() { return false; }
        self.sh.registry.set_own_fp(&self.sh.fingerprint());
        self.sh.registry.clear();
        let ok = self.avvia_server().await;
        self.discovery.announce(true).await;            // con il nuovo certificato
        self.sh.bus.emit("server");
        ok
    }

    // ------------------------------------------------------------------ wizard di primo collegamento
    /// Passo "verifica con codice": invia al telefono scelto un codice via LocalSend (connessione con
    /// impronta verificata). L'utente lo digita qui: prova che l'impronta appartiene al telefono che ha in mano.
    pub fn verifica_inizia(self: &Arc<Self>, fp: &str) -> Result<(), String> {
        let dev = self.sh.registry.get(fp).ok_or("Seleziona il telefono con LocalSend aperto e cifratura attiva.")?;
        let code = {
            let mut w = self.wizard.lock().unwrap();
            let reuse = w.pending.as_ref().filter(|c| c.fp == dev.fp && c.created.elapsed() < Duration::from_secs(300) && c.tries < 5).map(|c| c.code.clone());
            let code = reuse.unwrap_or_else(|| { use rand::Rng; rand::thread_rng().gen_range(100000..1_000_000).to_string() });
            w.pending = Some(Challenge { fp: dev.fp.clone(), alias: dev.alias.clone(), ip: dev.ip.clone(), port: dev.port, code: code.clone(), created: Instant::now(), tries: w.pending.as_ref().filter(|c| c.fp == dev.fp).map_or(0, |c| c.tries) });
            code
        };
        let msg = format!("Cable — codice di verifica: {code}. Inseriscilo solo in Cable sul PC che hai davanti. Scade tra 5 minuti.");
        let id = self.sender.invia(&dev, &[], Some(msg), true)?;
        self.wizard.lock().unwrap().job = Some(id);
        self.sh.bus.emit("wizard");
        Ok(())
    }

    pub fn verifica_conferma(&self, code: &str) -> Value {
        let mut w = self.wizard.lock().unwrap();
        let Some(c) = w.pending.as_mut() else { return json!({"ok": false, "motivo": "missing", "errore": "Nessun codice attivo: premi «Invia codice al telefono»."}) };
        if c.created.elapsed() > Duration::from_secs(300) { return json!({"ok": false, "motivo": "expired", "errore": "Sono passati 5 minuti: invia un nuovo codice."}); }
        if c.tries >= 5 { return json!({"ok": false, "motivo": "attempts", "errore": "Hai sbagliato 5 volte: invia un nuovo codice."}); }
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
            return json!({"ok": false, "motivo": "format", "errore": "Inserisci le 6 cifre del messaggio «Pixel Hub» ricevuto in LocalSend sul telefono."});
        }
        if !crate::util::ct_eq(code.as_bytes(), c.code.as_bytes()) {
            c.tries += 1;
            return json!({"ok": false, "motivo": "wrong", "errore": format!("Codice diverso da quello inviato al telefono. Tentativi rimasti: {}.", 5 - c.tries)});
        }
        let (fp, alias, ip, port) = (c.fp.clone(), c.alias.clone(), c.ip.clone(), c.port);
        w.pending = None;
        w.confermato_fp = Some(fp.clone());
        drop(w);
        self.sh.settings.conferma_dispositivo(&fp, &alias, &ip, port);
        self.sh.registry.mark_verified(&fp, Some(&ip));
        self.sh.bus.emit("dispositivi");
        self.sh.bus.emit("wizard");
        json!({"ok": true, "alias": alias})
    }

    /// Passo "prova inversa": da ora si osserva se dal dispositivo arriva qualcosa e viene accettato.
    pub fn wizard_prova(&self, fp: &str) -> bool {
        let Some(d) = self.sh.registry.get(fp) else { return false };
        self.wizard.lock().unwrap().prova = Some((d.ip, now_secs()));
        self.sh.bus.emit("wizard");
        true
    }

    pub fn wizard_completa(&self) {
        self.sh.settings.aggiorna(json!({"wizard_fatto": true}).as_object().unwrap());
        let mut w = self.wizard.lock().unwrap();
        w.pending = None;
        drop(w);
        self.sh.bus.emit("impostazioni");
    }

    fn wizard_snapshot(&self) -> Value {
        let w = self.wizard.lock().unwrap();
        let ch = w.pending.as_ref().map(|c| {
            let rest = 300u64.saturating_sub(c.created.elapsed().as_secs());
            json!({"attivo": rest > 0 && c.tries < 5, "secondi": rest, "tentativi": 5 - c.tries.min(5), "alias": c.alias, "fp": c.fp})
        }).unwrap_or(json!({"attivo": false, "secondi": 0, "tentativi": 5}));
        let invio = w.job.as_ref().and_then(|j| self.sender.stato_di(j)).map(|(s, e)| json!({"stato": s, "errore": e})).unwrap_or(Value::Null);
        let prova = w.prova.as_ref().map_or(false, |(ip, t)| self.receiver.ultimo_arrivo_da(ip, *t));
        json!({"sfida": ch, "invio": invio, "confermato_fp": w.confermato_fp, "prova_avviata": w.prova.is_some(), "prova_ricevuta": prova})
    }

    // ------------------------------------------------------------------ stato per la UI
    fn cached(&self, key: &'static str, ttl: Duration, f: impl FnOnce() -> Value) -> Value {
        if let Some((t, v)) = self.cache.lock().unwrap().get(key) { if t.elapsed() < ttl { return v.clone(); } }
        let v = f();
        self.cache.lock().unwrap().insert(key, (Instant::now(), v.clone()));
        v
    }

    fn recenti(&self) -> Value {
        let Ok(rd) = std::fs::read_dir(self.sh.settings.str("cartella")) else { return json!([]) };
        let mut v: Vec<(f64, Value)> = rd.flatten().filter(|e| !e.file_name().to_string_lossy().starts_with('.')).filter_map(|e| {
            let m = e.metadata().ok()?;
            let mt = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
            let n = e.file_name().to_string_lossy().into_owned();
            Some((mt, json!({"nome": n, "path": e.path().to_string_lossy(), "size": m.len(), "mtime": mt, "dir": m.is_dir(),
                              "tipo": if m.is_dir() { "cartella" } else { tipo_file(&n) }})))
        }).collect();
        v.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        Value::Array(v.into_iter().take(40).map(|x| x.1).collect())
    }

    fn cambi_identita(&self) -> Value {
        Value::Array(self.sh.registry.identity_changes().into_iter().map(|c| {
            let vecchio_confermato = self.sh.settings.preferiti().get(&c.old_fp).map_or(false, |r| r["confermato"] == json!(true));
            json!({"antico": c.old_fp, "nuovo": c.new_fp, "nome": c.alias, "ip": c.ip, "quando": c.when, "vecchio_confermato": vecchio_confermato})
        }).collect())
    }

    /// Per le richieste in attesa aggiunge, se l'indirizzo del mittente corrisponde a un dispositivo già visto, chi è
    /// *probabilmente*. Il mittente non è autenticato (LocalSend v2 non ha certificato client): è solo un'indicazione.
    fn arricchisci_ricezioni(&self, ric: Value) -> Value {
        let Value::Array(mut v) = ric else { return ric };
        for r in v.iter_mut().filter(|r| r["stato"] == "attesa") {
            if let Some(d) = self.sh.registry.find_by_ip(r["ip"].as_str().unwrap_or("")) {
                let rec = self.sh.settings.preferiti().get(&d.fp).cloned();
                r["noto"] = json!({"nome": d.alias, "ricordato": rec.is_some(), "confermato": rec.map_or(false, |x| x["confermato"] == json!(true))});
            }
        }
        Value::Array(v)
    }

    pub fn snapshot(&self) -> Value {
        let s = self.sh.settings.snapshot();
        let (ric, msg) = self.receiver.snapshot();
        let (adb, schermo) = self.android.snapshot();
        let id = self.sh.identity.read().unwrap();
        let ips = self.cached("rete_ip", Duration::from_secs(60), || json!(local_ipv4().iter().map(|i| i.to_string()).collect::<Vec<_>>()));
        let nome_rete = self.cached("rete_nome", Duration::from_secs(60), || json!(network_name()));
        let mut conf = Map::new();
        for k in CONF_KEYS.iter().chain(["wizard_fatto"].iter()) { conf.insert(k.to_string(), s.get(*k).cloned().unwrap_or(Value::Null)); }
        json!({
            "versione": VERSION, "utente": nome_utente(), "conf": conf, "pin_impostato": !s["pin_ricezione"].is_null(),
            "dispositivi": self.sh.registry.snapshot(), "cambi_identita": self.cambi_identita(), "ricezioni": self.arricchisci_ricezioni(ric), "messaggi": msg, "invii": self.sender.snapshot(),
            "server": {"attivo": self.server.try_lock().map(|g| g.is_some()).unwrap_or(true), "errore": *self.server_error.lock().unwrap(),
                       "porta": self.port(), "protocollo": "https"},
            "identita": {"stato": id.stato, "errore": id.errore, "impronta": id.fingerprint, "origine": id.origine},
            "rete": {"nome": nome_rete, "ip": ips, "multicast": *self.discovery.errore.lock().unwrap()},
            "recenti": self.cached("recenti", Duration::from_secs(2), || self.recenti()),
            "componenti": self.cached("componenti", Duration::from_secs(30), componenti),
            "avvisi": *self.sh.settings.avvisi.lock().unwrap(), "migrazione": s["migrato_da"],
            "adb": adb, "schermo": self.schermo_snapshot(schermo), "wizard": self.wizard_snapshot(), "aggiornamento": self.aggiornamento_snapshot(),
        })
    }

    // ------------------------------------------------------------------ schermo del telefono
    /// Quale duplicazione offrire: quella già attiva; altrimenti Android se c'è un telefono collegato via ADB;
    /// altrimenti in base ai dispositivi vicini (iPhone/iPad → AirPlay); infine quella installata.
    pub fn modo_schermo(&self) -> &'static str {
        if self.airplay.attivo() { return "iphone"; }
        let (adb, scrcpy) = self.android.snapshot();
        if scrcpy["attivo"] == json!(true) || adb["stato"] == "collegato" { return "android"; }
        let vicini = self.sh.registry.snapshot();
        let vicini: Vec<&Value> = vicini.as_array().map(|v| v.iter().filter(|d| d["vicino"] == json!(true) && d["tipo"] != "desktop").collect()).unwrap_or_default();
        let (ios, altri) = (vicini.iter().filter(|d| apple(d)).count(), vicini.iter().filter(|d| !apple(d)).count());
        let (ux, sc) = (AirPlay::installato(), scrcpy["installato"] == json!(true));
        if ios > 0 && (ux || altri == 0) { return "iphone"; }
        if altri > 0 && sc { return "android"; }
        if ux && !sc { "iphone" } else { "android" }
    }

    fn schermo_snapshot(&self, scrcpy: Value) -> Value {
        let modo = self.modo_schermo();
        let mut v = if modo == "iphone" { self.airplay.snapshot() } else { scrcpy.clone() };
        v["modo"] = json!(modo);
        v["nome"] = json!(self.sh.settings.str("nome_pc"));
        v["disponibili"] = json!({"android": scrcpy["installato"], "iphone": AirPlay::installato()});
        v
    }

    pub async fn avvia_schermo(self: &Arc<Self>) -> bool {
        if self.modo_schermo() == "iphone" { self.airplay.avvia() } else { self.android.avvia_schermo().await }
    }

    pub fn ferma_schermo(&self) { self.airplay.ferma(); self.android.ferma_schermo(); }

    // ------------------------------------------------------------------ diagnostica
    /// Testo per la sezione Diagnostica, con ipotesi distinte e non assolute.
    pub fn diagnostica(&self) -> String {
        let (firewall, comando_firewall) = desktop::firewall(self.port());
        let ips = local_ipv4();
        let srv_on = self.server.try_lock().map(|g| g.is_some()).unwrap_or(true);
        let err = self.server_error.lock().unwrap().clone();
        let id = self.sh.identity.read().unwrap();
        let mut l = vec![format!("Cable {VERSION} (Rust)")];
        let sistema = desktop::nome_sistema();
        if !sistema.is_empty() { l.push(sistema); }
        l.push(format!("Server HTTPS: {} (porta {})", if srv_on { "attivo".to_string() } else { format!("SPENTO: {}", if err.is_empty() { "?" } else { &err }) }, self.port()));
        l.push(format!("Identità: {} {}", id.stato, if id.fingerprint.is_empty() { String::new() } else { format!("{}…", &id.fingerprint[..16]) }));
        l.push(format!("Indirizzi: {}", if ips.is_empty() { "nessuno".into() } else { ips.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ") }));
        let mc = self.discovery.errore.lock().unwrap().clone();
        l.push(format!("Multicast: {}", if mc.is_empty() { "ok" } else { &mc }));
        l.push(format!("Eventi di rete: {}", match self.discovery.dbus.load(std::sync::atomic::Ordering::Relaxed) {
            1 => "D-Bus (NetworkManager, logind) in ascolto, più controllo periodico ogni 5 s",
            2 => "D-Bus non disponibile: solo controllo periodico delle interfacce ogni 5 s e rilevamento della ripresa dalla sospensione",
            _ => "in avvio" }));
        let nets = crate::discovery::local_nets();
        if !nets.is_empty() { l.push(format!("Interfacce: {}", nets.iter().map(|n| format!("{} {}", n.name, n.ip)).collect::<Vec<_>>().join(", "))); }
        let (vicini, ricordati) = {
            let v = self.sh.registry.snapshot();
            let a = v.as_array().cloned().unwrap_or_default();
            (a.iter().filter(|d| d["vicino"] == json!(true)).count(), a.iter().filter(|d| d["ricordato"] == json!(true)).count())
        };
        l.push(format!("Dispositivi: {vicini} vicini, {ricordati} ricordati"));
        l.push(format!("Dispositivi visti: {}", self.sh.registry.len()));
        l.push(format!("Firewall: {firewall}"));
        let agg = self.aggiornamento_snapshot();
        l.push(format!("Aggiornamenti: {}", if agg["verificabile"] != json!(true) { "non verificabili (compilato senza git)".to_string() }
            else if agg["attivo"] != json!(true) { "controllo spento nelle impostazioni".to_string() }
            else if agg["disponibile"] == json!(true) { format!("disponibile ({} → {})", agg["locale"].as_str().unwrap_or(""), agg["remoto"].as_str().unwrap_or("")) }
            else if !agg["errore"].as_str().unwrap_or("").is_empty() { format!("ultimo controllo non riuscito: {}", agg["errore"].as_str().unwrap_or("")) }
            else { format!("aggiornato ({})", agg["locale"].as_str().unwrap_or("")) }));
        l.push(String::new());
        let mut hints = vec![];
        if ips.is_empty() { hints.push("Nessuna rete attiva: connettiti al Wi-Fi o all'hotspot del telefono.".to_string()); }
        if err == "porta" { hints.push(format!("La porta {} è occupata, di solito da LocalSend per PC o da una versione precedente (Pixel Hub, iPhone Hub) ancora aperta: chiudila e premi Riprova.", self.port())); }
        if !mc.is_empty() { hints.push(format!("La scoperta automatica non parte ({mc}): puoi comunque inserire l'IP a mano.")); }
        if srv_on && self.sh.registry.is_empty() {
            hints.push(format!("Nessun dispositivo visto. Cause possibili: app compatibile (ad es. LocalSend) chiusa sullo smartphone, reti diverse, isolamento client del router, VPN, oppure firewall del PC (porta {} TCP/UDP): verifica una per volta, il firewall è solo una delle ipotesi.", self.port()));
            if let Some(c) = comando_firewall { hints.push(format!("Se è ufw a bloccare, apri la porta (solo sulla rete che consideri fidata): {c}")); }
        }
        if hints.is_empty() { l.push("• Nessun problema evidente.".into()); } else { for h in hints { l.push(format!("• {h}")); } }
        l.join("\n")
    }
}

/// iPhone/iPad secondo il modello o il nome dichiarati (solo per scegliere la duplicazione: non è un dato d'identità).
fn apple(d: &Value) -> bool {
    let t = format!("{} {}", d["modello"].as_str().unwrap_or(""), d["alias"].as_str().unwrap_or("")).to_lowercase();
    ["iphone", "ipad", "ipod", "apple", "ios"].iter().any(|k| t.split(|c: char| !c.is_alphanumeric()).any(|w| w == *k))
}

fn componenti() -> Value {
    let f = desktop::famiglia();
    json!([
        {"id": "adb", "nome": "ADB", "ok": desktop::has_command("adb"), "desc": "Facoltativo, solo Android: batteria, spazio e importazione foto via USB. Non serve per inviare o ricevere", "comando": desktop::comando_installazione(f, "adb")},
        {"id": "scrcpy", "nome": "scrcpy", "ok": desktop::has_command("scrcpy"), "desc": "Facoltativo, solo Android: schermo e controllo del telefono. Non serve per inviare o ricevere", "comando": desktop::comando_installazione(f, "scrcpy")},
        {"id": "uxplay", "nome": "UxPlay", "ok": AirPlay::installato(), "desc": "Facoltativo, solo iPhone/iPad: schermo del telefono sul PC via AirPlay (Duplica schermo). Non serve per inviare o ricevere", "comando": desktop::comando_installazione(f, "uxplay")},
    ])
}

fn network_name() -> String {
    let Ok(o) = Command::new("nmcli").args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"]).output() else { return String::new() };
    for l in String::from_utf8_lossy(&o.stdout).lines() {
        if let Some((n, t)) = l.rsplit_once(':') { if t.contains("wireless") || t.contains("ethernet") { return n.replace("\\:", ":"); } }
    }
    String::new()
}

fn nome_utente() -> String {
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() { return String::new(); }
        let g = std::ffi::CStr::from_ptr((*pw).pw_gecos).to_string_lossy().into_owned();
        g.split(',').next().unwrap_or("").split_whitespace().next().unwrap_or("").to_string()
    }
}

pub fn recenti_path(e: &Engine) -> PathBuf { PathBuf::from(e.sh.settings.str("cartella")) }

#[cfg(test)]
mod tests_schermo {
    use super::*;

    #[test]
    fn riconosce_iphone_e_ipad_ma_non_parole_simili() {
        assert!(apple(&json!({"modello": "iPhone", "alias": "Fiore"})));
        assert!(apple(&json!({"modello": "", "alias": "iPad di Sara"})));
        assert!(apple(&json!({"modello": "iOS", "alias": "x"})));
        assert!(!apple(&json!({"modello": "samsung", "alias": "Galaxy"})));
        assert!(!apple(&json!({"modello": "Pixel 8", "alias": "Biosfera"})));      // «ios» dentro una parola non conta
    }

    #[tokio::test]
    async fn nessun_telefono_vicino_sceglie_quella_installata() {
        let t = tempfile::tempdir().unwrap();
        let e = Engine::new(Paths::under(t.path()), 0, false);
        let atteso = if AirPlay::installato() && !desktop::has_command("scrcpy") { "iphone" } else { "android" };
        assert_eq!(e.modo_schermo(), atteso);
        let s = e.snapshot();
        assert_eq!(s["schermo"]["modo"], atteso);
        assert!(s["schermo"]["disponibili"].is_object());
    }
}
