//! Phase 0 embeddings (ADR-0019) — preuve sur le vrai cerveau : embed un
//! échantillon de documents et affiche leurs plus proches voisins sémantiques.
//! Nécessite le modèle BGE-M3 GGUF dans le dossier `models/` (le programme
//! affiche l'URL et l'emplacement s'il est absent).
//! Lancement : `cargo run --example embed`.

fn main() {
    match brainlink_lib::embed_demo() {
        Ok(report) => eprintln!("{report}"),
        Err(e) => eprintln!("Erreur : {e}"),
    }
}
