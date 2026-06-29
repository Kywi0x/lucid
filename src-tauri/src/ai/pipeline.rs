//! Pipeline d'analyse : conversations → extraction IA → agrégation → `brain.md`.
//!
//! Stratégie hybride (de-risque la qualité du clustering, risque #1 du projet) :
//!   - le modèle fait l'extraction *par conversation* (résumé, décisions, patterns, concepts) ;
//!   - l'agrégation, le clustering par projet et les connexions sont assemblés
//!     *déterministiquement* en Rust → résultat reproductible et robuste.

use crate::ai::LlamaEngine;
use crate::models::{BrainEdge, BrainGraph, BrainNode, Confidence, Conversation, SourceRef};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Nombre maximum de bulles "concept" dans le graphe (lisibilité).
const MAX_CONCEPT_NODES: usize = 60;

/// Budget de caractères du texte condensé envoyé au modèle par conversation
/// (~8k tokens de contexte ; on garde de la marge pour le prompt et la sortie).
const CONDENSE_CHARS: usize = 6000;
const MAX_OUTPUT_TOKENS: u32 = 512;

/// Résultat structuré attendu du modèle pour une conversation.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Extraction {
    /// Résumé d'une phrase. Demandé au modèle pour l'amener à « réfléchir » avant
    /// de lister décisions/concepts ; non rendu directement dans le brain.md.
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
            out.truncate(CONDENSE_CHARS);
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
fn extract_json(s: &str) -> Option<&str> {
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

/// Clé de groupe depuis container_path : chemin normalisé, profondeur ≤ 2.
/// Vide → "non-classé" (bac par défaut).
fn group_key(path: &[String]) -> String {
    if path.is_empty() {
        return "non-classé".to_string();
    }
    let depth = path.len().min(2);
    path[..depth].iter().map(|s| norm(s)).collect::<Vec<_>>().join("/")
}

/// Libellé d'affichage d'un groupe (dernier segment significatif).
fn group_display(path: &[String]) -> String {
    if path.is_empty() {
        return "Non classé".to_string();
    }
    path[path.len().min(2) - 1].clone()
}

/// Clé du groupe parent (remonte d'un niveau, ou "non-classé").
fn parent_group_key(key: &str) -> String {
    if key == "non-classé" {
        return "non-classé".to_string();
    }
    match key.rfind('/') {
        Some(i) => key[..i].to_string(),
        None => "non-classé".to_string(),
    }
}

/// Libellé depuis une clé normée (dernier segment). Utilisé pour les groupes parents fantômes.
fn display_from_key(key: &str) -> String {
    if key == "non-classé" {
        return "Non classé".to_string();
    }
    // ponytail: casse perdue ici — seuls les parents créés ex-nihilo par la passe de fusion passent par là
    key.rsplit('/').next().unwrap_or(key).to_string()
}

// ─── Agrégats ──────────────────────────────────────────────────────────────

struct ProjectAgg {
    display: String,
    conv_count: usize,
    decisions: Vec<(String, String)>, // (texte, date courte)
    patterns: Vec<String>,
    summaries: Vec<String>,                       // résumés par conversation
    concept_counts: BTreeMap<String, (String, usize)>, // norm -> (display, count)
    sources: Vec<SourceRef>,
    link: Option<String>, // chemin/cwd du projet
    objective: String,    // rempli par la passe de synthèse
}

struct ConceptAgg {
    display: String,
    occurrences: usize,
    projects: std::collections::BTreeSet<String>,
    sources: Vec<SourceRef>,
}

// ─── Cache incrémental ─────────────────────────────────────────────────────

/// Clé de cache : id + last_timestamp. Si la conv n'a pas changé, on réutilise.
fn cache_key(conv: &Conversation) -> String {
    format!("{}:{}", conv.summary.id, conv.summary.last_timestamp.as_deref().unwrap_or(""))
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

/// Événement de progression remonté au frontend.
pub struct Progress {
    pub current: usize,
    pub total: usize,
    pub label: String,
}

/// Génère le graphe à partir des conversations.
/// `cache_path` : chemin du fichier de cache incrémental (optionnel).
/// `progress` est appelé avant chaque conversation analysée.
pub fn generate_brain(
    engine: &LlamaEngine,
    conversations: &[Conversation],
    cache_path: Option<&Path>,
    mut progress: impl FnMut(Progress),
) -> Result<BrainGraph, String> {
    let total = conversations.len();
    let mut projects: BTreeMap<String, ProjectAgg> = BTreeMap::new();
    let mut concepts: BTreeMap<String, ConceptAgg> = BTreeMap::new();
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
            // Résilience : une conversation qui échoue est ignorée, pas fatale.
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
            let ex = parse_extraction(&raw);
            cache.insert(key, ex.clone());
            ex
        };

        // Provisoire chantier 2 : si container_path vide, utilise short_project
        // pour que Claude Code reste identique avant que le connecteur le remplisse (chantier 4).
        let effective_path: Vec<String> = if conv.summary.container_path.is_empty() {
            vec![short_project(&conv.summary.project)]
        } else {
            conv.summary.container_path.clone()
        };
        let proj_key = group_key(&effective_path);
        let date = conv
            .summary
            .last_timestamp
            .as_deref()
            .map(|t| t.chars().take(10).collect::<String>())
            .unwrap_or_default();

        // Référence de provenance pour cette conversation (multi-connecteurs).
        let src = SourceRef {
            connector: conv.summary.source.clone(),
            title: conv.summary.title.clone(),
            id: conv.summary.id.clone(),
            project_slug: conv.summary.project_slug.clone(),
            link: Some(conv.summary.project.clone()),
            timestamp: conv.summary.last_timestamp.clone(),
        };

        let entry = projects
            .entry(proj_key.clone())
            .or_insert_with(|| ProjectAgg {
                display: group_display(&effective_path),
                conv_count: 0,
                decisions: Vec::new(),
                patterns: Vec::new(),
                summaries: Vec::new(),
                concept_counts: BTreeMap::new(),
                sources: Vec::new(),
                link: None,
                objective: String::new(),
            });
        entry.conv_count += 1;
        entry.sources.push(src.clone());
        entry.link.get_or_insert_with(|| conv.summary.project.clone());
        if !ex.summary.trim().is_empty() {
            entry.summaries.push(ex.summary.clone());
        }
        for d in ex.decisions {
            if !d.trim().is_empty() {
                entry.decisions.push((d, date.clone()));
            }
        }
        for p in ex.patterns {
            if !p.trim().is_empty() {
                entry.patterns.push(p);
            }
        }

        for c in ex.concepts {
            let c = c.trim().to_string();
            if c.is_empty() {
                continue;
            }
            let ckey = norm(&c);
            // Compte par projet (pour les mots-clés du projet).
            let pc = entry
                .concept_counts
                .entry(ckey.clone())
                .or_insert_with(|| (c.clone(), 0));
            pc.1 += 1;

            let ce = concepts.entry(ckey).or_insert_with(|| ConceptAgg {
                display: c.clone(),
                occurrences: 0,
                projects: Default::default(),
                sources: Vec::new(),
            });
            ce.occurrences += 1;
            ce.projects.insert(proj_key.clone());
            ce.sources.push(src.clone());
        }
    }

    if projects.is_empty() {
        return Err(format!(
            "Aucune conversation n'a pu être analysée ({failures} échec(s))."
        ));
    }

    eprintln!("📦 Cache : {cache_hits}/{total} conversations réutilisées.");
    if let Some(path) = cache_path {
        save_cache(path, &cache);
    }

    // Garde-fou 2 : groupes à 1 item → fusionnés dans le parent.
    // Profondeur max = 2, donc une seule passe suffit si on traite les feuilles en premier.
    // ponytail: single pass avec re-check — pas de cascade au-delà de depth 2
    {
        let mut to_merge: Vec<(String, String)> = projects
            .iter()
            .filter(|(key, p)| p.conv_count == 1 && key.as_str() != "non-classé")
            .map(|(key, _)| (key.clone(), parent_group_key(key)))
            .collect();
        // Feuilles en premier (plus de '/' = plus profond) pour ne pas remonter un parent
        // encore vide avant qu'il ait reçu ses enfants.
        to_merge.sort_unstable_by(|(a, _), (b, _)| {
            b.chars().filter(|&c| c == '/').count()
                .cmp(&a.chars().filter(|&c| c == '/').count())
        });
        for (child_key, parent_key) in &to_merge {
            if child_key == parent_key { continue; }
            // Re-check : un parent peut avoir reçu des merges et ne plus être singleton.
            if projects.get(child_key.as_str()).map(|p| p.conv_count).unwrap_or(0) != 1 {
                continue;
            }
            let child = projects.remove(child_key.as_str()).unwrap();
            let parent = projects.entry(parent_key.clone()).or_insert_with(|| ProjectAgg {
                display: display_from_key(parent_key),
                conv_count: 0,
                decisions: Vec::new(),
                patterns: Vec::new(),
                summaries: Vec::new(),
                concept_counts: BTreeMap::new(),
                sources: Vec::new(),
                link: None,
                objective: String::new(),
            });
            parent.conv_count += child.conv_count;
            parent.decisions.extend(child.decisions);
            parent.patterns.extend(child.patterns);
            parent.summaries.extend(child.summaries);
            parent.sources.extend(child.sources);
            if parent.link.is_none() {
                parent.link = child.link;
            }
            for (ck, cv) in child.concept_counts {
                let e = parent.concept_counts.entry(ck).or_insert_with(|| (cv.0.clone(), 0));
                e.1 += cv.1;
            }
            // Repointe les concepts vers la clé parent.
            for ca in concepts.values_mut() {
                if ca.projects.remove(child_key.as_str()) {
                    ca.projects.insert(parent_key.clone());
                }
            }
        }
    }

    // Passe de synthèse : objectif + thématiques par projet.
    for p in projects.values_mut() {
        progress(Progress {
            current: total,
            total,
            label: format!("Synthèse : {}", p.display),
        });
        p.objective = synthesize_project(engine, p);
    }

    let (nodes, edges) = build_graph(&projects, &concepts);
    let markdown = assemble_markdown(&projects, &concepts);
    let report = generate_report(&projects, &concepts, &nodes);

    Ok(BrainGraph {
        nodes,
        edges,
        markdown,
        report,
        generated_at: chrono::Local::now().to_rfc3339(),
    })
}

#[derive(Deserialize, Default)]
struct ProjSynth {
    #[serde(default)]
    objective: String,
    #[serde(default)]
    themes: Vec<String>,
}

/// Synthèse projet : à partir des résumés de conversations + décisions, demande
/// au modèle un objectif global et les thématiques. Échec → chaîne vide (non bloquant).
fn synthesize_project(engine: &LlamaEngine, p: &ProjectAgg) -> String {
    let summaries: Vec<String> = p.summaries.iter().take(10).cloned().collect();
    let decisions: Vec<String> =
        dedup_pairs(&p.decisions).into_iter().take(8).map(|(d, _)| d).collect();
    if summaries.is_empty() && decisions.is_empty() {
        return String::new();
    }
    let user = format!(
        "Projet : {name}\n\nRésumés de conversations :\n{summaries}\n\nDécisions clés :\n{decisions}\n\n\
Produis UNIQUEMENT un JSON {{\"objective\": \"objectif global du projet en 1 à 2 phrases\", \
\"themes\": [\"thématique principale\"]}}. En français, concis.",
        name = p.display,
        summaries = summaries
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n"),
        decisions = decisions
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let raw = match engine.complete(Some(SYSTEM_PROMPT), &user, 300) {
        Ok(r) => r,
        Err(_) => return String::new(),
    };
    let synth: ProjSynth = extract_json(&raw)
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

/// Mots-clés d'un projet : ses concepts les plus fréquents.
fn top_keywords(counts: &BTreeMap<String, (String, usize)>, n: usize) -> Vec<String> {
    let mut v: Vec<&(String, usize)> = counts.values().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.into_iter().take(n).map(|(d, _)| d.clone()).collect()
}

/// Déduplique des sources par (connecteur, id).
fn dedup_sources(sources: &[SourceRef]) -> Vec<SourceRef> {
    let mut seen = std::collections::HashSet::new();
    sources
        .iter()
        .filter(|s| seen.insert((s.connector.clone(), s.id.clone())))
        .cloned()
        .collect()
}

/// Construit le graphe de bulles : cerveau central → projets → concepts.
/// Un concept partagé par plusieurs projets est une seule bulle reliée à
/// chacun → c'est ce qui tisse les connexions inter-projets (façon Obsidian).
fn build_graph(
    projects: &BTreeMap<String, ProjectAgg>,
    concepts: &BTreeMap<String, ConceptAgg>,
) -> (Vec<BrainNode>, Vec<BrainEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Communauté par projet : 0 = global, 1..n = projet (BTreeMap → ordre stable).
    let proj_community: BTreeMap<&str, u32> = projects
        .keys()
        .enumerate()
        .map(|(i, k)| (k.as_str(), (i + 1) as u32))
        .collect();

    let total_convs: usize = projects.values().map(|p| p.conv_count).sum();
    nodes.push(BrainNode {
        id: "root".into(),
        label: "Second Brain".into(),
        kind: "root".into(),
        weight: total_convs.max(1),
        summary: format!(
            "{} projets · {} conversations analysées.",
            projects.len(),
            total_convs
        ),
        keywords: Vec::new(),
        decisions: Vec::new(),
        patterns: Vec::new(),
        sources: Vec::new(),
        community: 0,
    });

    for (key, p) in projects {
        let pid = format!("p:{key}");
        let community = proj_community.get(key.as_str()).copied().unwrap_or(0);
        nodes.push(BrainNode {
            id: pid.clone(),
            label: p.display.clone(),
            kind: "project".into(),
            weight: p.conv_count,
            summary: p.objective.clone(),
            keywords: top_keywords(&p.concept_counts, 8),
            decisions: dedup_pairs(&p.decisions)
                .into_iter()
                .map(|(d, _)| d)
                .collect(),
            patterns: dedup(&p.patterns),
            sources: dedup_sources(&p.sources),
            community,
        });
        edges.push(BrainEdge {
            source: "root".into(),
            target: pid,
            kind: "project".into(),
            relation: "contains".into(),
            confidence: Confidence::Extracted,
            confidence_score: 1.0,
        });
    }

    // Concepts : les plus fréquents d'abord, plafonnés pour la lisibilité.
    let mut cvec: Vec<(&String, &ConceptAgg)> = concepts.iter().collect();
    cvec.sort_by(|a, b| b.1.occurrences.cmp(&a.1.occurrences));
    for (ckey, c) in cvec.into_iter().take(MAX_CONCEPT_NODES) {
        let cid = format!("c:{ckey}");
        let is_bridge = c.projects.len() >= 2;
        let proj_displays: Vec<String> = c
            .projects
            .iter()
            .map(|k| {
                projects
                    .get(k)
                    .map(|p| p.display.clone())
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let summary = if is_bridge {
            format!(
                "Pont entre {} projets : {}",
                proj_displays.len(),
                proj_displays.join(", ")
            )
        } else if let Some(d) = proj_displays.first() {
            format!("Concept du projet {d}")
        } else {
            String::new()
        };
        // Communauté : pont → 0, concept exclusif → communauté du projet.
        let community = if is_bridge {
            0
        } else {
            c.projects
                .iter()
                .next()
                .and_then(|k| proj_community.get(k.as_str()).copied())
                .unwrap_or(0)
        };
        nodes.push(BrainNode {
            id: cid.clone(),
            label: c.display.clone(),
            kind: "concept".into(),
            weight: c.occurrences,
            summary,
            keywords: Vec::new(),
            decisions: Vec::new(),
            patterns: Vec::new(),
            sources: dedup_sources(&c.sources),
            community,
        });
        for pk in &c.projects {
            // Confiance selon la fréquence du concept dans ce projet.
            let count = projects
                .get(pk)
                .and_then(|p| p.concept_counts.get(ckey))
                .map(|(_, n)| *n)
                .unwrap_or(1);
            let (confidence, confidence_score) = match count {
                n if n >= 3 => (Confidence::Extracted, 0.95),
                2 => (Confidence::Inferred, 0.80),
                _ => (Confidence::Ambiguous, 0.65),
            };
            let relation = if is_bridge { "bridges" } else { "uses" }.to_string();
            edges.push(BrainEdge {
                source: format!("p:{pk}"),
                target: cid.clone(),
                kind: "concept".into(),
                relation,
                confidence,
                confidence_score,
            });
        }
    }

    (nodes, edges)
}

/// Rapport compact optimisé pour l'injection LLM (~10× moins de tokens que brain.md).
fn generate_report(
    projects: &BTreeMap<String, ProjectAgg>,
    concepts: &BTreeMap<String, ConceptAgg>,
    nodes: &[BrainNode],
) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let total_convs: usize = projects.values().map(|p| p.conv_count).sum();
    let n_bridges = nodes.iter().filter(|n| n.community == 0 && n.kind == "concept").count();

    let mut r = format!(
        "# Brain Report — {date}\n> {total_convs} conversations · {} projets · {} concepts · {n_bridges} ponts\n\n",
        projects.len(),
        concepts.len(),
    );

    // Top projets (tableau compact)
    r.push_str("## Projets\n| Projet | Convs | Concepts clés |\n|---|---|---|\n");
    let mut projs: Vec<&ProjectAgg> = projects.values().collect();
    projs.sort_by(|a, b| b.conv_count.cmp(&a.conv_count));
    for p in &projs {
        let kw = top_keywords(&p.concept_counts, 4).join(", ");
        r.push_str(&format!("| {} | {} | {} |\n", p.display, p.conv_count, kw));
    }
    r.push('\n');

    // Ponts inter-projets
    let bridges: Vec<&ConceptAgg> =
        concepts.values().filter(|c| c.projects.len() >= 2).collect();
    if !bridges.is_empty() {
        r.push_str("## Ponts inter-projets\n");
        let mut bv = bridges;
        bv.sort_by(|a, b| b.occurrences.cmp(&a.occurrences));
        for c in bv.iter().take(10) {
            let ps: Vec<String> = c.projects.iter()
                .filter_map(|k| projects.get(k).map(|p| p.display.clone()))
                .collect();
            r.push_str(&format!("- **{}** ({} occ.) : {}\n", c.display, c.occurrences, ps.join(" ↔ ")));
        }
        r.push('\n');
    }

    // Décisions récentes (toutes sources, les 10 dernières)
    r.push_str("## Décisions récentes\n");
    let mut all_decisions: Vec<(String, String, String)> = projs
        .iter()
        .flat_map(|p| {
            dedup_pairs(&p.decisions)
                .into_iter()
                .map(|(d, date)| (p.display.clone(), d, date))
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
    items
        .iter()
        .filter(|s| seen.insert(norm(s)))
        .cloned()
        .collect()
}

fn assemble_markdown(
    projects: &BTreeMap<String, ProjectAgg>,
    concepts: &BTreeMap<String, ConceptAgg>,
) -> String {
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut md = format!("# 🧠 Second Brain — {date}\n\n");

    // Projets, triés par nombre de conversations décroissant.
    md.push_str("## 🗂 Projets actifs\n\n");
    let mut projs: Vec<&ProjectAgg> = projects.values().collect();
    projs.sort_by(|a, b| b.conv_count.cmp(&a.conv_count));
    for p in projs {
        md.push_str(&format!("### {} ({} conversations)\n", p.display, p.conv_count));
        if !p.objective.is_empty() {
            md.push_str(&format!("> {}\n", p.objective));
        }
        let connectors: std::collections::BTreeSet<&str> =
            p.sources.iter().map(|s| s.connector.as_str()).collect();
        let src_label = if connectors.is_empty() {
            "source".to_string()
        } else {
            connectors.into_iter().collect::<Vec<_>>().join(", ")
        };
        for (d, date) in dedup_pairs(&p.decisions) {
            if date.is_empty() {
                md.push_str(&format!("- {d} (source: {src_label})\n"));
            } else {
                md.push_str(&format!("- {d} (source: {src_label} · {date})\n"));
            }
        }
        for pat in dedup(&p.patterns) {
            md.push_str(&format!("- 🔁 {pat}\n"));
        }
        md.push('\n');
    }

    // Concepts récurrents (apparaissant au moins 2 fois), les plus fréquents d'abord.
    md.push_str("## 💡 Concepts récurrents\n\n");
    let mut cs: Vec<&ConceptAgg> = concepts.values().filter(|c| c.occurrences >= 2).collect();
    cs.sort_by(|a, b| b.occurrences.cmp(&a.occurrences));
    if cs.is_empty() {
        md.push_str("_Pas encore de concept récurrent détecté._\n\n");
    }
    let display_of = |key: &str| -> String {
        projects
            .get(key)
            .map(|p| p.display.clone())
            .unwrap_or_else(|| key.to_string())
    };

    for c in &cs {
        let projs: Vec<String> = c.projects.iter().map(|k| display_of(k)).collect();
        md.push_str(&format!(
            "### {}\n- {} occurrences · {} projet(s) : {}\n\n",
            c.display,
            c.occurrences,
            c.projects.len(),
            projs.join(", ")
        ));
    }

    // Connexions : concepts présents dans plusieurs projets → ponts entre projets.
    md.push_str("## 🔗 Connexions identifiées\n\n");
    let mut found = false;
    for c in concepts.values() {
        if c.projects.len() >= 2 {
            let projs: Vec<String> = c.projects.iter().map(|k| display_of(k)).collect();
            md.push_str(&format!("- {} : relient {}\n", c.display, projs.join(" ↔ ")));
            found = true;
        }
    }
    if !found {
        md.push_str("_Aucune connexion inter-projets pour l'instant._\n");
    }

    md
}

fn dedup_pairs(items: &[(String, String)]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
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
    fn short_project_takes_last_segment() {
        assert_eq!(short_project("/Users/x/GitHub/brainlink"), "brainlink");
    }

    #[test]
    fn group_key_empty_gives_non_classe() {
        assert_eq!(group_key(&[]), "non-classé");
    }

    #[test]
    fn group_key_caps_depth_at_2() {
        let path = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(group_key(&path), "a/b");
    }

    #[test]
    fn group_key_single_element() {
        let path = vec!["brainlink".into()];
        assert_eq!(group_key(&path), "brainlink");
    }

    #[test]
    fn parent_group_key_depth_2() {
        assert_eq!(parent_group_key("clients/béaux électricité"), "clients");
    }

    #[test]
    fn parent_group_key_depth_1_gives_non_classe() {
        assert_eq!(parent_group_key("brainlink"), "non-classé");
        assert_eq!(parent_group_key("non-classé"), "non-classé");
    }
}
