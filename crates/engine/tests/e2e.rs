//! Prove end-to-end fra due motori reali su HTTPS (loopback). Non provano la compatibilità con i
//! client LocalSend ufficiali: per quella vedi docs/CHECKLIST-MANUALE.md.
mod common;
use common::*;
use pixelhub_engine::client::json_request;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};

fn s(p: &std::path::Path) -> String { p.to_string_lossy().into_owned() }

#[tokio::test(flavor = "multi_thread")]
async fn discovery_register_both_ways() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("sorgente").await, r.engine("destinazione").await);
    pair(&a, &b).await;
    assert!(a.sh.registry.get(&b.sh.fingerprint()).unwrap().verified);       // certificato visto e coerente
    let peer = wait_for(|| async { b.sh.registry.get(&a.sh.fingerprint()) }, 5.0).await;
    assert_eq!(peer.alias, "sorgente");
    assert!(!peer.verified);                                                 // b non ha mai visto il certificato di a
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn send_file_text_reject_end_to_end() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    // file da 3 MiB + 17 byte
    let data: Vec<u8> = (0..3 * 1024 * 1024 + 17).map(|i| (i * 31 % 251) as u8).collect();
    let src = make(&r, "dati.bin", &data);
    let jid = a.invia(&fp, &[s(&src)], None).unwrap();
    accept_when_pending(&b, None).await;
    let j = wait_job(&a, &jid, 20.0).await;
    assert_eq!(j["stato"], "completato", "{j}");
    assert_eq!(hex::encode(Sha256::digest(std::fs::read(inbox(&b).join("dati.bin")).unwrap())), hex::encode(Sha256::digest(&data)));
    let rx = b.receiver.snapshot().0[0].clone();
    assert_eq!((rx["stato"].as_str().unwrap(), rx["ricevuti"].as_u64().unwrap()), ("completata", 1));
    // testo
    let jid = a.invia(&fp, &[], Some("ciao dal PC".into())).unwrap();
    accept_when_pending(&b, None).await;
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "completato");
    assert_eq!(b.receiver.snapshot().1[0]["testo"], "ciao dal PC");
    // testo rifiutato
    let jid = a.invia(&fp, &[], Some("no grazie".into())).unwrap();
    accept_when_pending(&b, Some(vec![])).await;
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "rifiutato");
    assert_eq!(b.receiver.snapshot().1.as_array().unwrap().len(), 1);
    // file rifiutati
    let jid = a.invia(&fp, &[s(&make(&r, "x.txt", b"x"))], None).unwrap();
    accept_when_pending(&b, Some(vec![])).await;
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "rifiutato");
    assert!(!listing(&inbox(&b)).contains(&"x.txt".to_string()));
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_selection_makes_job_partial_and_folders_keep_structure() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    let paths: Vec<String> = ["a.txt", "b.txt", "c.txt"].iter().map(|n| s(&make(&r, n, n.repeat(100).as_bytes()))).collect();
    let jid = a.invia(&fp, &paths, None).unwrap();
    let req = wait_for(|| async { b.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa").cloned() }, 10.0).await;
    let pick: Vec<String> = req["file"].as_array().unwrap().iter().filter(|f| f["nome"] != "b.txt").map(|f| f["id"].as_str().unwrap().to_string()).collect();
    b.decidi(req["id"].as_str().unwrap(), Some(pick));
    let j = wait_job(&a, &jid, 10.0).await;
    assert_eq!(j["stato"], "parziale", "{j}");                            // mai «completato» se il ricevente ne ha accettati solo alcuni
    assert_eq!(listing(&inbox(&b)), ["a.txt", "c.txt"]);
    // cartella
    make(&r, "album/uno.txt", b"1");
    make(&r, "album/sotto/due.txt", b"2");
    let jid = a.invia(&fp, &[s(&r.path("src/album"))], None).unwrap();
    accept_when_pending(&b, None).await;
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "completato");
    assert_eq!(std::fs::read_to_string(inbox(&b).join("album/sotto/due.txt")).unwrap(), "2");
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pin_flow_wrong_then_right_and_cancel_at_prompt() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    b.imposta_pin(Some("424242"));
    let jid = a.invia(&fp, &[s(&make(&r, "p.txt", b"pin"))], None).unwrap();
    let j = wait_for(|| async { let j = job(&a, &jid); (j["stato"] == "pin").then_some(j) }, 10.0).await;
    assert_eq!(j["pin_errato"], false);
    assert!(a.sender.fornisci_pin(&jid, "000000"));
    let j = wait_for(|| async { let j = job(&a, &jid); (j["stato"] == "pin" && j["pin_errato"] == true).then_some(j) }, 10.0).await;
    assert_eq!(j["pin_errato"], true);
    assert!(a.sender.fornisci_pin(&jid, "424242"));
    accept_when_pending(&b, None).await;
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "completato");
    assert!(!a.snapshot().to_string().contains("424242"));
    assert!(!std::fs::read_to_string(a.sh.paths.settings_file()).unwrap().contains("424242"));
    // annullo mentre chiede il PIN
    let jid = a.invia(&fp, &[s(&make(&r, "q.txt", b"q"))], None).unwrap();
    wait_for(|| async { (job(&a, &jid)["stato"] == "pin").then_some(()) }, 10.0).await;
    assert!(a.sender.annulla(&jid));
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "annullato");
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_during_large_transfer_stops_io_and_cleans_up() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let src = r.path("src/grande.bin");
    sparse(&src, 600 * 1024 * 1024);
    let jid = a.invia(&b.sh.fingerprint(), &[s(&src)], None).unwrap();
    accept_when_pending(&b, None).await;
    wait_for(|| async { (job(&a, &jid)["inviati"].as_u64().unwrap_or(0) > 1024 * 1024).then_some(()) }, 20.0).await;
    let t0 = Instant::now();
    assert!(a.sender.annulla(&jid));
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "annullato");
    assert!(t0.elapsed() < Duration::from_secs(5));
    wait_for(|| async { matches!(b.receiver.snapshot().0[0]["stato"].as_str().unwrap(), "annullata" | "parziale").then_some(()) }, 10.0).await;
    wait_for(|| async { listing(&inbox(&b)).is_empty().then_some(()) }, 10.0).await;     // nessun parziale né file finale
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn receiver_cancel_stops_sender() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let src = r.path("src/grande.bin");
    sparse(&src, 600 * 1024 * 1024);
    let jid = a.invia(&b.sh.fingerprint(), &[s(&src)], None).unwrap();
    let req = accept_when_pending(&b, None).await;
    wait_for(|| async { (b.receiver.snapshot().0[0]["byte"].as_u64().unwrap() > 1024 * 1024).then_some(()) }, 20.0).await;
    assert!(b.receiver.annulla(req["id"].as_str().unwrap()));
    let j = wait_job(&a, &jid, 20.0).await;
    assert!(["errore", "annullato"].contains(&j["stato"].as_str().unwrap()), "{j}");
    wait_for(|| async { listing(&inbox(&b)).is_empty().then_some(()) }, 10.0).await;
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn sender_cancel_while_receiver_is_still_asking_marks_request_cancelled() {
    // il mittente chiude la connessione mentre l'utente non ha ancora deciso: la richiesta non resta appesa
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let jid = a.invia(&b.sh.fingerprint(), &[s(&make(&r, "x.txt", b"x"))], None).unwrap();
    wait_for(|| async { b.receiver.snapshot().0.as_array().unwrap().iter().any(|i| i["stato"] == "attesa").then_some(()) }, 10.0).await;
    assert!(a.sender.annulla(&jid));
    wait_for(|| async { (b.receiver.snapshot().0[0]["stato"] == "annullata").then_some(()) }, 5.0).await;
    r.stop_all().await;
}

#[tokio::test(flavor = "current_thread")]
async fn source_modified_or_removed_after_selection() {
    // current_thread: il task d'invio non parte finché non aspettiamo, quindi la modifica è deterministica
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    let m = make(&r, "m.txt", b"originale");
    let jid = a.invia(&fp, &[s(&m)], None).unwrap();
    std::fs::write(&m, b"cambiato dopo la selezione").unwrap();
    let j = wait_job(&a, &jid, 10.0).await;
    assert_eq!(j["stato"], "errore");
    assert!(j["errore"].as_str().unwrap().contains("modificato"), "{j}");
    let g = make(&r, "sparisce.txt", b"x");
    let jid = a.invia(&fp, &[s(&g)], None).unwrap();
    std::fs::remove_file(&g).unwrap();
    let j = wait_job(&a, &jid, 10.0).await;
    assert_eq!(j["stato"], "errore");
    assert!(j["errore"].as_str().unwrap().contains("rimosso"), "{j}");
    assert!(listing(&inbox(&b)).is_empty());
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn source_changes_during_upload_is_detected_midway() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let src = r.path("src/vivo.bin");
    sparse(&src, 400 * 1024 * 1024);
    let jid = a.invia(&b.sh.fingerprint(), &[s(&src)], None).unwrap();
    accept_when_pending(&b, None).await;
    wait_for(|| async { (job(&a, &jid)["inviati"].as_u64().unwrap_or(0) > 1024 * 1024).then_some(()) }, 20.0).await;
    {
        use std::io::{Seek, Write};
        let mut f = std::fs::OpenOptions::new().write(true).open(&src).unwrap();
        f.seek(std::io::SeekFrom::Start(0)).unwrap();
        f.write_all(b"modificato").unwrap();
    }
    let j = wait_job(&a, &jid, 60.0).await;
    assert_eq!(j["stato"], "errore", "{j}");
    assert!(j["file"][0]["errore"].as_str().unwrap().contains("cambiato"), "{j}");
    assert!(j["inviati"].as_u64().unwrap() < 400 * 1024 * 1024);                 // interrotto prima della fine
    wait_for(|| async { (b.receiver.snapshot().0[0]["file"][0]["stato"] == "attesa").then_some(()) }, 10.0).await;
    assert!(listing(&inbox(&b)).is_empty());                                     // il destinatario non ha salvato nulla
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn identity_mismatch_unreachable_and_hostile_clients() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    // l'annuncio dichiarava un'impronta diversa dal certificato reale
    let mut dev = a.sh.registry.get(&b.sh.fingerprint()).unwrap();
    dev.fp = "F".repeat(64);
    a.sh.registry.upsert(&json!({"alias": "falso", "fingerprint": dev.fp, "port": b.port()}), "127.0.0.1", "multicast", None);
    let jid = a.invia(&"F".repeat(64), &[], Some("x".into())).unwrap();
    let j = wait_job(&a, &jid, 10.0).await;
    assert_eq!(j["stato"], "errore");
    assert!(j["errore"].as_str().unwrap().to_lowercase().contains("identità"), "{j}");
    assert!(listing(&inbox(&b)).is_empty());
    // HTTP in chiaro sulla porta HTTPS, spazzatura, corpo troppo grande: il server regge
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut raw = tokio::net::TcpStream::connect(("127.0.0.1", b.port())).await.unwrap();
    raw.write_all(b"GET /api/localsend/v2/info HTTP/1.1\r\nHost: x\r\n\r\n").await.unwrap();
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(Duration::from_secs(5), raw.read(&mut buf)).await.unwrap_or(Ok(0)).unwrap_or(0);
    assert!(!String::from_utf8_lossy(&buf[..n]).contains("fingerprint"));
    let mut raw = tokio::net::TcpStream::connect(("127.0.0.1", b.port())).await.unwrap();
    raw.write_all(&[0u8, 255].repeat(50)).await.unwrap();
    let big = json!({"x": "y".repeat(9 * 1024 * 1024)});
    // il server risponde 413 e chiude senza leggere il corpo: il client vede 413 oppure l'errore di scrittura
    match json_request(a.sh.client_tls().unwrap(), "127.0.0.1", b.port(), hyper::Method::POST, "/api/localsend/v2/prepare-upload", Some(&big), Duration::from_secs(20), None).await {
        Ok((st, _, _)) => assert_eq!(st, 413),
        Err(_) => {}
    }
    assert!(b.receiver.snapshot().0.as_array().unwrap().is_empty());          // nessuna richiesta creata
    let (st, v, fp) = json_request(a.sh.client_tls().unwrap(), "127.0.0.1", b.port(), hyper::Method::GET, "/api/localsend/v2/info", None, Duration::from_secs(5), None).await.unwrap();
    assert_eq!((st, fp), (200, b.sh.fingerprint()));
    assert_eq!(v.unwrap()["alias"], "b");
    // destinatario spento
    b.stop().await;
    let jid = a.invia(&dev.fp.clone(), &[], Some("x".into())).unwrap();
    assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "errore");
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn no_auto_accept_even_with_favorite_fingerprint() {
    let mut r = Rig::new();
    let (a, b, c) = (r.engine("a").await, r.engine("b").await, r.engine("estraneo").await);
    pair(&a, &b).await;
    b.preferito(&a.sh.fingerprint(), true);
    let body = json!({"info": {"alias": "furbo", "fingerprint": a.sh.fingerprint()},
                      "files": {"i": {"id": "i", "fileName": "evil.txt", "size": 1, "fileType": "text/plain"}}});
    let tls = c.sh.client_tls().unwrap();
    let port = b.port();
    let h = tokio::spawn(async move { json_request(tls, "127.0.0.1", port, hyper::Method::POST, "/api/localsend/v2/prepare-upload", Some(&body), Duration::from_secs(20), None).await });
    let req = wait_for(|| async { b.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa").cloned() }, 10.0).await;
    assert!(listing(&inbox(&b)).is_empty());
    b.decidi(req["id"].as_str().unwrap(), Some(vec![]));
    assert_eq!(h.await.unwrap().unwrap().0, 403);
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn big_transfer_keeps_memory_bounded_and_duplicates_are_not_overwritten() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    let src = r.path("src/enorme.bin");
    sparse(&src, 300 * 1024 * 1024);
    let before = rss_hwm_kb();
    let jid = a.invia(&fp, &[s(&src)], None).unwrap();
    accept_when_pending(&b, None).await;
    assert_eq!(wait_job(&a, &jid, 120.0).await["stato"], "completato");
    let grown_mb = (rss_hwm_kb() - before.min(rss_hwm_kb())) as f64 / 1024.0;
    assert!(grown_mb < 60.0, "la memoria è cresciuta di {grown_mb:.0} MB");
    assert_eq!(std::fs::metadata(inbox(&b).join("enorme.bin")).unwrap().len(), 300 * 1024 * 1024);
    for n in 0..2 {
        let jid = a.invia(&fp, &[s(&make(&r, "stesso.txt", format!("v{n}").as_bytes()))], None).unwrap();
        accept_when_pending(&b, None).await;
        assert_eq!(wait_job(&a, &jid, 10.0).await["stato"], "completato");
    }
    let l = listing(&inbox(&b));
    assert!(l.contains(&"stesso.txt".to_string()) && l.contains(&"stesso (1).txt".to_string()), "{l:?}");
    r.stop_all().await;
}

// ---------------------------------------------------------------------- wizard
#[tokio::test(flavor = "multi_thread")]
async fn wizard_code_verification_flow() {
    let mut r = Rig::new();
    let (pc, phone) = (r.engine("pc").await, r.engine("telefono").await);
    pair(&pc, &phone).await;
    let fp = phone.sh.fingerprint();
    assert!(!pc.snapshot()["conf"]["wizard_fatto"].as_bool().unwrap());
    pc.verifica_inizia(&fp).unwrap();
    // il «telefono» vede il messaggio con il codice e lo accetta
    accept_when_pending(&phone, None).await;
    let code = wait_for(|| async {
        let m = phone.receiver.snapshot().1;
        m.as_array().unwrap().first().map(|x| x["testo"].as_str().unwrap().to_string())
    }, 10.0).await;
    let code = code.split("codice di verifica: ").nth(1).unwrap()[..6].to_string();
    wait_for(|| async { (pc.snapshot()["wizard"]["invio"]["stato"] == "completato").then_some(()) }, 10.0).await;
    assert!(!pc.snapshot()["invii"].to_string().contains("codice di verifica"), "l'invio del codice non deve comparire fra i trasferimenti");
    // codici sbagliati e formato
    assert_eq!(pc.verifica_conferma("12")["motivo"], "format");
    let wrong = if code == "000000" { "111111" } else { "000000" };
    let r1 = pc.verifica_conferma(wrong);
    assert_eq!((r1["ok"].as_bool().unwrap(), r1["motivo"].as_str().unwrap()), (false, "wrong"));
    assert!(!pc.sh.settings.preferiti().contains_key(&fp));                  // nulla è stato salvato
    // codice giusto: dispositivo confermato e preferito
    let ok = pc.verifica_conferma(&code);
    assert_eq!(ok["ok"], true, "{ok}");
    let d = pc.snapshot()["dispositivi"].as_array().unwrap().iter().find(|d| d["fp"] == json!(fp)).cloned().unwrap();
    assert_eq!((d["preferito"].clone(), d["confermato"].clone(), d["verificato"].clone()), (json!(true), json!(true), json!(true)));
    assert_eq!(pc.verifica_conferma(&code)["motivo"], "missing");           // il codice vale una volta sola
    // prova inversa: il telefono manda un testo al PC e il PC lo accetta
    assert!(pc.wizard_prova(&fp));
    assert_eq!(pc.snapshot()["wizard"]["prova_ricevuta"], false);
    let jid = phone.invia(&pc.sh.fingerprint(), &[], Some("prova dal telefono".into())).unwrap();
    accept_when_pending(&pc, None).await;
    assert_eq!(wait_job(&phone, &jid, 10.0).await["stato"], "completato");
    wait_for(|| async { pc.snapshot()["wizard"]["prova_ricevuta"].as_bool().unwrap().then_some(()) }, 5.0).await;
    pc.wizard_completa();
    assert!(pc.snapshot()["conf"]["wizard_fatto"].as_bool().unwrap());
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wizard_five_wrong_codes_lock_until_new_code() {
    let mut r = Rig::new();
    let (pc, phone) = (r.engine("pc").await, r.engine("telefono").await);
    pair(&pc, &phone).await;
    let fp = phone.sh.fingerprint();
    assert!(pc.verifica_inizia("0".repeat(64).as_str()).is_err());           // dispositivo non scelto/sconosciuto
    pc.verifica_inizia(&fp).unwrap();
    for i in 0..5 {
        let v = pc.verifica_conferma("999999");
        if i < 5 { assert_eq!(v["motivo"], "wrong"); }
    }
    assert_eq!(pc.verifica_conferma("999999")["motivo"], "attempts");
    assert!(!pc.sh.settings.preferiti().contains_key(&fp));
    r.stop_all().await;
}

// ---------------------------------------------------------------------- motore: preferiti, ripristino, identità, porta
#[tokio::test(flavor = "multi_thread")]
async fn favorites_reset_identity_and_port_handling() {
    let mut r = Rig::new();
    let (a, b) = (r.engine("a").await, r.engine("b").await);
    pair(&a, &b).await;
    let fp = b.sh.fingerprint();
    a.preferito(&fp, true);
    let d = a.snapshot()["dispositivi"].as_array().unwrap().iter().find(|d| d["fp"] == json!(fp)).cloned().unwrap();
    assert_eq!((d["preferito"].clone(), d["verificato"].clone(), d["confermato"].clone()), (json!(true), json!(true), json!(false)));
    a.dimentica(&fp);
    assert!(a.snapshot()["dispositivi"].as_array().unwrap().iter().all(|d| d["fp"] != json!(fp)));
    // ripristino: file e identità restano
    a.aggiorna_impostazioni(json!({"nome_pc": "Altro"}).as_object().unwrap());
    a.imposta_pin(Some("123456"));
    let ident = a.sh.fingerprint();
    std::fs::write(inbox(&a).join("mio.txt"), "x").unwrap();
    let bk = a.ripristina_impostazioni();
    assert!(std::path::Path::new(&bk).exists());
    assert_eq!(a.sh.settings.str("nome_pc"), "Fedora Pixel Hub");
    assert!(!a.snapshot()["pin_impostato"].as_bool().unwrap());
    assert!(inbox(&a).join("mio.txt").exists() && a.sh.fingerprint() == ident);
    // rigenerazione esplicita
    assert!(a.rigenera_identita().await);
    assert_ne!(a.sh.fingerprint(), ident);
    assert!(a.snapshot()["server"]["attivo"].as_bool().unwrap());
    // porta occupata e ripresa
    let c = pixelhub_engine::Engine::new(pixelhub_engine::paths::Paths::under(&r.path("c")), a.port(), false);
    assert!(!c.start().await);
    assert_eq!(c.snapshot()["server"]["errore"], "porta");
    assert!(c.diagnostica().to_lowercase().contains("porta"));
    a.stop().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(c.riprova_server().await);
    c.stop().await;
    r.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn corrupt_identity_blocks_server_and_is_never_silently_regenerated() {
    let r = Rig::new();
    let paths = pixelhub_engine::paths::Paths::under(&r.path("x"));
    let e0 = pixelhub_engine::Engine::new(paths.clone(), free_port(), false);
    let key = paths.identity_dir().join("key.pem");
    drop(e0);
    std::fs::write(&key, "rotta").unwrap();
    let e = pixelhub_engine::Engine::new(paths, free_port(), false);
    assert!(!e.start().await);
    let s = e.snapshot();
    assert_eq!((s["identita"]["stato"].clone(), s["server"]["errore"].clone()), (json!("invalida"), json!("identita")));
    assert_eq!(std::fs::read_to_string(&key).unwrap(), "rotta");
    assert!(e.invia(&"B".repeat(64), &[], Some("x".into())).is_err());
    e.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn snapshot_is_complete_for_the_ui_and_has_no_secrets() {
    let mut r = Rig::new();
    let a = r.engine("a").await;
    a.imposta_pin(Some("987654"));
    let s: Value = a.snapshot();
    for k in ["versione", "utente", "conf", "pin_impostato", "dispositivi", "ricezioni", "messaggi", "invii", "server", "identita", "rete",
              "recenti", "componenti", "avvisi", "migrazione", "adb", "schermo", "wizard"] {
        assert!(s.get(k).is_some(), "manca {k}");
    }
    let txt = s.to_string();
    assert!(!txt.contains("987654") && !txt.contains("pin_ricezione") && !txt.contains("PRIVATE KEY"));
    r.stop_all().await;
}
