//! Test de RÉCUPÉRATION seule : pour les 16 questions de `docs/test-pertinence-mcp.md`,
//! la page qui contient la réponse arrive-t-elle dans le contexte injecté au modèle ?
//! Aucun modèle n'est appelé — `ask_context` est déterministe, donc un échec ici est
//! un échec de la recherche, pas du modèle.
//!
//! Lancement : `cargo run --example retrieval` (arrêter `tauri dev` d'abord : lock sur target/).
//! N'affiche que des TITRES de pages, jamais de contenu.

/// (question, titres attendus en minuscules ; vide = contrôle négatif, on n'attend rien)
const CASES: &[(&str, &[&str])] = &[
    ("Combien de factures Xano ai-je, et sur quelle période ?", &["xano"]),
    ("Liste tous mes fournisseurs de services web / hébergement et ce que je paie à chacun.", &["xano", "parabola", "ovh"]),
    ("Que dit le PDF du bot sniper BSC à l'étape 1 ?", &["sniper"]),
    ("Quel est le montant et l'échéance de ma dernière facture d'assurance auto ?", &["facture agence", "maaf"]),
    ("Quel ADR traite du rangement par embeddings, et quel est son statut ?", &["adr-0019"]),
    ("Qu'a-t-on décidé sur la recherche sémantique, et qu'est-ce qui ferait rouvrir la décision ?", &["adr-0022", "recherche du cerveau"]),
    ("Quelle est la répartition des parts entre Lucas et moi ?", &["gouvernance"]),
    ("Où est rangé le tuto de la chorale, et combien de documents contient-il ?", &["chorale"]),
    ("Quel est le montant de mes frais de dentiste en 2025 ?", &[]),
    ("À combien s'élève mon hypothèque sur l'appartement de Lyon ?", &[]),
    ("Ai-je de quoi organiser ma succession ?", &[]),
    ("Ai-je un document qui mentionne une vaccination ?", &["medecine_maladies"]),
    ("De quoi ai-je besoin pour constituer un dossier de location, côté locataire ?", &["identit", "avenant", "rib"]),
    ("Où en est le développement de Lucid : qu'est-ce qui est livré, qu'est-ce qui reste ?", &["roadmap", "tâches & sprints"]),
    ("Quels sont mes projets clients actifs, et pour chacun le dernier document en date ?", &["ideeri", "oxygen", "village immobilier", "pharmadvance"]),
    ("Ai-je des doublons évidents dans mes documents ?", &[]),
];

/// Titres des pages injectées, dans l'ordre. Une ligne de contexte vaut
/// `- <titre> (dans <parent>) : <résumé>`.
/// ponytail: un résumé qui contient une puce markdown produira un faux titre —
/// sans effet sur le score (on ne cherche que des titres attendus).
fn injected_titles(ctx: &str) -> Vec<String> {
    ctx.lines()
        .filter_map(|l| l.strip_prefix("- "))
        .map(|l| {
            let t = l.split(" (dans ").next().unwrap_or(l);
            t.split(" : ").next().unwrap_or(t).trim().to_lowercase()
        })
        .collect()
}

/// Rang (1-based) du premier titre attendu, `None` si absent.
fn rank_of(titles: &[String], expected: &[&str]) -> Option<usize> {
    titles.iter().position(|t| expected.iter().any(|e| t.contains(e))).map(|r| r + 1)
}

/// Comparaison mots-clés vs vecteurs sur les mêmes questions et le même corpus.
/// `SEMANTIC=1 cargo run --example retrieval`
fn semantic_run() {
    const TOP: usize = 40; // même fenêtre que les 40 pages injectées côté mots-clés
    let (mut hits, mut total, mut no_vector) = (0usize, 0usize, 0usize);
    for (i, (question, expected)) in CASES.iter().enumerate() {
        let ranked = match brainlink_lib::semantic_candidates(question, 5000) {
            Ok(r) => r,
            Err(e) => { eprintln!("Q{} — erreur : {e}", i + 1); continue; }
        };
        let labels: Vec<String> = ranked.iter().map(|(_, _, l)| l.to_lowercase()).collect();
        if expected.is_empty() {
            let top3: Vec<String> = ranked.iter().take(3)
                .map(|(s, _, l)| format!("{l} ({s:.2})")).collect();
            println!("Q{:<2} —      contrôle négatif · similarité du 1ᵉʳ : {:.3} · top3 : {}",
                i + 1, ranked.first().map(|(s, _, _)| *s).unwrap_or(0.0), top3.join(" | "));
            continue;
        }
        total += 1;
        match rank_of(&labels, expected) {
            Some(r) if r <= TOP => {
                hits += 1;
                // Similarité de la bonne page ET du 1ᵉʳ : c'est ce couple qui dit si
                // un seuil peut séparer « trouvé » de « rien à trouver ».
                println!("Q{:<2} — ✅ rang {r} / {TOP} · similarité de la bonne page : {:.3} (1ᵉʳ : {:.3})",
                    i + 1, ranked[r - 1].0, ranked[0].0);
            }
            Some(r) => println!("Q{:<2} — ❌ rang {r} (hors des {TOP} premiers)", i + 1),
            None => { no_vector += 1; println!("Q{:<2} — ❌ la page attendue n'a AUCUN vecteur en cache", i + 1); }
        }
    }
    println!("\nSémantique : {hits}/{total} (dont {no_vector} échec(s) faute de vecteur, pas faute de méthode).");
    println!("Vecteurs disponibles : {} pages.", brainlink_lib::semantic_candidates("x", 5000).map(|v| v.len()).unwrap_or(0));
}

fn main() {
    if std::env::var("SEMANTIC").is_ok() { return semantic_run(); }
    // HYBRID=1 : mots-clés + candidats vectoriels (ce que fait l'app).
    let hybrid = std::env::var("HYBRID").is_ok();
    println!("Mode : {}\n", if hybrid { "HYBRIDE (mots-clés + vecteurs)" } else { "mots-clés seuls" });
    let mut hits = 0usize;
    let mut expected_total = 0usize;

    for (i, (question, expected)) in CASES.iter().enumerate() {
        let ctx = match brainlink_lib::ask_context_for(question, hybrid) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Q{} — erreur : {e}", i + 1);
                continue;
            }
        };
        let titles = injected_titles(&ctx);
        let short: String = question.chars().take(58).collect();

        if expected.is_empty() {
            // Contrôle négatif : rien n'est attendu, on montre ce qui a été injecté
            // quand même — c'est ce bruit que le modèle doit savoir écarter.
            println!(
                "Q{:<2} —      contrôle négatif · {:>3} pages injectées, {} car. · top3 : {}\n        « {short}… »",
                i + 1,
                titles.len(),
                ctx.len(),
                titles.iter().take(3).cloned().collect::<Vec<_>>().join(" | ")
            );
            continue;
        }

        expected_total += 1;
        let rank = titles.iter().position(|t| expected.iter().any(|e| t.contains(e)));
        match rank {
            Some(r) => {
                hits += 1;
                println!(
                    "Q{:<2} — ✅ rang {:<3} sur {:>3} pages injectées, {} car.\n        « {short}… »",
                    i + 1,
                    r + 1,
                    titles.len(),
                    ctx.len()
                );
            }
            None => println!(
                "Q{:<2} — ❌ ABSENTE du contexte ({} pages injectées, {} car.) · attendu : {}\n        « {short}… »",
                i + 1,
                titles.len(),
                ctx.len(),
                expected.join(" / ")
            ),
        }
    }

    println!("\nRécupération : {hits}/{expected_total} questions dont la page attendue est injectée.");
    println!("Un ❌ = la recherche n'a pas trouvé ; le modèle ne pouvait pas répondre juste sans relancer une recherche.");
}
