//! Prova a mano degli eventi D-Bus: stampa gli eventi di rete/sospensione per 15 secondi.
//! Uso: cargo run -p pixelhub-engine --example netwatch_probe   (poi, ad es., `nmcli radio wifi off && nmcli radio wifi on`)
use pixelhub_engine::netwatch;
use std::sync::{atomic::Ordering, Arc};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cancel = tokio_util::sync::CancellationToken::new();
    let status: netwatch::Status = Arc::new(Default::default());
    netwatch::spawn(tx, cancel.clone(), status.clone());
    let t0 = Instant::now();
    let mut n = 0;
    while t0.elapsed() < Duration::from_secs(15) {
        if let Ok(Some(ev)) = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            n += 1;
            if n <= 10 {
                println!("{:>5.1}s evento: {ev:?}", t0.elapsed().as_secs_f32());
            }
        }
    }
    println!("stato D-Bus: {} (1 = in ascolto, 2 = non disponibile); eventi ricevuti: {n}", status.load(Ordering::Relaxed));
    cancel.cancel();
}
