//! Schermo dell'iPhone/iPad sul PC (facoltativo): ricevitore AirPlay con UxPlay.
//! Indipendente da LocalSend: non influisce su scoperta, identità o autorizzazioni dei trasferimenti Wi-Fi.
//! UxPlay si annuncia con Avahi (mDNS); sull'iPhone: Centro di Controllo › Duplica schermo › nome del PC.
use crate::bus::Bus;
use crate::desktop::has_command;
use crate::settings::Settings;
use crate::util::now_secs;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

#[derive(Default)]
struct PState { uxplay: Option<Child>, connesso: bool, log: VecDeque<String>, dal: f64 }

pub struct AirPlay {
    settings: Arc<Settings>,
    bus: Bus,
    st: Mutex<PState>,
}

/// Esito di una riga di UxPlay sullo stato della duplicazione: Some(true) iniziata, Some(false) finita.
pub fn evento(riga: &str) -> Option<bool> {
    let l = riga.to_lowercase();
    if l.contains("begin streaming") { Some(true) }
    else if l.contains("open connections: 0") || l.contains("connection closed") { Some(false) }
    else { None }
}

/// Argomenti di UxPlay. Porte fisse (`-p`: TCP 7000, 7001, 7100 e UDP 6000, 6001, 7011) così un firewall si configura una volta.
pub fn argomenti(nome: &str, schermo_intero: bool) -> Vec<String> {
    let mut a = vec!["-n".to_string(), nome.to_string(), "-nh".into(), "-p".into()];
    if schermo_intero { a.push("-fs".into()); }
    a
}

impl AirPlay {
    pub fn new(settings: Arc<Settings>, bus: Bus) -> Arc<AirPlay> {
        Arc::new(AirPlay { settings, bus, st: Mutex::new(PState::default()) })
    }

    pub fn installato() -> bool { has_command("uxplay") }

    pub fn attivo(&self) -> bool { self.st.lock().unwrap().uxplay.as_mut().map_or(false, |c| c.try_wait().ok().flatten().is_none()) }

    pub fn avvia(self: &Arc<Self>) -> bool {
        if self.attivo() { return true; }
        if !Self::installato() { return false; }
        let argv = argomenti(&self.settings.str("nome_pc"), self.settings.bool("schermo_intero"));
        // UxPlay scrive su una pipe a blocchi: con stdbuf le righe arrivano subito (serve a capire quando parte lo schermo)
        let mut c = if has_command("stdbuf") { let mut c = Command::new("stdbuf"); c.args(["-oL", "-eL", "uxplay"]); c } else { Command::new("uxplay") };
        let mut child = match c.args(&argv).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).spawn() {
            Ok(c) => c,
            Err(e) => { self.log_line(e.to_string()); return false; }
        };
        self.log_line(format!("$ uxplay {}", argv.join(" ")));
        let (out, err) = (child.stdout.take(), child.stderr.take());
        { let mut s = self.st.lock().unwrap(); s.uxplay = Some(child); s.connesso = false; s.dal = now_secs(); }
        for r in [out.map(|o| Box::new(o) as Box<dyn tokio::io::AsyncRead + Send + Unpin>), err.map(|e| Box::new(e) as Box<dyn tokio::io::AsyncRead + Send + Unpin>)].into_iter().flatten() {
            let me = self.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(r).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    if let Some(on) = evento(&l) { me.st.lock().unwrap().connesso = on; }
                    me.log_line(l);
                }
                me.bus.emit("android");          // processo terminato: la pagina torna su «Avvia»
            });
        }
        self.bus.emit("android");
        true
    }

    pub fn ferma(&self) {
        let mut s = self.st.lock().unwrap();
        if let Some(c) = s.uxplay.as_mut() { let _ = c.start_kill(); }
        s.uxplay = None; s.connesso = false;
        drop(s);
        self.bus.emit("android");
    }

    fn log_line(&self, l: String) {
        let mut s = self.st.lock().unwrap();
        s.log.push_back(l);
        while s.log.len() > 120 { s.log.pop_front(); }
        drop(s);
        self.bus.emit("android");
    }

    /// Stessa forma dello stato di scrcpy, così la pagina Schermo li tratta allo stesso modo.
    pub fn snapshot(&self) -> Value {
        let attivo = self.attivo();
        let s = self.st.lock().unwrap();
        let log: Vec<&String> = s.log.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev().collect();
        json!({"installato": Self::installato(), "attivo": attivo, "connesso": attivo && s.connesso,
               "dal": if attivo { s.dal } else { 0.0 }, "log": log})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn righe_di_uxplay() {
        assert_eq!(evento("Begin streaming to GStreamer video pipeline"), Some(true));
        assert_eq!(evento("Connection closed on socket 31"), Some(false));
        assert_eq!(evento("Open connections: 0"), Some(false));
        assert_eq!(evento("Open connections: 1"), None);
        assert_eq!(evento("Accepted IPv4 client on socket 31, port 7000"), None);
    }

    #[test]
    fn argomenti_con_nome_e_schermo_intero() {
        assert_eq!(argomenti("Cable portatile", false), ["-n", "Cable portatile", "-nh", "-p"]);
        assert!(argomenti("x", true).contains(&"-fs".to_string()));
    }

    /// Avvio reale (annuncia per un attimo «Cable prova» sulla rete): `cargo test -p pixelhub-engine -- --ignored airplay`.
    #[tokio::test]
    #[ignore = "richiede uxplay e avahi"]
    async fn avvia_e_ferma_uxplay() {
        if !AirPlay::installato() { return; }
        let t = tempfile::tempdir().unwrap();
        let settings = Arc::new(Settings::load(&crate::paths::Paths::under(t.path())));
        settings.aggiorna(json!({"nome_pc": "Cable prova"}).as_object().unwrap());
        let a = AirPlay::new(settings, Bus::new());
        assert!(a.avvia());
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let s = a.snapshot();
        eprintln!("{s}");
        assert_eq!(s["attivo"], true, "uxplay è uscito subito: {s}");
        assert_eq!(s["connesso"], false);
        a.ferma();
        assert_eq!(a.snapshot()["attivo"], false);
    }
}
