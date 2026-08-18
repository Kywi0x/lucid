//! Rejoue exactement le chemin de l'assistant de l'app (`ask_brain`) hors de
//! l'app, avec le temps de chaque étape. Sert à diagnostiquer un « réflexion »
//! qui ne rend jamais la main : on voit laquelle des étapes ne finit pas.
//!
//!   cargo run --example ask -- "Que faut-il installer à l'étape 1 du bot sniper ?"
//!
//! La réponse est écrite dans un fichier plutôt qu'affichée : elle peut contenir
//! des données personnelles (factures, contrats) qui n'ont rien à faire dans un
//! terminal partagé ou un log de session.

fn main() {
    let question: String = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if question.is_empty() {
        eprintln!("Usage : cargo run --example ask -- \"<question>\"");
        std::process::exit(1);
    }
    // DUMP=1 : écrit aussi le contexte injecté, pour vérifier ce que le modèle a
    // réellement reçu quand sa réponse surprend.
    if std::env::var("DUMP").is_ok() {
        match brainlink_lib::ask_context_for(&question, true) {
            Ok(ctx) => {
                let out = std::env::temp_dir().join("lucid-context.txt");
                let _ = std::fs::write(&out, &ctx);
                eprintln!("contexte injecté ({} car.) → {}", ctx.len(), out.display());
            }
            Err(e) => eprintln!("contexte indisponible : {e}"),
        }
    }
    let started = std::time::Instant::now();
    match brainlink_lib::answer_question(&question) {
        Ok(answer) => {
            let out = std::env::temp_dir().join("lucid-ask.txt");
            let _ = std::fs::write(&out, &answer);
            eprintln!(
                "\n✅ {} caractères en {:.1}s → {}",
                answer.chars().count(),
                started.elapsed().as_secs_f32(),
                out.display()
            );
        }
        Err(e) => eprintln!("\n❌ après {:.1}s : {e}", started.elapsed().as_secs_f32()),
    }
}
