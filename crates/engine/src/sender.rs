//! Invio LocalSend: lettura in streaming dalla sorgente, PIN, accettazione parziale, annullamento.
//! Annullare = abbandonare il future del trasferimento: i socket si chiudono subito.
use crate::client::{connect, full, json_req, Body, Client, ClientError};
use crate::devices::Device;
use crate::protocol::API;
use crate::shared::Shared;
use crate::util::{now_secs, random_hex, Progress};
use bytes::Bytes;
use futures_util::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::{Method, Request};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

const CHUNK: usize = 1 << 20;
const HASH_MAX: u64 = 32 * 1024 * 1024;
const MAX_FILES: usize = 5000;
const PREPARE_TIMEOUT: Duration = Duration::from_secs(150);
const PIN_WAIT: Duration = Duration::from_secs(180);

/// Stati di un file: attesa → inviando → fatto | errore | rifiutato (il destinatario ha scelto di non riceverlo).
/// `uncertain`: la connessione è caduta DOPO l'ultimo byte inviato, senza risposta. Il destinatario potrebbe
/// aver già salvato il file: il protocollo non permette di verificarlo da qui.
#[derive(Clone, PartialEq, Debug)]
struct OutFile {
    id: String, name: String, path: Option<PathBuf>, size: u64, mtime_ns: i128, text: Option<String>,
    state: &'static str, error: String, sha256: Option<String>, uncertain: bool,
}

struct JobInner {
    state: &'static str, error: String, items: Vec<OutFile>, pin_wrong: bool,
    pin_tx: Option<oneshot::Sender<String>>, session: String,
    /// la connessione è caduta durante lo scambio (≠ dispositivo mai raggiunto)
    interrupted: bool,
    finished: f64,
}

pub struct Job {
    pub id: String, pub fp: String, pub alias: String, is_text: bool, pub silent: bool,
    /// indirizzo con cui si sta effettivamente parlando (scelto fra i candidati all'inizio dell'invio)
    addr: Mutex<(String, u16)>,
    created: f64, cancel: CancellationToken, progress: Progress, inner: Mutex<JobInner>,
    /// byte inviati per ogni elemento (stesso ordine di `items`)
    file_sent: Vec<AtomicU64>,
    retry_of: Option<String>, retried: AtomicBool,
}

struct JobEnd(&'static str, String);
enum Fail { End(JobEnd), Client(ClientError) }
impl From<JobEnd> for Fail { fn from(e: JobEnd) -> Self { Fail::End(e) } }
impl From<ClientError> for Fail { fn from(e: ClientError) -> Self { Fail::Client(e) } }

/// Stati finali. Distinti: rifiuto (lo ha deciso il destinatario), annullamento (lo hai deciso tu),
/// scadenza (nessuna decisione/risposta entro il tempo), interruzione (la connessione è caduta), errore.
pub const DONE: [&str; 7] = ["completato", "parziale", "rifiutato", "annullato", "scaduto", "interrotto", "errore"];

/// Espande file e cartelle in (percorso, nome relativo). Ignora collegamenti e file speciali.
pub fn collect(paths: &[String]) -> Result<Vec<(PathBuf, String)>, String> {
    let mut out = vec![];
    for raw in paths {
        let p = PathBuf::from(raw);
        let Ok(meta) = std::fs::symlink_metadata(&p) else { continue };
        if meta.is_dir() {
            let base = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "cartella".into());
            let mut stack = vec![p.clone()];
            while let Some(d) = stack.pop() {
                let Ok(rd) = std::fs::read_dir(&d) else { continue };
                let mut entries: Vec<_> = rd.flatten().collect();
                entries.sort_by_key(|e| e.file_name());
                for e in entries {
                    let Ok(ft) = e.file_type() else { continue };
                    if ft.is_symlink() { continue; }
                    if ft.is_dir() { stack.push(e.path()); }
                    else if ft.is_file() {
                        let rel = e.path().strip_prefix(&p).unwrap().components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>();
                        out.push((e.path(), format!("{}/{}", base, rel.join("/"))));
                        if out.len() > MAX_FILES { return Err(format!("Troppi file: massimo {MAX_FILES} per invio")); }
                    }
                }
            }
        } else if p.metadata().map(|m| m.is_file()).unwrap_or(false) {
            out.push((p.clone(), p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())));
        }
    }
    Ok(out)
}

fn mtime_ns(m: &std::fs::Metadata) -> i128 { m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128 }

pub struct Sender {
    sh: Arc<Shared>,
    jobs: Mutex<HashMap<String, Arc<Job>>>,
}

impl Sender {
    pub fn new(sh: Arc<Shared>) -> Arc<Sender> { Arc::new(Sender { sh, jobs: Mutex::new(HashMap::new()) }) }

    /// Restituisce l'id del lavoro. Non legge i file: lo fa il task, in streaming.
    pub fn invia(self: &Arc<Self>, dev: &Device, paths: &[String], text: Option<String>, silent: bool) -> Result<String, String> {
        let mut items = vec![];
        if let Some(t) = &text {
            items.push(OutFile { id: random_hex(16), name: format!("{}.txt", random_hex(4)), path: None, size: t.len() as u64, mtime_ns: 0,
                                  text: Some(t.clone()), state: "attesa", error: String::new(), sha256: None, uncertain: false });
        }
        let found = collect(paths)?;
        for (p, name) in &found {
            let Ok(m) = std::fs::metadata(p) else { continue };
            items.push(OutFile { id: random_hex(16), name: name.clone(), path: Some(p.clone()), size: m.len(), mtime_ns: mtime_ns(&m),
                                  text: None, state: "attesa", error: String::new(), sha256: None, uncertain: false });
        }
        if items.is_empty() { return Err("Nessun file valido da inviare".into()); }
        Ok(self.avvia(dev, items, text.is_some() && found.is_empty(), silent, None))
    }

    fn avvia(self: &Arc<Self>, dev: &Device, items: Vec<OutFile>, is_text: bool, silent: bool, retry_of: Option<String>) -> String {
        let file_sent = items.iter().map(|_| AtomicU64::new(0)).collect();
        let job = Arc::new(Job {
            id: random_hex(6), fp: dev.fp.clone(), alias: dev.alias.clone(), is_text, silent,
            addr: Mutex::new((dev.ip.clone(), dev.port)), created: now_secs(), cancel: CancellationToken::new(), progress: Progress::new(),
            inner: Mutex::new(JobInner { state: "preparazione", error: String::new(), items, pin_wrong: false, pin_tx: None, session: String::new(),
                                         interrupted: false, finished: 0.0 }),
            file_sent, retry_of, retried: AtomicBool::new(false),
        });
        {
            let mut j = self.jobs.lock().unwrap();
            self.prune_locked(&mut j);
            j.insert(job.id.clone(), job.clone());
        }
        self.sh.bus.emit("invii");
        let me = self.clone();
        let id = job.id.clone();
        tokio::spawn(async move { me.run(job).await });
        id
    }

    /// «Riprova i file non completati»: un nuovo invio, verso lo stesso dispositivo, con i soli elementi che non
    /// risultano consegnati e che il destinatario non ha rifiutato. Quelli già riusciti non si rimandano.
    /// Non è una ripresa dal byte interrotto (il protocollo non la prevede): ogni file ricomincia da capo.
    /// Se l'esito remoto di un file è incerto, il destinatario può ritrovarsi una copia doppia (non si sovrascrive mai).
    /// Restituisce (id nuovo invio, file saltati perché non più leggibili).
    pub fn riprova(self: &Arc<Self>, jid: &str, dev: Option<Device>) -> Result<(String, Vec<String>), String> {
        let job = self.jobs.lock().unwrap().get(jid).cloned().ok_or("Trasferimento non trovato")?;
        let pending: Vec<OutFile> = {
            let i = job.inner.lock().unwrap();
            if !DONE.contains(&i.state) { return Err("Il trasferimento è ancora in corso".into()); }
            if i.state == "completato" { return Err("Il trasferimento è già completato".into()); }
            i.items.iter().filter(|x| !matches!(x.state, "fatto" | "rifiutato")).cloned().collect()
        };
        if pending.is_empty() { return Err("Non ci sono file da riprovare".into()); }
        if job.retried.swap(true, Ordering::SeqCst) { return Err("Questo invio è già stato riprovato: usa il nuovo trasferimento".into()); }
        let mut items = vec![];
        let mut skipped = vec![];
        for it in pending {
            let mut n = OutFile { id: random_hex(16), state: "attesa", error: String::new(), sha256: None, uncertain: false, ..it.clone() };
            if let Some(p) = &it.path {
                match std::fs::metadata(p) {
                    Ok(m) if m.is_file() => { n.size = m.len(); n.mtime_ns = mtime_ns(&m); }
                    _ => { skipped.push(it.name.clone()); continue; }
                }
            }
            items.push(n);
        }
        if items.is_empty() {
            job.retried.store(false, Ordering::SeqCst);
            return Err("I file non più presenti sul PC non si possono riprovare".into());
        }
        let dev = dev.unwrap_or_else(|| {
            let (ip, port) = job.addr.lock().unwrap().clone();
            Device { fp: job.fp.clone(), alias: job.alias.clone(), model: String::new(), kind: String::new(), ip, port, addrs: vec![],
                     seen: 0.0, source: "riprova".into(), verified: false, reach: None }
        });
        let is_text = items.iter().all(|x| x.text.is_some());
        Ok((self.avvia(&dev, items, is_text, job.silent, Some(job.id.clone())), skipped))
    }

    pub fn annulla(&self, jid: &str) -> bool {
        let Some(job) = self.jobs.lock().unwrap().get(jid).cloned() else { return false };
        if DONE.contains(&job.inner.lock().unwrap().state) { return false; }
        job.cancel.cancel();
        true
    }

    pub fn fornisci_pin(&self, jid: &str, pin: &str) -> bool {
        if pin.is_empty() || pin.len() > 64 { return false; }
        let Some(job) = self.jobs.lock().unwrap().get(jid).cloned() else { return false };
        let tx = { let mut i = job.inner.lock().unwrap(); if i.state != "pin" { return false; } i.pin_tx.take() };
        tx.is_some_and(|t| t.send(pin.to_string()).is_ok())
    }

    pub fn fp_di(&self, jid: &str) -> Option<String> { self.jobs.lock().unwrap().get(jid).map(|j| j.fp.clone()) }

    pub fn stato_di(&self, jid: &str) -> Option<(String, String)> {
        let job = self.jobs.lock().unwrap().get(jid).cloned()?;
        let i = job.inner.lock().unwrap();
        Some((i.state.to_string(), i.error.clone()))
    }

    // ------------------------------------------------------------------ esecuzione
    async fn run(self: Arc<Self>, job: Arc<Job>) {
        let res = tokio::select! {
            biased;
            _ = job.cancel.cancelled() => Err(Fail::End(JobEnd("annullato", String::new()))),
            r = self.execute(&job) => r,
        };
        match res {
            Ok(()) => {}
            Err(Fail::End(JobEnd(state, e))) => self.finalize(&job, Some(state), &e),
            Err(Fail::Client(c)) => {
                if job.cancel.is_cancelled() { self.finalize(&job, Some("annullato"), ""); } else { self.finalize_client(&job, &c); }
            }
        }
        if job.cancel.is_cancelled() {
            let session = job.inner.lock().unwrap().session.clone();
            if !session.is_empty() { self.cancel_remote(&job, &session).await; }
        }
    }

    fn addr(&self, job: &Job) -> (String, u16) { job.addr.lock().unwrap().clone() }

    /// Errore di rete a livello di invio: scadenza, interruzione o errore, con fatti osservati e cause possibili.
    fn finalize_client(&self, job: &Arc<Job>, c: &ClientError) {
        let (ip, port) = self.addr(job);
        let msg = c.describe(&ip, port);
        let state = if matches!(c, ClientError::Timeout) {
            "scaduto"
        } else if c.is_interruption() {
            job.inner.lock().unwrap().interrupted = true;
            "interrotto"
        } else { "errore" };
        let msg = if state == "scaduto" { "Nessuna risposta dal destinatario entro il tempo previsto: non ha né accettato né rifiutato. Controlla che LocalSend sia aperto e riprova.".to_string() } else { msg };
        self.finalize(job, Some(state), &msg);
    }

    /// Sceglie fra gli indirizzi noti del dispositivo il primo che risponde con l'impronta giusta (in parallelo, al massimo
    /// 3 secondi), così un IP cambiato o una seconda interfaccia non fanno aspettare né falliscono l'invio.
    async fn resolve(&self, job: &Arc<Job>) -> Result<(), Fail> {
        let mut cands: Vec<(String, u16)> = self.sh.registry.get(&job.fp).map(|d| d.candidates().into_iter().map(|a| (a.ip, a.port)).collect()).unwrap_or_default();
        let first = job.addr.lock().unwrap().clone();
        if !cands.contains(&first) { cands.insert(0, first); }
        cands.truncate(3);
        let make = || self.sh.client_tls();
        match crate::client::connect_any(&make, &cands, Duration::from_secs(3), &job.fp).await {
            Ok((c, ip, port)) => {
                drop(c);
                *job.addr.lock().unwrap() = (ip.clone(), port);
                self.sh.registry.mark_verified(&job.fp, Some(&ip));
                Ok(())
            }
            Err(e) => {
                // l'indirizzo che si mostra nel messaggio è il più probabile fra i candidati
                if let Some(c0) = cands.first() { *job.addr.lock().unwrap() = c0.clone(); }
                self.sh.registry.mark_unreachable(&job.fp);
                Err(e.into())
            }
        }
    }

    async fn execute(&self, job: &Arc<Job>) -> Result<(), Fail> {
        let items = job.inner.lock().unwrap().items.clone();
        for it in items.iter().filter(|x| x.text.is_none()) {
            self.verify_source(it).map_err(|e| JobEnd("errore", format!("«{}»: {e}", it.name)))?;
        }
        // prima di calcolare gli hash: se il destinatario non è raggiungibile lo si scopre subito
        self.resolve(job).await?;
        let mut files = serde_json::Map::new();
        for (idx, it) in items.iter().enumerate() {
            let mut sha = None;
            if it.text.is_none() && it.size <= HASH_MAX { sha = Some(self.sha256(it).await?); }
            let mut m = json!({"id": it.id, "fileName": it.name, "size": it.size,
                "fileType": if it.text.is_some() { "text/plain".to_string() } else { guess_mime(&it.name) },
                "sha256": sha, "preview": it.text});
            if let Some(p) = &it.path { if let Ok(md) = std::fs::metadata(p) { m["metadata"] = json!({"modified": chrono::DateTime::<chrono::Utc>::from_timestamp(md.mtime(), 0).map(|d| d.format("%Y-%m-%dT%H:%M:%SZ").to_string())}); } }
            files.insert(it.id.clone(), m);
            job.inner.lock().unwrap().items[idx].sha256 = sha;
        }
        let body = json!({"info": self.sh.info(true), "files": files});
        let (status, resp) = self.prepare(job, &body).await?;
        if status == 204 {
            let mut i = job.inner.lock().unwrap();
            for it in i.items.iter_mut() { it.state = "fatto"; }
            drop(i);
            for (idx, it) in items.iter().enumerate() { job.file_sent[idx].store(it.size, Ordering::Relaxed); }
            self.finalize(job, None, "");
            return Ok(());
        }
        let resp = resp.unwrap_or(Value::Null);
        let session = resp["sessionId"].as_str().unwrap_or("").to_string();
        let tokens = resp["files"].as_object().cloned().ok_or_else(|| JobEnd("errore", "Risposta del destinatario non valida".into()))?;
        if session.is_empty() { return Err(JobEnd("errore", "Risposta del destinatario non valida".into()).into()); }
        {
            let mut i = job.inner.lock().unwrap();
            i.session = session.clone();
            for it in i.items.iter_mut() { if !tokens.contains_key(&it.id) { it.state = "rifiutato"; } }
            i.state = "invio";
        }
        self.sh.bus.emit("invii");
        let mut fatal: Option<ClientError> = None;
        for idx in 0..items.len() {
            let (it, accepted) = { let i = job.inner.lock().unwrap(); (i.items[idx].clone(), i.items[idx].state != "rifiutato") };
            if !accepted { continue; }
            if let Some(c) = &fatal {
                // la connessione è caduta: i file non ancora iniziati restano «in attesa» e si possono riprovare
                let _ = c;
                continue;
            }
            job.inner.lock().unwrap().items[idx].state = "inviando";
            let token = tokens[&it.id].as_str().unwrap_or("").to_string();
            let r = self.upload(job, idx, &it, &session, &token).await;
            let mut i = job.inner.lock().unwrap();
            match r {
                Ok(()) => i.items[idx].state = "fatto",
                Err(Fail::End(JobEnd(_, e))) => { i.items[idx].state = "errore"; i.items[idx].error = e; }
                Err(Fail::Client(ClientError::Identity)) => { drop(i); return Err(ClientError::Identity.into()); }
                Err(Fail::Client(c)) => {
                    let (ip, port) = self.addr(job);
                    i.items[idx].state = "errore";
                    i.items[idx].error = c.describe(&ip, port);
                    // tutti i byte sono partiti ma non è arrivata la risposta: il destinatario potrebbe averlo salvato
                    i.items[idx].uncertain = c.is_interruption() && job.file_sent[idx].load(Ordering::Relaxed) >= it.size;
                    if c.is_interruption() { i.interrupted = true; }
                    fatal = Some(c);
                }
            }
            drop(i);
            self.sh.bus.emit("invii");
        }
        self.finalize(job, None, "");
        Ok(())
    }

    fn finalize(&self, job: &Arc<Job>, state: Option<&'static str>, error: &str) {
        {
            let mut i = job.inner.lock().unwrap();
            if DONE.contains(&i.state) { return; }
            let ok = i.items.iter().filter(|x| x.state == "fatto").count();
            let refused = i.items.iter().filter(|x| x.state == "rifiutato").count();
            // «inviando» = interrotto a metà; «attesa» = mai iniziato (la connessione era già caduta)
            let bad: Vec<&OutFile> = i.items.iter().filter(|x| matches!(x.state, "errore" | "inviando" | "attesa")).collect();
            let n = i.items.len();
            let uncertain = i.items.iter().filter(|x| x.uncertain).count();
            let interrupted = i.interrupted;
            let (st, err) = match state {
                None if !bad.is_empty() && interrupted => ("interrotto", format!("Connessione interrotta: {ok} file su {n} consegnati. Puoi riprovare quelli rimasti.")),
                None if !bad.is_empty() && ok > 0 => ("parziale", format!("Inviati {ok} file su {n}")),
                None if !bad.is_empty() => ("errore", { let e = &bad[0].error; if e.is_empty() { "Invio non riuscito".to_string() } else { e.clone() } }),
                None if refused > 0 && ok > 0 => ("parziale", format!("Il destinatario ha accettato {ok} file su {n}")),
                None if refused > 0 => ("rifiutato", "Il destinatario non ha accettato nessun file".to_string()),
                None => ("completato", String::new()),
                Some(s @ ("annullato" | "errore" | "scaduto" | "interrotto")) if ok > 0 => (s, format!("{}{}({ok} file già inviati)", error, if error.is_empty() { "" } else { " " })),
                Some(s) => (s, error.to_string()),
            };
            let err = if uncertain > 0 {
                format!("{err} Per {uncertain} file l'esito sul destinatario è incerto: se li ha già ricevuti, riprovando potrebbero comparire copie doppie.")
            } else { err };
            i.state = st; i.error = err; i.pin_tx = None; i.finished = now_secs();
        }
        self.sh.bus.emit("invii");
        let (st, er) = { let i = job.inner.lock().unwrap(); (i.state, i.error.clone()) };
        // chi ha accettato dei file (anche in parte) è un dispositivo con cui si è parlato davvero: lo si ricorda
        if !job.silent && matches!(st, "completato" | "parziale") { self.sh.registry.remember(&job.fp); }
        if !job.silent { self.sh.notifier.send("invio", json!({"id": job.id, "alias": job.alias, "stato": st, "errore": er})); }
    }

    // ------------------------------------------------------------------ prepare-upload con PIN
    async fn prepare(&self, job: &Arc<Job>, body: &Value) -> Result<(u16, Option<Value>), Fail> {
        self.set_state(job, "attesa");
        let mut pin: Option<String> = None;
        for _ in 0..5 {
            let path = match &pin {
                Some(p) => format!("{API}/prepare-upload?pin={}", percent_encoding::utf8_percent_encode(p, percent_encoding::NON_ALPHANUMERIC)),
                None => format!("{API}/prepare-upload"),
            };
            let (status, resp) = self.request(job, Method::POST, &path, Some(body), PREPARE_TIMEOUT).await?;
            match status {
                200 | 204 => return Ok((status, resp)),
                401 => {
                    let (tx, rx) = oneshot::channel();
                    {
                        let mut i = job.inner.lock().unwrap();
                        i.pin_wrong = pin.is_some(); i.state = "pin"; i.pin_tx = Some(tx);
                    }
                    self.sh.bus.emit("invii");
                    pin = Some(match tokio::time::timeout(PIN_WAIT, rx).await {
                        Ok(Ok(p)) => p,
                        _ => return Err(JobEnd("scaduto", "PIN non inserito in tempo: riprova l'invio quando hai il PIN.".into()).into()),
                    });
                    self.set_state(job, "attesa");
                }
                403 => return Err(JobEnd("rifiutato", "Il destinatario ha rifiutato l'invio. Se non era voluto puoi ripeterlo.".into()).into()),
                409 => return Err(JobEnd("errore", "Il destinatario sta già gestendo un altro trasferimento: attendi che finisca e riprova.".into()).into()),
                429 => return Err(JobEnd("errore", "Troppe richieste o tentativi di PIN: attendi qualche minuto prima di riprovare.".into()).into()),
                507 => return Err(JobEnd("errore", "Il destinatario non ha spazio sufficiente: libera spazio sul dispositivo e riprova.".into()).into()),
                s => return Err(JobEnd("errore", format!("Il destinatario ha risposto con un errore ({s}).")).into()),
            }
        }
        Err(JobEnd("errore", "PIN errato: controlla il PIN impostato sul destinatario.".into()).into())
    }

    fn set_state(&self, job: &Arc<Job>, s: &'static str) { job.inner.lock().unwrap().state = s; self.sh.bus.emit("invii"); }

    async fn conn(&self, job: &Job, t: Duration) -> Result<Client, ClientError> {
        let tls = self.sh.client_tls().map_err(ClientError::Tls)?;
        let (ip, port) = self.addr(job);
        let c = connect(tls, &ip, port, t, Some(&job.fp)).await?;
        self.sh.registry.mark_verified(&job.fp, Some(&ip));
        Ok(c)
    }

    async fn request(&self, job: &Job, m: Method, path: &str, body: Option<&Value>, t: Duration) -> Result<(u16, Option<Value>), ClientError> {
        let mut c = self.conn(job, Duration::from_secs(10)).await?;
        let (ip, port) = self.addr(job);
        tokio::time::timeout(t, c.send(json_req(&ip, port, m, path, body))).await.map_err(|_| ClientError::Timeout)?
    }

    async fn cancel_remote(&self, job: &Job, session: &str) {
        let path = format!("{API}/cancel?sessionId={session}");
        let _ = tokio::time::timeout(Duration::from_secs(5), self.request(job, Method::POST, &path, None, Duration::from_secs(5))).await;
    }

    // ------------------------------------------------------------------ sorgente
    fn verify_source(&self, it: &OutFile) -> Result<(), String> {
        let p = it.path.as_ref().unwrap();
        match std::fs::metadata(p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err("Il file è stato rimosso".into()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err("Permesso negato".into()),
            Err(e) => Err(e.to_string()),
            Ok(m) if m.len() != it.size || mtime_ns(&m) != it.mtime_ns => Err("Il file è stato modificato dopo la selezione".into()),
            Ok(_) => Ok(()),
        }
    }

    async fn sha256(&self, it: &OutFile) -> Result<String, Fail> {
        use sha2::{Digest, Sha256};
        let mut f = tokio::fs::File::open(it.path.as_ref().unwrap()).await.map_err(|e| JobEnd("errore", match e.kind() {
            std::io::ErrorKind::NotFound => format!("Il file «{}» è stato rimosso", it.name),
            std::io::ErrorKind::PermissionDenied => format!("Permesso negato su «{}»", it.name),
            _ => e.to_string() }))?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).await.map_err(|e| JobEnd("errore", e.to_string()))?;
            if n == 0 { break; }
            h.update(&buf[..n]);
        }
        Ok(hex::encode(h.finalize()))
    }

    // ------------------------------------------------------------------ upload
    async fn upload(&self, job: &Arc<Job>, idx: usize, it: &OutFile, session: &str, token: &str) -> Result<(), Fail> {
        let q = format!("sessionId={session}&fileId={}&token={token}", it.id);
        let path = format!("{API}/upload?{q}");
        let mut c = self.conn(job, Duration::from_secs(10)).await?;
        let changed_at_end = Arc::new(AtomicBool::new(false));
        let source_err: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let body: Body = match (&it.text, &it.path) {
            (Some(t), _) => { job.progress.add(t.len() as u64); job.file_sent[idx].fetch_add(t.len() as u64, Ordering::Relaxed); full(t.clone().into_bytes()) }
            (None, Some(p)) => self.file_body(job, idx, p, it, changed_at_end.clone(), source_err.clone()).await?,
            _ => unreachable!(),
        };
        let (ip, port) = self.addr(job);
        let req = Request::builder().method(Method::POST).uri(&path).header("Host", format!("{ip}:{port}"))
            .header("Content-Type", "application/octet-stream").header("Content-Length", it.size).body(body).unwrap();
        let r = tokio::time::timeout(Duration::from_secs(3600 * 6), c.send(req)).await.map_err(|_| ClientError::Timeout)?;
        if let Some(e) = source_err.lock().unwrap().take() { return Err(JobEnd("errore", e).into()); }
        let (status, _) = r?;
        if status != 200 {
            return Err(JobEnd("errore", match status {
                403 => "Il destinatario non ha accettato il trasferimento (sessione annullata o scaduta)".into(),
                409 => "Il destinatario sta già ricevendo questo file".into(),
                422 => "Il destinatario ha rilevato un file danneggiato: riprova a inviarlo".into(),
                507 => "Il destinatario non ha spazio sufficiente: libera spazio e riprova".into(),
                s => format!("Il destinatario ha risposto con un errore ({s})"),
            }).into());
        }
        if changed_at_end.load(Ordering::Relaxed) {
            return Err(JobEnd("errore", "Il file è cambiato durante l'ultimo blocco: il destinatario potrebbe aver salvato una copia incoerente, controllala o inviala di nuovo".into()).into());
        }
        Ok(())
    }

    /// Corpo in streaming dalla sorgente: dimensione e data si controllano a ogni blocco da 1 MiB.
    async fn file_body(&self, job: &Arc<Job>, idx: usize, p: &Path, it: &OutFile, at_end: Arc<AtomicBool>, serr: Arc<Mutex<Option<String>>>) -> Result<Body, Fail> {
        let file = tokio::fs::File::open(p).await.map_err(|e| JobEnd("errore", if e.kind() == std::io::ErrorKind::NotFound { "Il file è stato rimosso".into() } else { "Permesso negato".to_string() }))?;
        let m = file.metadata().await.map_err(|e| JobEnd("errore", e.to_string()))?;
        if m.len() != it.size || mtime_ns(&m) != it.mtime_ns { return Err(JobEnd("errore", "Il file è stato modificato prima dell'invio".into()).into()); }
        struct St { idx: usize, f: tokio::fs::File, left: u64, size: u64, mtime: i128, job: Arc<Job>, bus: crate::bus::Bus, last: Instant, at_end: Arc<AtomicBool>, serr: Arc<Mutex<Option<String>>> }
        let st = St { idx, f: file, left: it.size, size: it.size, mtime: it.mtime_ns, job: job.clone(), bus: self.sh.bus.clone(), last: Instant::now(), at_end, serr };
        let s = stream::unfold(st, |mut s| async move {
            if s.left == 0 { return None; }
            let want = (s.left as usize).min(CHUNK);
            let mut buf = vec![0u8; want];
            let mut got = 0;
            while got < want {
                match s.f.read(&mut buf[got..]).await {
                    Ok(0) => { *s.serr.lock().unwrap() = Some("Il file si è accorciato durante l'invio".into()); return Some((Err(std::io::Error::other("sorgente accorciata")), s)); }
                    Ok(n) => got += n,
                    Err(e) => { *s.serr.lock().unwrap() = Some(format!("Errore di lettura: {e}")); return Some((Err(e), s)); }
                }
            }
            s.left -= want as u64;
            s.job.progress.add(want as u64);
            s.job.file_sent[s.idx].fetch_add(want as u64, Ordering::Relaxed);
            if s.last.elapsed() >= Duration::from_millis(100) { s.last = Instant::now(); s.bus.emit("invii"); }
            if let Ok(m) = s.f.metadata().await {
                if m.len() != s.size || mtime_ns(&m) != s.mtime {
                    if s.left > 0 {
                        *s.serr.lock().unwrap() = Some("Il file è cambiato durante l'invio".into());
                        return Some((Err(std::io::Error::other("sorgente cambiata")), s));
                    }
                    s.at_end.store(true, Ordering::Relaxed);
                }
            }
            Some((Ok(Frame::data(Bytes::from(buf))), s))
        });
        Ok(StreamBody::new(s).boxed())
    }

    // ------------------------------------------------------------------ stato
    fn prune_locked(&self, j: &mut HashMap<String, Arc<Job>>) {
        let now = now_secs();
        let mut done: Vec<(String, f64)> = j.values().filter(|x| DONE.contains(&x.inner.lock().unwrap().state)).map(|x| (x.id.clone(), x.created)).collect();
        done.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let n = done.len();
        for (idx, (id, created)) in done.into_iter().enumerate() {
            if now - created > 900.0 || n - idx > 30 { j.remove(&id); }
        }
    }

    pub fn prune(&self) { let mut j = self.jobs.lock().unwrap(); self.prune_locked(&mut j); }

    pub fn snapshot(&self) -> Value {
        let jobs = self.jobs.lock().unwrap();
        let mut v: Vec<&Arc<Job>> = jobs.values().filter(|j| !j.silent).collect();
        v.sort_by(|a, b| b.created.partial_cmp(&a.created).unwrap());
        Value::Array(v.into_iter().map(|j| {
            let i = j.inner.lock().unwrap();
            // i file che il destinatario non ha voluto non fanno parte del lavoro da svolgere
            let total: u64 = i.items.iter().filter(|x| x.state != "rifiutato").map(|x| x.size).sum();
            let sent = j.progress.total().min(total);
            let finished = DONE.contains(&i.state);
            let retryable = if finished && i.state != "completato" { i.items.iter().filter(|x| !matches!(x.state, "fatto" | "rifiutato")).count() } else { 0 };
            let live = i.state == "invio";
            json!({"id": j.id, "verso": j.alias, "fp": j.fp, "stato": i.state, "errore": i.error, "n": i.items.len(), "totale": total,
                   "inviati": sent, "fatti": i.items.iter().filter(|x| x.state == "fatto").count(),
                   "errori": i.items.iter().filter(|x| x.state == "errore").count(),
                   "rifiutati": i.items.iter().filter(|x| x.state == "rifiutato").count(),
                   "incerti": i.items.iter().filter(|x| x.uncertain).count(),
                   "velocita": if live { j.progress.speed().map(Value::from).unwrap_or(Value::Null) } else { Value::Null },
                   "residuo": if live { j.progress.eta(total.saturating_sub(sent)).map(Value::from).unwrap_or(Value::Null) } else { Value::Null },
                   "nomi": i.items.iter().take(3).map(|x| x.name.clone()).collect::<Vec<_>>(), "testo": j.is_text,
                   "pin_errato": i.pin_wrong, "creato": j.created, "finito": i.finished,
                   "riprovabili": retryable, "riprovato": j.retried.load(Ordering::Relaxed), "riprova_di": j.retry_of,
                   "file": i.items.iter().enumerate().take(100).map(|(n, x)| json!({"nome": x.name, "stato": x.state, "errore": x.error, "size": x.size,
                        "inviati": if x.state == "fatto" { x.size } else { j.file_sent[n].load(Ordering::Relaxed).min(x.size) }, "incerto": x.uncertain})).collect::<Vec<_>>(),
                   "altri_file": i.items.len().saturating_sub(100)})
        }).collect())
    }

    pub fn file_errors(&self, jid: &str) -> Vec<String> {
        self.jobs.lock().unwrap().get(jid).map(|j| j.inner.lock().unwrap().items.iter().map(|x| x.error.clone()).collect()).unwrap_or_default()
    }
}

fn guess_mime(name: &str) -> String {
    let e = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match e.as_str() {
        "jpg" | "jpeg" => "image/jpeg", "png" => "image/png", "gif" => "image/gif", "webp" => "image/webp",
        "mp4" => "video/mp4", "mov" => "video/quicktime", "mp3" => "audio/mpeg", "pdf" => "application/pdf",
        "txt" => "text/plain", "zip" => "application/zip", "json" => "application/json", _ => "application/octet-stream",
    }.into()
}
