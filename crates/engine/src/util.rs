//! Nomi file sicuri, scrittura senza sovrascrittura, formati e misura della velocità.
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const MAX_COMPONENT_BYTES: usize = 200;

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("collegamento simbolico nel percorso")]
    Symlink,
    #[error("il percorso contiene un file")]
    NotDir,
    #[error("percorso fuori dalla cartella autorizzata")]
    Outside,
    #[error("{0}")]
    Io(#[from] io::Error),
}

pub fn clean_label(v: &str, limit: usize, default: &str) -> String {
    let s: String = v.chars().filter(|c| !c.is_control()).collect();
    let s: String = s.trim().chars().take(limit).collect();
    if s.is_empty() { default.to_string() } else { s }
}

fn truncate_bytes(s: &str, limit: usize) -> String {
    if s.len() <= limit { return s.to_string(); }
    let mut end = limit;
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    s[..end].to_string()
}

pub fn truncate_name(name: &str) -> String {
    if name.len() <= MAX_COMPONENT_BYTES { return name.to_string(); }
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty() && ext.len() <= 20 {
            return format!("{}.{}", truncate_bytes(stem, MAX_COMPONENT_BYTES - ext.len() - 1), ext);
        }
    }
    truncate_bytes(name, MAX_COMPONENT_BYTES)
}

/// Scompone un nome ricevuto in componenti sicuri (niente '..', separatori, caratteri di controllo).
pub fn safe_parts(name: &str) -> Vec<String> {
    let mut parts: Vec<String> = name
        .split(['/', '\\'])
        .map(|p| p.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string())
        .filter(|p| !p.is_empty() && p != "." && p != "..")
        .map(|p| truncate_name(&p))
        .collect();
    if parts.is_empty() { parts.push("file".into()); }
    parts
}

/// Crea le sottocartelle di `parts[..n-1]` sotto `root` e restituisce il percorso finale.
/// Rifiuta ogni componente che sia un collegamento simbolico, anche se punta dentro `root`.
pub fn prepare_target(root: &Path, parts: &[String]) -> Result<PathBuf, PathError> {
    let root = fs::canonicalize(root)?;
    let mut cur = root.clone();
    for comp in &parts[..parts.len() - 1] {
        let next = cur.join(comp);
        if fs::symlink_metadata(&next).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err(PathError::Symlink);
        }
        match fs::create_dir(&next) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        if !next.is_dir() { return Err(PathError::NotDir); }
        cur = next;
    }
    if !fs::canonicalize(&cur)?.starts_with(&root) { return Err(PathError::Outside); }
    let fin = cur.join(parts.last().unwrap());
    if fs::symlink_metadata(&fin).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return Err(PathError::Symlink);
    }
    Ok(fin)
}

/// Sposta `tmp` in `dest` senza mai sovrascrivere: se esiste sceglie "nome (1).ext".
/// Usa un hard link (atomico, fallisce se la destinazione esiste); dove non è supportato
/// ripiega sulla rinomina dopo un controllo di esistenza.
pub fn commit_unique(tmp: &Path, dest: &Path) -> io::Result<PathBuf> {
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let suffix = dest.extension().map(|s| format!(".{}", s.to_string_lossy())).unwrap_or_default();
    for i in 0..10000 {
        let cand = if i == 0 { dest.to_path_buf() } else { dest.with_file_name(format!("{stem} ({i}){suffix}")) };
        match fs::hard_link(tmp, &cand) {
            Ok(()) => { fs::remove_file(tmp)?; return Ok(cand); }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                if fs::symlink_metadata(&cand).is_ok() { continue; }
                fs::rename(tmp, &cand)?;
                return Ok(cand);
            }
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, "troppi nomi in conflitto"))
}

pub fn tipo_file(nome: &str) -> &'static str {
    let e = Path::new(nome).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match e.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "heic" | "heif" | "avif" => "foto",
        "mov" | "mp4" | "m4v" | "avi" | "mkv" | "webm" => "video",
        "mp3" | "m4a" | "wav" | "aac" | "flac" | "ogg" | "opus" => "audio",
        "pdf" => "pdf",
        "zip" | "rar" | "7z" | "tar" | "gz" => "archivio",
        "txt" | "md" | "json" | "csv" | "js" | "py" | "html" | "css" | "ts" => "testo",
        _ => "file",
    }
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn random_hex(bytes: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut b);
    hex::encode(b)
}

pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() { return false; }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Contatore di byte con velocità su finestra mobile.
pub struct Progress {
    inner: Mutex<ProgressInner>,
}
struct ProgressInner { total: u64, samples: VecDeque<(Instant, u64)> }

impl Default for Progress { fn default() -> Self { Self::new() } }
impl Progress {
    const WINDOW: Duration = Duration::from_secs(4);
    pub fn new() -> Self { Progress { inner: Mutex::new(ProgressInner { total: 0, samples: VecDeque::new() }) } }
    pub fn add(&self, n: u64) { self.add_at(Instant::now(), n) }
    fn add_at(&self, now: Instant, n: u64) {
        let mut g = self.inner.lock().unwrap();
        g.total += n;
        let t = g.total;
        if g.samples.back().is_none_or(|(ts, _)| now.duration_since(*ts) >= Duration::from_millis(250)) {
            g.samples.push_back((now, t));
        }
        while g.samples.front().is_some_and(|(ts, _)| now.duration_since(*ts) > Self::WINDOW) {
            g.samples.pop_front();
        }
    }
    pub fn total(&self) -> u64 { self.inner.lock().unwrap().total }
    /// (byte/s, secondi di campioni) se la misura è attendibile (almeno 1 s di campioni recenti).
    fn measure(&self) -> Option<(f64, f64)> { self.measure_at(Instant::now()) }
    fn measure_at(&self, now: Instant) -> Option<(f64, f64)> {
        let g = self.inner.lock().unwrap();
        let (t0, b0) = *g.samples.front()?;
        let (t1, b1) = *g.samples.back()?;
        if g.samples.len() < 2 || now.duration_since(t1) > Duration::from_secs(2) || t1.duration_since(t0) < Duration::from_secs(1) {
            return None;
        }
        let span = t1.duration_since(t0).as_secs_f64();
        Some(((b1.saturating_sub(b0)) as f64 / span, span))
    }
    /// Byte/s se la misura è attendibile (almeno 1 s di campioni recenti).
    pub fn speed(&self) -> Option<f64> { self.measure().map(|m| m.0) }
    /// Secondi rimanenti, solo se la velocità è stabile da almeno 3 s, non nulla, e la stima è sensata (< 24 h).
    /// Altrimenti `None`: meglio non mostrare nulla che un numero inventato.
    pub fn eta(&self, remaining: u64) -> Option<f64> { self.eta_at(Instant::now(), remaining) }
    fn eta_at(&self, now: Instant, remaining: u64) -> Option<f64> {
        let (v, span) = self.measure_at(now)?;
        if span < 3.0 || v < 1.0 || remaining == 0 { return None; }
        let eta = remaining as f64 / v;
        (eta < 86_400.0).then_some(eta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_and_eta_only_when_reliable() {
        let t0 = Instant::now();
        let p = Progress::new();
        assert_eq!((p.measure_at(t0), p.eta_at(t0, 1000)), (None, None), "nessun dato");
        p.add_at(t0, 0);
        p.add_at(t0 + Duration::from_millis(500), 500_000);
        assert!(p.measure_at(t0 + Duration::from_millis(600)).is_none(), "meno di 1 s di campioni");
        for i in 1..=4u64 { p.add_at(t0 + Duration::from_millis(500 + 500 * i), 500_000); }
        let now = t0 + Duration::from_millis(2600);
        let (v, span) = p.measure_at(now).unwrap();
        assert!((v - 1_000_000.0).abs() < 1.0 && span >= 1.5, "{v} {span}");
        assert!(p.eta_at(now, 10_000_000).is_none(), "meno di 3 s di storia: niente tempo residuo");
        p.add_at(t0 + Duration::from_millis(3000), 500_000);
        let now = t0 + Duration::from_millis(3100);
        let eta = p.eta_at(now, 10_000_000).unwrap();
        assert!((eta - 10.0).abs() < 0.5, "{eta}");
        assert!(p.eta_at(now, 0).is_none(), "nulla da trasferire");
        assert!(p.measure_at(now + Duration::from_secs(5)).is_none(), "campioni vecchi: il trasferimento è fermo");
        assert!(p.eta_at(now, u64::MAX / 2).is_none(), "stima assurda (> 24 h)");
    }

    #[test]
    fn traversal_and_separators() {
        assert_eq!(safe_parts("../../etc/passwd"), ["etc", "passwd"]);
        assert_eq!(safe_parts("a\\..\\b/./c"), ["a", "b", "c"]);
        assert_eq!(safe_parts("/abs/path.txt"), ["abs", "path.txt"]);
        assert_eq!(safe_parts(""), ["file"]);
        assert_eq!(safe_parts(".."), ["file"]);
        assert_eq!(safe_parts("a\u{0}b\n.txt"), ["ab.txt"]);
    }

    #[test]
    fn unicode_and_length() {
        assert_eq!(safe_parts("città/è già.txt"), ["città", "è già.txt"]);
        let long = format!("{}.jpg", "é".repeat(300));
        let out = truncate_name(&long);
        assert!(out.len() <= 200);
        assert!(out.ends_with(".jpg"));
    }

    #[test]
    fn symlink_component_rejected() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        let outside = d.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        let r = prepare_target(&root, &["link".into(), "x.txt".into()]);
        assert!(matches!(r, Err(PathError::Symlink)));
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
        std::os::unix::fs::symlink(d.path().join("victim"), root.join("f.txt")).unwrap();
        assert!(matches!(prepare_target(&root, &["f.txt".into()]), Err(PathError::Symlink)));
    }

    #[test]
    fn creates_subdirs() {
        let d = tempfile::tempdir().unwrap();
        let p = prepare_target(d.path(), &["a".into(), "b".into(), "c.txt".into()]).unwrap();
        assert!(p.ends_with("a/b/c.txt"));
        assert!(d.path().join("a/b").is_dir());
    }

    #[test]
    fn commit_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("f.txt");
        fs::write(&dest, "vecchio").unwrap();
        for i in 0..3 {
            let t = d.path().join(format!("t{i}"));
            fs::write(&t, format!("nuovo{i}")).unwrap();
            commit_unique(&t, &dest).unwrap();
            assert!(!t.exists());
        }
        assert_eq!(fs::read_to_string(&dest).unwrap(), "vecchio");
        assert_eq!(fs::read_to_string(d.path().join("f (1).txt")).unwrap(), "nuovo0");
        assert_eq!(fs::read_to_string(d.path().join("f (3).txt")).unwrap(), "nuovo2");
    }

    #[test]
    fn speed_needs_window() {
        let p = Progress::new();
        p.add(100);
        assert!(p.speed().is_none());
        assert_eq!(p.total(), 100);
    }
}
