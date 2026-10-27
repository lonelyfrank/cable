// Incorpora nel binario il commit da cui è compilato e la cartella dei sorgenti: servono al controllo degli aggiornamenti.
// Senza git (tarball, pacchetto RPM) il commit resta vuoto e il controllo si disattiva da solo.
use std::process::Command;

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&root).args(args).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let commit = git(&["rev-parse", "HEAD"]);
    let src = if commit.is_empty() { String::new() } else { root.canonicalize().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default() };
    println!("cargo:rustc-env=CABLE_COMMIT={commit}");
    println!("cargo:rustc-env=CABLE_SRC={src}");
    // il registro di HEAD cambia a ogni commit, checkout e pull: ricompila solo allora
    println!("cargo:rerun-if-changed=../../.git/logs/HEAD");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=build.rs");
}
