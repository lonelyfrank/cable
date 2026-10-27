//! Motore LocalSend v2 di Cable. Nessuna dipendenza da GUI: la UI lo pilota e ascolta il bus.
pub mod android;
pub mod bus;
pub mod client;
pub mod desktop;
pub mod devices;
pub mod discovery;
pub mod engine;
pub mod identity;
pub mod paths;
pub mod migrate;
pub mod netwatch;
pub mod protocol;
pub mod receiver;
pub mod sender;
pub mod server;
pub mod settings;
pub mod shared;
pub mod util;

pub use engine::Engine;
pub mod ui_api;
