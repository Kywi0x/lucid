//! Pipeline d'analyse : conversations → extraction IA → arbre de nœuds → `brain.md`.
//!
//! Modèle de données : arbre récursif agnostique de la source.
//! Feuille  = une source (conv Claude Code, page Notion, fichier Drive).
//! Conteneur = espace / dossier, profondeur illimitée (container_path).
//! Agrégation déterministe en Rust, aucun appel API après sync.

use crate::ai::LlamaEngine;
use crate::models::{BrainEdge, BrainGraph, BrainNode, Conversation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Budget de caractères du texte condensé envoyé au modèle par conversation.
const CONDENSE_CHARS: usize = 6000;
const MAX_OUTPUT_TOKENS: u32 = 256;

/// Résultat structuré attendu du modèle pour une conversation.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Extraction {
    #[serde(default)]
    #[allow(dead_code)]
    pub summary: String,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub concepts: Vec<String>,
}

const SYSTEM_PROMPT: &str = "Tu es un assistant qui structure des connaissances techniques. \
Tu réponds STRICTEMENT en JSON valide, sans aucun texte ni balise autour du JSON.";

/// Condense une conversation : prompts utilisateur + réponses assistant, tronqué.
fn condense(conv: &Conversation) -> String {
    let mut out = String::new();
    for m in &conv.messages {
        let who = if m.role == "user" { "DEV" } else { "IA" };
        out.push_str(who);
        out.push_str(": ");
        out.push_str(m.text.trim());
        out.push('\n');
        if out.len() >= CONDENSE_CHARS {
            let mut end = CONDENSE_CHARS;
            while !out.is_char_boundary(end) { end -= 1; }
            out.truncate(end);
            out.push_str("\n[…tronqué]");
            break;
        }
    }
    out
}

/// Construit le prompt d'extraction pour une conversation.
fn extraction_prompt(conv: &Conversation) -> String {
    let intro = match conv.summary.source.as_str() {
        "google-drive" => {
            "Analyse ce document ou fichier Google Drive.\n\
Si le contenu est uniquement un nom de fichier, infère les concepts, \
l'entité (client, fournisseur, entreprise) et le thème depuis ce nom."
        }
        "notion" => {
            "Analyse cette page ou base de données Notion.\n\
Extrais les concepts, entités, thèmes et décisions clés présents dans le contenu."
        }
        "obsidian" => {
            "Analyse cette note Obsidian (markdown).\n\
Extrais les concepts, thèmes, décisions et entités clés présents dans le contenu."
        }
        _ => "Analyse cette conversation entre un développeur et une IA.",
    };
    format!(
        "{intro}\n\
Projet : {project}\n\n\
Renvoie UNIQUEMENT un objet JSON avec ce schéma exact :\n\
{{\n\
  \"summary\": \"résumé ou description en une phrase\",\n\
  \"decisions\": [\"décision ou action clé\"],\n\
  \"patterns\": [\"pratique, thème ou type de document récurrent\"],\n\
  \"concepts\": [\"concept, technologie, entité (client/entreprise/lieu) ou thématique clé\"]\n\
}}\n\
Règles : 0 à 5 éléments par liste, en français, concis. Listes vides si rien de pertinent.\n\n\
CONTENU :\n{body}",
        project = crate::ai::pipeline::short_project(&conv.summary.project),
        body = condense(conv),
    )
}

/// Nom court de projet (dernier segment du chemin).
pub fn short_project(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Extrait le premier objet JSON équilibré d'une chaîne (sortie modèle parfois bruitée).
pub fn extract_json(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_extraction(raw: &str) -> Extraction {
    extract_json(raw)
        .and_then(|j| serde_json::from_str::<Extraction>(j).ok())
        .unwrap_or_default()
}

/// Normalise un concept pour le regroupement (minuscules, espaces compactés).
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Clé de groupe : chemin normalisé complet, profondeur illimitée.
fn group_key(path: &[String]) -> String {
    if path.is_empty() {
        return "non-classé".to_string();
    }
    path.iter().map(|s| norm(s)).collect::<Vec<_>>().join("/")
}

/// Libellé d'affichage : dernier segment du chemin.
fn group_display(path: &[String]) -> String {
    if path.is_empty() {
        return "Non classé".to_string();
    }
    path.last().unwrap().clone()
}

// ─── Agrégat conteneur ────────────────────────────────────────────────────────

/// Remplace ProjectAgg + ConceptAgg. Un par nœud conteneur (espace ou sous-dossier).
struct ContainerAgg {
    display: String,
    /// "root" ou "p:{parent_path_normalisé}"
    parent_id: String,
    summaries: Vec<String>,
    decisions: Vec<(String, String)>,
    patterns: Vec<String>,
    keyword_counts: BTreeMap<String, (String, usize)>,
    /// Nombre de feuilles dans le sous-arbre (calculé pendant la boucle principale).
    leaf_count: usize,
    /// Objectif synthétisé par Gemma (rempli en passe synthèse).
    objective: String,
}

// ─── Cache incrémental ────────────────────────────────────────────────────────

fn cache_key(conv: &Conversation) -> String {
    format!("{}:{}:{}", conv.summary.source, conv.summary.id, conv.summary.last_timestamp.as_deref().unwrap_or(""))
}

fn load_cache(path: &Path) -> HashMap<String, Extraction> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_cache(path: &Path, cache: &HashMap<String, Extraction>) {
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(path, json);
    }
}

fn synth_cache_key(c_key: &str, summaries: &[String], decisions: &[(String, String)]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    c_key.hash(&mut h);
    summaries.hash(&mut h);
    decisions.iter().for_each(|(d, _)| d.hash(&mut h));
    format!("{:x}", h.finish())
}

/// Événement de progression remonté au frontend.
pub struct Progress {
    pub current: usize,
    pub total: usize,
    pub label: String,
}

/// Génère le graphe à partir des conversations (arbre récursif agnostique de la source).
pub fn generate_brain(
    engine: &LlamaEngine,
    conversations: &[Conversation],
    cache_path: Option<&Path>,
    mut progress: impl FnMut(Progress),
    mut on_node: impl FnMut(&str, usize, usize),
) -> Result<BrainGraph, String> {
    let total = conversations.len();
    let mut containers: BTreeMap<String, ContainerAgg> = BTreeMap::new();
    let mut leaves: Vec<BrainNode> = Vec::new();
    let mut failures = 0usize;

    let mut cache: HashMap<String, Extraction> =
        cache_path.map(load_cache).unwrap_or_default();
    let mut cache_hits = 0usize;

    for (i, conv) in conversations.iter().enumerate() {
        progress(Progress {
            current: i + 1,
            total,
            label: conv.summary.title.clone(),
        });

        let key = cache_key(conv);
        let ex = if let Some(cached) = cache.get(&key) {
            cache_hits += 1;
            cached.clone()
        } else {
            let total_text: usize = conv.messages.iter().map(|m| m.text.len()).sum();
            let ex = if total_text < 150 {
                Extraction::default()
            } else {
                let raw = match engine.complete(
                    Some(SYSTEM_PROMPT),
                    &extraction_prompt(conv),
                    MAX_OUTPUT_TOKENS,
                ) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("⚠️ Conversation ignorée ({}) : {e}", conv.summary.id);
                        failures += 1;
                        continue;
                    }
                };
                parse_extraction(&raw)
            };
            cache.insert(key, ex.clone());
            ex
        };

        // Chemin effectif (fallback sur short_project pour Claude Code sans container_path).
        let effective_path: Vec<String> = if conv.summary.container_path.is_empty() {
            vec![short_project(&conv.summary.project)]
        } else {
            conv.summary.container_path.clone()
        };

        // Créer les conteneurs ancêtres si inexistants (racine → feuille).
        for depth in 1..=effective_path.len() {
            let c_key = group_key(&effective_path[..depth]);
            if !containers.contains_key(&c_key) {
                let parent_id = if depth == 1 {
                    "root".to_string()
                } else {
                    format!("p:{}", group_key(&effective_path[..depth - 1]))
                };
                let label = effective_path[depth - 1].clone();
                // Notifie l'UI uniquement pour les conteneurs de premier niveau.
                if depth == 1 {
                    on_node(&label, i + 1, total);
                }
                containers.insert(c_key, ContainerAgg {
                    display: label,
                    parent_id,
                    summaries: Vec::new(),
                    decisions: Vec::new(),
                    patterns: Vec::new(),
                    keyword_counts: BTreeMap::new(),
                    leaf_count: 0,
                    objective: String::new(),
                });
            }
        }

        // Incrémenter leaf_count sur tous les ancêtres.
        for depth in 1..=effective_path.len() {
            let c_key = group_key(&effective_path[..depth]);
            if let Some(c) = containers.get_mut(&c_key) {
                c.leaf_count += 1;
            }
        }

        // Alimenter le conteneur direct avec l'extraction de cette conv.
        let parent_key = group_key(&effective_path);
        let date = conv.summary.last_timestamp.as_deref()
            .map(|t| t.chars().take(10).collect::<String>())
            .unwrap_or_default();
        if let Some(c) = containers.get_mut(&parent_key) {
            if !ex.summary.trim().is_empty() {
                c.summaries.push(ex.summary.clone());
            }
            for d in &ex.decisions {
                if !d.trim().is_empty() {
                    c.decisions.push((d.clone(), date.clone()));
                }
            }
            for p in &ex.patterns {
                if !p.trim().is_empty() {
                    c.patterns.push(p.clone());
                }
            }
            for concept in &ex.concepts {
                let cn = norm(concept);
                if cn.is_empty() { continue; }
                let e = c.keyword_counts.entry(cn).or_insert_with(|| (concept.clone(), 0));
                e.1 += 1;
            }
        }

        // Nœud feuille.
        leaves.push(BrainNode {
            id: format!("leaf:{}", conv.summary.id),
            label: conv.summary.title.clone(),
            kind: "leaf".into(),
            weight: 1,
            summary: ex.summary.clone(),
            keywords: unique_concepts(&ex.concepts, 5),
            decisions: ex.decisions.iter().filter(|d| !d.trim().is_empty()).cloned().collect(),
            patterns: ex.patterns.iter().filter(|p| !p.trim().is_empty()).cloned().collect(),
            community: 0,
            parent_id: Some(format!("p:{parent_key}")),
            synthesized_at: None,
            content: String::new(),
            connector: Some(conv.summary.source.clone()),
            source_id: Some(conv.summary.id.clone()),
            source_project: Some(conv.summary.project_slug.clone()),
        });
    }

    if containers.is_empty() && leaves.is_empty() {
        return Err(format!(
            "Aucune conversation n'a pu être analysée ({failures} échec(s))."
        ));
    }

    eprintln!("📦 Cache : {cache_hits}/{total} conversations réutilisées.");
    if let Some(path) = cache_path {
        save_cache(path, &cache);
    }

    // Passe de synthèse : objectif par conteneur (mis en cache).
    let synth_cache_path: Option<std::path::PathBuf> =
        cache_path.map(|p| p.with_file_name("brain_synth_cache.json"));
    let mut synth_cache: HashMap<String, String> = synth_cache_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut synth_hits = 0usize;

    for (c_key, c) in containers.iter_mut() {
        let skey = synth_cache_key(c_key, &c.summaries, &c.decisions);
        if let Some(cached_obj) = synth_cache.get(&skey) {
            c.objective = cached_obj.clone();
            synth_hits += 1;
            continue;
        }
        progress(Progress {
            current: total,
            total,
            label: format!("Synthèse : {}", c.display),
        });
        c.objective = synthesize_container(engine, &c.display, &c.summaries, &c.decisions);
        synth_cache.insert(skey, c.objective.clone());
    }

    if let Some(path) = synth_cache_path.as_deref() {
        if let Ok(json) = serde_json::to_string(&synth_cache) {
            let _ = std::fs::write(path, json);
        }
    }
    eprintln!("🔄 Synthèse : {synth_hits}/{} depuis cache.", containers.len());

    let (nodes, edges) = build_graph(&containers, &leaves);
    let markdown = assemble_markdown(&nodes);
    let report = generate_report(&containers, &leaves);

    Ok(BrainGraph {
        nodes,
        edges,
        markdown,
        report,
        generated_at: chrono::Local::now().to_rfc3339(),
    })
}

#[derive(Deserialize, Default)]
struct ContainerSynth {
    #[serde(default)]
    objective: String,
    #[serde(default)]
    themes: Vec<String>,
}

fn synthesize_container(
    engine: &LlamaEngine,
    display: &str,
    summaries: &[String],
    decisions: &[(String, String)],
) -> String {
    let summaries: Vec<String> = summaries.iter().take(10).cloned().collect();
    let decisions: Vec<String> =
        dedup_pairs(decisions).into_iter().take(8).map(|(d, _)| d).collect();
    if summaries.is_empty() && decisions.is_empty() {
        return String::new();
    }
    let user = format!(
        "Projet : {display}\n\nRésumés :\n{s}\n\nDécisions clés :\n{d}\n\n\
Produis UNIQUEMENT un JSON {{\"objective\": \"objectif global en 1 à 2 phrases\", \
\"themes\": [\"thème\"]}}. En français, concis.",
        s = summaries.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"),
        d = decisions.iter().map(|s| format!("- {s}")).collect::<Vec<_>>().join("\n"),
    );
    let raw = match engine.complete(Some(SYSTEM_PROMPT), &user, 300) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let synth: ContainerSynth = extract_json(&raw)
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    let mut out = synth.objective.trim().to_string();
    if !synth.themes.is_empty() {
        let themes = synth.themes.join(", ");
        if out.is_empty() {
            out = format!("Thématiques : {themes}");
        } else {
            out.push_str(&format!(" · Thématiques : {themes}"));
        }
    }
    out
}

/// Mots-clés d'un conteneur : concepts les plus fréquents.
fn top_keywords(counts: &BTreeMap<String, (String, usize)>, n: usize) -> Vec<String> {
    let mut v: Vec<&(String, usize)> = counts.values().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.into_iter().take(n).map(|(d, _)| d.clone()).collect()
}

/// Déduplique les concepts d'une feuille pour ses mots-clés.
fn unique_concepts(concepts: &[String], n: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    concepts.iter()
        .filter(|c| seen.insert(norm(c)))
        .take(n)
        .cloned()
        .collect()
}

/// Construit les vecteurs nodes + edges depuis les conteneurs et les feuilles.
fn build_graph(
    containers: &BTreeMap<String, ContainerAgg>,
    leaves: &[BrainNode],
) -> (Vec<BrainNode>, Vec<BrainEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    let total_leaves = leaves.len();
    let root_containers = containers.values().filter(|c| c.parent_id == "root").count();

    nodes.push(BrainNode {
        id: "root".into(),
        label: "Lucid".into(),
        kind: "root".into(),
        weight: total_leaves.max(1),
        summary: format!("{root_containers} espaces · {total_leaves} sources analysées."),
        keywords: Vec::new(),
        decisions: Vec::new(),
        patterns: Vec::new(),
        community: 0,
        parent_id: None,
        synthesized_at: None,
        content: String::new(),
        connector: None,
        source_id: None,
        source_project: None,
    });

    for (c_key, c) in containers {
        let cid = format!("p:{c_key}");
        nodes.push(BrainNode {
            id: cid.clone(),
            label: c.display.clone(),
            kind: "container".into(),
            weight: c.leaf_count,
            summary: c.objective.clone(),
            keywords: top_keywords(&c.keyword_counts, 8),
            decisions: dedup_pairs(&c.decisions).into_iter().map(|(d, _)| d).collect(),
            patterns: dedup(&c.patterns),
            community: 0,
            parent_id: Some(c.parent_id.clone()),
            synthesized_at: None,
            content: String::new(),
            connector: None,
            source_id: None,
            source_project: None,
        });
        edges.push(BrainEdge {
            source: c.parent_id.clone(),
            target: cid,
            kind: "contains".into(),
            relation: "contains".into(),
        });
    }

    for leaf in leaves {
        let parent = leaf.parent_id.clone().unwrap_or_else(|| "root".to_string());
        edges.push(BrainEdge {
            source: parent,
            target: leaf.id.clone(),
            kind: "contains".into(),
            relation: "contains".into(),
        });
        nodes.push(leaf.clone());
    }

    (nodes, edges)
}

/// Rapport compact optimisé pour l'injection LLM.
fn generate_report(containers: &BTreeMap<String, ContainerAgg>, leaves: &[BrainNode]) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let top_level = containers.values().filter(|c| c.parent_id == "root").count();

    let mut r = format!(
        "# Brain Report — {date}\n> {} sources · {} espaces\n\n",
        leaves.len(),
        top_level,
    );

    r.push_str("## Espaces\n| Espace | Sources | Concepts clés |\n|---|---|---|\n");
    let mut cs: Vec<(&String, &ContainerAgg)> = containers.iter()
        .filter(|(_, c)| c.parent_id == "root")
        .collect();
    cs.sort_by(|a, b| b.1.leaf_count.cmp(&a.1.leaf_count));
    for (_, c) in &cs {
        let kw = top_keywords(&c.keyword_counts, 4).join(", ");
        r.push_str(&format!("| {} | {} | {} |\n", c.display, c.leaf_count, kw));
    }
    r.push('\n');

    r.push_str("## Décisions récentes\n");
    let mut all_decisions: Vec<(String, String, String)> = containers.iter()
        .flat_map(|(_, c)| {
            dedup_pairs(&c.decisions).into_iter()
                .map(|(d, date)| (c.display.clone(), d, date))
                .collect::<Vec<_>>()
        })
        .collect();
    all_decisions.sort_by(|a, b| b.2.cmp(&a.2));
    for (proj, decision, date) in all_decisions.iter().take(10) {
        let d = if date.is_empty() { String::new() } else { format!(" ({date})") };
        r.push_str(&format!("- **{proj}**{d} : {decision}\n"));
    }

    r
}

fn dedup(items: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    items.iter().filter(|s| seen.insert(norm(s))).cloned().collect()
}

/// Émet un nœud comme section markdown parseable.
pub fn emit_section(buf: &mut String, n: &BrainNode, level: usize) {
    let hashes = "#".repeat(level);
    let kind_label = match n.kind.as_str() {
        "container" | "group" | "espace" => "Espace",
        "leaf" | "page" => "Page",
        _ => return,
    };
    buf.push_str(&format!("{hashes} {kind_label}: {}\n", n.label));

    // Métadonnées machine-readable (lues par brain_md.rs pour reconstruire l'arbre).
    let parent_part = n.parent_id.as_deref()
        .filter(|p| !p.is_empty())
        .map(|p| format!(" | parent: {p}"))
        .unwrap_or_default();
    let mut connector_part = String::new();
    if let Some(c) = &n.connector {
        connector_part.push_str(&format!(" | connector: {c}"));
    }
    if let Some(sid) = &n.source_id {
        connector_part.push_str(&format!(" | source_id: {sid}"));
    }
    if let Some(sp) = &n.source_project {
        connector_part.push_str(&format!(" | source_project: {sp}"));
    }
    buf.push_str(&format!(
        "<!-- id: {} | weight: {}{parent_part} | community: {}{connector_part} -->\n\n",
        n.id, n.weight, n.community,
    ));

    if !n.summary.is_empty() {
        buf.push_str(&format!("{}\n\n", n.summary));
    }
    if !n.keywords.is_empty() {
        buf.push_str(&format!("**Mots-clés** : {}\n\n", n.keywords.join(", ")));
    }
    if !n.decisions.is_empty() {
        buf.push_str("**Décisions** :\n");
        for d in &n.decisions { buf.push_str(&format!("- {d}\n")); }
        buf.push('\n');
    }
    if !n.patterns.is_empty() {
        buf.push_str("**Patterns** :\n");
        for p in &n.patterns { buf.push_str(&format!("- {p}\n")); }
        buf.push('\n');
    }
    if !n.content.is_empty() {
        buf.push_str(&n.content);
        buf.push_str("\n\n");
    }
}

/// Sérialise le graphe en brain.md structuré — arbre récursif, profondeur illimitée.
fn assemble_markdown(nodes: &[BrainNode]) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let node_count = nodes.iter().filter(|n| n.kind != "root").count();
    let mut md = format!("# Lucid\n<!-- generated: {date} | nodes: {node_count} | version: 3 -->\n\n");

    // Index parent_id → enfants, triés par poids décroissant.
    let mut children_of: HashMap<String, Vec<&BrainNode>> = HashMap::new();
    for n in nodes {
        if let Some(pid) = &n.parent_id {
            children_of.entry(pid.clone()).or_default().push(n);
        }
    }
    for v in children_of.values_mut() {
        v.sort_by(|a, b| b.weight.cmp(&a.weight));
    }

    emit_recursive(&mut md, "root", &children_of, 2);
    md
}

fn emit_recursive(
    buf: &mut String,
    parent_id: &str,
    index: &HashMap<String, Vec<&BrainNode>>,
    level: usize,
) {
    let Some(children) = index.get(parent_id) else { return };
    for child in children {
        emit_section(buf, child, level);
        emit_recursive(buf, &child.id, index, level + 1);
    }
}

// ── Création de structure par l'assistant (arborescence de pages) ────────────

/// Arbre de pages proposé par le modèle pour `create_structure`.
#[derive(serde::Deserialize, Debug)]
pub struct StructureSpec {
    pub label: String,
    #[serde(default)]
    pub summary: String,
    /// Corps markdown template de la page (optionnel).
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub children: Vec<StructureSpec>,
}

/// Parse la sortie (bruitée) du modèle en arbre de pages. Borne la taille :
/// profondeur ≤ 3, ≤ 30 nœuds au total (garde-fou contre un modèle qui divague).
pub fn parse_structure(raw: &str) -> Option<StructureSpec> {
    let mut spec: StructureSpec = serde_json::from_str(extract_json(raw)?).ok()?;
    fn clamp(node: &mut StructureSpec, depth: usize, budget: &mut usize) {
        node.label = node.label.trim().to_string();
        if depth >= 3 { node.children.clear(); }
        node.children.retain(|c| !c.label.trim().is_empty());
        node.children.truncate(*budget);
        *budget = budget.saturating_sub(node.children.len());
        for c in &mut node.children { clamp(c, depth + 1, budget); }
    }
    let mut budget = 29usize; // 30 nœuds max, racine comprise
    clamp(&mut spec, 1, &mut budget);
    if spec.label.is_empty() { return None; }
    Some(spec)
}

fn dedup_pairs(items: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    items.iter()
        .filter(|(s, _)| seen.insert(norm(s)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_balanced_json() {
        let s = "blabla {\"summary\": \"x\", \"concepts\": [\"a\"]} trailing";
        let j = extract_json(s).unwrap();
        let ex: Extraction = serde_json::from_str(j).unwrap();
        assert_eq!(ex.summary, "x");
        assert_eq!(ex.concepts, vec!["a"]);
    }

    #[test]
    fn handles_nested_and_strings_with_braces() {
        let s = "{\"summary\": \"a } b\", \"decisions\": [\"{x}\"]}";
        let j = extract_json(s).unwrap();
        assert!(serde_json::from_str::<Extraction>(j).is_ok());
    }

    #[test]
    fn bad_output_yields_default() {
        let ex = parse_extraction("désolé, pas de json ici");
        assert!(ex.summary.is_empty() && ex.concepts.is_empty());
    }

    #[test]
    fn parses_structure_tree_from_noisy_output() {
        let s = "voilà : {\"label\": \"Projet web\", \"children\": [\
{\"label\": \"Page client\", \"summary\": \"Espace client\"}, \
{\"label\": \"Mentions légales\", \"children\": [{\"label\": \"RGPD\"}]}]} fin";
        let spec = parse_structure(s).unwrap();
        assert_eq!(spec.label, "Projet web");
        assert_eq!(spec.children.len(), 2);
        assert_eq!(spec.children[1].children[0].label, "RGPD");
    }

    #[test]
    fn structure_restarted_json_fails_prefixed_but_parses_raw() {
        // Gemma régénère parfois le JSON complet au lieu de continuer le préfixe.
        let completion = "{\n  \"label\": \"Projet Web\",\n  \"children\": [{\"label\": \"Design\"}]\n}";
        // Re-préfixé → JSON cassé → doit échouer (pas de faux positif)
        assert!(parse_structure(&format!("{{\"label\": \"{completion}")).is_none());
        // Brut → doit parser
        let spec = parse_structure(completion).unwrap();
        assert_eq!(spec.label, "Projet Web");
    }

    #[test]
    fn structure_rejects_garbage_and_clamps_depth() {
        assert!(parse_structure("pas de json").is_none());
        // profondeur 4 → coupée à 3
        let s = "{\"label\":\"a\",\"children\":[{\"label\":\"b\",\"children\":[{\"label\":\"c\",\"children\":[{\"label\":\"d\"}]}]}]}";
        let spec = parse_structure(s).unwrap();
        assert!(spec.children[0].children[0].children.is_empty());
    }

    #[test]
    fn short_project_takes_last_segment() {
        assert_eq!(short_project("/Users/x/GitHub/brainlink"), "brainlink");
    }

    #[test]
    fn group_key_empty_gives_non_classe() {
        assert_eq!(group_key(&[]), "non-classé");
    }

    #[test]
    fn group_key_uses_full_path() {
        let path = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(group_key(&path), "a/b/c");
    }

    #[test]
    fn group_key_single_element() {
        let path = vec!["brainlink".into()];
        assert_eq!(group_key(&path), "brainlink");
    }

    #[test]
    fn group_display_takes_last_segment() {
        let path = vec!["Clients".into(), "Béaux Electricité".into()];
        assert_eq!(group_display(&path), "Béaux Electricité");
    }
}
