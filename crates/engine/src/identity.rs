//! Identità TLS persistente (certificato autofirmato + chiave) nei dati utente.
use crate::paths::Paths;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{ring as ring_provider, CryptoProvider};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, ServerConfig, SignatureScheme};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

pub fn fingerprint_der(der: &[u8]) -> String { hex::encode_upper(Sha256::digest(der)) }

fn provider() -> Arc<CryptoProvider> { Arc::new(ring_provider::default_provider()) }

fn write_private(p: &PathBuf, data: &[u8]) -> std::io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(p)?;
    f.write_all(data)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))
}

fn parse(cert_pem: &[u8], key_pem: &[u8]) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>, String), String> {
    let chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &cert_pem[..]).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
    let first = chain.first().ok_or("certificato vuoto")?;
    let key = rustls_pemfile::private_key(&mut &key_pem[..]).map_err(|e| e.to_string())?.ok_or("chiave mancante")?;
    let signing = ring_provider::sign::any_supported_type(&key).map_err(|e| format!("chiave non supportata: {e}"))?;
    rustls::sign::CertifiedKey::new(chain.clone(), signing).keys_match().map_err(|_| "la chiave non corrisponde al certificato".to_string())?;
    let fp = fingerprint_der(first.as_ref());
    Ok((chain, key, fp))
}

fn generate() -> Result<(String, String), String> {
    use rcgen::{date_time_ymd, CertificateParams, DistinguishedName, DnType, KeyPair};
    let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| e.to_string())?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "Cable");
    params.distinguished_name = dn;
    params.not_before = date_time_ymd(2025, 1, 1);
    params.not_after = date_time_ymd(2045, 1, 1);
    let kp = KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&kp).map_err(|e| e.to_string())?;
    Ok((cert.pem(), kp.serialize_pem()))
}

/// stato: "ok" | "invalida". Un'identità illeggibile NON viene rigenerata da sola.
pub struct Identity {
    paths: Paths,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub fingerprint: String,
    pub stato: String,
    pub errore: String,
    pub origine: String,
    chain: Vec<CertificateDer<'static>>,
    key: Option<PrivateKeyDer<'static>>,
}

impl Identity {
    pub fn load(paths: &Paths) -> Identity {
        let dir = paths.identity_dir();
        let mut me = Identity {
            paths: paths.clone(), cert_path: dir.join("cert.pem"), key_path: dir.join("key.pem"),
            fingerprint: String::new(), stato: "invalida".into(), errore: String::new(), origine: String::new(),
            chain: vec![], key: None,
        };
        me.carica();
        me
    }

    fn adopt(&mut self, cert: &[u8], key: &[u8], origine: &str) -> Result<(), String> {
        let (chain, k, fp) = parse(cert, key)?;
        self.chain = chain; self.key = Some(k); self.fingerprint = fp;
        self.stato = "ok".into(); self.errore.clear(); self.origine = origine.into();
        Ok(())
    }

    fn carica(&mut self) {
        let dir = self.cert_path.parent().unwrap().to_path_buf();
        if fs::create_dir_all(&dir).is_err() { self.errore = "Cartella dell'identità non utilizzabile".into(); return; }
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
        match (self.cert_path.exists(), self.key_path.exists()) {
            (true, true) => {
                let r = fs::read(&self.cert_path).and_then(|c| fs::read(&self.key_path).map(|k| (c, k)));
                match r {
                    Ok((c, k)) => if let Err(e) = self.adopt(&c, &k, "esistente") { self.errore = format!("Identità non valida: {e}"); },
                    Err(e) => self.errore = format!("Identità non leggibile: {e}"),
                }
            }
            (false, false) => {
                if self.importa_v3() { return; }
                match generate().and_then(|(c, k)| {
                    fs::write(&self.cert_path, &c).map_err(|e| e.to_string())?;
                    write_private(&self.key_path, k.as_bytes()).map_err(|e| e.to_string())?;
                    self.adopt(c.as_bytes(), k.as_bytes(), "creata")
                }) {
                    Ok(()) => {}
                    Err(e) => self.errore = format!("Creazione dell'identità non riuscita: {e}"),
                }
            }
            _ => self.errore = "Identità incompleta: manca certificato o chiave.".into(),
        }
    }

    fn importa_v3(&mut self) -> bool {
        let old = self.paths.legacy_cache();
        let (Ok(c), Ok(k)) = (fs::read(old.join("cert.pem")), fs::read(old.join("key.pem"))) else { return false };
        if parse(&c, &k).is_err() { return false; }
        if fs::write(&self.cert_path, &c).is_err() || write_private(&self.key_path, &k).is_err() { return false; }
        self.adopt(&c, &k, "importata-v3").is_ok()
    }

    /// Azione esplicita dell'utente: conserva i vecchi file in una cartella di backup.
    pub fn rigenera(&mut self) -> Result<String, String> {
        let bk = self.cert_path.parent().unwrap().join(format!("backup-{}", chrono::Local::now().format("%Y%m%d-%H%M%S")));
        fs::create_dir_all(&bk).map_err(|e| e.to_string())?;
        let _ = fs::set_permissions(&bk, fs::Permissions::from_mode(0o700));
        for p in [&self.cert_path, &self.key_path] {
            if p.exists() { fs::rename(p, bk.join(p.file_name().unwrap())).map_err(|e| e.to_string())?; }
        }
        let (c, k) = generate()?;
        fs::write(&self.cert_path, &c).map_err(|e| e.to_string())?;
        write_private(&self.key_path, k.as_bytes()).map_err(|e| e.to_string())?;
        self.adopt(c.as_bytes(), k.as_bytes(), "creata")?;
        Ok(self.fingerprint.clone())
    }

    pub fn server_config(&self) -> Result<Arc<ServerConfig>, String> {
        let key = self.key.as_ref().ok_or("identità non valida")?.clone_key();
        let mut cfg = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions().map_err(|e| e.to_string())?
            .with_no_client_auth().with_single_cert(self.chain.clone(), key).map_err(|e| e.to_string())?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(cfg))
    }

    /// Configurazione client che accetta qualunque certificato autofirmato e registra l'impronta vista:
    /// l'identità LocalSend è l'impronta SHA-256, confrontata dal chiamante con quella attesa.
    pub fn client_config(&self) -> Result<(Arc<ClientConfig>, Arc<Mutex<Option<String>>>), String> {
        let seen = Arc::new(Mutex::new(None));
        let verifier = Arc::new(Recorder { seen: seen.clone(), provider: provider() });
        let key = self.key.as_ref().ok_or("identità non valida")?.clone_key();
        let mut cfg = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions().map_err(|e| e.to_string())?
            .dangerous().with_custom_certificate_verifier(verifier)
            .with_client_auth_cert(self.chain.clone(), key).map_err(|e| e.to_string())?;
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok((Arc::new(cfg), seen))
    }
}

#[derive(Debug)]
struct Recorder {
    seen: Arc<Mutex<Option<String>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Recorder {
    fn verify_server_cert(&self, end_entity: &CertificateDer<'_>, _i: &[CertificateDer<'_>], _n: &ServerName<'_>,
                          _o: &[u8], _now: UnixTime) -> Result<ServerCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = Some(fingerprint_der(end_entity.as_ref()));
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> { self.provider.signature_verification_algorithms.supported_schemes() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_reload_permissions_and_location() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let a = Identity::load(&p);
        assert_eq!(a.stato, "ok", "{}", a.errore);
        assert_eq!(a.fingerprint.len(), 64);
        assert_eq!(fs::metadata(&a.key_path).unwrap().permissions().mode() & 0o777, 0o600);
        assert!(a.key_path.starts_with(&p.data));
        assert_eq!(Identity::load(&p).fingerprint, a.fingerprint);
        a.server_config().unwrap();
        a.client_config().unwrap();
    }

    /// Una chiave RSA come quelle create dalle versioni Python, generata al momento con openssl
    /// (nessuna chiave è versionata nel repository). Salta il test se openssl non c'è.
    #[test]
    fn imports_rsa_identity_from_v3_cache_and_reads_it_back() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        fs::create_dir_all(p.legacy_cache()).unwrap();
        let (c, k) = (p.legacy_cache().join("cert.pem"), p.legacy_cache().join("key.pem"));
        let gen = std::process::Command::new("openssl")
            .args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=Cable", "-keyout"])
            .arg(&k).arg("-out").arg(&c).output();
        if !gen.map(|o| o.status.success()).unwrap_or(false) { eprintln!("openssl non disponibile: test saltato"); return; }
        let der = std::process::Command::new("openssl").args(["x509", "-outform", "DER", "-in"]).arg(&c).output().unwrap().stdout;
        let i = Identity::load(&p);
        assert_eq!(i.stato, "ok", "{}", i.errore);
        assert_eq!(i.origine, "importata-v3");
        assert_eq!(i.fingerprint, fingerprint_der(&der));
        i.server_config().unwrap();       // la chiave RSA è usabile per firmare in TLS
        assert!(k.exists());              // l'originale non viene toccato
    }

    #[test]
    fn corrupt_not_silently_replaced_and_explicit_regeneration() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let mut a = Identity::load(&p);
        let old = a.fingerprint.clone();
        fs::write(&a.key_path, "spazzatura").unwrap();
        let b = Identity::load(&p);
        assert_eq!(b.stato, "invalida");
        assert_eq!(fs::read_to_string(&b.key_path).unwrap(), "spazzatura");
        let new = a.rigenera().unwrap();
        assert_ne!(new, old);
        assert_eq!(fs::read_dir(p.identity_dir()).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().starts_with("backup-")).count(), 1);
        assert_eq!(Identity::load(&p).stato, "ok");
    }

    #[test]
    fn mismatched_pair_invalid() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::under(t.path());
        let a = Identity::load(&p);
        let other = Identity::load(&Paths::under(&t.path().join("x")));
        fs::copy(&other.key_path, &a.key_path).unwrap();
        assert_eq!(Identity::load(&p).stato, "invalida");
    }
}
