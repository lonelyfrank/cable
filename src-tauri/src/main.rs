// Cable: guscio Tauri attorno al motore Rust (finestra, notifiche, trascinamento, istanza unica).
use pixelhub_engine::engine::VERSION;
use pixelhub_engine::paths::Paths;
use pixelhub_engine::ui_api::{self, Host};
use pixelhub_engine::{desktop, Engine};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_dialog::DialogExt;

mod apertura;
#[cfg(feature = "ui-test")]
mod uitest;

struct AppState {
    engine: Arc<Engine>,
    host: Arc<TauriHost>,
    aperture: apertura::Coda,
}

struct TauriHost {
    app: AppHandle,
}

fn into_str(f: tauri_plugin_dialog::FilePath) -> Option<String> {
    f.into_path().ok().map(|p| p.to_string_lossy().into_owned())
}

impl Host for TauriHost {
    fn scegli_file(&self) -> Vec<String> {
        self.app
            .dialog()
            .file()
            .set_title("Scegli i file da inviare")
            .blocking_pick_files()
            .map(|v| v.into_iter().filter_map(into_str).collect())
            .unwrap_or_default()
    }
    fn scegli_cartella(&self, iniziale: &str) -> Option<String> {
        self.app.dialog().file().set_title("Scegli una cartella").set_directory(iniziale).blocking_pick_folder().and_then(into_str)
    }
    fn esci(&self) {
        self.app.exit(0);
    }
}

#[tauri::command]
async fn api(state: State<'_, AppState>, m: String, a: Option<Value>) -> Result<Value, String> {
    #[cfg(feature = "ui-test")]
    if m == "__test_result" {
        uitest::deliver(a.unwrap_or(Value::Null));
        return Ok(json!({"ok": true}));
    }
    Ok(ui_api::chiama(&state.engine, &*state.host, &m, a.unwrap_or(Value::Null)).await)
}

/// Richieste di apertura non ancora confermate dalla pagina (non le consuma: vedi `apertura`).
#[tauri::command]
fn richieste_apertura(state: State<'_, AppState>) -> Vec<apertura::Richiesta> {
    state.aperture.in_attesa()
}

#[tauri::command]
fn conferma_apertura(state: State<'_, AppState>, ids: Vec<u64>) {
    state.aperture.conferma(&ids);
}

/// Una richiesta di apertura (seconda istanza o file manager): accodata nel backend, poi la finestra
/// viene mostrata (o ricreata) e svegliata. Il contenuto non viaggia nell'evento.
fn apri_con_file(app: &AppHandle, args: &[String], cwd: &std::path::Path) {
    if let Some(st) = app.try_state::<AppState>() {
        st.aperture.accoda(args, cwd);
    }
    mostra(app);
    let _ = app.emit("hub-apertura", ());
}

/// La finestra (e quindi WebKit, il grosso della memoria) esiste solo quando serve:
/// in background resta in vita soltanto il motore.
fn mostra(app: &AppHandle) {
    let w = match app.get_webview_window("main") {
        Some(w) => w,
        None => match WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
            .title("Cable")
            .inner_size(1120.0, 780.0)
            .min_inner_size(400.0, 560.0)
            .build()
        {
            Ok(w) => w,
            Err(e) => {
                eprintln!("impossibile creare la finestra: {e}");
                return;
            }
        },
    };
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
    if let Some(st) = app.try_state::<AppState>() {
        st.engine.android.ui_attiva(true);
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        push(&app).await;
    });
}

fn visibile(app: &AppHandle) -> bool {
    app.get_webview_window("main").is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false))
}

async fn push(app: &AppHandle) {
    if !visibile(app) {
        return;
    }
    let Some(st) = app.try_state::<AppState>() else { return };
    let e = st.engine.clone();
    if let Ok(snap) = tauri::async_runtime::spawn_blocking(move || e.snapshot()).await {
        let _ = app.emit("hub-stato", snap);
    }
}

fn consenti_anteprime(app: &AppHandle, e: &Engine) {
    let scope = app.asset_protocol_scope();
    for k in ["cartella", "cartella_foto"] {
        let p = e.sh.settings.str(k);
        if !p.is_empty() {
            let _ = std::fs::create_dir_all(&p);
            let _ = scope.allow_directory(&p, true);
        }
    }
}

fn notifica(app: &AppHandle, engine: &Arc<Engine>, kind: &str, d: Value) {
    let attiva = app.get_webview_window("main").is_some_and(|w| w.is_focused().unwrap_or(false));
    if attiva && kind != "fine" {
        return;
    }
    use notify_rust::{Notification, Timeout, Urgency};
    let s = |k: &str| d[k].as_str().unwrap_or("").to_string();
    let mut n = Notification::new();
    n.appname("Cable").icon("it.frank.Cable").hint(notify_rust::Hint::DesktopEntry("it.frank.Cable".into()));
    match kind {
        "richiesta" => {
            let tot = d["size"].as_u64().unwrap_or(0);
            let nf = d["n"].as_u64().unwrap_or(1);
            n.summary(&s("alias"))
                .body(&if d["tipo"] == "testo" {
                    "vuole inviarti un messaggio".to_string()
                } else {
                    format!("vuole inviarti {} ({})", if nf == 1 { "un file".to_string() } else { format!("{nf} file") }, umano(tot))
                })
                .action("accetta", "Accetta")
                .action("rifiuta", "Rifiuta")
                .action("default", "Apri")
                .urgency(Urgency::Critical)
                .timeout(Timeout::Milliseconds(120_000));
        }
        "fine" => match d["stato"].as_str().unwrap_or("") {
            "completata" => {
                n.summary(&format!("Ricevuto da {}", s("alias"))).body(&format!("{} file", d["ricevuti"])).action("mostra", "Mostra nella cartella");
                // con un solo file «Apri» apre il file; altrimenti la cartella (vedi il gestore sotto)
                if d["ricevuti"].as_u64() == Some(1) && !s("percorso").is_empty() {
                    n.action("apri", "Apri");
                }
            }
            "parziale" | "errore" | "scaduta" => {
                n.summary(&format!("Ricezione da {} non completata", s("alias")))
                    .body(&s("errore"))
                    .action("cartella", "Apri cartella")
                    .action("default", "Apri Cable");
            }
            _ => return,
        },
        "messaggio" => {
            n.summary(&format!("Messaggio da {}", s("alias"))).body(&s("testo").chars().take(120).collect::<String>()).action("default", "Apri");
        }
        "invio" => {
            let titolo = match d["stato"].as_str().unwrap_or("") {
                "completato" => "Invio completato",
                "parziale" => "Invio parziale",
                "rifiutato" => "Invio rifiutato",
                "scaduto" => "Invio scaduto",
                "interrotto" => "Invio interrotto",
                "errore" => "Invio non riuscito",
                _ => return,
            };
            n.summary(titolo)
                .body(&format!("{}{}", s("alias"), if s("errore").is_empty() { String::new() } else { format!(": {}", s("errore")) }))
                .action("default", "Apri");
        }
        _ => return,
    }
    let (app, engine, id, percorso) = (app.clone(), engine.clone(), s("id"), s("percorso"));
    std::thread::spawn(move || {
        let Ok(h) = n.show() else { return };
        h.wait_for_action(|a| match a {
            "accetta" => {
                engine.decidi(&id, None);
            }
            "rifiuta" => {
                engine.decidi(&id, Some(vec![]));
            }
            "cartella" => {
                let _ = desktop::apri(std::path::Path::new(&engine.sh.settings.str("cartella")));
            }
            "apri" => {
                let _ = desktop::apri(std::path::Path::new(&percorso));
            }
            "mostra" => {
                let p = std::path::Path::new(&percorso);
                if percorso.is_empty() {
                    let _ = desktop::apri(std::path::Path::new(&engine.sh.settings.str("cartella")));
                } else {
                    let _ = desktop::mostra(p);
                }
            }
            "default" => mostra(&app),
            _ => {}
        });
    });
}

fn umano(n: u64) -> String {
    let mut x = n as f64;
    for u in ["B", "KB", "MB", "GB"] {
        if x < 1024.0 {
            return if u == "B" { format!("{x:.0} {u}") } else { format!("{x:.1} {u}") };
        }
        x /= 1024.0;
    }
    format!("{x:.1} TB")
}

fn cli() -> Option<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--versione" || a == "--version") {
        println!("{VERSION}");
        return Some(0);
    }
    if args.iter().any(|a| a == "--help" || a == "--aiuto") {
        println!("Cable {VERSION}\n\n  cable [FILE...]        apre Cable e fa scegliere il destinatario per i file indicati\n  cable --background     parte nascosta, pronta a ricevere\n  cable --migra          importa i dati di Pixel Hub (impostazioni, dispositivi, identità) e mostra l'esito\n  cable --diagnostica    stampa la diagnostica di rete, senza aprire la finestra\n  cable --versione");
        return Some(0);
    }
    if args.iter().any(|a| a == "--migra") {
        let paths = Paths::system();
        let rep = pixelhub_engine::migrate::run(&paths);
        let s = pixelhub_engine::settings::Settings::load(&paths);
        for a in &rep.avvisi {
            println!("{a}");
        }
        let av = s.avvisi.lock().unwrap().clone();
        if av.is_empty() && rep.avvisi.is_empty() {
            println!("Nessuna migrazione necessaria: i dati sono già nel formato di Cable.");
        } else {
            println!("{}", av.join("\n"));
        }
        println!("Migrazione registrata: {}", s.get("migrato_da"));
        return Some(0);
    }
    if args.iter().any(|a| a == "--diagnostica") {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _g = rt.enter();
        println!("{}", Engine::new(Paths::system(), 53317, false).diagnostica());
        return Some(0);
    }
    let pixel = if std::env::var_os("HUB_LS_PORT").is_some() { vec![] } else { desktop::processi_pixel_hub() }; // con una porta propria (prove, sandbox) non c'è conflitto
    if !pixel.is_empty() {
        eprintln!(
            "Pixel Hub è ancora in esecuzione (PID {}) e usa la stessa porta di Cable. Chiudilo (Impostazioni › Esci) e riprova.",
            pixel.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", ")
        );
        return Some(3);
    }
    let vecchi = desktop::processi_python();
    if !vecchi.is_empty() {
        eprintln!("Una versione Python di Pixel Hub è ancora in esecuzione (PID {}). Chiudila prima di avviare questa:\n  pkill -f '^python3 .*(pixel_hub.py|-m pixelhub)'",
                  vecchi.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(", "));
        return Some(3);
    }
    None
}

fn main() {
    if let Some(code) = cli() {
        std::process::exit(code);
    }
    let background = std::env::args().any(|a| a == "--background");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            // cwd è la directory del processo che ha chiamato, non quella di questa istanza
            apri_con_file(app, &argv[argv.len().min(1)..], std::path::Path::new(&cwd));
        }))
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![api, richieste_apertura, conferma_apertura])
        .on_window_event(|window, event| {
            let app = window.app_handle();
            match event {
                WindowEvent::CloseRequested { .. } => {
                    // la finestra viene distrutta; se la ricezione in background è attiva l'app non esce (vedi RunEvent::ExitRequested)
                    if let Some(st) = app.try_state::<AppState>() {
                        st.engine.android.ui_attiva(false);
                    }
                }
                WindowEvent::DragDrop(ev) => match ev {
                    DragDropEvent::Enter { .. } => {
                        let _ = app.emit("hub-drag", true);
                    }
                    DragDropEvent::Leave => {
                        let _ = app.emit("hub-drag", false);
                    }
                    DragDropEvent::Drop { paths, position } => {
                        let f = window.scale_factor().unwrap_or(1.0);
                        let _ = app.emit("hub-drag", false);
                        let p: Vec<String> = paths.iter().map(|p| p.to_string_lossy().into_owned()).collect();
                        let _ = app.emit("hub-drop", json!({"paths": p, "x": position.x / f, "y": position.y / f}));
                    }
                    _ => {}
                },
                _ => {}
            }
        })
        .setup(move |app| {
            let port = std::env::var("HUB_LS_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(53317);
            let engine = Engine::new(Paths::system(), port, std::env::var_os("HUB_NO_MULTICAST").is_none());
            let handle = app.handle().clone();
            let host = Arc::new(TauriHost { app: handle.clone() });
            let aperture = apertura::Coda::default();
            let args: Vec<String> = std::env::args().skip(1).collect();
            aperture.accoda(&args, &std::env::current_dir().unwrap_or_default());
            app.manage(AppState { engine: engine.clone(), host, aperture });
            {
                let (h, e) = (handle.clone(), engine.clone());
                engine.notifier().set(Arc::new(move |k, d| notifica(&h, &e, k, d)));
            }
            tauri::async_runtime::block_on(engine.start());
            consenti_anteprime(&handle, &engine);
            // il motore spinge lo stato alla pagina: eventi accorpati, al massimo ~10 aggiornamenti al secondo
            let (h, e) = (handle.clone(), engine.clone());
            tauri::async_runtime::spawn(async move {
                let mut rx = e.bus().subscribe();
                loop {
                    let topic = match rx.recv().await {
                        Ok(t) => t,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => "stato",
                        Err(_) => break,
                    };
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    let mut scope = topic == "impostazioni";
                    loop {
                        match rx.try_recv() {
                            Ok(t) => scope |= t == "impostazioni",
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                            Err(_) => break,
                        }
                    }
                    if scope {
                        consenti_anteprime(&h, &e);
                    }
                    push(&h).await;
                }
            });
            if !background {
                mostra(&handle);
            }
            #[cfg(feature = "ui-test")]
            uitest::spawn(handle.clone(), engine.clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("errore nell'avvio di Cable")
        .run(|app, ev| match ev {
            tauri::RunEvent::ExitRequested { api, code, .. } => {
                // code == None: è stata chiusa l'ultima finestra (non un'uscita esplicita)
                if code.is_none() {
                    let resta = app.try_state::<AppState>().is_some_and(|st| st.engine.sh.settings.bool("background"));
                    if resta {
                        api.prevent_exit();
                    }
                }
            }
            tauri::RunEvent::Exit => {
                if let Some(st) = app.try_state::<AppState>() {
                    let e = st.engine.clone();
                    tauri::async_runtime::block_on(async move { e.stop().await });
                }
            }
            _ => {}
        });
}
