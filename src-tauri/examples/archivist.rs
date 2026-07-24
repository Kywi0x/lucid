//! Test de l'Archiviste (prototype) sur le vrai cerveau de l'user — une
//! passe unique, écrit des propositions dans `mcp_pending/` (visibles en
//! bulles fantômes dans l'app si elle est ouverte). N'applique jamais rien
//! directement : chaque proposition attend une acceptation.
//! Lancement : `cargo run --example archivist`.

fn main() {
    match brainlink_lib::run_archivist_scan_once() {
        Ok(report) => eprintln!("{report}"),
        Err(e) => eprintln!("Erreur : {e}"),
    }
}
