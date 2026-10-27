//! Banco di prova della finestra reale. Compilato solo con `--features ui-test`.
//! Esegue lo scenario del wizard e dell'uso normale con un secondo motore in-process e salva screenshot.
use pixelhub_engine::paths::Paths;
use pixelhub_engine::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tokio::sync::oneshot;

static PENDING: OnceLock<Mutex<HashMap<u64, oneshot::Sender<String>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

pub fn deliver(v: Value) {
    let id = v["id"].as_u64().unwrap_or(0);
    if let Some(tx) = PENDING.get_or_init(Default::default).lock().unwrap().remove(&id) { let _ = tx.send(v["v"].as_str().unwrap_or("").to_string()); }
}

/// Esegue uno script (corpo di funzione async con `return`) nella pagina e ne restituisce il risultato.
async fn js(app: &AppHandle, script: &str) -> String {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    PENDING.get_or_init(Default::default).lock().unwrap().insert(id, tx);
    let wrapped = format!("(async()=>{{let __v;try{{__v=await (async()=>{{{script}}})();}}catch(e){{__v='ERR:'+e}} window.__TAURI__.core.invoke('api',{{m:'__test_result',a:{{id:{id},v:String(__v)}}}})}})()");
    app.get_webview_window("main").unwrap().eval(&wrapped).unwrap();
    tokio::time::timeout(Duration::from_secs(15), rx).await.expect("timeout script").unwrap()
}

async fn until(app: &AppHandle, cond: &str, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if js(app, &format!("return String(!!({cond}))")).await == "true" { return true; }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn shot(app: &AppHandle, dir: &PathBuf, name: &str) {
    tokio::time::sleep(Duration::from_millis(700)).await;      // lascia finire le animazioni
    let (tx, rx) = oneshot::channel::<()>();
    let path = dir.join(format!("{name}.png"));
    let txm = Arc::new(Mutex::new(Some(tx)));
    app.get_webview_window("main").unwrap().with_webview(move |wv| {
        use webkit2gtk::WebViewExt;
        let txm = txm.clone();
        wv.inner().snapshot(webkit2gtk::SnapshotRegion::Visible, webkit2gtk::SnapshotOptions::NONE, None::<&gio::Cancellable>, move |res| {
            if let Ok(surf) = res {
                if let Ok(img) = cairo::ImageSurface::try_from(surf) {
                    if let Ok(mut f) = std::fs::File::create(&path) { let _ = img.write_to_png(&mut f); }
                }
            }
            if let Some(t) = txm.lock().unwrap().take() { let _ = t.send(()); }
        });
    }).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(10), rx).await;
}

struct Checks(Vec<(String, bool)>);
impl Checks {
    fn ok(&mut self, name: &str, cond: bool, detail: &str) {
        println!("{} {}{}", if cond { "OK  " } else { "FAIL" }, name, if cond || detail.is_empty() { String::new() } else { format!(" — {detail}") });
        self.0.push((name.into(), cond));
    }
}

pub fn spawn(app: AppHandle, engine: Arc<Engine>) {
    tauri::async_runtime::spawn(async move {
        let shots = PathBuf::from(std::env::var("HUB_UI_SHOTS").unwrap_or_else(|_| "/tmp/pixelhub-shots".into()));
        std::fs::create_dir_all(&shots).unwrap();
        let mut c = Checks(vec![]);
        scenario(&app, &engine, &shots, &mut c).await;
        let bad = c.0.iter().filter(|x| !x.1).count();
        println!("\n{} controlli, {} falliti. Screenshot in {}", c.0.len(), bad, shots.display());
        app.exit(if bad == 0 { 0 } else { 1 });
    });
}

async fn scenario(app: &AppHandle, a: &Arc<Engine>, shots: &PathBuf, c: &mut Checks) {
    let tmp = tempfile::tempdir().unwrap();
    let sock = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let pport = sock.local_addr().unwrap().port();
    drop(sock);
    c.ok("pagina caricata e stato ricevuto dal motore", until(app, "typeof S !== 'undefined' && S", 30).await, "");
    c.ok("primo avvio: il wizard si apre da solo", until(app, "document.getElementById('wizard').classList.contains('on')", 5).await, "");
    shot(app, shots, "01-wizard-prepara").await;
    c.ok("passo 1: controlli (rete, ricezione, identità) presenti", js(app, "return document.querySelectorAll('#wizChecks > div').length").await == "3", "");

    let peer = Engine::new(Paths::under(tmp.path()), pport, false);
    peer.sh.settings.aggiorna(json!({"nome_pc": "Pixel di prova"}).as_object().unwrap());
    peer.start().await;
    c.ok("passo 2: ricerca per IP trova il telefono", a.cerca(Some("127.0.0.1"), Some(pport)).await == 1, "");
    js(app, "AZ.wizAvanti(); return 1").await;
    c.ok("passo 2: il telefono compare nell'elenco", until(app, "document.querySelectorAll('#wizDevs [data-azione=wizScegli]').length === 1", 10).await, "");
    c.ok("passo 2: «Avanti» è disabilitato finché non scegli tu", js(app, "return document.getElementById('wizNext').disabled").await == "true", "");
    shot(app, shots, "02-wizard-trova").await;
    js(app, "document.querySelector('#wizDevs [data-azione=wizScegli]').click(); return 1").await;
    c.ok("passo 2: scelto esplicitamente, «Avanti» si abilita", until(app, "document.getElementById('wizNext').disabled === false", 5).await, "");
    js(app, "AZ.wizAvanti(); return 1").await;

    // ---- passo 3: codice
    c.ok("passo 3: senza codice inviato non si può verificare", js(app, "return document.getElementById('wizVerifica').disabled").await == "true", "");
    js(app, "document.getElementById('wizInvia').click(); return 1").await;
    let rid = loop {
        if let Some(r) = peer.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa") { break r["id"].as_str().unwrap().to_string(); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    peer.decidi(&rid, None);                                  // l'utente tocca «Accetta» sul telefono
    let code = loop {
        if let Some(m) = peer.receiver.snapshot().1.as_array().unwrap().first() { break m["testo"].as_str().unwrap().split("codice di verifica: ").nth(1).unwrap()[..6].to_string(); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    c.ok("passo 3: l'interfaccia mostra il conto alla rovescia e abilita il campo", until(app, "!document.getElementById('wizCode').disabled && /Scade/.test(document.getElementById('wizCount').textContent)", 10).await, "");
    shot(app, shots, "03-wizard-verifica").await;
    let wrong = if code == "000000" { "111111" } else { "000000" };
    js(app, &format!("document.getElementById('wizCode').value='{wrong}'; document.getElementById('wizVerifica').click(); return 1")).await;
    c.ok("passo 3: un codice sbagliato è rifiutato con i tentativi rimasti", until(app, "/Tentativi rimasti: 4/.test(document.getElementById('wizMsg').textContent)", 5).await, "");
    c.ok("passo 3: nulla salvato dopo un codice sbagliato", !a.sh.settings.preferiti().contains_key(&peer.sh.fingerprint()), "");
    js(app, &format!("document.getElementById('wizCode').value='{code}'; document.getElementById('wizVerifica').click(); return 1")).await;
    c.ok("passo 3: codice giusto → telefono verificato e «Avanti» abilitato", until(app, "document.getElementById('wizNext').disabled === false && /Verificato/.test(document.getElementById('wizMsg').textContent)", 10).await, "");
    c.ok("passo 3: salvato come preferito confermato", a.sh.settings.preferiti().get(&peer.sh.fingerprint()).map_or(false, |f| f["confermato"] == json!(true)), "");
    js(app, "AZ.wizAvanti(); return 1").await;

    // ---- passo 4: prova inversa
    c.ok("passo 4: in attesa di qualcosa dal telefono", until(app, "document.getElementById('wizProva').textContent.includes('In attesa')", 10).await, "");
    shot(app, shots, "04-wizard-prova").await;
    let jid = peer.invia(&a.sh.fingerprint(), &[], Some("prova dal telefono".into())).unwrap();
    c.ok("passo 4: la richiesta compare sopra il wizard", until(app, "document.getElementById('velo').classList.contains('on')", 10).await, "");
    shot(app, shots, "05-wizard-richiesta-sopra").await;
    js(app, "AZ.accetta(); return 1").await;
    c.ok("passo 4: ricezione confermata nell'interfaccia", until(app, "document.getElementById('wizProva').textContent.includes('Ricezione funzionante')", 10).await, "");
    let _ = jid;
    js(app, "AZ.wizAvanti(); return 1").await;

    // ---- passo 5: fine
    shot(app, shots, "06-wizard-fine").await;
    js(app, "document.getElementById('wizNomePc').value='Fedora di prova'; document.getElementById('wizNomePc').dispatchEvent(new Event('change')); return 1").await;
    js(app, "document.getElementById('wizPin').value='2468'; AZ.wizAvanti(); return 1").await;
    c.ok("fine: il wizard si chiude e non ricompare", until(app, "!document.getElementById('wizard').classList.contains('on') && S.conf.wizard_fatto === true", 10).await, "");
    c.ok("fine: nome e PIN applicati (PIN solo come hash)", a.sh.settings.str("nome_pc") == "Fedora di prova" && !a.sh.settings.get("pin_ricezione").is_null()
         && !std::fs::read_to_string(a.sh.paths.settings_file()).unwrap().contains("2468"), "");
    c.ok("fine: il telefono verificato è il destinatario", js(app, "return destinazione() ? destinazione().alias : ''").await == "Pixel di prova", "");
    a.imposta_pin(None);

    // ---- uso normale
    let src = tmp.path().join("dati");
    std::fs::create_dir_all(&src).unwrap();
    for n in ["foto1.jpg", "relazione.pdf", "note.txt"] { std::fs::write(src.join(n), vec![7u8; 200_000]).unwrap(); }
    let paths: Vec<String> = ["foto1.jpg", "relazione.pdf", "note.txt"].iter().map(|n| src.join(n).to_string_lossy().into_owned()).collect();
    let jid = peer.invia(&a.sh.fingerprint(), &paths, None).unwrap();
    c.ok("ricezione: sheet con 3 file selezionabili", until(app, "document.getElementById('velo').classList.contains('on') && document.querySelectorAll('#sheet input[data-fid]').length === 3", 10).await, "");
    shot(app, shots, "07-richiesta-in-arrivo").await;
    js(app, "document.querySelector('#sheet input[data-fid]:last-of-type').checked = false; AZ.accetta(); return 1").await;
    let j = loop {
        let j = peer.sender.snapshot().as_array().unwrap().iter().find(|x| x["id"] == jid.as_str()).cloned().unwrap();
        if ["completato", "parziale", "errore"].contains(&j["stato"].as_str().unwrap()) { break j; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    c.ok("ricezione: accettazione parziale → invio parziale", j["stato"] == "parziale", &j.to_string());
    c.ok("ricezione: arrivati solo i 2 file scelti", std::fs::read_dir(a.sh.settings.str("cartella")).unwrap().count() == 2, "");
    shot(app, shots, "08-dopo-ricezione").await;

    peer.imposta_pin(Some("2468"));
    let f = tmp.path().join("uscita.bin");
    std::fs::write(&f, vec![9u8; 300_000]).unwrap();
    js(app, &format!("inviaA(destinazione(), [{}]); return 1", serde_json::to_string(&f.to_string_lossy()).unwrap())).await;
    c.ok("invio: l'interfaccia chiede il PIN del destinatario", until(app, "document.getElementById('veloPin').classList.contains('on')", 10).await, "");
    shot(app, shots, "09-richiesta-pin").await;
    js(app, "document.getElementById('pinCampo').value='2468'; document.querySelector('[data-azione=pinSi]').click(); return 1").await;
    let r = loop {
        if let Some(r) = peer.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa") { break r["id"].as_str().unwrap().to_string(); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    peer.decidi(&r, None);
    let dest = PathBuf::from(peer.sh.settings.str("cartella")).join("uscita.bin");
    c.ok("invio con PIN dall'interfaccia completato", { let end = Instant::now() + Duration::from_secs(10); loop { if dest.exists() && std::fs::metadata(&dest).unwrap().len() == 300_000 { break true; } if Instant::now() > end { break false; } tokio::time::sleep(Duration::from_millis(50)).await; } }, "");
    peer.imposta_pin(None);

    // ---- trascinamento: evento nativo di Tauri → pagina → invio del percorso reale
    let g = tmp.path().join("trascinato.bin");
    std::fs::write(&g, vec![5u8; 100_000]).unwrap();
    js(app, "vai('condividi'); return 1").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    js(app, "window.__r = document.querySelector('.disp[data-fp]').getBoundingClientRect(); return 1").await;
    js(app, &format!("window.hubDrop([{}], __r.x + __r.width/2, __r.y + __r.height/2); return 1", serde_json::to_string(&g.to_string_lossy()).unwrap())).await;
    let r = loop {
        if let Some(r) = peer.receiver.snapshot().0.as_array().unwrap().iter().find(|i| i["stato"] == "attesa") { break r["id"].as_str().unwrap().to_string(); }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    peer.decidi(&r, None);
    let dest = PathBuf::from(peer.sh.settings.str("cartella")).join("trascinato.bin");
    c.ok("rilascio sul dispositivo invia il file reale", { let end = Instant::now() + Duration::from_secs(10); loop { if dest.exists() { break true; } if Instant::now() > end { break false; } tokio::time::sleep(Duration::from_millis(50)).await; } }, "");
    c.ok("nessuna cache di uscita", !a.sh.paths.cache.join("uscita").exists(), "");
    shot(app, shots, "10-condividi").await;

    // ---- altre pagine
    for p in ["home", "schermo", "pixel", "impostazioni"] { js(app, &format!("vai('{p}'); return 1")).await; shot(app, shots, &format!("11-{p}")).await; }
    let d = js(app, "const r = await api('diagnostica'); return r.testo.split('\\n')[0]").await;
    c.ok("diagnostica raggiungibile dalla pagina", d.contains("Pixel Hub"), &d);
    js(app, "AZ.wizApri(); return 1").await;
    c.ok("il wizard si riapre da Impostazioni", until(app, "document.getElementById('wizard').classList.contains('on')", 5).await, "");
    js(app, "AZ.wizSalta(); return 1").await;
    c.ok("«Salta per ora» chiude il wizard", until(app, "!document.getElementById('wizard').classList.contains('on')", 5).await, "");
    c.ok("nessun errore JavaScript", js(app, "return window.__errs").await == "0", "");
    peer.stop().await;
}
