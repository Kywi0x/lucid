//! Démo end-to-end du pipeline IA : analyse quelques conversations réelles et
//! affiche le `brain.md` généré.
//! Lancement : `cargo run --example brain` (limite à 4 convs ; `LIMIT=0` pour tout).

fn main() {
    let limit: usize = std::env::var("LIMIT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    eprintln!("Analyse de {limit} conversation(s)…\n");
    match brainlink_lib::run_pipeline_demo(limit) {
        Ok(graph) => {
            let projects = graph.nodes.iter().filter(|n| n.kind == "project").count();
            let concepts = graph.nodes.iter().filter(|n| n.kind == "concept").count();
            eprintln!(
                "\nGraphe : {} nœuds ({projects} projets, {concepts} concepts), {} liens.",
                graph.nodes.len(),
                graph.edges.len()
            );
            println!("\n========== brain.md ==========\n");
            println!("{}", graph.markdown);
        }
        Err(e) => eprintln!("Erreur : {e}"),
    }
}
