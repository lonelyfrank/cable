//! Eventi di rete e di sospensione da D-Bus (NetworkManager e logind), solo come **accelerazione**.
//!
//! Se il bus di sistema o questi servizi non ci sono (altre distribuzioni, container, sessioni minimali) il
//! task termina in silenzio: la scoperta continua a funzionare con il controllo periodico delle interfacce e
//! con il rilevamento del salto d'orologio dopo la sospensione (vedi `discovery`).
use crate::discovery::NetEvent;
use futures_util::StreamExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;
use zbus::message::Type;
use zbus::{Connection, MatchRule, MessageStream};

async fn watch(tx: UnboundedSender<NetEvent>, cancel: CancellationToken) -> zbus::Result<()> {
    let conn = Connection::system().await?;
    crate::dlog!("eventi D-Bus: in ascolto su NetworkManager e logind");
    // logind: PrepareForSleep(false) = ripresa dalla sospensione
    let sleep_rule = MatchRule::builder().msg_type(Type::Signal)
        .interface("org.freedesktop.login1.Manager")?.member("PrepareForSleep")?.build();
    // NetworkManager: qualunque segnale sotto il suo albero di oggetti (stato, connessioni attive, configurazioni IP).
    // Sono tanti e a raffica: chi li riceve li accorpa e confronta gli indirizzi prima di agire.
    let nm_rule = MatchRule::builder().msg_type(Type::Signal)
        .path_namespace("/org/freedesktop/NetworkManager")?.build();
    let mut sleep = MessageStream::for_match_rule(sleep_rule, &conn, Some(8)).await?;
    let mut nm = MessageStream::for_match_rule(nm_rule, &conn, Some(64)).await?;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            m = sleep.next() => {
                let Some(Ok(m)) = m else { return Ok(()) };
                if let Ok(going_to_sleep) = m.body().deserialize::<bool>() {
                    if !going_to_sleep && tx.send(NetEvent::Resume).is_err() { return Ok(()); }
                }
            }
            m = nm.next() => {
                let Some(Ok(_)) = m else { return Ok(()) };
                if tx.send(NetEvent::Network).is_err() { return Ok(()); }
            }
        }
    }
}

/// Stato per la diagnostica: 0 = non ancora noto, 1 = in ascolto, 2 = non disponibile
pub type Status = std::sync::Arc<std::sync::atomic::AtomicU8>;

pub fn spawn(tx: UnboundedSender<NetEvent>, cancel: CancellationToken, status: Status) {
    use std::sync::atomic::Ordering::Relaxed;
    tokio::spawn(async move {
        status.store(1, Relaxed);
        if let Err(e) = watch(tx, cancel).await {
            status.store(2, Relaxed);
            crate::dlog!("eventi D-Bus non disponibili: {e}");
        }
    });
}
