//! Coda delle richieste di apertura (riga di comando, file manager, seconda istanza).
//!
//! La coda vive nel backend ed è l'unica fonte di verità: l'evento `hub-apertura` è solo una sveglia
//! senza contenuto, quindi non conta se la pagina c'è già, non c'è ancora o viene ricreata. La pagina
//! ritira le richieste con `richieste_apertura` (che non le consuma) e le conferma per `id` con
//! `conferma_apertura`: una richiesta non confermata viene riproposta, una confermata mai più.
use percent_encoding::percent_decode_str;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Richiesta {
    pub id: u64,
    /// percorsi assoluti esistenti
    pub paths: Vec<String>,
    /// percorsi assoluti che non esistono (più): segnalati all'utente, mai inviati
    pub mancanti: Vec<String>,
}

#[derive(Default)]
pub struct Coda {
    inner: Mutex<Inner>,
}
#[derive(Default)]
struct Inner {
    prossimo: u64,
    attesa: Vec<Richiesta>,
}

/// Argomenti → percorsi assoluti, risolti rispetto a `cwd` (la directory del processo che ha chiamato,
/// non quella della prima istanza). Ignora le opzioni (`--x`); dopo `--` tutto è un file.
/// Accetta anche URI `file://`. Non richiede che il file esista.
pub fn risolvi(args: &[String], cwd: &Path) -> Vec<PathBuf> {
    let mut solo_file = false;
    let mut out = Vec::new();
    for a in args {
        if !solo_file && a == "--" { solo_file = true; continue; }
        if !solo_file && a.starts_with("--") { continue; }
        if a.is_empty() { continue; }
        let p = match a.strip_prefix("file://") {
            Some(rest) => {
                let rest = rest.strip_prefix("localhost").unwrap_or(rest);
                PathBuf::from(percent_decode_str(rest).decode_utf8_lossy().into_owned())
            }
            None => PathBuf::from(a),
        };
        let p = if p.is_absolute() { p } else { cwd.join(p) };
        out.push(p.components().collect());      // toglie i «.» senza risolvere «..» (che con i link simbolici cambierebbe file)
    }
    out
}

impl Coda {
    /// `args` esclude argv[0]. Restituisce l'id assegnato, o `None` se non c'era alcun file.
    pub fn accoda(&self, args: &[String], cwd: &Path) -> Option<u64> {
        let tutti = risolvi(args, cwd);
        if tutti.is_empty() { return None; }
        let (mut paths, mut mancanti) = (Vec::new(), Vec::new());
        for p in tutti {
            let s = p.to_string_lossy().into_owned();
            let target = if p.symlink_metadata().is_ok() { &mut paths } else { &mut mancanti };
            if !target.contains(&s) { target.push(s); }
        }
        let mut g = self.inner.lock().unwrap();
        g.prossimo += 1;
        let id = g.prossimo;
        g.attesa.push(Richiesta { id, paths, mancanti });
        Some(id)
    }

    /// Richieste non ancora confermate, in ordine di arrivo. Non le consuma.
    pub fn in_attesa(&self) -> Vec<Richiesta> { self.inner.lock().unwrap().attesa.clone() }

    /// Rimuove le richieste confermate; id sconosciuti o già confermati sono ignorati.
    pub fn conferma(&self, ids: &[u64]) { self.inner.lock().unwrap().attesa.retain(|r| !ids.contains(&r.id)); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }
    fn dir_con(nomi: &[&str]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for n in nomi { std::fs::write(d.path().join(n), b"x").unwrap(); }
        d
    }

    #[test]
    fn percorsi_relativi_sono_risolti_rispetto_al_chiamante_non_alla_prima_istanza() {
        let (a, b) = (dir_con(&["foto.jpg"]), dir_con(&["foto.jpg", "altro.txt"]));
        let c = Coda::default();
        c.accoda(&v(&["foto.jpg"]), a.path());
        c.accoda(&v(&["foto.jpg", "altro.txt"]), b.path());
        let r = c.in_attesa();
        assert_eq!(r[0].paths, vec![a.path().join("foto.jpg").to_string_lossy().into_owned()]);
        assert_eq!(r[1].paths, vec![b.path().join("foto.jpg").to_string_lossy().into_owned(), b.path().join("altro.txt").to_string_lossy().into_owned()]);
        assert_ne!(r[0].paths[0], r[1].paths[0]);
    }

    #[test]
    fn spazi_unicode_e_uri_file() {
        let d = dir_con(&["una foto è più bella.jpg", "日本語 ☃.txt"]);
        let c = Coda::default();
        let uri = format!("file://{}/una%20foto%20%C3%A8%20pi%C3%B9%20bella.jpg", d.path().display());
        c.accoda(&v(&[&uri, "日本語 ☃.txt"]), d.path());
        let r = &c.in_attesa()[0];
        assert_eq!(r.paths.len(), 2);
        assert!(r.paths[0].ends_with("una foto è più bella.jpg") && r.paths[1].ends_with("日本語 ☃.txt"));
        assert!(r.mancanti.is_empty());
    }

    #[test]
    fn file_inesistenti_sono_segnalati_e_non_inviati() {
        let d = dir_con(&["c'è.txt"]);
        let c = Coda::default();
        c.accoda(&v(&["c'è.txt", "sparito.txt"]), d.path());
        let r = &c.in_attesa()[0];
        assert_eq!(r.paths.len(), 1);
        assert_eq!(r.mancanti, vec![d.path().join("sparito.txt").to_string_lossy().into_owned()]);
    }

    #[test]
    fn opzioni_ignorate_e_doppio_trattino() {
        let d = dir_con(&["--strano.txt", "a.txt"]);
        let c = Coda::default();
        assert_eq!(c.accoda(&v(&["--background"]), d.path()), None);          // nessun file: nessuna richiesta
        assert_eq!(c.accoda(&[], d.path()), None);
        c.accoda(&v(&["--background", "a.txt", "--", "--strano.txt"]), d.path());
        assert_eq!(c.in_attesa()[0].paths.len(), 2);
    }

    #[test]
    fn richieste_ravvicinate_non_si_sovrascrivono_e_hanno_id_distinti() {
        let d = dir_con(&["1", "2", "3"]);
        let c = Coda::default();
        let ids: Vec<_> = ["1", "2", "3"].iter().map(|n| c.accoda(&v(&[n]), d.path()).unwrap()).collect();
        assert_eq!(ids, vec![1, 2, 3]);
        let r = c.in_attesa();
        assert_eq!(r.len(), 3);
        assert!(r.iter().zip(["1", "2", "3"]).all(|(x, n)| x.paths[0].ends_with(n)));
    }

    #[test]
    fn ritirare_non_consuma_confermare_si_e_mai_piu_riproposte() {
        let d = dir_con(&["1", "2"]);
        let c = Coda::default();
        c.accoda(&v(&["1"]), d.path());
        // finestra distrutta prima della conferma: la richiesta viene riproposta alla nuova pagina
        assert_eq!(c.in_attesa().len(), 1);
        assert_eq!(c.in_attesa().len(), 1);
        c.accoda(&v(&["2"]), d.path());
        c.conferma(&[1]);
        let r = c.in_attesa();
        assert_eq!(r.len(), 1);
        assert!(r[0].paths[0].ends_with("2"));
        c.conferma(&[1, 99]);                       // doppia conferma e id sconosciuto: nessun effetto
        assert_eq!(c.in_attesa().len(), 1);
        c.conferma(&[2]);
        assert!(c.in_attesa().is_empty());
        // gli id non vengono riusati dopo la conferma
        assert_eq!(c.accoda(&v(&["1"]), d.path()), Some(3));
    }

    #[test]
    fn duplicati_nella_stessa_richiesta_sono_unificati() {
        let d = dir_con(&["a"]);
        let c = Coda::default();
        c.accoda(&v(&["a", "./a", "a"]), d.path());
        assert_eq!(c.in_attesa()[0].paths, vec![d.path().join("a").to_string_lossy().into_owned()]);
    }
}
