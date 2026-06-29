//! Exemple de validation : scanne les conversations Claude Code réelles et affiche
//! des statistiques agrégées (jamais le contenu) pour vérifier le parsing.
//! Lancement : `cargo run --example scan`

fn main() {
    let convs = brainlink_lib::list_conversations_pub();
    println!("Conversations détectées : {}", convs.len());
    let total_msgs: usize = convs.iter().map(|c| c.message_count).sum();
    println!("Messages exploitables au total : {total_msgs}");

    use std::collections::BTreeSet;
    let projects: BTreeSet<_> = convs.iter().map(|c| c.project.clone()).collect();
    println!("Projets distincts : {}", projects.len());

    println!("\n5 conversations les plus récentes (titre tronqué) :");
    for c in convs.iter().take(5) {
        let title: String = c.title.chars().take(60).collect();
        println!(
            "  [{:>3} msgs] {}  —  {}",
            c.message_count,
            title,
            c.last_timestamp.as_deref().unwrap_or("?")
        );
    }
}
