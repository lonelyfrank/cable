//! Ricezione LocalSend: richieste, sessioni, upload in streaming e pulizia.
//! Indipendente da HTTP e GUI: `server.rs` lo chiama e i test lo esercitano direttamente.
//!
//! Un handler HTTP può essere annullato in qualunque punto (il client chiude la connessione):
//! per questo la pulizia dei parziali sta in guardie con `Drop`, non nel codice dopo l'`await`.
use crate::bus::{Bus, Notifier};
use crate::protocol::{is_text_message, parse_prepare_upload, Limits, ProtocolError};
use crate::settings::{verify_pin_record, Settings};
use crate::util::{clean_label, commit_unique, ct_eq, now_secs, prepare_target, random_hex, tipo_file, PathError, Progress};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RState {
    Attesa,
    Accettata,
    Ricezione,
    Completata,
    Parziale,
    Rifiutata,
    Scaduta,
    Annullata,
    Errore,
}

impl RState {
    pub fn as_str(self) -> &'static str {
        match self {
            RState::Attesa => "attesa",
            RState::Accettata => "accettata",
            RState::Ricezione => "ricezione",
            RState::Completata => "completata",
            RState::Parziale => "parziale",
            RState::Rifiutata => "rifiutata",
            RState::Scaduta => "scaduta",
            RState::Annullata => "annullata",
            RState::Errore => "errore",
        }
    }
    pub fn terminal(self) -> bool {
        !matches!(self, RState::Attesa | RState::Accettata | RState::Ricezione)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FState {
    Attesa,
    Ricezione,
    Fatto,
    Rifiutato,
}
impl FState {
    fn as_str(self) -> &'static str {
        match self {
            FState::Attesa => "attesa",
            FState::Ricezione => "ricezione",
            FState::Fatto => "fatto",
            FState::Rifiutato => "rifiutato",
        }
    }
}

struct InFile {
    id: String,
    name: String,
    parts: Vec<String>,
    size: u64,
    sha256: String,
    preview: String,
    token: String,
    state: FState,
    received: u64,
    path: String,
    tmp: Option<PathBuf>,
    error: String,
}

struct Incoming {
    id: String,
    sid: String,
    ip: IpAddr,
    alias: String,
    kind: &'static str,
    files: Vec<InFile>,
    selected: Vec<String>,
    state: RState,
    error: String,
    created: f64,
    finished: f64,
    root: PathBuf,
    decision: Arc<Notify>,
    cancel: CancellationToken,
    active: usize,
    last: Instant,
    progress: Arc<Progress>,
}

impl Incoming {
    fn total(&self) -> u64 {
        if self.selected.is_empty() {
            self.files.iter().map(|f| f.size).sum()
        } else {
            self.files.iter().filter(|f| self.selected.contains(&f.id)).map(|f| f.size).sum()
        }
    }
    fn done_count(&self) -> usize {
        self.files.iter().filter(|f| f.state == FState::Fatto && self.selected.contains(&f.id)).count()
    }
    fn bytes(&self) -> u64 {
        self.files.iter().filter(|f| self.selected.contains(&f.id)).map(|f| if f.state == FState::Fatto { f.size } else { f.received }).sum()
    }
}

#[derive(Debug)]
pub enum RxError {
    Proto(ProtocolError),
    /// Il mittente ha chiuso la connessione durante l'upload: nessuna risposta da inviare.
    Stream,
}
impl From<ProtocolError> for RxError {
    fn from(e: ProtocolError) -> Self {
        RxError::Proto(e)
    }
}
fn perr(status: u16, m: &str) -> RxError {
    RxError::Proto(ProtocolError::new(status, m))
}

#[derive(Debug)]
pub struct StreamError;

#[derive(Default)]
struct State {
    incoming: HashMap<String, Incoming>,
    by_sid: HashMap<String, String>,
    messages: VecDeque<Value>,
    pin_failures: VecDeque<Instant>,
}

pub struct Receiver {
    st: Mutex<State>,
    settings: Arc<Settings>,
    bus: Bus,
    pub limits: Limits,
    notifier: Arc<Notifier>,
    mode: u32,
}

impl Receiver {
    pub fn new(settings: Arc<Settings>, bus: Bus, limits: Limits, notifier: Arc<Notifier>) -> Arc<Receiver> {
        let um = unsafe {
            let u = libc::umask(0);
            libc::umask(u);
            u
        };
        Arc::new(Receiver { st: Mutex::new(State::default()), settings, bus, limits, notifier, mode: 0o666 & !(um as u32) })
    }

    // ------------------------------------------------------------------ PIN
    fn check_pin(&self, pin: Option<&str>) -> Result<(), ProtocolError> {
        let rec = self.settings.get("pin_ricezione");
        if rec.is_null() {
            return Ok(());
        }
        let now = Instant::now();
        {
            let mut st = self.st.lock().unwrap();
            while st.pin_failures.front().is_some_and(|t| now.duration_since(*t) > self.limits.pin_window) {
                st.pin_failures.pop_front();
            }
            if st.pin_failures.len() >= self.limits.pin_failures {
                return Err(ProtocolError::new(429, "troppi tentativi"));
            }
        }
        let Some(pin) = pin.filter(|p| !p.is_empty()) else { return Err(ProtocolError::new(401, "PIN richiesto")) };
        if !verify_pin_record(&rec, pin) {
            self.st.lock().unwrap().pin_failures.push_back(now);
            return Err(ProtocolError::new(401, "PIN errato"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------ prepare-upload
    /// Gestisce prepare-upload. Restituisce (stato HTTP, corpo JSON).
    pub async fn prepare(self: &Arc<Self>, ip: IpAddr, pin: Option<String>, body: Value) -> Result<(u16, Option<Value>), ProtocolError> {
        self.check_pin(pin.as_deref())?;
        let (info, metas) = parse_prepare_upload(&body, &self.limits)?;
        let alias = clean_label(info["alias"].as_str().unwrap_or(""), 60, "Un dispositivo");
        let kind = if is_text_message(&metas) { "testo" } else { "file" };
        let (rid, decision, size, n) = {
            self.sweep();
            let mut st = self.st.lock().unwrap();
            let waiting = st.incoming.values().filter(|i| i.state == RState::Attesa).count();
            if waiting >= self.limits.max_pending {
                return Err(ProtocolError::new(429, "troppe richieste in attesa"));
            }
            if st
                .incoming
                .values()
                .any(|i| matches!(i.state, RState::Attesa | RState::Accettata | RState::Ricezione) && (i.ip == ip || i.state != RState::Attesa))
            {
                return Err(ProtocolError::new(409, "un altro trasferimento è in corso"));
            }
            let id = random_hex(6);
            let inc = Incoming {
                id: id.clone(),
                sid: String::new(),
                ip,
                alias: alias.clone(),
                kind,
                files: metas
                    .iter()
                    .map(|m| InFile {
                        id: m.id.clone(),
                        name: m.name.clone(),
                        parts: m.parts.clone(),
                        size: m.size,
                        sha256: m.sha256.clone(),
                        preview: m.preview.clone(),
                        token: String::new(),
                        state: FState::Attesa,
                        received: 0,
                        path: String::new(),
                        tmp: None,
                        error: String::new(),
                    })
                    .collect(),
                selected: vec![],
                state: RState::Attesa,
                error: String::new(),
                created: now_secs(),
                finished: 0.0,
                root: PathBuf::new(),
                decision: Arc::new(Notify::new()),
                cancel: CancellationToken::new(),
                active: 0,
                last: Instant::now(),
                progress: Arc::new(Progress::new()),
            };
            let out = (id.clone(), inc.decision.clone(), inc.total(), metas.len());
            st.incoming.insert(id, inc);
            out
        };
        self.bus.emit("ricezioni");
        self.notifier.send("richiesta", json!({"id": rid, "alias": alias, "n": n, "size": size, "tipo": kind}));

        // Se l'handler viene annullato (il mittente chiude la connessione) la richiesta risulta annullata.
        let mut guard = PendingGuard { rx: self.clone(), id: rid.clone(), armed: true };
        let deadline = tokio::time::sleep(self.limits.decision_timeout);
        tokio::pin!(deadline);
        loop {
            if self.state_of(&rid) != Some(RState::Attesa) {
                break;
            }
            tokio::select! {
                _ = decision.notified() => {}
                _ = &mut deadline => {
                    guard.armed = false;
                    self.finish(&rid, RState::Scaduta, "Nessuna risposta entro il tempo limite");
                    return Err(ProtocolError::new(403, "scaduta"));
                }
            }
        }
        guard.armed = false;
        if self.state_of(&rid) != Some(RState::Accettata) {
            return Err(ProtocolError::new(403, "rifiutata"));
        }
        if kind == "testo" {
            Ok(self.deliver_text(&rid))
        } else {
            self.open_session(&rid)
        }
    }

    fn state_of(&self, rid: &str) -> Option<RState> {
        self.st.lock().unwrap().incoming.get(rid).map(|i| i.state)
    }

    fn deliver_text(&self, rid: &str) -> (u16, Option<Value>) {
        let mut first = None;
        let alias;
        {
            let mut st = self.st.lock().unwrap();
            let Some(inc) = st.incoming.get_mut(rid) else { return (204, None) };
            alias = inc.alias.clone();
            let sel = inc.selected.clone();
            let mut texts = vec![];
            for f in inc.files.iter_mut().filter(|f| sel.contains(&f.id)) {
                texts.push(f.preview.clone());
                f.state = FState::Fatto;
            }
            let ip = inc.ip.to_string();
            for t in texts {
                first.get_or_insert_with(|| t.clone());
                st.messages.push_front(json!({"id": random_hex(5), "alias": alias, "testo": t, "quando": now_secs(), "ip": ip}));
                st.messages.truncate(30);
            }
        }
        self.finish(rid, RState::Completata, "");
        self.notifier.send("messaggio", json!({"alias": alias, "testo": first.unwrap_or_default()}));
        (204, None)
    }

    fn open_session(&self, rid: &str) -> Result<(u16, Option<Value>), ProtocolError> {
        let root = PathBuf::from(self.settings.str("cartella"));
        let free = std::fs::create_dir_all(&root)
            .map_err(|e| format!("Cartella di ricezione non utilizzabile: {e}"))
            .and_then(|_| fs4::available_space(&root).map_err(|e| e.to_string()));
        let free = match free {
            Ok(f) => f,
            Err(e) => {
                self.finish(rid, RState::Errore, &e);
                return Err(ProtocolError::new(500, "cartella"));
            }
        };
        let need = {
            let st = self.st.lock().unwrap();
            st.incoming.get(rid).map(|i| i.total()).unwrap_or(0)
        };
        if need + self.limits.disk_margin > free {
            self.finish(rid, RState::Errore, &format!("Spazio insufficiente: servono {}, liberi {}", human(need), human(free)));
            return Err(ProtocolError::new(507, "spazio insufficiente"));
        }
        let mut st = self.st.lock().unwrap();
        let sid = random_hex(16);
        let Some(inc) = st.incoming.get_mut(rid) else { return Err(ProtocolError::new(403, "annullata")) };
        if inc.state != RState::Accettata {
            return Err(ProtocolError::new(403, "annullata"));
        }
        inc.root = root;
        inc.sid = sid.clone();
        let mut tokens = serde_json::Map::new();
        let sel = inc.selected.clone();
        for f in inc.files.iter_mut() {
            if sel.contains(&f.id) {
                f.token = random_hex(16);
                tokens.insert(f.id.clone(), json!(f.token));
            } else {
                f.state = FState::Rifiutato;
            }
        }
        inc.state = RState::Ricezione;
        inc.last = Instant::now();
        st.by_sid.insert(sid.clone(), rid.to_string());
        drop(st);
        self.bus.emit("ricezioni");
        Ok((200, Some(json!({"sessionId": sid, "files": tokens}))))
    }

    // ------------------------------------------------------------------ decisione dell'utente
    /// `None` accetta tutto; lista vuota rifiuta; altrimenti accettazione parziale.
    pub fn decide(&self, rid: &str, accept: Option<Vec<String>>) -> bool {
        {
            let mut st = self.st.lock().unwrap();
            let Some(inc) = st.incoming.get_mut(rid) else { return false };
            if inc.state != RState::Attesa {
                return false;
            }
            let ids: Vec<String> = match accept {
                None => inc.files.iter().map(|f| f.id.clone()).collect(),
                Some(a) => inc.files.iter().filter(|f| a.contains(&f.id)).map(|f| f.id.clone()).collect(),
            };
            if ids.is_empty() {
                inc.state = RState::Rifiutata;
                inc.finished = now_secs();
            } else {
                inc.selected = ids;
                inc.state = RState::Accettata;
            }
            inc.decision.notify_one();
        }
        self.bus.emit("ricezioni");
        true
    }

    /// Annullamento locale: ferma subito l'I/O e scarta i parziali.
    pub fn annulla(&self, rid: &str) -> bool {
        match self.state_of(rid) {
            Some(RState::Attesa) => self.decide(rid, Some(vec![])),
            Some(RState::Accettata | RState::Ricezione) => {
                self.finish(rid, RState::Annullata, "Annullato da te");
                true
            }
            _ => false,
        }
    }

    // ------------------------------------------------------------------ upload
    pub async fn upload<S>(self: &Arc<Self>, ip: IpAddr, q: &HashMap<String, String>, content_length: Option<u64>, body: S) -> Result<PathBuf, RxError>
    where
        S: Stream<Item = Result<Bytes, StreamError>> + Unpin,
    {
        let (sid, fid, token) = (q.get("sessionId"), q.get("fileId"), q.get("token"));
        let (Some(sid), Some(fid), Some(token)) = (sid.filter(|s| !s.is_empty()), fid.filter(|s| !s.is_empty()), token.filter(|s| !s.is_empty())) else {
            return Err(perr(400, "parametri mancanti"));
        };
        let (rid, root, parts, size, sha, cancel, progress) = {
            let mut st = self.st.lock().unwrap();
            let rid = st.by_sid.get(sid.as_str()).cloned().ok_or_else(|| perr(403, "sessione sconosciuta"))?;
            let inc = st.incoming.get_mut(&rid).ok_or_else(|| perr(403, "sessione sconosciuta"))?;
            let selected = inc.selected.clone();
            let f = inc.files.iter_mut().find(|f| &f.id == fid && selected.contains(&f.id)).ok_or_else(|| perr(403, "file sconosciuto"))?;
            if inc.ip != ip || !ct_eq(f.token.as_bytes(), token.as_bytes()) {
                return Err(perr(403, "token o indirizzo non validi"));
            }
            if inc.state != RState::Ricezione || matches!(f.state, FState::Ricezione | FState::Fatto) {
                return Err(perr(409, "file già in ricezione o completato"));
            }
            if content_length.is_some_and(|l| l != f.size) {
                return Err(perr(400, "lunghezza diversa da quella dichiarata"));
            }
            f.state = FState::Ricezione;
            f.received = 0;
            f.error.clear();
            let out = (rid.clone(), inc.root.clone(), f.parts.clone(), f.size, f.sha256.clone(), inc.cancel.clone(), inc.progress.clone());
            inc.active += 1;
            inc.last = Instant::now();
            out
        };
        self.bus.emit("ricezioni");
        let mut guard = UploadGuard { rx: self.clone(), rid: rid.clone(), fid: fid.clone(), result: None, error: String::new() };
        match self.write(&rid, fid, &root, &parts, size, &sha, &cancel, &progress, body).await {
            Ok(dest) => {
                guard.result = Some(dest.clone());
                drop(guard);
                Ok(dest)
            }
            Err(e) => {
                guard.error = match &e {
                    WriteErr::Cancelled => "Annullato".into(),
                    WriteErr::Size => "Dimensione diversa da quella dichiarata".into(),
                    WriteErr::Hash => "Controllo di integrità fallito (sha256)".into(),
                    WriteErr::Path(p) => format!("Percorso rifiutato: {p}"),
                    WriteErr::Stream => "Connessione interrotta".into(),
                    WriteErr::Io(io) if io.raw_os_error() == Some(libc::ENOSPC) => "Spazio insufficiente".into(),
                    WriteErr::Io(io) => format!("Errore di scrittura: {io}"),
                };
                let out = match e {
                    WriteErr::Cancelled => perr(409, "annullato"),
                    WriteErr::Size => perr(400, "dimensione diversa da quella dichiarata"),
                    WriteErr::Hash => perr(422, "controllo di integrità fallito"),
                    WriteErr::Path(_) => perr(403, "percorso rifiutato"),
                    WriteErr::Stream => RxError::Stream,
                    WriteErr::Io(io) if io.raw_os_error() == Some(libc::ENOSPC) => perr(507, "spazio insufficiente"),
                    WriteErr::Io(_) => perr(500, "errore di scrittura"),
                };
                drop(guard);
                Err(out)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn write<S>(
        &self,
        rid: &str,
        fid: &str,
        root: &Path,
        parts: &[String],
        size: u64,
        sha: &str,
        cancel: &CancellationToken,
        progress: &Progress,
        mut body: S,
    ) -> Result<PathBuf, WriteErr>
    where
        S: Stream<Item = Result<Bytes, StreamError>> + Unpin,
    {
        let fin = prepare_target(root, parts).map_err(WriteErr::from)?;
        let tmp = fin.parent().unwrap().join(format!(".cable-{}.part", random_hex(8)));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&tmp).map_err(WriteErr::Io)?;
        self.set_tmp(rid, fid, Some(tmp.clone()));
        let mut out = tokio::fs::File::from_std(file);
        let mut hasher = if sha.is_empty() { None } else { Some(Sha256::new()) };
        let mut got: u64 = 0;
        loop {
            let next = tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(WriteErr::Cancelled),
                r = tokio::time::timeout(READ_TIMEOUT, body.next()) => r,
            };
            let chunk = match next {
                Err(_) => return Err(WriteErr::Stream),
                Ok(None) => break,
                Ok(Some(Err(_))) => return Err(WriteErr::Stream),
                Ok(Some(Ok(c))) => c,
            };
            got += chunk.len() as u64;
            if got > size {
                return Err(WriteErr::Size);
            }
            out.write_all(&chunk).await.map_err(WriteErr::Io)?;
            if let Some(h) = hasher.as_mut() {
                h.update(&chunk);
            }
            progress.add(chunk.len() as u64);
            self.note_progress(rid, fid, got);
        }
        if got != size {
            return Err(WriteErr::Size);
        }
        out.flush().await.map_err(WriteErr::Io)?;
        out.sync_all().await.map_err(WriteErr::Io)?;
        drop(out);
        if let Some(h) = hasher {
            if hex::encode(h.finalize()) != sha {
                return Err(WriteErr::Hash);
            }
        }
        if cancel.is_cancelled() {
            return Err(WriteErr::Cancelled);
        }
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(self.mode)).map_err(WriteErr::Io)?;
        let dest = commit_unique(&tmp, &fin).map_err(WriteErr::Io)?;
        self.set_tmp(rid, fid, None);
        Ok(dest)
    }

    fn set_tmp(&self, rid: &str, fid: &str, t: Option<PathBuf>) {
        if let Some(f) = self.st.lock().unwrap().incoming.get_mut(rid).and_then(|i| i.files.iter_mut().find(|f| f.id == fid)) {
            f.tmp = t;
        }
    }

    fn note_progress(&self, rid: &str, fid: &str, got: u64) {
        let mut st = self.st.lock().unwrap();
        if let Some(inc) = st.incoming.get_mut(rid) {
            inc.last = Instant::now();
            if let Some(f) = inc.files.iter_mut().find(|f| f.id == fid) {
                f.received = got;
            }
        }
        drop(st);
        self.bus.emit("ricezioni");
    }

    // ------------------------------------------------------------------ annullamento dal mittente
    pub fn cancel_from_sender(&self, ip: IpAddr, sid: &str) -> Result<(), ProtocolError> {
        let rid = {
            let st = self.st.lock().unwrap();
            let rid = st.by_sid.get(sid).cloned().ok_or_else(|| ProtocolError::new(403, "sessione sconosciuta"))?;
            if st.incoming.get(&rid).map(|i| i.ip) != Some(ip) {
                return Err(ProtocolError::new(403, "sessione sconosciuta"));
            }
            rid
        };
        self.finish(&rid, RState::Annullata, "Annullato dal mittente");
        Ok(())
    }

    // ------------------------------------------------------------------ chiusura e pulizia
    pub fn finish(&self, rid: &str, state: RState, error: &str) {
        let notice;
        {
            let mut st = self.st.lock().unwrap();
            let sid;
            {
                let Some(inc) = st.incoming.get_mut(rid) else { return };
                if inc.state.terminal() {
                    return;
                }
                let done = inc.done_count();
                let mut state = state;
                if (state == RState::Completata && done < inc.selected.len())
                    || (matches!(state, RState::Annullata | RState::Scaduta | RState::Errore) && done > 0)
                {
                    state = RState::Parziale;
                }
                let error =
                    if state == RState::Parziale && error.is_empty() { format!("Ricevuti {} file su {}", done, inc.selected.len()) } else { error.to_string() };
                inc.state = state;
                inc.error = error.clone();
                inc.finished = now_secs();
                inc.cancel.cancel();
                inc.decision.notify_one();
                for f in inc.files.iter_mut() {
                    if let Some(t) = f.tmp.take() {
                        let _ = std::fs::remove_file(t);
                    }
                }
                sid = inc.sid.clone();
                notice = (inc.kind == "file").then(|| json!({"id": inc.id, "alias": inc.alias, "stato": state.as_str(),
                                                              "ricevuti": done, "totale": inc.selected.len(), "errore": error,
                                                              // il primo file consegnato: serve a «Apri» e «Mostra nella cartella»
                                                              "percorso": inc.files.iter().find(|f| f.state == FState::Fatto && !f.path.is_empty()).map(|f| f.path.clone()).unwrap_or_default()}));
            }
            st.by_sid.remove(&sid);
        }
        self.bus.emit("ricezioni");
        if let Some(n) = notice {
            self.notifier.send("fine", n);
        }
    }

    /// Scade le sessioni inattive e dimentica quelle concluse da tempo.
    pub fn sweep(&self) {
        let now = Instant::now();
        let idle: Vec<String> = {
            let st = self.st.lock().unwrap();
            st.incoming
                .values()
                .filter(|i| i.state == RState::Ricezione && i.active == 0 && now.duration_since(i.last) > self.limits.session_idle)
                .map(|i| i.id.clone())
                .collect()
        };
        for id in idle {
            self.finish(&id, RState::Scaduta, "Il mittente non ha completato l'invio");
        }
        let t = now_secs();
        let mut st = self.st.lock().unwrap();
        let mut finished: Vec<(String, f64)> = st.incoming.values().filter(|i| i.state.terminal()).map(|i| (i.id.clone(), i.finished)).collect();
        finished.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        for (idx, (id, fin)) in finished.iter().enumerate() {
            if t - fin > 3600.0 || idx >= 30 {
                st.incoming.remove(id);
            }
        }
    }

    pub fn rimuovi_messaggio(&self, mid: &str) {
        self.st.lock().unwrap().messages.retain(|m| m["id"] != mid);
        self.bus.emit("ricezioni");
    }

    /// All'avvio elimina i soli temporanei di Cable (e delle versioni precedenti), vecchi, in cima alla cartella.
    pub fn pulisci_residui(&self) {
        let root = PathBuf::from(self.settings.str("cartella"));
        let Ok(rd) = std::fs::read_dir(root) else { return };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if (n.starts_with(".cable-") || n.starts_with(".pixelhub-"))
                && n.ends_with(".part")
                && e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|d| d > Duration::from_secs(86400))
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    // ------------------------------------------------------------------ stato per la UI
    pub fn snapshot(&self) -> (Value, Value) {
        let st = self.st.lock().unwrap();
        let mut lst: Vec<&Incoming> = st.incoming.values().collect();
        lst.sort_by(|a, b| b.created.partial_cmp(&a.created).unwrap());
        let ric: Vec<Value> = lst
            .iter()
            .map(|i| {
                json!({
                    "id": i.id, "alias": i.alias, "ip": i.ip.to_string(), "stato": i.state.as_str(), "tipo": i.kind,
                    "totale": i.total(), "byte": i.bytes(),
                    "velocita": if i.state == RState::Ricezione { i.progress.speed().map(Value::from).unwrap_or(Value::Null) } else { Value::Null },
                    "errore": i.error, "creata": i.created, "finita": i.finished,
                    "ricevuti": i.done_count(), "selezionati": i.selected.len(),
                    "file": i.files.iter().take(200).map(|f| json!({"id": f.id, "nome": f.name, "size": f.size, "tipo": tipo_file(&f.name),
                               "stato": f.state.as_str(), "errore": f.error, "percorso": f.path})).collect::<Vec<_>>(),
                    "altri": i.files.len().saturating_sub(200),
                })
            })
            .collect();
        (Value::Array(ric), Value::Array(st.messages.iter().cloned().collect()))
    }

    /// Dispositivi che hanno appena consegnato un messaggio/file (per la prova inversa del wizard).
    pub fn ultimo_arrivo_da(&self, ip: &str, dopo: f64) -> bool {
        let st = self.st.lock().unwrap();
        st.incoming.values().any(|i| i.ip.to_string() == ip && i.state == RState::Completata && i.finished >= dopo)
    }
}

fn human(n: u64) -> String {
    let mut n = n as f64;
    for u in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 {
            return if u == "B" { format!("{n:.0} {u}") } else { format!("{n:.1} {u}") };
        }
        n /= 1024.0;
    }
    format!("{n:.1} TB")
}

enum WriteErr {
    Cancelled,
    Size,
    Hash,
    Path(PathError),
    Stream,
    Io(std::io::Error),
}
impl From<PathError> for WriteErr {
    fn from(e: PathError) -> Self {
        WriteErr::Path(e)
    }
}

struct PendingGuard {
    rx: Arc<Receiver>,
    id: String,
    armed: bool,
}
impl Drop for PendingGuard {
    fn drop(&mut self) {
        if self.armed {
            self.rx.finish(&self.id, RState::Annullata, "Il mittente ha annullato la richiesta");
        }
    }
}

/// Alla fine (o se l'handler viene annullato) riporta lo stato del file e scarta il temporaneo.
struct UploadGuard {
    rx: Arc<Receiver>,
    rid: String,
    fid: String,
    result: Option<PathBuf>,
    error: String,
}
impl Drop for UploadGuard {
    fn drop(&mut self) {
        let mut done_now = false;
        {
            let mut st = self.rx.st.lock().unwrap();
            let Some(inc) = st.incoming.get_mut(&self.rid) else { return };
            inc.active = inc.active.saturating_sub(1);
            if let Some(f) = inc.files.iter_mut().find(|f| f.id == self.fid) {
                match &self.result {
                    Some(dest) => {
                        f.state = FState::Fatto;
                        f.path = dest.to_string_lossy().into_owned();
                        f.received = f.size;
                        f.tmp = None;
                    }
                    None => {
                        f.state = FState::Attesa;
                        f.received = 0;
                        if !self.error.is_empty() {
                            f.error = self.error.clone();
                        }
                        if let Some(t) = f.tmp.take() {
                            let _ = std::fs::remove_file(t);
                        }
                    }
                }
            }
            if self.result.is_some() && inc.state == RState::Ricezione && inc.done_count() == inc.selected.len() {
                done_now = true;
            }
        }
        if done_now {
            self.rx.finish(&self.rid, RState::Completata, "");
        }
        self.rx.bus.emit("ricezioni");
    }
}
