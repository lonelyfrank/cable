//! Ponte pagina → motore: i metodi che l'interfaccia può chiamare. Indipendente da Tauri/GTK.
//! Gli esiti hanno la stessa forma della versione Python, così l'interfaccia HTML resta valida.
use crate::desktop;
use crate::engine::Engine;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Servizi che solo la shell sa fornire (finestre di dialogo, uscita, notifiche).
pub trait Host: Send + Sync {
    fn scegli_file(&self) -> Vec<String>;
    fn scegli_cartella(&self, iniziale: &str) -> Option<String>;
    fn esci(&self);
    fn ritira_notifica(&self, _id: &str) {}
}

/// Solo la cartella di ricezione e quella delle foto: la pagina non può aprire altro.
pub fn consentito(e: &Engine, p: &str) -> Option<PathBuf> {
    let p = std::fs::canonicalize(p).ok()?;
    ["cartella", "cartella_foto"].iter().filter_map(|k| std::fs::canonicalize(e.sh.settings.str(k)).ok()).find(|r| p.starts_with(r)).map(|_| p)
}

fn ok(b: bool) -> Value { json!({"ok": b}) }
fn s<'a>(a: &'a Value, k: &str) -> &'a str { a[k].as_str().unwrap_or("") }

pub async fn chiama(e: &Arc<Engine>, host: &dyn Host, metodo: &str, a: Value) -> Value {
    let a = if a.is_null() { json!({}) } else { a };
    if !a.is_object() { return json!({"ok": false, "errore": "Chiamata non valida"}); }
    match metodo {
        "stato" => { let e = e.clone(); tokio::task::spawn_blocking(move || e.snapshot()).await.unwrap_or(ok(false)) }
        "diagnostica" => { let e = e.clone(); json!({"ok": true, "testo": tokio::task::spawn_blocking(move || e.diagnostica()).await.unwrap_or_default()}) }
        "cerca" => {
            let ip = a["ip"].as_str().filter(|i| !i.is_empty());
            if let Some(i) = ip { if i.parse::<std::net::Ipv4Addr>().is_err() { return json!({"ok": false, "errore": "Indirizzo IP non valido"}); } }
            json!({"ok": true, "trovati": e.cerca(ip, None).await})
        }
        "invia" => {
            let paths: Vec<String> = a["percorsi"].as_array().map(|v| v.iter().filter_map(|p| p.as_str().map(String::from)).collect()).unwrap_or_default();
            match e.invia(s(&a, "fp"), &paths, a["testo"].as_str().map(String::from)) {
                Ok(id) => json!({"ok": true, "id": id, "errore": ""}),
                Err(er) => json!({"ok": false, "id": null, "errore": er}),
            }
        }
        "annulla_invio" => ok(e.sender.annulla(s(&a, "id"))),
        "pin_invio" => ok(e.sender.fornisci_pin(s(&a, "id"), s(&a, "pin"))),
        "preferito" => ok(e.preferito(s(&a, "fp"), a["on"].as_bool().unwrap_or(false))),
        "dimentica" => ok(e.dimentica(s(&a, "fp"))),
        "decidi" => {
            let ids = match &a["ids"] { Value::Null => None, Value::Array(v) => Some(v.iter().filter_map(|x| x.as_str().map(String::from)).collect()), _ => return ok(false) };
            let r = e.decidi(s(&a, "id"), ids);
            host.ritira_notifica(s(&a, "id"));
            ok(r)
        }
        "annulla_ricezione" => ok(e.receiver.annulla(s(&a, "id"))),
        "msg_rimuovi" => { e.receiver.rimuovi_messaggio(s(&a, "id")); ok(true) }
        "conf" => { let m: Map<String, Value> = a.as_object().cloned().unwrap_or_default(); json!({"ok": true, "aggiornati": e.aggiorna_impostazioni(&m)}) }
        "pin" => {
            let r = e.imposta_pin(a["pin"].as_str());
            json!({"ok": r, "errore": if r { "" } else { "Il PIN deve avere da 4 a 32 caratteri" }})
        }
        "ripristina" => json!({"ok": true, "backup": e.ripristina_impostazioni()}),
        "rigenera_identita" => ok(e.rigenera_identita().await),
        "riprova_server" => ok(e.riprova_server().await),
        "apri" => ok(consentito(e, s(&a, "path")).map_or(false, |p| desktop::apri(&p))),
        "mostra" => ok(consentito(e, s(&a, "path")).map_or(false, |p| desktop::mostra(&p))),
        "apri_cartella" => {
            let k = if s(&a, "quale") == "foto" { "cartella_foto" } else { "cartella" };
            let p = PathBuf::from(e.sh.settings.str(k));
            let _ = std::fs::create_dir_all(&p);
            ok(desktop::apri(&p))
        }
        "scegli_file" => { let p = host.scegli_file(); json!({"ok": !p.is_empty(), "percorsi": p}) }
        "scegli_cartella" => {
            let k = if s(&a, "quale") == "foto" { "cartella_foto" } else { "cartella" };
            match host.scegli_cartella(&e.sh.settings.str(k)) {
                Some(p) => { e.aggiorna_impostazioni(json!({k: p}).as_object().unwrap()); ok(true) }
                None => ok(false),
            }
        }
        // ---- Android opzionale
        "schermo" => if s(&a, "azione") == "avvia" { ok(e.android.avvia_schermo().await) } else { e.android.ferma_schermo(); ok(true) },
        "android_importa" => ok(e.android.importa()),
        "android_apri" => ok(e.android.apri_file_manager()),
        "android_riprova" => ok(e.android.riprova().await),
        // ---- wizard di primo collegamento
        "wizard_verifica_inizia" => match e.verifica_inizia(s(&a, "fp")) { Ok(()) => ok(true), Err(er) => json!({"ok": false, "errore": er}) },
        "wizard_verifica_conferma" => e.verifica_conferma(s(&a, "code")),
        "wizard_prova" => ok(e.wizard_prova(s(&a, "fp"))),
        "wizard_completa" => { e.wizard_completa(); ok(true) }
        // ---- applicazione
        "esci" => { host.esci(); ok(true) }
        "js_error" => { crate::dlog!("JS {} {}", s(&a, "m"), a["l"]); ok(true) }
        _ => json!({"ok": false, "errore": "Chiamata sconosciuta"}),
    }
}

/// Anteprima consentita solo per immagini dentro le cartelle configurate.
pub fn anteprima_ok(e: &Engine, path: &str) -> Option<PathBuf> {
    let p = consentito(e, path)?;
    let ext = Path::new(&p).extension()?.to_string_lossy().to_lowercase();
    (p.is_file() && ["jpg", "jpeg", "png", "gif", "webp", "bmp", "avif"].contains(&ext.as_str())).then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;

    struct H;
    impl Host for H {
        fn scegli_file(&self) -> Vec<String> { vec!["/x".into()] }
        fn scegli_cartella(&self, _: &str) -> Option<String> { None }
        fn esci(&self) {}
    }

    #[tokio::test]
    async fn paths_limited_dispatch_rejects_and_conf_filters() {
        let t = tempfile::tempdir().unwrap();
        let e = Engine::new(Paths::under(t.path()), 0, false);
        let root = PathBuf::from(e.sh.settings.str("cartella"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("f.txt"), "x").unwrap();
        assert!(consentito(&e, root.join("f.txt").to_str().unwrap()).is_some());
        assert!(consentito(&e, "/etc/passwd").is_none());
        assert!(consentito(&e, root.join("../../x").to_str().unwrap()).is_none());
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();
        assert!(consentito(&e, root.join("link").to_str().unwrap()).is_none());
        assert_eq!(chiama(&e, &H, "apri", json!({"path": "/etc/passwd"})).await["ok"], false);
        assert_eq!(chiama(&e, &H, "__class__", json!({})).await["ok"], false);
        assert_eq!(chiama(&e, &H, "cerca", json!({"ip": "not-an-ip"})).await["ok"], false);
        assert_eq!(chiama(&e, &H, "decidi", json!({"id": "x", "ids": "tutti"})).await["ok"], false);
        let r = chiama(&e, &H, "conf", json!({"nome_pc": "Nuovo", "pin_ricezione": null, "schema": 1, "preferiti": {"x": 1}})).await;
        assert_eq!(r["aggiornati"], json!(["nome_pc"]));
        std::fs::write(root.join("a.png"), b"\x89PNG").unwrap();
        assert!(anteprima_ok(&e, root.join("a.png").to_str().unwrap()).is_some());
        assert!(anteprima_ok(&e, root.join("f.txt").to_str().unwrap()).is_none());
        assert!(anteprima_ok(&e, "/etc/hostname").is_none());
    }
}
