//! Controllo degli aggiornamenti: confronta il commit da cui Cable è stato compilato con la testa del ramo `main`
//! del repository. Usa `git ls-remote` (nessuna libreria HTTP in più, nessun token, niente scaricato oltre a un hash).
//! Si può spegnere dalle impostazioni; senza commit incorporato (RPM, tarball) non fa nulla.
use crate::engine::Engine;
use crate::util::now_secs;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;

pub const REPO_URL: &str = "https://github.com/lonelyfrank/cable.git";
const RAMO: &str = "refs/heads/main";
/// Commit di compilazione (vuoto se non c'era git)
pub const COMMIT: &str = env!("CABLE_COMMIT");
/// Cartella dei sorgenti usata per compilare (vuota se non c'era git)
pub const SORGENTI: &str = env!("CABLE_SRC");
const PRIMO_CONTROLLO: Duration = Duration::from_secs(30);
const INTERVALLO: Duration = Duration::from_secs(6 * 3600);
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct Stato {
    pub controllato: f64,
    pub remoto: String,
    pub disponibile: bool,
    pub errore: String,
    avvisato: String,
    /// `installa-utente.sh aggiorna` lanciato da qui; se finisce bene Cable viene riavviato e questo stato sparisce con lui
    figlio: Option<std::process::Child>,
    esito: String,
}

/// Primo hash di 40 cifre esadecimali nell'output di `git ls-remote`.
pub fn primo_hash(out: &str) -> Option<String> {
    let h = out.split_whitespace().next()?;
    (h.len() == 40 && h.bytes().all(|b| b.is_ascii_hexdigit())).then(|| h.to_ascii_lowercase())
}

fn script() -> std::path::PathBuf { std::path::Path::new(SORGENTI).join("scripts/installa-utente.sh") }

/// Comando da eseguire per aggiornare, mostrato all'utente.
pub fn comando() -> String { format!("cd '{SORGENTI}' && scripts/installa-utente.sh aggiorna") }

async fn git(args: &[&str]) -> Option<(bool, String)> {
    let mut c = Command::new("git");
    c.args(args).env("GIT_TERMINAL_PROMPT", "0").stdin(std::process::Stdio::null()).kill_on_drop(true);
    let o = tokio::time::timeout(TIMEOUT, c.output()).await.ok()?.ok()?;
    Some((o.status.success(), String::from_utf8_lossy(&o.stdout).into_owned()))
}

/// Ok(Some(hash)) se il remoto ha un commit diverso e non già contenuto in quello di compilazione; Ok(None) se si è aggiornati.
async fn remoto_nuovo() -> Result<Option<String>, String> {
    let (ok, out) = git(&["ls-remote", REPO_URL, RAMO]).await.ok_or("git non risponde o non è installato")?;
    let remoto = primo_hash(&out).filter(|_| ok).ok_or("il repository non risponde (rete assente o GitHub non raggiungibile)")?;
    if remoto == COMMIT { return Ok(None); }
    // già nella nostra storia (compilazione più recente del remoto, ad es. commit non ancora spinti): nessun aggiornamento
    let in_storia = git(&["-C", SORGENTI, "merge-base", "--is-ancestor", &remoto, COMMIT]).await.map_or(false, |(ok, _)| ok);
    Ok((!in_storia).then_some(remoto))
}

impl Engine {
    /// Esegue un controllo adesso e, se c'è una novità mai segnalata in questa sessione, manda una notifica.
    pub async fn controlla_aggiornamenti(self: &Arc<Self>) {
        if COMMIT.is_empty() { return; }
        let esito = remoto_nuovo().await;
        let nuovo = {
            let mut s = self.agg.lock().unwrap();
            s.controllato = now_secs();
            match &esito {
                Ok(r) => { s.errore.clear(); s.disponibile = r.is_some(); s.remoto = r.clone().unwrap_or_default(); }
                Err(e) => { s.errore = e.clone(); }       // l'ultimo esito noto resta valido
            }
            (s.disponibile && s.avvisato != s.remoto).then(|| { s.avvisato = s.remoto.clone(); s.remoto.clone() })
        };
        if let Some(r) = nuovo { self.sh.notifier.send("aggiornamento", json!({"remoto": &r[..7], "comando": comando()})); }
        self.sh.bus.emit("aggiornamento");
    }

    /// «Aggiorna e riavvia»: git pull, compilazione e installazione in un processo staccato da Cable (lo script, se va
    /// tutto bene, chiude questa istanza e apre la nuova). Se fallisce Cable resta com'è e mostra dove leggere il motivo.
    pub fn avvia_aggiornamento(self: &Arc<Self>) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        if COMMIT.is_empty() || !script().is_file() { return Err("Questa copia non è stata compilata da un clone git: aggiornala con il gestore di pacchetti.".into()); }
        let mut s = self.agg.lock().unwrap();
        if s.figlio.as_mut().map_or(false, |c| c.try_wait().ok().flatten().is_none()) { return Ok(()); }
        let log = self.log_aggiornamento();
        if let Some(d) = log.parent() { let _ = std::fs::create_dir_all(d); }
        let f = std::fs::File::create(&log).map_err(|e| format!("Non riesco a scrivere {}: {e}", log.display()))?;
        let f2 = f.try_clone().map_err(|e| e.to_string())?;
        let c = std::process::Command::new("bash").arg(script()).args(["aggiorna", "--apri"]).current_dir(SORGENTI)
            .stdin(std::process::Stdio::null()).stdout(f).stderr(f2).env("GIT_TERMINAL_PROMPT", "0")
            .process_group(0)                 // non riceve i segnali destinati a Cable: sopravvive al riavvio che fa lui stesso
            .spawn().map_err(|e| format!("Non riesco ad avviare l'aggiornamento: {e}"))?;
        s.figlio = Some(c);
        s.esito.clear();
        drop(s);
        self.sh.bus.emit("aggiornamento");
        // se lo script fallisce Cable resta in vita: la pagina deve saperlo senza aspettare altri eventi
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let finito = me.agg.lock().unwrap().figlio.as_mut().map_or(true, |c| c.try_wait().ok().flatten().is_some());
                if finito { me.sh.bus.emit("aggiornamento"); return; }
            }
        });
        Ok(())
    }

    fn log_aggiornamento(&self) -> std::path::PathBuf { self.sh.paths.cache.join("aggiornamento.log") }

    pub fn aggiornamento_snapshot(&self) -> Value {
        let mut s = self.agg.lock().unwrap();
        if let Some(st) = s.figlio.as_mut().and_then(|c| c.try_wait().ok().flatten()) {
            s.figlio = None;
            // uscito con successo ma Cable è ancora questo: lo script non l'ha riavviato (non dovrebbe succedere)
            s.esito = if st.success() { "Installato: chiudi e riapri Cable per usare la nuova versione.".into() }
                      else { format!("Aggiornamento non riuscito: Cable resta alla versione attuale. Dettagli in {}", self.log_aggiornamento().display()) };
        }
        json!({"in_corso": s.figlio.is_some(), "esito": s.esito, "verificabile": !COMMIT.is_empty(), "attivo": self.sh.settings.bool("controlla_aggiornamenti"),
               "disponibile": s.disponibile, "remoto": s.remoto.get(..7).unwrap_or(""), "locale": COMMIT.get(..7).unwrap_or(""),
               "errore": s.errore, "controllato": s.controllato, "comando": comando()})
    }

    /// Controllo all'avvio (dopo qualche secondo) e poi ogni 6 ore. Non parte nei motori isolati (prove, sandbox).
    pub(crate) fn avvia_controllo_aggiornamenti(self: &Arc<Self>) {
        if COMMIT.is_empty() || self.isolato { return; }
        let me = self.clone();
        tokio::spawn(async move {
            let mut attesa = PRIMO_CONTROLLO;
            loop {
                tokio::select! { _ = me.cancel.cancelled() => return, _ = tokio::time::sleep(attesa) => {} }
                if me.sh.settings.bool("controlla_aggiornamenti") { me.controlla_aggiornamenti().await; }
                attesa = INTERVALLO;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_da_ls_remote() {
        let h = "70906aed3264f871412c6ab492958bf6ccf47cc2";
        assert_eq!(primo_hash(&format!("{h}\trefs/heads/main\n")).as_deref(), Some(h));
        assert_eq!(primo_hash(&format!("{}\trefs/heads/main", h.to_uppercase())).as_deref(), Some(h));
        assert_eq!(primo_hash(""), None);
        assert_eq!(primo_hash("fatal: unable to access\n"), None);
        assert_eq!(primo_hash("abc123\trefs/heads/main"), None);
    }

    #[test]
    fn il_comando_usa_lo_script_di_aggiornamento() {
        assert!(comando().ends_with("scripts/installa-utente.sh aggiorna"));
    }

    /// Prova reale contro GitHub: `cargo test -p pixelhub-engine -- --ignored aggiornamenti`.
    #[tokio::test]
    #[ignore = "richiede rete"]
    async fn controllo_reale_contro_il_repository() {
        assert!(!COMMIT.is_empty(), "compilato senza git");
        let r = remoto_nuovo().await;
        eprintln!("esito: {r:?}");
        assert!(r.is_ok(), "{r:?}");
    }
}
