//! Validazione dei messaggi LocalSend v2 e limiti applicati alle richieste in ingresso.
use crate::util::{clean_label, safe_parts};
use serde_json::Value;

pub const API: &str = "/api/localsend/v2";

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_json_body: usize,
    pub max_files: usize,
    pub max_file_size: u64,
    pub max_total_size: u64,
    pub max_preview: usize,
    pub max_pending: usize,
    pub decision_timeout: std::time::Duration,
    pub session_idle: std::time::Duration,
    pub max_connections: usize,
    pub pin_failures: usize,
    pub pin_window: std::time::Duration,
    pub disk_margin: u64,
}

impl Default for Limits {
    fn default() -> Self {
        use std::time::Duration as D;
        Limits {
            max_json_body: 8 * 1024 * 1024, max_files: 5000, max_file_size: 1 << 50, max_total_size: 1 << 50,
            max_preview: 64 * 1024, max_pending: 5, decision_timeout: D::from_secs(120),
            session_idle: D::from_secs(120), max_connections: 32, pin_failures: 10, pin_window: D::from_secs(300), disk_margin: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolError {
    pub status: u16,
    pub message: String,
}

impl ProtocolError {
    pub fn new(status: u16, message: &str) -> Self { ProtocolError { status, message: message.into() } }
}
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{} {}", self.status, self.message) }
}
impl std::error::Error for ProtocolError {}

#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub fp: String,
    pub alias: String,
    pub model: String,
    pub kind: String,
    pub port: u16,
}

fn is_fp(s: &str) -> bool { s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) }

/// Valida le informazioni di un dispositivo; solo HTTPS e impronta SHA-256 a 64 cifre esadecimali.
pub fn parse_peer_info(v: &Value) -> Option<PeerInfo> {
    let o = v.as_object()?;
    let fp = o.get("fingerprint")?.as_str()?;
    if !is_fp(fp) { return None; }
    let port = match o.get("port") {
        None => 53317,
        Some(p) => { let n = p.as_u64()?; if n == 0 || n > 65535 { return None; } n as u16 }
    };
    if let Some(p) = o.get("protocol") { if p.as_str() != Some("https") { return None; } }
    let s = |k: &str, lim, def: &str| clean_label(o.get(k).and_then(Value::as_str).unwrap_or(""), lim, def);
    Some(PeerInfo {
        fp: fp.to_uppercase(), alias: s("alias", 60, "Dispositivo"), model: s("deviceModel", 60, ""),
        kind: s("deviceType", 20, "mobile"), port,
    })
}

#[derive(Debug, Clone)]
pub struct FileMeta {
    pub id: String,
    pub name: String,
    pub parts: Vec<String>,
    pub size: u64,
    pub file_type: String,
    pub sha256: String,
    pub preview: String,
}

pub fn parse_prepare_upload(body: &Value, lim: &Limits) -> Result<(Value, Vec<FileMeta>), ProtocolError> {
    let bad = |m: &str| ProtocolError::new(400, m);
    let o = body.as_object().ok_or_else(|| bad("corpo non valido"))?;
    let files = o.get("files").and_then(Value::as_object).ok_or_else(|| bad("corpo non valido"))?;
    let info = o.get("info").filter(|i| i.is_object()).ok_or_else(|| bad("info mancante"))?.clone();
    if files.is_empty() || files.len() > lim.max_files { return Err(bad("numero di file non valido")); }
    let mut out = Vec::with_capacity(files.len());
    let mut total: u64 = 0;
    for (key, f) in files {
        let f = f.as_object().ok_or_else(|| bad("file non valido"))?;
        if key.is_empty() || key.len() > 128 { return Err(bad("file non valido")); }
        let size = f.get("size").and_then(|s| if s.is_u64() { s.as_u64() } else { None })
            .filter(|s| *s <= lim.max_file_size).ok_or_else(|| bad("dimensione non valida"))?;
        let name = f.get("fileName").and_then(Value::as_str).filter(|n| !n.is_empty() && n.len() <= 1024)
            .ok_or_else(|| bad("nome file non valido"))?;
        let sha = match f.get("sha256") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) if is_fp(s) => s.to_lowercase(),
            _ => return Err(bad("sha256 non valido")),
        };
        let preview = match f.get("preview") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(s)) if s.len() <= lim.max_preview => s.clone(),
            _ => return Err(bad("anteprima non valida")),
        };
        let file_type: String = f.get("fileType").and_then(Value::as_str).unwrap_or("").chars().take(100).collect();
        total = total.saturating_add(size);
        if total > lim.max_total_size { return Err(bad("totale troppo grande")); }
        out.push(FileMeta { id: key.clone(), name: name.to_string(), parts: safe_parts(name), size, file_type, sha256: sha, preview });
    }
    Ok((info, out))
}

/// Un invio di solo testo: ogni "file" ha un'anteprima testuale (come l'app ufficiale).
pub fn is_text_message(files: &[FileMeta]) -> bool {
    !files.is_empty() && files.iter().all(|f| !f.preview.is_empty() && f.file_type.starts_with("text/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(files: Value) -> Value { json!({"info": {"alias": "x", "fingerprint": "A".repeat(64)}, "files": files}) }
    fn meta(extra: Value) -> Value {
        let mut m = json!({"id": "i", "fileName": "a.txt", "size": 3, "fileType": "text/plain"});
        for (k, v) in extra.as_object().unwrap() { m[k] = v.clone(); }
        m
    }
    fn bad(b: Value, l: &Limits) { assert_eq!(parse_prepare_upload(&b, l).unwrap_err().status, 400, "{b}"); }

    #[test]
    fn ok_and_rejects() {
        let l = Limits::default();
        let (_, f) = parse_prepare_upload(&body(json!({"i": meta(json!({"sha256": "a".repeat(64)}))})), &l).unwrap();
        assert_eq!(f[0].size, 3);
        assert_eq!(f[0].sha256, "a".repeat(64));
        for extra in [json!({"size": -1}), json!({"size": "3"}), json!({"size": true}), json!({"size": 1.5}),
                      json!({"size": 1u64 << 60}), json!({"fileName": ""}), json!({"fileName": "x".repeat(2000)}),
                      json!({"sha256": "zz"}), json!({"preview": 5}), json!({"preview": "x".repeat(70000)})] {
            bad(body(json!({"i": meta(extra)})), &l);
        }
        bad(Value::Null, &l);
        bad(json!({"info": {}, "files": {}}), &l);
        bad(json!({"files": {"i": meta(json!({}))}}), &l);
        bad(body(json!({"i": "nope"})), &l);
    }

    #[test]
    fn limits_and_text() {
        let l = Limits { max_files: 2, max_total_size: 10, ..Limits::default() };
        bad(body(json!({"a": meta(json!({})), "b": meta(json!({})), "c": meta(json!({}))})), &l);
        bad(body(json!({"a": meta(json!({"size": 6})), "b": meta(json!({"size": 6}))})), &l);
        let l = Limits::default();
        let (_, f) = parse_prepare_upload(&body(json!({"i": meta(json!({"preview": "ciao"}))})), &l).unwrap();
        assert!(is_text_message(&f));
        let (_, f) = parse_prepare_upload(&body(json!({"i": meta(json!({})), "j": meta(json!({"id": "j", "preview": "x"}))})), &l).unwrap();
        assert!(!is_text_message(&f));
    }

    #[test]
    fn peer_info() {
        let ok = json!({"alias": "Pixel\u{0}", "fingerprint": "a".repeat(64), "port": 53317, "protocol": "https"});
        assert_eq!(parse_peer_info(&ok).unwrap().fp, "A".repeat(64));
        for (k, v) in [("protocol", json!("http")), ("fingerprint", json!("short")), ("port", json!(0)), ("port", json!(true))] {
            let mut x = ok.clone();
            x[k] = v;
            assert!(parse_peer_info(&x).is_none());
        }
        assert!(parse_peer_info(&json!("x")).is_none());
    }
}
