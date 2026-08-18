//! Second Brain — point d'entrée de l'application Tauri.
//! Expose les commandes appelables depuis le frontend React.

/// Log vers stderr sans paniquer : `crate::elog!` panique si stderr est fermé
/// (parent mort → Broken pipe), ce qui tuait la génération. Ici on ignore l'échec.
/// Doublé dans `lucid.log` (dossier de données machine) : en `tauri dev` le
/// terminal suffit, mais une fois l'app lancée seule (double-clic, packagée)
/// il n'y a plus de terminal du tout — sans fichier, aucun des diagnostics
/// posés ici (respawn `llama-server`, boucle Archiviste…) n'est récupérable
/// après coup (demandé par Liam le 2026-07-26).
#[macro_export]
macro_rules! elog {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let msg = format!($($arg)*);
        let _ = writeln!(std::io::stderr(), "{msg}");
        $crate::log_to_file(&msg);
    }};
}

/// Écriture best-effort dans `<dossier machine>/lucid.log` — jamais bloquant,
/// jamais de panique si le dossier est absent ou le fichier verrouillé.
/// Plafond de taille : une boucle de log (bug `arch-merge` du 2026-07-28 :
/// 14,3 M de lignes = 1,37 Go sur le disque de Liam) ne doit plus pouvoir
/// remplir le disque. Au-delà du plafond on repart à zéro — un log de
/// diagnostic, pas une archive.
const LOG_MAX_BYTES: u64 = 20 * 1024 * 1024;

pub fn log_to_file(msg: &str) {
    use std::io::Write as _;
    let Some(dir) = ai::llama::shared_data_dir() else { return };
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join("lucid.log");
    let too_big = std::fs::metadata(&path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false);
    let opened = if too_big {
        std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path)
    } else {
        std::fs::OpenOptions::new().create(true).append(true).open(&path)
    };
    if let Ok(mut f) = opened {
        let _ = writeln!(f, "[{secs}] {msg}");
    }
}

mod ai;
mod archivist;
mod backup;
mod connectors;
mod models;
mod secrets;
mod storage;

use ai::{pipeline, LlamaEngine};
use models::{BrainEdge, BrainGraph, BrainNode, ConnectorStatus, Conversation, ConversationSummary};
use tauri::Emitter;

/// Accès public au scan (utilisé par les exemples / tests d'intégration).
pub fn list_conversations_pub() -> Vec<ConversationSummary> {
    connectors::claude_code::list_conversations()
}


/// Insère (ou met à jour) une note « Parcours de démo » à la racine du cerveau —
/// utilisée par `examples/seed_note.rs` pour déposer une note pédagogique dans
/// l'app réelle. Id stable (`note-tour`) : ré-exécutable sans créer de doublon.
pub fn seed_walkthrough_note(content: &str) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut graph: BrainGraph = backup::load_brain_cached(&dir)
        .map_err(|e| format!("brain.json introuvable ({e}) — génère d'abord un cerveau."))?;
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone())
        .ok_or("Aucun nœud racine dans brain.json.")?;
    let id = "note-tour".to_string();
    if let Some(existing) = graph.nodes.iter_mut().find(|n| n.id == id) {
        existing.content = content.to_string();
    } else {
        graph.nodes.push(BrainNode {
            id: id.clone(),
            label: "🧭 Parcours de démo — Lucid".into(),
            kind: "note".into(),
            weight: 0,
            summary: String::new(),
            keywords: vec![],
            decisions: vec![],
            patterns: vec![],
            community: 0,
            parent_id: Some(root_id.clone()),
            synthesized_at: None,
            date: Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
            content: content.to_string(),
            connector: None,
            source_id: None,
            source_project: None,
            source_text: String::new(),
            updated_at: None,
        });
        graph.edges.push(BrainEdge {
            source: root_id, target: id.clone(), kind: "contains".into(), relation: "contains".into(),
        });
    }
    backup::write_brain(&dir, &mut graph)?;
    Ok(id)
}

/// Démo en ligne de commande du pipeline complet (utilisé par `examples/brain.rs`).
/// Persiste le graphe (`brain.json` + `brain.md`) comme le ferait la commande Tauri.
pub fn run_pipeline_demo(limit: usize) -> Result<BrainGraph, String> {
    let engine = LlamaEngine::detect().ok();
    let mut convs = load_all_conversations();
    if limit > 0 {
        convs.truncate(limit);
    }
    let cache_path = ai::llama::app_data_dir().map(|d| d.join("brain_cache.json"));
    let graph = pipeline::generate_brain(engine.as_ref(), &convs, cache_path.as_deref(), |p| {
        crate::elog!("[{}/{}] {}", p.current, p.total, p.label);
    }, |_, _, _| {}, |_, _| {})?;
    if let Some(dir) = ai::llama::app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("brain.md"), &graph.markdown);
        let _ = std::fs::write(dir.join("brain_report.md"), &graph.report);
        let mut graph = graph.clone();
        let _ = backup::write_brain(&dir, &mut graph);
    }
    Ok(graph)
}

/// Dépose une proposition dans `mcp_pending/` — même circuit que le MCP
/// (bulles fantômes pollées par le front), champs non pertinents pour `action`
/// laissés vides. Id fourni par l'appelant (préfixe `arch-`, jamais de collision
/// avec les ids MCP qui sont des uuids).
#[allow(clippy::too_many_arguments)]
fn write_pending_proposal(
    dir: &std::path::Path,
    id: &str,
    action: &str,
    parent_id: &str,
    label: &str,
    target_id: &str,
    new_parent_id: &str,
    merge_ids: &[String],
) -> Result<(), String> {
    let pdir = dir.join("mcp_pending");
    std::fs::create_dir_all(&pdir).map_err(|e| e.to_string())?;
    let proposal = serde_json::json!({
        "id": id, "action": action, "parent_id": parent_id, "label": label, "content": "",
        "target_id": target_id, "new_parent_id": new_parent_id, "merge_ids": merge_ids,
        "link_target": "", "relation": "", "created_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(pdir.join(format!("{id}.json")), proposal.to_string()).map_err(|e| e.to_string())
}

/// Proposition de LIEN (wikilink) entre deux nœuds — `write_pending_proposal`
/// force `link_target`/`relation` à vide, on écrit donc le JSON directement.
fn write_link_proposal(dir: &std::path::Path, id: &str, source_id: &str, link_target: &str, relation: &str) -> Result<(), String> {
    let pdir = dir.join("mcp_pending");
    std::fs::create_dir_all(&pdir).map_err(|e| e.to_string())?;
    let proposal = serde_json::json!({
        "id": id, "action": "link", "parent_id": "", "label": "", "content": "",
        "target_id": source_id, "new_parent_id": "", "merge_ids": [],
        "link_target": link_target, "relation": relation, "created_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(pdir.join(format!("{id}.json")), proposal.to_string()).map_err(|e| e.to_string())
}

/// Cache d'extraction d'entités par document (`archivist_entities.json`) : évite
/// de rappeler Gemma sur un doc inchangé. `sig` = signature bon marché
/// (updated_at + taille du texte) — change dès que le doc change.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct EntityCacheEntry { sig: String, entities: Vec<String> }

fn entity_cache_path(dir: &std::path::Path) -> std::path::PathBuf { dir.join("archivist_entities.json") }

fn load_entity_cache(dir: &std::path::Path) -> std::collections::HashMap<String, EntityCacheEntry> {
    std::fs::read_to_string(entity_cache_path(dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

fn save_entity_cache(dir: &std::path::Path, cache: &std::collections::HashMap<String, EntityCacheEntry>) {
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(entity_cache_path(dir), json);
    }
}

/// Phase 0 embeddings (ADR-0019) — preuve sur le vrai cerveau. Embed un
/// échantillon de documents (leur CONTENU réel, pas le titre) et affiche, pour
/// chacun, son plus proche voisin sémantique + la paire la plus proche et la
/// plus éloignée du lot. Sert à juger la qualité AVANT de refondre l'Archiviste.
/// Lancement : `cargo run --example embed` (nécessite le modèle BGE-M3 dans models/).
pub fn embed_demo() -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    if !ai::llama::embed_model_available() {
        return Err(format!(
            "Modèle d'embedding absent. Télécharge BGE-M3 (~600 Mo) :\n  {}\net place-le dans : {}\n(ou pointe LUCID_EMBED_MODEL vers un .gguf).",
            ai::llama::EMBED_MODEL_URL,
            dir.parent().map(|p| p.join("models").display().to_string()).unwrap_or_default()
        ));
    }
    let graph = backup::load_brain_cached(&dir)?;
    let label_of: std::collections::HashMap<&str, &str> = graph.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str())).collect();
    // Échantillon DIVERS : on pioche des feuilles de dossiers (parents) DIFFÉRENTS
    // en round-robin, pour ne pas se retrouver avec 14 docs du même thème (sinon
    // tout est proche et on ne voit pas le contraste). (doc_label, dossier, texte)
    let mut by_parent: std::collections::BTreeMap<&str, Vec<(&str, String)>> = std::collections::BTreeMap::new();
    for n in &graph.nodes {
        if n.kind != "leaf" { continue; }
        let text = if !n.content.trim().is_empty() { n.content.clone() } else { n.source_text.clone() };
        if text.trim().chars().count() < 40 { continue; }
        let parent = n.parent_id.as_deref().and_then(|p| label_of.get(p).copied()).unwrap_or("?");
        by_parent.entry(parent).or_default().push((n.label.as_str(), text));
    }
    // On garde les dossiers ayant AU MOINS 2 docs (sinon aucune paire intra-dossier
    // à mesurer) et on en prend jusqu'à 3 chacun, en round-robin → un mélange qui
    // contient de vraies paires « même thème » ET « thèmes différents ».
    // On garde les 5 dossiers les PLUS peuplés et on prend jusqu'à 4 docs chacun
    // → de vraies paires « même thème » à mesurer, sur peu de thèmes bien contrastés.
    let mut docs: Vec<(&str, &str, String)> = Vec::new(); // (label, dossier, texte)
    let mut buckets: Vec<_> = by_parent.into_iter().filter(|(_, v)| v.len() >= 2).collect();
    buckets.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    buckets.truncate(5);
    for (parent, v) in &mut buckets {
        for (label, text) in v.drain(..).take(4) { docs.push((label, parent, text)); }
    }
    if docs.len() < 4 {
        return Err("Pas assez de dossiers peuplés (≥2 docs) pour la démo.".into());
    }
    let texts: Vec<String> = docs.iter().map(|(label, _, text)| format!("{label}\n{text}")).collect();
    let vecs = ai::llama::embed_texts(&texts)?;

    // Le VRAI indicateur : la similarité moyenne INTRA-dossier (paires de même
    // thème) doit être NETTEMENT au-dessus de l'INTER-dossier (thèmes différents).
    // Si oui → les embeddings séparent bien les thèmes, indépendamment du fait
    // que les scores absolus soient tassés en haut (propre à BGE-M3).
    let (mut si, mut ci, mut se, mut ce) = (0.0f32, 0u32, 0.0f32, 0u32);
    for i in 0..vecs.len() {
        for j in (i + 1)..vecs.len() {
            let s = ai::llama::cosine(&vecs[i], &vecs[j]);
            if docs[i].1 == docs[j].1 { si += s; ci += 1; } else { se += s; ce += 1; }
        }
    }
    let avg_intra = if ci > 0 { si / ci as f32 } else { 0.0 };
    let avg_inter = if ce > 0 { se / ce as f32 } else { 0.0 };

    let mut out = format!(
        "Embeddings de {} documents (dimension {}).\n\n\
         ➤ Similarité MOYENNE même thème (intra-dossier) : {:.3}  ({} paires)\n\
         ➤ Similarité MOYENNE thèmes différents (inter)   : {:.3}  ({} paires)\n\
         ➤ Écart : {:+.3}  → {}\n\n\
         Plus proche voisin de chaque doc (✓ = même dossier) :\n\n",
        vecs.len(), vecs.first().map(|v| v.len()).unwrap_or(0),
        avg_intra, ci, avg_inter, ce, avg_intra - avg_inter,
        if avg_intra - avg_inter > 0.03 { "les thèmes se séparent bien ✅" } else { "séparation faible ⚠️" }
    );
    let mut same = 0u32;
    for i in 0..vecs.len() {
        let mut best = (usize::MAX, -2.0f32);
        for j in 0..vecs.len() {
            if i == j { continue; }
            let s = ai::llama::cosine(&vecs[i], &vecs[j]);
            if s > best.1 { best = (j, s); }
        }
        let hit = docs[i].1 == docs[best.0].1;
        if hit { same += 1; }
        out.push_str(&format!("• [{:.12}] {:.28}\n   ↳ [{:.12}] {:.28}  ({:.2}){}\n",
            docs[i].1, docs[i].0, docs[best.0].1, docs[best.0].0, best.1, if hit { " ✓" } else { "" }));
    }
    out.push_str(&format!("\nPlus proche voisin dans le MÊME dossier : {}/{}\n", same, vecs.len()));
    Ok(out)
}

/// Point d'entrée de l'Archiviste (prototype) — une passe unique : script
/// (déplacements sûrs) + Gemma (fusions ambiguës), écrit des propositions,
/// n'applique jamais rien directement. Utilisé par `examples/archivist.rs`
/// pour tester sur le vrai cerveau de l'user avant de le brancher en fond.
pub fn run_archivist_scan_once() -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    run_archivist_scan_once_in(&dir)
}

/// Variante injectable (dossier explicite) — testable sans toucher au vrai
/// dossier de données de l'utilisateur, cf. `save_node_content_in` et consorts.
fn run_archivist_scan_once_in(dir: &std::path::Path) -> Result<String, String> {
    run_archivist_scan_once_in_progress(dir, |_, _| {})
}

/// Marqueur de passage en cours — écrit au début d'un passage, retiré
/// uniquement s'il va jusqu'au bout (`Ok`). S'il traîne encore au lancement
/// suivant de l'app, la dernière passe a été interrompue (app fermée/crashée
/// en cours de route) : `archivist_was_interrupted` sert ce diagnostic au
/// front, qui relance une passe silencieuse (cf. `runArchivistNow`). Comme le
/// scan repart de l'état actuel du cerveau, les groupes déjà décidés avant la
/// coupure (propositions déjà sur disque, appliquées au montage par le poll
/// MCP habituel) ne réapparaissent pas — seul le reliquat est retraité.
fn archivist_marker_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("archivist_running.marker")
}

#[tauri::command]
fn archivist_was_interrupted() -> bool {
    ai::llama::app_data_dir().is_some_and(|dir| archivist_marker_path(&dir).exists())
}

/// Rapport de diagnostic Archiviste, à copier/coller pour analyse — **sans aucun
/// contenu de document** (RGPD). Métriques + distribution des domaines + structure.
/// `mask` = true : remplace les noms de dossiers (qui peuvent contenir un nom de
/// client, ex. « Factures Parabola ») par leur DOMAINE majoritaire — à utiliser
/// dès que les données ne sont pas de la démo (bêta-testeurs, vraies données).
#[tauri::command]
fn archivist_diagnostic(mask: bool) -> Result<String, String> {
    use std::collections::{BTreeMap, HashMap};
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph = backup::load_brain_cached(&dir)?;

    let is_theme = |id: &str| id.starts_with("arch-theme-") || id.starts_with("arch-cat-") || id.starts_with("arch-group-");
    // Les dossiers créés par l'Archiviste sont eux-mêmes des nœuds `kind:"note"`
    // (cf. archivist.rs:1011) : sans exclure les ids `arch-`, ils étaient comptés
    // comme des documents et gonflaient à la fois le total et les « rangés »
    // (420/205 annoncés pour 394/187 réels — diagnostiqué le 2026-08-02).
    let is_doc = |n: &BrainNode| (n.kind == "leaf" || n.kind == "note") && !n.id.starts_with("arch-");
    let total = graph.nodes.iter().filter(|n| is_doc(n)).count();
    let sorted = graph.nodes.iter().filter(|n| is_doc(n) && n.parent_id.as_deref().is_some_and(is_theme)).count();
    let non_triable = graph.nodes.iter().filter(|n| is_doc(n) && n.parent_id.as_deref().is_some_and(|p| p.starts_with("arch-non-triable"))).count();
    let themes = graph.nodes.iter().filter(|n| n.id.starts_with("arch-theme-")).count();
    let cats = graph.nodes.iter().filter(|n| n.id.starts_with("arch-cat-") || n.id.starts_with("arch-group-")).count();

    let domains: HashMap<String, DomainEntry> = std::fs::read_to_string(domain_cache_path(&dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    let mut dom_counts: BTreeMap<String, usize> = BTreeMap::new();
    for e in domains.values() { *dom_counts.entry(e.domain.clone()).or_default() += 1; }

    // Domaine majoritaire des enfants tagués d'un dossier (pour le mode masqué).
    let folder_domain = |fid: &str| -> String {
        let mut c: HashMap<&str, usize> = HashMap::new();
        for n in &graph.nodes {
            if n.parent_id.as_deref() == Some(fid) {
                if let Some(e) = domains.get(&n.id) { *c.entry(e.domain.as_str()).or_default() += 1; }
            }
        }
        c.into_iter().max_by_key(|(_, n)| *n).map(|(d, _)| d.to_string()).unwrap_or_else(|| "—".into())
    };

    let mut out = String::from("==================== RAPPORT ARCHIVISTE ====================\n\n");
    // Le rapport est fait pour être collé tel quel à un tiers (humain ou IA) qui
    // ne connaît ni Lucid ni l'Archiviste : sans cette légende il était illisible
    // hors contexte (retour Liam, 2026-08-02).
    out.push_str(
        "— Comment lire ce rapport —\n\
         Lucid indexe les documents locaux d'un utilisateur et un module nommé\n\
         « l'Archiviste » les range automatiquement dans des dossiers thématiques\n\
         qu'il crée lui-même. Ce rapport ne décrit QUE la structure obtenue :\n\
         aucun contenu, aucun nom de fichier, aucun nom de dossier réel.\n\
         \n\
         Documents totaux  : documents connus du cerveau.\n\
         Rangés en thèmes  : rangés par l'Archiviste dans un dossier thématique.\n\
         Non triable       : examinés, mais aucun thème pertinent trouvé.\n\
         Hors périmètre    : jamais traités (ni rangés, ni marqués non triable) —\n\
         \x20                   éclatés par cause juste après, les causes sont\n\
         \x20                   exclusives et leur somme fait le total.\n\
         Domaines          : étiquette donnée à chaque document par le modèle local.\n\
         Arbre             : « N ici » = documents directement dans ce dossier,\n\
         \x20                   « N au total » = en incluant les sous-dossiers.\n\
         [Domaine] #N      : nom de dossier masqué. Le #N identifie le dossier de\n\
         \x20                   façon unique DANS CE RAPPORT — deux lignes du même\n\
         \x20                   domaine avec des numéros différents sont bien deux\n\
         \x20                   dossiers distincts, pas un doublon d'affichage.\n\n",
    );
    // Environnement : un rapport venu d'une autre machine est illisible sans lui.
    // Le sidecar Windows est CPU-only (`bundle-sidecars.ps1` prend l'archive
    // `bin-win-cpu-x64`), donc plusieurs fois plus lent qu'un build Metal — sans
    // cette section, une passe de 40 minutes passerait pour un blocage.
    out.push_str(&format!(
        "— Environnement —\nOS / arch                     : {} / {}\nRAM détectée                  : {:.0} Go\nbinaire llama-completion      : {}\nbinaire llama-server          : {}\nmodèle de génération          : {}\nmodèle d'embedding            : {}\n\n",
        std::env::consts::OS,
        std::env::consts::ARCH,
        ai::llama::detected_ram_gb(),
        ai::llama::completion_binary_available(),
        ai::llama::server_binary_available(),
        ai::llama::generation_model_available(),
        ai::llama::embed_model_available(),
    ));

    // Extraction : sur un corpus qu'on n'a pas sous les yeux, « 300 en Non triable »
    // est indéchiffrable sans savoir combien de fichiers ont produit du texte.
    // Regroupé PAR RAISON — jamais un nom de fichier, même en mode local (la
    // raison seule suffit au diagnostic, et le bloc reste partageable tel quel).
    if let Some(sync) = connectors::local_folder::last_sync() {
        out.push_str("— Extraction (dernier scan des dossiers) —\n");
        out.push_str(&format!("Fichiers indexés  : {}\nExtraits ce scan  : {}\nIllisibles        : {}\n",
            sync.total, sync.new, sync.skipped.len()));
        if !sync.skipped.is_empty() {
            let mut by_reason: BTreeMap<&str, usize> = BTreeMap::new();
            for line in &sync.skipped {
                // Chaque entrée est « <chemin> — <raison> » : on ne garde que la raison.
                let reason = line.split(" — ").nth(1).unwrap_or("raison inconnue");
                *by_reason.entry(reason).or_default() += 1;
            }
            let mut rows: Vec<_> = by_reason.into_iter().collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1));
            for (reason, n) in rows { out.push_str(&format!("  {n:>4}  {reason}\n")); }
        }
        out.push('\n');
    }

    // Réglages EN TÊTE : sans eux, impossible de savoir à quelle configuration
    // correspond un rapport quand on en compare plusieurs (demande Liam, 2026-08-03).
    let last_pass: Option<PassMetrics> = std::fs::read_to_string(pass_metrics_path(&dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok());
    match &last_pass {
        Some(m) => out.push_str(&format!(
            "— Réglages de la dernière passe —\ngarde de domaine (ancrage)    : {}\ngarde de domaine (clustering) : {}\nseuil d'ancrage               : {:.2}\nmarge d'ancrage               : {:.2}\nregroupement en parents       : {}\nrespect des dossiers utilisateur : {}\n\n",
            m.tuning.domain_guard_anchor, m.tuning.domain_guard_cluster,
            m.tuning.anchor_sim_threshold, m.tuning.anchor_min_margin,
            m.tuning.taxonomy_grouping, m.tuning.respect_user_folders,
        )),
        None => out.push_str("— Réglages de la dernière passe —\n(aucune passe enregistrée : lance l'Archiviste une fois)\n\n"),
    }

    // Ni rangé ni marqué non-triable = jamais passé entre les mains de
    // l'Archiviste. Sans cette ligne les trois compteurs ne bouclaient pas sur le
    // total et on ne voyait pas les documents hors périmètre. La population est
    // constituée en extension (pas en soustraction) pour pouvoir l'éclater par
    // cause juste après — un total sans explication n'est pas actionnable
    // (retour Liam, 2026-08-03).
    let hors_ids: std::collections::HashSet<&str> = graph.nodes.iter()
        .filter(|n| is_doc(n))
        .filter(|n| !n.parent_id.as_deref().is_some_and(|p| is_theme(p) || p.starts_with("arch-non-triable")))
        .map(|n| n.id.as_str())
        .collect();
    let hors = hors_ids.len();
    let why = archivist::skip_breakdown(&graph, &hors_ids, last_pass.as_ref().map(|m| m.tuning.respect_user_folders).unwrap_or(true));
    out.push_str(&format!(
        "— Chiffres clés —\nDocuments totaux   : {total}\nRangés en thèmes   : {sorted}\nNon triable        : {non_triable}\nHors périmètre     : {hors}\nThèmes / catégories: {themes} / {cats}\n\n"
    ));
    if hors > 0 {
        out.push_str("— Hors périmètre, par cause —\n");
        for (n, cause) in [
            (why.duplicate_title, "titre partagé avec un autre nœud → réservé à la décision de fusion, jamais rangé"),
            (why.outside_scan_scope, "ni à la racine ni sous un dossier scanné → réputé déjà rangé"),
            (why.has_children, "a des sous-pages → traité comme un dossier, pas comme un document"),
            (why.user_filed, "rangé à la main par toi dans un sous-dossier → volontairement respecté"),
            (why.routed_pending, "rangé par l'Archiviste, mais la proposition n'est pas appliquée"),
            (why.wrong_kind, "type de nœud inattendu"),
        ] {
            if n > 0 { out.push_str(&format!("  {n:>4}  {cause}\n")); }
        }
        out.push('\n');
    }
    // Métriques de la dernière passe : ce que le cerveau seul ne dit pas.
    if let Some(m) = &last_pass {
        out.push_str("— Dernière passe de l'Archiviste —\n");
        out.push_str(&format!("Ancrés sur un dossier existant : {}\n", m.anchored));
        out.push_str(&format!("Clusters formés                : {}\n", m.clusters));
        out.push_str(&format!("Plus gros cluster              : {}\n", m.largest_cluster));
        out.push_str(&format!("Envoyés en « Non triable »     : {}\n", m.non_triable_this_pass));
        out.push_str(&format!("Noms repris du cache           : {}\n", m.names_reused));
        out.push_str(&format!("Durée de la passe              : {} s\n", m.duration_secs));
        out.push_str(&format!("Appels au modèle              : {}\n", m.llm_calls));
        out.push_str(&format!("Vecteurs calculés / en cache   : {} / {}\n", m.embeddings_new, m.embeddings_cached));
        out.push_str(&format!("Doublons exacts fusionnés      : {}\n", m.exact_duplicates));
        // Ancrages détaillés : c'est ce qui permet d'attribuer une pollution de
        // dossier à l'ancrage plutôt qu'au clustering, ou l'inverse.
        if !m.anchors.is_empty() {
            let fresh = m.anchors.iter().filter(|a| a.fresh).count();
            let mut per: BTreeMap<&str, (usize, f32)> = BTreeMap::new();
            for a in &m.anchors {
                let e = per.entry(a.folder.as_str()).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += a.similarity;
            }
            let biggest = per.values().map(|(n, _)| *n).max().unwrap_or(0);
            let avg = m.anchors.iter().map(|a| a.similarity).sum::<f32>() / m.anchors.len() as f32;
            out.push_str(&format!(
                "  dont vers un dossier créé cette passe : {fresh}\n  dont vers un dossier préexistant     : {}\n  plus gros apport dans un dossier     : {biggest}\n  similarité moyenne des ancrages      : {avg:.3}\n",
                m.anchors.len() - fresh,
            ));
        }
        if m.cohesion.is_empty() {
            out.push_str("Similarité interne des clusters : (aucun cluster)\n");
        } else {
            let avg = m.cohesion.iter().sum::<f32>() / m.cohesion.len() as f32;
            let mut sorted = m.cohesion.clone();
            sorted.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let each: Vec<String> = sorted.iter().map(|c| format!("{c:.3}")).collect();
            out.push_str(&format!("Similarité interne moyenne     : {avg:.3}\n"));
            out.push_str(&format!("  par cluster (décroissant)    : {}\n", each.join(", ")));
        }
        out.push('\n');
    }

    out.push_str("— Domaines (étiquette attribuée par le modèle local) —\n");
    let mut dv: Vec<_> = dom_counts.iter().collect();
    dv.sort_by(|a, b| b.1.cmp(a.1));
    for (d, c) in dv { out.push_str(&format!("  {c:>4}  {d}\n")); }
    out.push_str(&format!(
        "  {} documents étiquetés sur {total} ({} sans étiquette)\n\n",
        domains.len(),
        total.saturating_sub(domains.len())
    ));

    out.push_str("— Arbre des dossiers créés par l'Archiviste —\n");
    // Mode masqué : le domaine seul ne suffit pas à identifier un dossier — cinq
    // dossiers distincts de facturation s'affichaient tous « [Facturation] », et
    // le parent réduit à un « [parent] » constant faisait perdre TOUTE la
    // hiérarchie (rapport illisible sur données réelles, constaté 2026-08-02).
    // On numérote chaque dossier : l'arbre redevient reconstructible sans jamais
    // révéler un label. Le numéro n'a de sens que dans un rapport donné.
    let arch: Vec<&BrainNode> = graph.nodes.iter().filter(|n| n.id.starts_with("arch-")).collect();
    let masked: HashMap<&str, String> = arch.iter().enumerate()
        .map(|(i, n)| {
            let name = if n.id.starts_with("arch-non-triable") {
                format!("Non triable #{i}")
            } else {
                format!("[{}] #{i}", folder_domain(&n.id))
            };
            (n.id.as_str(), name)
        })
        .collect();

    // Liste plate « dossier ← parent » auparavant : le lecteur devait reconstruire
    // l'arbre de tête, et le compteur affiché mélangeait documents et sous-dossiers
    // (un dossier de 4 sous-dossiers affichait « 4 », lu comme 4 documents).
    let names: HashMap<&str, String> = if mask {
        masked
    } else {
        arch.iter().map(|n| (n.id.as_str(), n.label.clone())).collect()
    };
    let direct_docs: HashMap<&str, usize> = arch.iter()
        .map(|n| {
            let k = graph.nodes.iter()
                .filter(|c| c.parent_id.as_deref() == Some(n.id.as_str()) && is_doc(c))
                .count();
            (n.id.as_str(), k)
        })
        .collect();

    // Noms des documents rangés dans chaque dossier — UNIQUEMENT en mode non
    // masqué. C'est le mode de vérification LOCALE : sans les noms, impossible de
    // dire pourquoi un document se retrouve dans le mauvais dossier (demande de
    // Liam le 2026-08-03). Le mode masqué reste le mode de PARTAGE et ne doit
    // jamais en contenir un seul.
    let doc_labels: Option<std::collections::HashMap<&str, Vec<(&str, &str)>>> = (!mask).then(|| {
        let mut m: std::collections::HashMap<&str, Vec<(&str, &str)>> = std::collections::HashMap::new();
        for n in &graph.nodes {
            if !is_doc(n) { continue; }
            if let Some(p) = n.parent_id.as_deref() {
                if p.starts_with("arch-") { m.entry(p).or_default().push((n.id.as_str(), n.label.as_str())); }
            }
        }
        for v in m.values_mut() { v.sort_unstable_by_key(|(_, l)| *l); }
        m
    });

    // id de document → (ancré sur un dossier créé cette passe ?, similarité).
    // Sert à marquer les arrivants par ANCRAGE dans l'arbre local : sans marqueur,
    // on ne peut pas dire si un intrus vient de l'ancrage ou du clustering.
    let anchor_marks: std::collections::HashMap<&str, (bool, f32)> = last_pass.as_ref()
        .map(|m| m.anchors.iter().map(|a| (a.doc.as_str(), (a.fresh, a.similarity))).collect())
        .unwrap_or_default();

    let arch_ids: std::collections::HashSet<&str> = arch.iter().map(|n| n.id.as_str()).collect();
    let mut kids: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut roots: Vec<&str> = Vec::new();
    for n in &arch {
        match n.parent_id.as_deref() {
            Some(p) if arch_ids.contains(p) => kids.entry(p).or_default().push(n.id.as_str()),
            _ => roots.push(n.id.as_str()),
        }
    }

    let mut trees: Vec<(usize, String)> = roots.iter()
        .map(|r| {
            let mut buf = String::new();
            let t = diag_tree(r, 0, &kids, &direct_docs, &names, doc_labels.as_ref(), &anchor_marks, &mut buf);
            (t, buf)
        })
        .collect();
    trees.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, buf) in trees { out.push_str(&buf); }

    if mask {
        out.push_str("\n(aucun contenu ni nom de fichier dans ce rapport — RGPD)\n");
        out.push_str("(mode masqué : noms de dossiers remplacés par leur domaine + un numéro)\n");
    } else {
        out.push_str("\n⚠️ MODE LOCAL — ce rapport contient les VRAIS noms de dossiers et de\n");
        out.push_str("documents. À garder sur ta machine : ne le colle ni dans un chat, ni dans\n");
        out.push_str("un ticket, ni à une IA tierce. Pour partager, utilise le diagnostic anonymisé.\n");
    }
    Ok(out)
}

/// Rend un dossier de l'Archiviste et sa descendance en arbre indenté ; renvoie
/// le nombre de documents du sous-arbre. Sous-arbres triés par taille pour que
/// l'essentiel arrive en haut. Fonction libre plutôt que closure : elle récurse.
fn diag_tree(
    id: &str,
    depth: usize,
    kids: &std::collections::HashMap<&str, Vec<&str>>,
    docs: &std::collections::HashMap<&str, usize>,
    names: &std::collections::HashMap<&str, String>,
    doc_labels: Option<&std::collections::HashMap<&str, Vec<(&str, &str)>>>,
    anchor_marks: &std::collections::HashMap<&str, (bool, f32)>,
    out: &mut String,
) -> usize {
    let direct = docs.get(id).copied().unwrap_or(0);
    let mut parts: Vec<(usize, String)> = kids.get(id).map(|v| v.as_slice()).unwrap_or(&[]).iter()
        .map(|c| {
            let mut buf = String::new();
            let t = diag_tree(c, depth + 1, kids, docs, names, doc_labels, anchor_marks, &mut buf);
            (t, buf)
        })
        .collect();
    parts.sort_by(|a, b| b.0.cmp(&a.0));

    let total = direct + parts.iter().map(|p| p.0).sum::<usize>();
    let name = names.get(id).map(|s| s.as_str()).unwrap_or("?");
    let indent = "    ".repeat(depth);
    let bullet = if depth == 0 { "" } else { "└ " };
    if parts.is_empty() {
        out.push_str(&format!("  {indent}{bullet}{name} — {direct} ici\n"));
    } else {
        out.push_str(&format!("  {indent}{bullet}{name} — {direct} ici, {total} au total\n"));
    }
    // Mode local : le contenu du dossier, un document par ligne — c'est là qu'on
    // voit d'un coup d'œil ce qui n'a rien à y faire.
    if let Some(labels) = doc_labels {
        for (doc_id, label) in labels.get(id).map(|v| v.as_slice()).unwrap_or(&[]) {
            match anchor_marks.get(doc_id) {
                // Arrivé par ANCRAGE : distingué, avec sa similarité et sa phase.
                // « thème neuf » = phase §④, la plus suspecte (centroïde calculé
                // sur 3 à 5 documents seulement).
                Some((fresh, sim)) => out.push_str(&format!(
                    "  {indent}    ↳ {label}  [ancré {sim:.3}{}]\n",
                    if *fresh { ", thème neuf" } else { "" },
                )),
                None => out.push_str(&format!("  {indent}    · {label}\n")),
            }
        }
    }
    for (_, buf) in parts { out.push_str(&buf); }
    total
}

// ── Cache d'embeddings (incrémental) ─────────────────────────────────────────
// Sans lui, chaque passe ré-embed tout le cerveau (des centaines de docs → très
// lent). Clé = id du doc ; on ré-embed seulement si sa signature (updated_at +
// taille du texte) a changé. Même esprit que le cache d'extraction du pipeline.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct EmbedEntry { sig: String, vec: Vec<f32> }

fn embed_cache_path(dir: &std::path::Path) -> std::path::PathBuf { dir.join("archivist_embeddings.json") }

/// Signature = empreinte du TEXTE réellement embeddé, jamais `updated_at`.
/// L'estampille bouge dès qu'un CHAMP du nœud change — or l'Archiviste
/// DÉPLACE les documents qu'il range (`parent_id` → nouvelle estampille) :
/// keyer dessus invalidait tout le cache à chaque passe, donc ré-embeddait et
/// re-taggait les ~230 documents à chaque fois (~6 min de GPU à fond pour une
/// seule note neuve, plus la surchauffe — diagnostiqué le 2026-07-31 sur les
/// logs de Liam : « 231 nouveau(x) vecteur(s), 0 déjà en cache » à CHAQUE run).
/// Sur le texte, un déplacement ne change rien : le cache tient.
fn embed_sig(n: &BrainNode) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(embed_text(n).as_bytes());
    format!("{:x}", h.finalize())
}
fn embed_text(n: &BrainNode) -> String {
    let content = if !n.content.trim().is_empty() { n.content.as_str() } else { n.source_text.as_str() };
    format!("{}\n{}", n.label, content)
}

/// Embeddings d'un ensemble de nœuds, avec cache disque : ne ré-embed que les
/// docs neufs/modifiés. Renvoie `id → vecteur` (les docs non embeddables, ex.
/// moteur indisponible, sont simplement absents de la map).
/// Renvoie `(id → vecteur, nombre de vecteurs CALCULÉS pendant cet appel)`. Le
/// second terme remonte au rapport : sur une machine lente, il dit si le temps est
/// parti dans l'embedding ou ailleurs.
fn embed_nodes_cached(dir: &std::path::Path, nodes: &[&BrainNode]) -> (std::collections::HashMap<String, Vec<f32>>, usize) {
    let mut cache: std::collections::HashMap<String, EmbedEntry> =
        std::fs::read_to_string(embed_cache_path(dir)).ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default();
    // À (ré)embedder : entrée absente, signature changée, ou vecteur vide (échec
    // précédent). On n'écrit dans le cache qu'APRÈS un embed réussi — sinon un
    // échec transitoire marquerait le doc « fait » à vide et il ne serait jamais
    // re-tenté.
    let mut todo_ids: Vec<String> = Vec::new();
    let mut todo_sigs: Vec<String> = Vec::new();
    let mut todo_texts: Vec<String> = Vec::new();
    for n in nodes {
        let sig = embed_sig(n);
        let fresh = cache.get(&n.id).map(|e| e.sig == sig && !e.vec.is_empty()).unwrap_or(false);
        if !fresh {
            todo_ids.push(n.id.clone());
            todo_sigs.push(sig);
            todo_texts.push(embed_text(n));
        }
    }
    if !todo_texts.is_empty() {
        let started = std::time::Instant::now();
        match ai::llama::embed_texts(&todo_texts) {
            Ok(vecs) => {
                for (i, id) in todo_ids.iter().enumerate() {
                    if let Some(v) = vecs.get(i) {
                        cache.insert(id.clone(), EmbedEntry { sig: todo_sigs[i].clone(), vec: v.clone() });
                    }
                }
                if let Ok(json) = serde_json::to_string(&cache) { let _ = std::fs::write(embed_cache_path(dir), json); }
                // Débit affiché : le run à froid est rejoué à chaque test de Liam,
                // il faut pouvoir comparer deux versions sans chronomètre à la main.
                let secs = started.elapsed().as_secs_f32();
                crate::elog!("🗂️ embeddings: {} nouveau(x) vecteur(s) en {:.0}s ({:.2}s/doc), {} déjà en cache.",
                    todo_texts.len(), secs, secs / todo_texts.len() as f32,
                    nodes.len().saturating_sub(todo_texts.len()));
            }
            Err(e) => crate::elog!("🗂️ embeddings indisponibles ({e})."),
        }
    }
    // Vecteurs RÉELLEMENT calculés, pas tentés : en cas d'échec du moteur, `todo`
    // reste plein alors que rien n'a été produit. Rapporter les tentatives ferait
    // croire à un embedding réussi sur une machine où il a échoué — exactement le
    // contresens à éviter sur un run distant (Windows sans llama-server).
    let computed = todo_ids.iter()
        .filter(|id| cache.get(*id).is_some_and(|e| !e.vec.is_empty()))
        .count();
    let out = nodes.iter()
        .filter_map(|n| cache.get(&n.id).filter(|e| !e.vec.is_empty()).map(|e| (n.id.clone(), e.vec.clone())))
        .collect();
    (out, computed)
}

// Cache des tags de domaine (même esprit que le cache d'embeddings) : un doc
// n'est reclassé par Gemma que si sa signature change → payé une fois. Sans
// moteur, on rend simplement ce qui est déjà en cache.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct DomainEntry { sig: String, domain: String }

fn domain_cache_path(dir: &std::path::Path) -> std::path::PathBuf { dir.join("archivist_domains.json") }

/// Renvoie `(id → domaine, lots_en_échec)`. Le second terme remonte jusqu'au
/// rapport de l'Archiviste : une passe avec des lots en échec a des trous, et
/// doit le dire plutôt que de se lire comme une passe réussie.
fn domain_tags_cached(dir: &std::path::Path, nodes: &[&BrainNode], engine: Option<&LlamaEngine>) -> (std::collections::HashMap<String, String>, usize) {
    let mut cache: std::collections::HashMap<String, DomainEntry> =
        std::fs::read_to_string(domain_cache_path(dir)).ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default();
    let mut failed_batches = 0usize;
    if let Some(e) = engine {
        // À (re)tagger : entrée absente ou signature changée.
        let todo: Vec<(String, String, String)> = nodes.iter()
            .filter(|n| cache.get(&n.id).map(|d| d.sig != embed_sig(n)).unwrap_or(true))
            .map(|n| {
                let raw = if !n.content.trim().is_empty() { n.content.as_str() } else { n.source_text.as_str() };
                let snippet = raw.split_whitespace().collect::<Vec<_>>().join(" ");
                (n.id.clone(), n.label.clone(), snippet.chars().take(200).collect::<String>())
            })
            .collect();
        if !todo.is_empty() {
            let sigs: std::collections::HashMap<String, String> =
                nodes.iter().map(|n| (n.id.clone(), embed_sig(n))).collect();
            // Seuls les docs des lots RÉUSSIS reviennent ici : ceux des lots en
            // échec sont absents, donc rien n'est écrit pour eux et ils seront
            // retentés au prochain scan (cf. `ai_domain_tags`).
            let (tags, fails) = archivist::ai_domain_tags(e, &todo);
            failed_batches = fails;
            let written = tags.len();
            for (id, domain) in tags {
                if let Some(sig) = sigs.get(&id) {
                    cache.insert(id.clone(), DomainEntry { sig: sig.clone(), domain });
                }
            }
            if let Ok(json) = serde_json::to_string(&cache) { let _ = std::fs::write(domain_cache_path(dir), json); }
            // Diagnostic : combien de docs ont VRAIMENT reçu un domaine (≠ « Autre ») —
            // si ~0, le tagging a échoué (réponse Gemma inexploitable) et la garde ne
            // filtre rien (bug 2026-07-29 : réponse tronquée → 202/202 « Autre »).
            crate::elog!("🗂️ domaines: {written} nouveau(x) tag(s) sur {} demandé(s), {} déjà en cache — {} tag(s) au total, {failed_batches} lot(s) en échec.",
                todo.len(), nodes.len().saturating_sub(todo.len()), cache.len());
        }
    }
    let tags = nodes.iter()
        .filter_map(|n| cache.get(&n.id).map(|d| (n.id.clone(), d.domain.clone())))
        .collect();
    (tags, failed_batches)
}

/// Purge unique des « Autre » fossilisés par l'ancien `ai_domain_tags` : un lot
/// en échec y écrivait « Autre » sous la signature courante des documents, qui
/// n'étaient donc plus jamais reclassés. On retire ces entrées UNE fois — les
/// docs concernés seront simplement retaggés à la passe suivante (ceux qui sont
/// légitimement « Autre » le redeviendront, au prix d'un appel par 30 docs).
///
/// Marqueur sur disque, sinon purger à chaque lancement forcerait le
/// reclassement de tous les « Autre » légitimes indéfiniment. Le marqueur n'est
/// posé qu'après une écriture réussie : un disque plein fait retenter au
/// lancement suivant plutôt que de perdre la purge en silence.
///
/// Appelée au début de chaque passe de l'Archiviste plutôt qu'au démarrage de
/// l'app : c'est le seul endroit où le dossier de données du compte ACTIF est
/// garanti (il dépend de `active_user`, qui peut changer après un login), et ça
/// enchaîne directement sur le retaggage au lieu d'attendre la passe suivante.
fn purge_fossil_autre_once(dir: &std::path::Path) {
    // Marqueur VERSIONNÉ : « Autre » a été retiré de la taxonomie le 2026-08-03,
    // les entrées « Autre » encore en cache doivent donc être retaguées une fois
    // de plus. Bump du suffixe = la purge se rejoue exactement une fois.
    let marker = dir.join("archivist_domains.purged.v2");
    if marker.exists() { return; }
    let path = domain_cache_path(dir);
    let mut cache: std::collections::HashMap<String, DomainEntry> =
        match std::fs::read_to_string(&path).ok().and_then(|r| serde_json::from_str(&r).ok()) {
            Some(c) => c,
            // Pas de cache (ou illisible) : rien à purger, mais on pose quand
            // même le marqueur — le nouveau code ne peut plus produire de fossile.
            None => { let _ = std::fs::write(&marker, b""); return; }
        };
    let before = cache.len();
    cache.retain(|_, d| d.domain != "Autre");
    let removed = before - cache.len();
    if removed > 0 {
        let Ok(json) = serde_json::to_string(&cache) else { return };
        if std::fs::write(&path, json).is_err() { return; }
        crate::elog!("🗂️ domaines: {removed} tag(s) « Autre » purgé(s) (fossiles d'un lot en échec) — reclassement à cette passe.");
    }
    let _ = std::fs::write(&marker, b"");
}

// ── Cache des NOMS de thèmes (stabilité entre deux passes) ───────────────────
// Gemma nomme librement, et par échantillonnage (temp 0.2) : deux passes sur le
// MÊME groupe de documents rendaient deux noms différents. Comme l'id du dossier
// est dérivé du nom (`arch-theme-<label>`), un renommage ne renomme pas — il crée
// un dossier JUMEAU à côté de l'ancien (« Données Communales France » ET « Données
// Géographiques Communes » pour les mêmes fichiers, remonté par Liam le
// 2026-07-31 : « le tri se fait pareil mais le nom des catégories change »).
// On mémorise donc le nom par COMPOSITION du groupe : même ensemble de documents
// → même nom, sans appel Gemma. Un groupe qui gagne ou perd quelques documents
// reste reconnu par recouvrement, sinon le moindre ajout relancerait le nommage.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ClusterName { members: Vec<String>, name: String }

fn cluster_names_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("archivist_cluster_names.json")
}

fn load_cluster_names(dir: &std::path::Path) -> Vec<ClusterName> {
    std::fs::read_to_string(cluster_names_path(dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

/// Recouvrement minimal (Jaccard) pour considérer que c'est LE MÊME groupe qu'à
/// la passe précédente. 0.6 = le groupe peut gagner/perdre ~1 document sur 3 et
/// garder son nom ; en dessous, c'est un autre groupe et il mérite son nom.
const CLUSTER_RENAME_OVERLAP: f32 = 0.6;

fn recall_cluster_name(cache: &[ClusterName], members: &[String]) -> Option<String> {
    let set: std::collections::HashSet<&str> = members.iter().map(String::as_str).collect();
    let mut best: Option<(f32, &str)> = None;
    for entry in cache {
        let prev: std::collections::HashSet<&str> = entry.members.iter().map(String::as_str).collect();
        let inter = set.intersection(&prev).count() as f32;
        let union = set.union(&prev).count() as f32;
        if union == 0.0 { continue; }
        let j = inter / union;
        if j >= CLUSTER_RENAME_OVERLAP && best.map(|(bj, _)| j > bj).unwrap_or(true) {
            best = Some((j, entry.name.as_str()));
        }
    }
    best.map(|(_, name)| name.to_string())
}

/// Enregistre (ou rafraîchit) la composition associée à un nom — le groupe
/// mémorisé suit ainsi les documents qui s'y ajoutent au fil des passes.
fn remember_cluster_name(cache: &mut Vec<ClusterName>, members: &[String], name: &str) {
    cache.retain(|e| e.name != name);
    cache.push(ClusterName { members: members.to_vec(), name: name.to_string() });
}

// ── Cache des décisions de fusion ────────────────────────────────────────────
// `decide_group` = un appel Gemma par groupe de titres en doublon, à CHAQUE
// passe. Tant que Liam n'a pas accepté les fusions proposées, les doublons
// restent dans le cerveau, le scan les retrouve et Gemma re-décide exactement la
// même chose — ~20 appels de plus par passe, indéfiniment (le ventilateur qu'il
// entend après chaque run). Un groupe est identifié par SES pages : mêmes pages
// → même décision, déjà prise. `ParseFailed` n'est jamais mémorisé (ce n'est pas
// une décision, c'est un échec : il faut le retenter).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MergeDecision { members: Vec<String>, survivor: String, dropped: Vec<String>, reason: String }

fn merge_decisions_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("archivist_merge_decisions.json")
}

fn load_merge_decisions(dir: &std::path::Path) -> std::collections::HashMap<String, MergeDecision> {
    std::fs::read_to_string(merge_decisions_path(dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

fn group_key(node_ids: &[String]) -> String {
    let mut ids: Vec<&str> = node_ids.iter().map(String::as_str).collect();
    ids.sort_unstable();
    ids.join("|")
}

/// `decide_group` avec mémoire. `survivor` vide = « garder séparées » (mémorisé
/// aussi : une non-fusion re-jugée à chaque passe coûte autant qu'une fusion).
fn decide_group_cached(
    engine: &LlamaEngine,
    group: &archivist::DuplicateGroup,
    graph: &BrainGraph,
    cache: &mut std::collections::HashMap<String, MergeDecision>,
) -> archivist::GroupOutcome {
    let key = group_key(&group.node_ids);
    if let Some(d) = cache.get(&key) {
        return if d.survivor.is_empty() {
            archivist::GroupOutcome::KeepSeparate { reason: d.reason.clone() }
        } else {
            archivist::GroupOutcome::Merge(archivist::GroupDecision {
                survivor_id: d.survivor.clone(),
                dropped_ids: d.dropped.clone(),
                reason: d.reason.clone(),
            })
        };
    }
    let outcome = archivist::decide_group(engine, group, graph);
    match &outcome {
        archivist::GroupOutcome::Merge(d) => { cache.insert(key, MergeDecision {
            members: group.node_ids.clone(), survivor: d.survivor_id.clone(),
            dropped: d.dropped_ids.clone(), reason: d.reason.clone() }); }
        archivist::GroupOutcome::KeepSeparate { reason } => { cache.insert(key, MergeDecision {
            members: group.node_ids.clone(), survivor: String::new(),
            dropped: Vec::new(), reason: reason.clone() }); }
        archivist::GroupOutcome::ParseFailed { .. } => {} // pas une décision → à retenter
    }
    outcome
}

/// Similarité au centroïde (moyenne des vecteurs d'un groupe). Le centroïde n'a
/// pas besoin d'être normalisé : `cosine` renormalise.
fn centroid(vecs: &[&Vec<f32>]) -> Vec<f32> {
    if vecs.is_empty() { return Vec::new(); }
    let dim = vecs[0].len();
    let mut c = vec![0.0f32; dim];
    for v in vecs { for i in 0..dim.min(v.len()) { c[i] += v[i]; } }
    for x in &mut c { *x /= vecs.len() as f32; }
    c
}

/// Plan de rangement PAR EMBEDDINGS (ADR-0019) — tout est du calcul de vecteurs,
/// Gemma ne fait que NOMMER (§⑤ du flow). Deux sorties :
///  - `anchors` : (doc → dossier EXISTANT) par proximité au centroïde du dossier (§②) ;
///  - `clusters` : nouveaux thèmes formés par clustering du reste (§③), nommés par Gemma.
/// Un ancrage, tracé. Sans ce détail, rien ne distinguait dans le cerveau un
/// document arrivé par ANCRAGE d'un document arrivé par CLUSTERING — on ne pouvait
/// donc pas attribuer une pollution de dossier à l'un ou à l'autre, et on en a
/// débattu trois fois sans preuve (2026-08-03).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct AnchorRecord {
    doc: String,
    folder: String,
    /// `true` = dossier créé dans CETTE passe (§④), `false` = dossier préexistant (§②).
    /// C'est la distinction qui dit si le coupable est le nouvel ancrage ou l'ancien.
    fresh: bool,
    similarity: f32,
}

struct EmbedPlan {
    anchors: Vec<AnchorRecord>,
    clusters: Vec<archivist::ThemeCluster>,
    /// Lots de classement de domaine dont l'appel IA a échoué — remontés au
    /// rapport pour ne pas présenter une passe trouée comme une passe complète.
    domain_failures: usize,
    /// Similarité interne moyenne de chaque cluster retenu (membres ↔ centroïde),
    /// même mesure que la garde de cohésion. Même ordre que `clusters`.
    cohesion: Vec<f32>,
    /// Clusters dont le nom vient du cache (donc aucun appel de nommage).
    names_reused: usize,
    /// Vecteurs calculés / réutilisés pendant cette passe.
    embeddings_new: usize,
    embeddings_cached: usize,
}

/// Seuil pour rattacher un document à un dossier EXISTANT (§②).
/// Défaut seulement : la valeur effective vient de `ArchivistTuning`.
///
/// 0,88 et non 0,84 : mesuré le 2026-08-03 sur 110 candidats réels et 22 dossiers,
/// 0,84 produisait 32 bons ancrages pour 10 fautifs (un guide crypto dans un
/// dossier de contrats, un plan de déménagement dans le trading…). 0,84 → 0,88
/// coûte 5 bons et retire 6 fautifs.
const ANCHOR_SIM_THRESHOLD: f32 = 0.88;

/// Avance minimale du meilleur dossier sur le DEUXIÈME meilleur, à domaine égal.
/// Un document qui ressemble autant à deux dossiers est ambigu : on préfère le
/// laisser en « Non triable » que de le ranger au hasard des décimales.
///
/// Même principe que `MIN_MARGIN` du rattachement mécanique (`archivist.rs`), et
/// même raison — « mieux vaut laisser un orphelin de côté que le ranger au mauvais
/// endroit ». Mesuré : avec la marge, 23 bons ancrages pour 1 fautif ; sans, 27
/// pour 4. Une erreur de rangement coûte plus cher qu'une omission, qui reste
/// honnête et sera retentée.
const ANCHOR_MIN_MARGIN: f32 = 0.02;

// ── Réglages de passe (fichier, sans recompilation) ──────────────────────────
// Les deux gardes inter-domaines et le seuil d'ancrage sont pilotables par
// fichier pour MESURER leur effet sur des passes successives (demande Liam,
// 2026-08-03 — hypothèse à tester : depuis le k-NN mutuel + la garde de cohésion,
// la garde de domaine ne protégerait plus rien et empêcherait l'ancrage, qui
// exige simultanément cosinus ≥ seuil ET même domaine). Les défauts reproduisent
// exactement le comportement d'avant ce flag.

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
struct ArchivistTuning {
    /// Garde inter-domaines à l'ANCRAGE sur un dossier existant.
    #[serde(default = "tuning_true")]
    domain_guard_anchor: bool,
    /// Garde inter-domaines au CLUSTERING (un paquet par domaine).
    #[serde(default = "tuning_true")]
    domain_guard_cluster: bool,
    /// Similarité cosinus minimale pour rejoindre un dossier existant.
    #[serde(default = "tuning_anchor_sim")]
    anchor_sim_threshold: f32,
    /// Avance minimale sur le 2ᵉ meilleur dossier (0 = pas de marge).
    #[serde(default = "tuning_anchor_margin")]
    anchor_min_margin: f32,
    /// ADR-0022 phase 1.2/1.3 : respecter les sous-dossiers montés à la main, et en
    /// faire des destinations d'ancrage. `false` restaure le comportement d'avant le
    /// 2026-08-03 sans recompilation — c'est la marche arrière si la décision produit
    /// change (ADR-0022 encore `proposé`).
    #[serde(default = "tuning_true")]
    respect_user_folders: bool,
    /// Passe de consolidation qui invente des dossiers PARENTS pour chapeauter les
    /// thèmes. `false` par défaut (décision Liam, 2026-08-03) : les feuilles sont
    /// fiables, les parents étaient devinés d'après un nom et rangeaient de travers.
    /// Repasser à `true` pour remesurer.
    #[serde(default)]
    taxonomy_grouping: bool,
}

fn tuning_true() -> bool { true }
fn tuning_anchor_sim() -> f32 { ANCHOR_SIM_THRESHOLD }
fn tuning_anchor_margin() -> f32 { ANCHOR_MIN_MARGIN }

impl Default for ArchivistTuning {
    fn default() -> Self {
        Self {
            domain_guard_anchor: true,
            domain_guard_cluster: true,
            anchor_sim_threshold: ANCHOR_SIM_THRESHOLD,
            anchor_min_margin: ANCHOR_MIN_MARGIN,
            respect_user_folders: true,
            taxonomy_grouping: false,
        }
    }
}

fn tuning_path(dir: &std::path::Path) -> std::path::PathBuf { dir.join("archivist_tuning.json") }

/// Lit les réglages de la passe. Crée le fichier avec les défauts s'il est absent
/// — sinon il faudrait deviner son nom ET son schéma pour le modifier. Fichier
/// illisible ⇒ défauts : un réglage cassé ne doit jamais empêcher une passe.
/// Chaque champ a son `serde(default)` : un fichier partiel garde les défauts
/// pour le reste, il n'y a donc pas de garde désactivée par omission.
fn load_tuning(dir: &std::path::Path) -> ArchivistTuning {
    let path = tuning_path(dir);
    match std::fs::read_to_string(&path).ok().and_then(|r| serde_json::from_str(&r).ok()) {
        Some(t) => t,
        None => {
            let t = ArchivistTuning::default();
            if let Ok(json) = serde_json::to_string_pretty(&t) { let _ = std::fs::write(&path, json); }
            t
        }
    }
}

// ── Métriques de la dernière passe ───────────────────────────────────────────
// Le rapport de diagnostic lit le CERVEAU, pas une passe : ces chiffres-là ne
// sont visibles que pendant l'exécution. On les persiste donc pour pouvoir
// comparer deux passes lancées avec deux réglages différents.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone)]
struct PassMetrics {
    tuning: ArchivistTuning,
    anchored: usize,
    clusters: usize,
    largest_cluster: usize,
    cohesion: Vec<f32>,
    /// Documents envoyés en « Non triable » PAR CETTE PASSE (≠ total du bac).
    non_triable_this_pass: usize,
    names_reused: usize,
    /// Coût de la passe — indispensable pour interpréter un run sur une machine
    /// dont on n'a pas la main (le sidecar Windows est CPU-only, donc plusieurs
    /// fois plus lent qu'un build Metal).
    #[serde(default)]
    duration_secs: u64,
    #[serde(default)]
    llm_calls: usize,
    #[serde(default)]
    embeddings_new: usize,
    #[serde(default)]
    embeddings_cached: usize,
    /// Détail des ancrages : qui est allé où, par quelle phase, à quelle distance.
    #[serde(default)]
    anchors: Vec<AnchorRecord>,
    /// Copies redondantes supprimées du cerveau (doublons au texte identique).
    #[serde(default)]
    exact_duplicates: usize,
}

fn pass_metrics_path(dir: &std::path::Path) -> std::path::PathBuf { dir.join("archivist_last_pass.json") }

/// Meilleure cible d'ancrage pour un document, ou `None` s'il vaut mieux le
/// laisser de côté. `targets` = (id du dossier, centroïde, domaine majoritaire).
///
/// Deux refus possibles, et c'est le second qui fait la qualité : un document
/// presque aussi proche de deux dossiers ne se range PAS. Extrait en fonction
/// libre pour servir les deux phases d'ancrage (dossiers existants, puis clusters
/// tout juste formés) avec exactement la même règle.
fn best_anchor(
    v: &[f32],
    doc_dom: Option<&str>,
    targets: &[(String, Vec<f32>, Option<String>)],
    tuning: &ArchivistTuning,
) -> Option<(String, f32)> {
    if tuning.domain_guard_anchor && doc_dom.is_none() { return None; }
    let mut best: Option<(&str, f32)> = None;
    let mut second: Option<f32> = None;
    for (fid, c, fdom) in targets {
        if tuning.domain_guard_anchor && fdom.as_deref() != doc_dom { continue; }
        let s = ai::llama::cosine(v, c);
        match best {
            Some((_, bs)) if s <= bs => { if second.is_none_or(|sc| s > sc) { second = Some(s); } }
            Some((_, bs)) => { second = Some(bs); best = Some((fid.as_str(), s)); }
            None => best = Some((fid.as_str(), s)),
        }
    }
    let (fid, s) = best?;
    if s < tuning.anchor_sim_threshold { return None; }
    // Pas de 2ᵉ cible dans ce domaine ⇒ aucune ambiguïté possible, la marge ne
    // s'applique pas (sinon un domaine à dossier unique n'ancrerait jamais).
    if second.is_some_and(|sc| s - sc < tuning.anchor_min_margin) { return None; }
    Some((fid.to_string(), s))
}

fn embed_organize(dir: &std::path::Path, pool: &[(String, String)], graph: &BrainGraph, engine: Option<&LlamaEngine>, tuning: &ArchivistTuning) -> EmbedPlan {
    let mut plan = EmbedPlan {
        anchors: Vec::new(), clusters: Vec::new(), domain_failures: 0,
        cohesion: Vec::new(), names_reused: 0,
        embeddings_new: 0, embeddings_cached: 0,
    };
    if pool.is_empty() { return plan; }

    // Dossiers thématiques existants (jamais « Non triable » ni dossiers-source)
    // et leurs enfants (documents déjà rangés) → pour calculer leurs centroïdes.
    let is_theme = |id: &str| id.starts_with("arch-theme-") || id.starts_with("arch-cat-") || id.starts_with("arch-group-");
    // ADR-0022 phase 1.3 — les dossiers montés par l'humain deviennent aussi des
    // destinations. INDISSOCIABLE de la phase 1.2 : respecter un dossier `Ideeri`
    // sans pouvoir y ranger une facture qui traîne pousse l'Archiviste à créer un
    // second dossier `Factures Ideeri` à côté. La règle seule AUGMENTE la
    // fragmentation ; c'est mécanique, pas de la malchance.
    //
    // Un conteneur qualifie s'il n'est pas au premier niveau : `p:Documents` ou
    // `p:Drive` sont les emplacements que le système a donnés, `p:Documents/Clients/X`
    // et `p:Drive/Ideeri/Factures` sont des actes de classement. Pas besoin de
    // connaître les connecteurs pour faire cette distinction.
    let root_id_ref = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.as_str());
    let is_user_folder = |n: &BrainNode| {
        n.kind == "container"
            && Some(n.id.as_str()) != root_id_ref
            && n.parent_id.as_deref() != root_id_ref
            && n.parent_id.is_some()
    };
    // Un conteneur « appartient » à la source MAJORITAIRE de ses documents (les nœuds
    // conteneurs ne portent pas eux-mêmes de connecteur). Majorité et non « aucun
    // intrus » : un dossier Drive ayant déjà reçu un document local ne doit pas
    // cesser d'être une destination pour autant.
    let user_folders: std::collections::HashSet<&str> = if tuning.respect_user_folders {
        let mut sources: std::collections::HashMap<&str, std::collections::HashMap<&str, usize>> =
            std::collections::HashMap::new();
        for n in &graph.nodes {
            if n.kind != "leaf" && n.kind != "note" { continue; }
            if let (Some(p), Some(c)) = (n.parent_id.as_deref(), n.connector.as_deref()) {
                *sources.entry(p).or_default().entry(c).or_default() += 1;
            }
        }
        graph.nodes.iter()
            .filter(|n| is_user_folder(n))
            .filter(|n| {
                sources.get(n.id.as_str())
                    .and_then(|c| c.iter().max_by_key(|(_, k)| **k).map(|(src, _)| *src))
                    .is_some_and(|src| archivist::RECEIVING_SOURCES.contains(&src))
            })
            .map(|n| n.id.as_str())
            .collect()
    } else {
        std::collections::HashSet::new()
    };
    let folder_children: std::collections::HashMap<&str, Vec<&BrainNode>> = {
        let mut m: std::collections::HashMap<&str, Vec<&BrainNode>> = std::collections::HashMap::new();
        for n in &graph.nodes {
            if let Some(p) = n.parent_id.as_deref() {
                let eligible = is_theme(p) || user_folders.contains(p);
                if eligible && (n.kind == "leaf" || n.kind == "note") { m.entry(p).or_default().push(n); }
            }
        }
        m
    };

    // Embed (caché) le pool ET les enfants des dossiers existants, en un seul lot.
    let by_id: std::collections::HashMap<&str, &BrainNode> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut to_embed: Vec<&BrainNode> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (id, _) in pool { if let Some(n) = by_id.get(id.as_str()) { if seen.insert(n.id.as_str()) { to_embed.push(n); } } }
    for kids in folder_children.values() { for n in kids { if seen.insert(n.id.as_str()) { to_embed.push(n); } } }
    let (vecs, embedded_now) = embed_nodes_cached(dir, &to_embed);
    plan.embeddings_new = embedded_now;
    plan.embeddings_cached = to_embed.len().saturating_sub(embedded_now);
    if vecs.is_empty() { return plan; } // moteur indispo → repli (rien rangé par embeddings)

    // Tag de domaine (le signal SUJET) pour tout ce qu'on manipule. Sert de GARDE :
    // on n'ancre/ne regroupe jamais à travers deux domaines, même si l'embedding
    // les croit proches (forme partagée). Caché → payé une fois par doc.
    let (domains, domain_failures) = domain_tags_cached(dir, &to_embed, engine);
    plan.domain_failures = domain_failures;
    // `None` = document non tagué. Plus de repli « Autre » : sans domaine il n'y a
    // pas de garde à évaluer, donc on ne regroupe pas — le document repart en
    // « Non triable » et sera retagué au scan suivant (son absence du cache le
    // garantit). Assimiler « pas de domaine » à un domaine revenait à autoriser
    // tous ces documents à se regrouper entre eux, ce que la garde était
    // précisément censée empêcher.
    let domain_of = |id: &str| domains.get(id).map(|s| s.as_str());

    // Centroïde + domaine MAJORITAIRE des enfants de chaque dossier existant.
    // Le domaine est `Option` : un dossier dont aucun enfant n'est tagué n'en a
    // pas. Garde ON, il devient inancrable (rien à comparer) ; garde OFF, il
    // redevient candidat comme les autres — c'est ce qu'on veut pouvoir mesurer.
    let centroids: Vec<(String, Vec<f32>, Option<String>)> = folder_children.iter()
        .filter_map(|(fid, kids)| {
            let kv: Vec<&Vec<f32>> = kids.iter().filter_map(|n| vecs.get(&n.id)).collect();
            if kv.is_empty() { return None; }
            let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for n in kids {
                if let Some(d) = domain_of(&n.id) { *counts.entry(d).or_default() += 1; }
            }
            let dom = counts.into_iter().max_by_key(|(_, c)| *c).map(|(d, _)| d.to_string());
            Some((fid.to_string(), centroid(&kv), dom))
        })
        .collect();

    // §② ANCRAGE : rejoint le dossier existant le plus proche par centroïde, MAIS
    // uniquement de MÊME DOMAINE — sans ça un journal de trading (proche en forme
    // d'une proposition immobilière : 0.95) était aspiré dans « Immobilier »
    // (mesuré le 2026-07-29 : 11/11 docs trading happés). Le domaine coupe net.
    let mut anchored: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (id, _) in pool {
        let Some(v) = vecs.get(id) else { continue };
        if let Some((fid, sim)) = best_anchor(v, domain_of(id), &centroids, tuning) {
            plan.anchors.push(AnchorRecord { doc: id.clone(), folder: fid, fresh: false, similarity: sim });
            anchored.insert(id.clone());
        }
    }

    // §③ CLUSTERING du reste (non ancré), PAR DOMAINE (jamais à travers) → nouveaux
    // thèmes, nommés par Gemma (§⑤). Le clustering fin (quel client) reste fait par
    // l'embedding À L'INTÉRIEUR de chaque domaine.
    let rest: Vec<&(String, String)> = pool.iter().filter(|(id, _)| !anchored.contains(id) && vecs.contains_key(id)).collect();
    let mut name_cache = load_cluster_names(dir);
    let mut reused = 0usize;
    let mut by_domain: std::collections::BTreeMap<&str, Vec<usize>> = std::collections::BTreeMap::new();
    // Garde ON : un paquet par domaine, les non tagués ne forment aucun paquet.
    // Garde OFF : UN seul paquet, le k-NN mutuel et la garde de cohésion font seuls
    // le travail — c'est l'hypothèse à mesurer. La suite (seuils, MIN_CLUSTER,
    // cohésion) est identique dans les deux cas.
    for (i, (id, _)) in rest.iter().enumerate() {
        if !tuning.domain_guard_cluster {
            by_domain.entry("(garde désactivée)").or_default().push(i);
        } else if let Some(d) = domain_of(id) {
            by_domain.entry(d).or_default().push(i);
        }
    }
    for (_dom, idxs) in &by_domain {
        if idxs.len() < 3 { continue; }
        let sub_vecs: Vec<Vec<f32>> = idxs.iter().map(|&i| vecs[&rest[i].0].clone()).collect();
        for g in archivist::cluster_indices(&sub_vecs) {
            if g.len() < 3 { continue; }
            let global: Vec<usize> = g.iter().map(|&li| idxs[li]).collect();
            let node_ids: Vec<String> = global.iter().map(|&i| rest[i].0.clone()).collect();
            // Déjà nommé à une passe précédente ? On reprend le nom tel quel —
            // pas d'appel Gemma, donc pas de dossier jumeau.
            let label = match recall_cluster_name(&name_cache, &node_ids) {
                Some(known) => { reused += 1; known }
                None => {
                    // Échantillons = titre + extrait de CONTENU : le nommage peut ainsi
                    // repérer un client/entreprise récurrent (« Parabola ») que le titre
                    // seul (« Invoice-14545AA1-… ») ne révèle jamais.
                    let samples: Vec<String> = global.iter().take(12).map(|&i| {
                        let (id, label) = (&rest[i].0, &rest[i].1);
                        let snippet = by_id.get(id.as_str()).map(|n| {
                            let raw = if !n.content.trim().is_empty() { n.content.as_str() } else { n.source_text.as_str() };
                            raw.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(200).collect::<String>()
                        }).unwrap_or_default();
                        if snippet.is_empty() { label.clone() } else { format!("{label} — {snippet}") }
                    }).collect();
                    let Some(fresh) = engine.and_then(|e| archivist::ai_name_cluster(e, &samples)) else { continue; };
                    fresh
                }
            };
            remember_cluster_name(&mut name_cache, &node_ids, &label);
            // Cohésion du cluster RETENU, dans le même ordre que `clusters` : mesure
            // seule, elle n'entre dans aucune décision (la garde de cohésion a déjà
            // statué dans `cluster_indices`).
            let member_vecs: Vec<&Vec<f32>> = node_ids.iter().filter_map(|id| vecs.get(id)).collect();
            let coh = if member_vecs.is_empty() { 0.0 } else {
                let c = centroid(&member_vecs);
                member_vecs.iter().map(|v| ai::llama::cosine(v, &c)).sum::<f32>() / member_vecs.len() as f32
            };
            plan.cohesion.push(coh);
            plan.clusters.push(archivist::ThemeCluster { label, node_ids });
        }
    }
    if let Ok(json) = serde_json::to_string(&name_cache) {
        let _ = std::fs::write(cluster_names_path(dir), json);
    }

    // §④ ANCRAGE SUR LES CLUSTERS DE CETTE PASSE. Sans ça, un document proche d'un
    // thème tout juste formé devait attendre une passe suivante pour le rejoindre —
    // or l'utilisateur n'en lance qu'une (reset + scan + une passe auto). Mesuré le
    // 2026-08-03 : c'est 92 % de ce qu'une seconde exécution récupérait (36 des 39
    // documents), le reclustering du reliquat ne pesant que les 8 % restants.
    //
    // Les cibles n'existent pas encore dans le graphe : leur id est celui que
    // `apply_theme_cluster` calculera (`arch-theme-<label>`), et le circuit de
    // résolution applique déjà « create » du dossier avant le « move » vers lui.
    let fresh_targets: Vec<(String, Vec<f32>, Option<String>)> = plan.clusters.iter()
        .filter_map(|c| {
            let vs: Vec<&Vec<f32>> = c.node_ids.iter().filter_map(|id| vecs.get(id)).collect();
            if vs.is_empty() { return None; }
            let mut dd: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
            for id in &c.node_ids {
                if let Some(d) = domain_of(id) { *dd.entry(d).or_default() += 1; }
            }
            let dom = dd.into_iter().max_by_key(|(_, n)| *n).map(|(d, _)| d.to_string());
            Some((format!("arch-theme-{}", c.label.to_lowercase()), centroid(&vs), dom))
        })
        .collect();
    if !fresh_targets.is_empty() {
        let clustered: std::collections::HashSet<&str> = plan.clusters.iter()
            .flat_map(|c| c.node_ids.iter().map(String::as_str))
            .collect();
        let mut joined = 0usize;
        for (id, _) in pool {
            if anchored.contains(id) || clustered.contains(id.as_str()) { continue; }
            let Some(v) = vecs.get(id) else { continue };
            if let Some((fid, sim)) = best_anchor(v, domain_of(id), &fresh_targets, tuning) {
                plan.anchors.push(AnchorRecord { doc: id.clone(), folder: fid, fresh: true, similarity: sim });
                anchored.insert(id.clone());
                joined += 1;
            }
        }
        if joined > 0 {
            crate::elog!("🗂️ embed_organize: {joined} page(s) rattachée(s) à un thème créé dans CETTE passe.");
        }
    }

    plan.names_reused = reused;
    crate::elog!("🗂️ embed_organize: {} ancré(s) sur l'existant, {} thème(s) (dont {} noms repris du cache).",
        plan.anchors.len(), plan.clusters.len(), reused);
    plan
}

/// Comme `run_archivist_scan_once_in`, avec un callback de progression appelé
/// avant chaque décision Gemma (`current`, `total` groupes de doublons) — c'est
/// la partie qui domine le temps (un appel = un rechargement complet du modèle,
/// cf. `ai/llama.rs`), donc c'est elle qu'on rapporte à l'UI plutôt que le reste
/// (script, quasi instantané).
fn run_archivist_scan_once_in_progress(
    dir: &std::path::Path,
    on_progress: impl Fn(usize, usize),
) -> Result<String, String> {
    let _ = std::fs::write(archivist_marker_path(dir), b"");
    purge_fossil_autre_once(dir);
    let tuning = load_tuning(dir);
    let mut metrics = PassMetrics { tuning, ..Default::default() };
    crate::elog!("🗂️ réglages: garde_ancrage={}, garde_clustering={}, seuil_ancrage={:.2}",
        tuning.domain_guard_anchor, tuning.domain_guard_cluster, tuning.anchor_sim_threshold);
    let graph: BrainGraph = backup::load_brain_cached(dir)?;
    let result = archivist::scan(&graph, tuning.respect_user_folders);
    let engine = LlamaEngine::detect().ok();
    // Relevé d'avant-passe : le compteur du moteur est cumulatif depuis le
    // lancement, seul le delta concerne CETTE passe.
    let failed_calls_before = ai::llama::failed_calls();
    let total_calls_before = ai::llama::total_calls();
    let pass_started = std::time::Instant::now();
    let mut domain_failures = 0usize;

    let mut report = String::new();
    let mut n = 0usize;
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());

    // TOUT par embeddings (ADR-0019 — demande de Liam le 2026-07-29 : « l'embedding
    // doit marcher pour tout, pas que le Non triable »). On abandonne le rangement
    // thématique MÉCANIQUE (par mot/token, fragile — c'est lui qui mettait « Lyon »
    // sous « Projets ») : on rassemble TOUS les documents « à ranger » dans UN pool,
    // et l'ancrage sur l'existant puis le clustering par EMBEDDINGS font tout le
    // travail. Du mécanique, on ne garde que la fusion des doublons de titre exact
    // (plus bas). Un document DÉJÀ rangé sous un thème n'est jamais repris → la
    // stabilité/incrémental est préservée.
    let label_of: std::collections::HashMap<&str, &str> =
        graph.nodes.iter().map(|nd| (nd.id.as_str(), nd.label.as_str())).collect();
    let mut candidate_ids: Vec<String> = Vec::new();
    for mv in &result.moves { candidate_ids.push(mv.node_id.clone()); }              // rattachements mécaniques → au pool
    candidate_ids.extend(result.orphans_unresolved_ids.iter().cloned());            // orphelins
    for c in &result.theme_clusters { candidate_ids.extend(c.node_ids.iter().cloned()); } // ex-clustering mécanique → au pool
    if let Some(cid) = &result.catchall_id {                                        // backlog du bac « Non triable »
        for nd in &graph.nodes {
            if nd.parent_id.as_deref() == Some(cid.as_str()) && (nd.kind == "leaf" || nd.kind == "note") {
                candidate_ids.push(nd.id.clone());
            }
        }
    }
    let mut in_pool: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ai_pool: Vec<(String, String)> = candidate_ids.into_iter()
        .filter(|id| in_pool.insert(id.clone()))
        .map(|id| { let label = label_of.get(id.as_str()).copied().unwrap_or("").to_string(); (id, label) })
        .collect();

    // Applique un cluster de thème (mécanique OU IA, même forme) : crée le
    // conteneur s'il n'existe pas encore et y range ses pages dans LE MÊME
    // passage — le circuit de résolution des propositions boucle déjà jusqu'à
    // ce que plus rien ne progresse, donc un "move" vers un id tout juste créé
    // se retente une fois le "create" appliqué (sinon un thème neuf sortait
    // vide de ce passage, ses pages restant noyées dans "Non triable" en
    // attendant un passage suivant qui n'arrive pas forcément — bug réel
    // remonté par Liam le 2026-07-23). Id déterministe (`arch-theme-<slug>`) :
    // ne rematche QUE le conteneur que l'Archiviste a lui-même créé pour ce
    // thème, jamais un nœud existant qui porterait le même libellé par pure
    // coïncidence (autre bug réel du même jour).
    let apply_theme_cluster = |cluster: &archivist::ThemeCluster, n: &mut usize, report: &mut String| -> Result<(), String> {
        let theme_id = format!("arch-theme-{}", cluster.label.to_lowercase());
        let existing = graph.nodes.iter().find(|c| c.id == theme_id);
        match existing {
            Some(container) => {
                for node_id in &cluster.node_ids {
                    let id = format!("arch-move-{n}");
                    *n += 1;
                    write_pending_proposal(dir, &id, "move", "", "", node_id, &container.id, &[])?;
                }
                report.push_str(&format!(
                    "→ déplacer {} page(s) vers « {} »\n",
                    cluster.node_ids.len(), cluster.label
                ));
            }
            None => {
                if let Some(root_id) = &root_id {
                    write_pending_proposal(dir, &theme_id, "create", root_id, &cluster.label, "", "", &[])?;
                    for node_id in &cluster.node_ids {
                        let id = format!("arch-move-{n}");
                        *n += 1;
                        write_pending_proposal(dir, &id, "move", "", "", node_id, &theme_id, &[])?;
                    }
                    report.push_str(&format!(
                        "→ créer « {} » et y ranger {} page(s)\n",
                        cluster.label, cluster.node_ids.len()
                    ));
                }
            }
        }
        Ok(())
    };

    // (Plus de clustering mécanique par mot : tout est dans `ai_pool`, traité par
    // embeddings ci-dessous. `apply_theme_cluster` reste utilisé pour appliquer les
    // clusters — quelle que soit leur origine — juste après.)

    // ── Rangement PAR EMBEDDINGS (ADR-0019) : ancrage sur l'existant (§②, par
    // CENTROÏDE — calcul, plus Gemma) + clustering du reste (§③) → nouveaux thèmes
    // nommés par Gemma (§⑤). Tout le TRI est déterministe. Repli sur l'ancien flux
    // Gemma (ancrage + clustering LLM) si le modèle d'embedding est absent (parité).
    let mut ai_clustered: std::collections::HashSet<String> = std::collections::HashSet::new();
    // Thèmes CRÉÉS ce run (id déterministe = celui d'apply_theme_cluster, label, taille,
    // échantillon de labels d'enfants) → donnés à la consolidation dans LA MÊME passe,
    // pour ne plus exiger un 2e lancement manuel (demande de Liam le 2026-07-29).
    let mut created_themes: Vec<(String, String, usize, Vec<String>)> = Vec::new();
    let theme_sample = |ids: &[String]| ids.iter().take(5)
        .filter_map(|id| graph.nodes.iter().find(|n| &n.id == id).map(|n| n.label.clone()))
        .collect::<Vec<_>>();
    let capture_theme = |created: &mut Vec<(String, String, usize, Vec<String>)>, c: &archivist::ThemeCluster| {
        created.push((format!("arch-theme-{}", c.label.to_lowercase()), c.label.clone(), c.node_ids.len(), theme_sample(&c.node_ids)));
    };
    crate::elog!("🗂️ archiviste: engine={}, embed={}, ai_pool={} candidats (catchall_id={:?})",
        engine.is_some(), ai::llama::embed_model_available(), ai_pool.len(), result.catchall_id);

    if ai::llama::embed_model_available() {
        let plan = embed_organize(dir, &ai_pool, &graph, engine.as_ref(), &tuning);
        domain_failures = plan.domain_failures;
        metrics.anchored = plan.anchors.len();
        metrics.clusters = plan.clusters.len();
        metrics.largest_cluster = plan.clusters.iter().map(|c| c.node_ids.len()).max().unwrap_or(0);
        metrics.cohesion = plan.cohesion.clone();
        metrics.names_reused = plan.names_reused;
        metrics.anchors = plan.anchors.clone();
        metrics.embeddings_new = plan.embeddings_new;
        metrics.embeddings_cached = plan.embeddings_cached;
        let mut anchored: std::collections::HashSet<String> = std::collections::HashSet::new();
        for a in &plan.anchors {
            let mv = format!("arch-move-{n}"); n += 1;
            write_pending_proposal(dir, &mv, "move", "", "", &a.doc, &a.folder, &[])?;
            anchored.insert(a.doc.clone());
        }
        if !plan.anchors.is_empty() {
            report.push_str(&format!("→ {} page(s) rangée(s) dans des dossiers existants (par similarité)\n", plan.anchors.len()));
        }
        for cluster in &plan.clusters {
            for id in &cluster.node_ids { ai_clustered.insert(id.clone()); }
            apply_theme_cluster(cluster, &mut n, &mut report)?;
            capture_theme(&mut created_themes, cluster);
        }
        if !plan.clusters.is_empty() {
            report.push_str(&format!("→ {} nouveau(x) thème(s) créé(s) par embeddings\n", plan.clusters.len()));
        }
        ai_pool.retain(|(id, _)| !anchored.contains(id));
    } else if let Some(e) = &engine {
        // Repli sans embeddings : ancrage + clustering via Gemma (ancien comportement).
        let snippet_of = |id: &str| -> String {
            graph.nodes.iter().find(|nd| nd.id == id).map(|nd| {
                let raw = if !nd.content.trim().is_empty() { nd.content.as_str() } else { nd.source_text.as_str() };
                raw.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(160).collect::<String>()
            }).unwrap_or_default()
        };
        let folder_sample = |id: &str| graph.nodes.iter()
            .filter(|nd| nd.parent_id.as_deref() == Some(id))
            .take(5).map(|nd| nd.label.clone()).collect::<Vec<_>>();
        let existing_folders: Vec<(String, String, Vec<String>)> = graph.nodes.iter()
            .filter(|nd| nd.id.starts_with("arch-theme-") || nd.id.starts_with("arch-cat-") || nd.id.starts_with("arch-group-"))
            .map(|nd| (nd.id.clone(), nd.label.clone(), folder_sample(&nd.id)))
            .collect();
        if !existing_folders.is_empty() && !ai_pool.is_empty() {
            let pages: Vec<(String, String, String)> = ai_pool.iter()
                .map(|(id, label)| (id.clone(), label.clone(), snippet_of(id))).collect();
            let assigns = archivist::ai_assign_to_folders(e, &pages, &existing_folders);
            let mut anchored: std::collections::HashSet<String> = std::collections::HashSet::new();
            for (page, folder) in &assigns {
                let mv = format!("arch-move-{n}"); n += 1;
                write_pending_proposal(dir, &mv, "move", "", "", page, folder, &[])?;
                anchored.insert(page.clone());
            }
            if !assigns.is_empty() {
                ai_pool.retain(|(id, _)| !anchored.contains(id));
                report.push_str(&format!("→ {} page(s) rangée(s) dans des dossiers existants\n", assigns.len()));
            }
        }
        for cluster in &archivist::ai_cluster_leftovers(e, &ai_pool, &graph) {
            for id in &cluster.node_ids { ai_clustered.insert(id.clone()); }
            apply_theme_cluster(cluster, &mut n, &mut report)?;
            capture_theme(&mut created_themes, cluster);
        }
    }

    // 2ᵉ filet — regroupement par ENTITÉ distinctive sur les restes que les
    // embeddings n'ont pas su clusterer (notes courtes : titre parlant mais
    // contenu trop court pour une cohésion fiable, cf. session 2026-07-30).
    // Rareté (IDF) + nom propre + MIN 3 → produit ex. « Papiris » sans refaire
    // le « Lyon → Projets » de l'ancien clustering par mot.
    {
        let leftover_now: Vec<(String, String)> = ai_pool.iter()
            .filter(|(id, _)| !ai_clustered.contains(id))
            .cloned()
            .collect();
        for cluster in archivist::cluster_by_entity(&graph, &leftover_now) {
            for id in &cluster.node_ids { ai_clustered.insert(id.clone()); }
            apply_theme_cluster(&cluster, &mut n, &mut report)?;
            capture_theme(&mut created_themes, &cluster);
        }
    }

    // Bac "Non triable" : ce qui reste après le tri mécanique ET la passe IA.
    // Crée le bac s'il n'existe pas encore, ou route vers son id réel
    // (`result.catchall_id`, peut différer de la constante — dossier créé
    // manuellement ou par une version antérieure) sinon.
    // Ce qui n'a été ni ancré (retiré de `ai_pool`) ni regroupé par embeddings.
    let still_leftover: Vec<(String, String)> = ai_pool.iter()
        .filter(|(id, _)| !ai_clustered.contains(id))
        .cloned()
        .collect();
    metrics.non_triable_this_pass = still_leftover.len();
    if !still_leftover.is_empty() {
        let target_id = match &result.catchall_id {
            Some(existing) => existing.clone(),
            None => {
                if let Some(root_id) = &root_id {
                    write_pending_proposal(
                        dir, archivist::CATCHALL_ID, "create",
                        root_id, archivist::CATCHALL_LABEL, "", "", &[],
                    )?;
                }
                archivist::CATCHALL_ID.to_string()
            }
        };
        for (orphan_id, _) in &still_leftover {
            let id = format!("arch-move-{n}");
            n += 1;
            write_pending_proposal(dir, &id, "move", "", "", orphan_id, &target_id, &[])?;
        }
        report.push_str(&format!(
            "→ ranger {} page(s) sans destination évidente dans « {} »\n",
            still_leftover.len(), archivist::CATCHALL_LABEL
        ));
    }

    // ── Consolidation en arborescence — DÉSACTIVÉE PAR DÉFAUT ────────────────
    // Elle regroupe les dossiers-thèmes sous des parents inventés par Gemma, à
    // partir de leur NOM et d'un échantillon de 5 libellés d'enfants. Sur données
    // réelles, c'est la couche qui produit les erreurs les plus visibles : des
    // devis d'un client pharmaceutique classés sous « Santé », avec des notes de
    // physique et des fichiers de nutrition pour voisins. Elle sur-structure aussi
    // (« Administration › Facturation › Factures X » = trois niveaux pour dire
    // « des factures »), et `collapse_taxonomy` n'y suffit pas.
    //
    // Décision Liam, 2026-08-03 : « vaut mieux 25 dossiers bien rangés que 8
    // nuls. » Les FEUILLES sont bonnes (elles viennent du clustering par
    // embeddings, déterministe) ; les parents sont des devinettes. On garde donc
    // les dossiers à plat. Le code reste en place derrière `taxonomy_grouping`
    // pour pouvoir le remesurer sans le réécrire.
    if tuning.taxonomy_grouping {
    if let (Some(e), Some(root_id)) = (&engine, root_id.as_ref()) {
        // Candidats = tous les regroupements thématiques Archiviste au niveau racine
        // (thèmes plats + groupes/catégories déjà créés) + les clusters de CE run,
        // avec leur taille (nb de pages) pour aider l'IA à juger. Jamais les
        // dossiers-source (p:*) ni le bac « Non triable ».
        let label_of: std::collections::HashMap<&str, &str> = graph.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str())).collect();
        let child_count = |id: &str| graph.nodes.iter().filter(|nd| nd.parent_id.as_deref() == Some(id)).count();
        // Échantillon du CONTENU d'un dossier (labels d'enfants) : donné à l'IA de
        // consolidation pour qu'elle range sur le SENS RÉEL et pas sur un nom
        // ambigu (« Carte » = cartes d'identité, pas géographie — bug remonté par
        // Liam le 2026-07-28).
        let child_sample = |id: &str| graph.nodes.iter()
            .filter(|nd| nd.parent_id.as_deref() == Some(id))
            .take(5).map(|nd| nd.label.clone()).collect::<Vec<_>>();
        let mut themes: Vec<(String, String, usize, Vec<String>)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let _ = &label_of; // (réservé si on réintègre un échantillon par node_ids)
        // Thèmes DÉJÀ présents au niveau racine (runs précédents).
        for node in &graph.nodes {
            let is_theme = node.id.starts_with("arch-theme-") || node.id.starts_with("arch-group-") || node.id.starts_with("arch-cat-");
            if is_theme && node.parent_id.as_deref() == Some(root_id.as_str()) && seen.insert(node.id.clone()) {
                themes.push((node.id.clone(), node.label.clone(), child_count(&node.id), child_sample(&node.id)));
            }
        }
        // + les thèmes CRÉÉS ce run (encore des propositions, absents de graph.nodes) :
        // ainsi la consolidation chapeaute DÈS CETTE PASSE, plus besoin d'un 2e run
        // manuel. Le circuit de résolution applique create(thème) puis create(cat) puis
        // move(thème→cat) dans le même lot (dépendances gérées par re-boucle).
        for (id, label, size, sample) in &created_themes {
            if seen.insert(id.clone()) {
                themes.push((id.clone(), label.clone(), *size, sample.clone()));
            }
        }

        let slugify = |s: &str| s.to_lowercase().chars().map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>().split('-').filter(|x| !x.is_empty()).collect::<Vec<_>>().join("-");
        let mut created: std::collections::HashSet<String> = std::collections::HashSet::new();

        for grp in archivist::ai_group_themes(e, &themes) {
            // Crée la chaîne de conteneurs du chemin, du plus général au plus précis.
            let mut cumulative = String::new();
            let mut parent = root_id.clone();
            for segment in &grp.path {
                cumulative = if cumulative.is_empty() { slugify(segment) } else { format!("{cumulative}-{}", slugify(segment)) };
                let cat_id = format!("arch-cat-{cumulative}");
                let exists = graph.nodes.iter().any(|c| c.id == cat_id);
                if !exists && created.insert(cat_id.clone()) {
                    write_pending_proposal(dir, &cat_id, "create", &parent, segment, "", "", &[])?;
                }
                parent = cat_id;
            }
            // Range les thèmes sous le dernier segment (sauf ceux déjà dedans).
            for theme_id in &grp.theme_ids {
                if graph.nodes.iter().any(|nd| nd.id == *theme_id && nd.parent_id.as_deref() == Some(parent.as_str())) { continue; }
                let id = format!("arch-move-{n}");
                n += 1;
                write_pending_proposal(dir, &id, "move", "", "", theme_id, &parent, &[])?;
            }
            report.push_str(&format!("→ regrouper {} dossier(s) sous « {} »\n", grp.theme_ids.len(), grp.path.join(" / ")));
        }
    }
    }

    // Fusions calculées AVANT l'aplatissement : un dossier absorbé par une fusion
    // ne doit pas recevoir en plus un « move » vers la racine. Sinon, la fusion
    // s'appliquant d'abord, sa cible disparaît et le déplacement reste bloqué pour
    // toujours dans `mcp_pending/` — c'est le scénario qui rechargeait le canvas
    // toutes les 30 s (bug du 2026-08-03).
    let doc_domains: std::collections::HashMap<String, String> =
        std::fs::read_to_string(domain_cache_path(dir)).ok()
            .and_then(|r| serde_json::from_str::<std::collections::HashMap<String, DomainEntry>>(&r).ok())
            .map(|c| c.into_iter().map(|(k, v)| (k, v.domain)).collect())
            .unwrap_or_default();
    let merges = archivist::folder_merges(&graph, &doc_domains);
    let absorbed: std::collections::HashSet<&str> =
        merges.iter().flat_map(|m| m.absorbed_ids.iter().map(String::as_str)).collect();

    if !tuning.taxonomy_grouping {
      if let Some(root_id) = root_id.as_ref() {
        // Aplatissement : ramène à la racine les thèmes qu'une passe précédente a
        // rangés sous un parent inventé. Sans ça, désactiver la consolidation
        // arrêterait seulement d'en créer de NOUVEAUX — les mauvais parents déjà
        // en place resteraient affichés, donc le problème resterait entier.
        //
        // Idempotent sans marqueur : on ne propose que pour un thème dont le parent
        // ACTUEL est un conteneur de consolidation. Une fois à la racine, plus rien
        // n'est proposé. Passe par le circuit de propositions comme le reste — pas
        // d'écriture directe dans le cerveau.
        let is_cat = |id: &str| id.starts_with("arch-cat-") || id.starts_with("arch-group-");
        let mut flattened = 0usize;
        for node in &graph.nodes {
            if !node.id.starts_with("arch-theme-") { continue; }
            if !node.parent_id.as_deref().is_some_and(is_cat) { continue; }
            if absorbed.contains(node.id.as_str()) { continue; } // la fusion s'en charge
            let id = format!("arch-move-{n}");
            n += 1;
            write_pending_proposal(dir, &id, "move", "", "", &node.id, root_id, &[])?;
            flattened += 1;
        }
        if flattened > 0 {
            report.push_str(&format!(
                "→ {flattened} dossier(s) ramené(s) à la racine (regroupement automatique désactivé)\n"
            ));
        }
      }
    }

    // Doublons EXACTS (même texte extrait) — bande « certain » de l'ADR-0022 :
    // aucune inférence, donc aucun appel au modèle et rien à faire trancher. Émis
    // AVANT les autres propositions pour que l'ordre du rapport reflète l'ordre de
    // certitude. `ids[0]` survit (cf. `merge_nodes_on`).
    for group in &result.exact_duplicates {
        let id = format!("arch-merge-{n}");
        n += 1;
        write_pending_proposal(dir, &id, "merge", "", "", "", "", group)?;
        report.push_str(&format!(
            "→ fusionner {} copies d'un même document (texte identique)\n",
            group.len(),
        ));
    }
    metrics.exact_duplicates = result.exact_duplicates.iter().map(|g| g.len() - 1).sum();

    // Fusion des dossiers évidemment redondants — aucun appel au modèle.
    for m in &merges {
        let mut ids = vec![m.survivor_id.clone()]; // ids[0] survit (cf. merge_nodes_on)
        ids.extend(m.absorbed_ids.iter().cloned());
        let id = format!("arch-merge-{n}");
        n += 1;
        write_pending_proposal(dir, &id, "merge", "", "", "", "", &ids)?;
        report.push_str(&format!(
            "→ fusionner {} dossier(s) redondant(s) — {}\n",
            ids.len(), m.reason
        ));
    }

    let total_groups = result.groups.len();
    let mut merge_cache = load_merge_decisions(dir);
    for (gi, group) in result.groups.iter().enumerate() {
        on_progress(gi + 1, total_groups);
        match &engine {
            Some(e) => match decide_group_cached(e, group, &graph, &mut merge_cache) {
                archivist::GroupOutcome::Merge(d) => {
                    let mut ids = vec![d.survivor_id.clone()];
                    ids.extend(d.dropped_ids.clone());
                    // Une fusion exige AU MOINS 2 ids : sans ce garde, une décision
                    // dégénérée (survivant sans doublon) produisait une proposition
                    // `merge` à 1 id, impossible à appliquer et rejouée en boucle
                    // (bug du 2026-07-28). On l'ignore proprement plutôt que de la créer.
                    if ids.len() >= 2 {
                        let id = format!("arch-merge-{n}");
                        n += 1;
                        write_pending_proposal(dir, &id, "merge", "", "", "", "", &ids)?;
                        report.push_str(&format!(
                            "→ fusionner « {} » ({} pages) — {}\n",
                            group.label, group.node_ids.len(), d.reason
                        ));
                    }
                }
                archivist::GroupOutcome::KeepSeparate { reason } => report.push_str(&format!(
                    "· « {} » ({} pages) : gardées séparées — {}\n",
                    group.label, group.node_ids.len(), reason
                )),
                archivist::GroupOutcome::ParseFailed { raw_excerpt } => report.push_str(&format!(
                    "! « {} » ({} pages) : réponse IA inexploitable — {:?}\n",
                    group.label, group.node_ids.len(), raw_excerpt
                )),
            },
            None => report.push_str(&format!(
                "· « {} » ({} pages) : IA locale indisponible, ignoré\n",
                group.label, group.node_ids.len()
            )),
        }
    }
    if let Ok(json) = serde_json::to_string(&merge_cache) {
        let _ = std::fs::write(merge_decisions_path(dir), json);
    }

    // ─── Couche entités : relie les documents à leurs sociétés/clients ──────────
    // Fait passer d'un arbre de dossiers à un vrai graphe : « Société Y » devient
    // un nœud-hub, chaque doc qui la cite y est relié par un `link` (pont visible
    // sur le canvas). Extraction Gemma par doc, CACHÉE (seuls les docs neufs/
    // modifiés repassent) — incrémental comme l'extraction du pipeline. Sort en
    // propositions (create entité + link), jamais d'écriture directe.
    //
    // DÉSACTIVÉE le 2026-07-28 : sur des données mixtes (transcripts Claude Code +
    // docs dev), Gemma ramasse surtout des noms d'OUTILS (claude, gemini, xano,
    // supabase, cursor, obsidian...) pris pour des « organisations » → 80 entités +
    // 301 liens = graphe illisible (retour Liam, captures à l'appui). À réactiver
    // seulement avec : (1) stoplist de noms tech/produits/IA, (2) extraction limitée
    // aux sources « document business » (pas les transcripts/notes dev), (3) rendu
    // des liens borné au nœud sélectionné (sinon hairball quel que soit le nombre).
    const ENTITY_LAYER_ENABLED: bool = false;
    if ENTITY_LAYER_ENABLED {
    if let Some(e) = &engine {
        let mut cache = load_entity_cache(dir);
        // Flush périodique : sans ça une passe interrompue (app fermée) redoit
        // TOUTES les extractions Gemma de la passe au prochain lancement. On
        // persiste le cache tous les 10 docs extraits → une reprise ne reperd
        // au pire que ~10 extractions.
        let mut dirty = 0usize;
        // slug -> (libellé d'affichage, ids des docs qui la citent)
        let mut entity_docs: std::collections::HashMap<String, (String, Vec<String>)> = std::collections::HashMap::new();
        let docs: Vec<&BrainNode> = graph.nodes.iter().filter(|nd| nd.kind == "leaf").collect();
        let total_docs = docs.len();
        for (i, node) in docs.iter().enumerate() {
            on_progress(i + 1, total_docs);
            let sig = format!("{}:{}", node.updated_at.unwrap_or(0), node.source_text.len());
            let names = match cache.get(&node.id) {
                Some(c) if c.sig == sig => c.entities.clone(),
                _ => {
                    let names = archivist::extract_doc_entities(e, node);
                    cache.insert(node.id.clone(), EntityCacheEntry { sig, entities: names.clone() });
                    dirty += 1;
                    if dirty >= 10 { save_entity_cache(dir, &cache); dirty = 0; }
                    names
                }
            };
            for name in names {
                let slug = archivist::entity_slug(&name);
                if slug.is_empty() { continue; }
                let entry = entity_docs.entry(slug).or_insert_with(|| (name.clone(), Vec::new()));
                if !entry.1.contains(&node.id) { entry.1.push(node.id.clone()); }
            }
        }
        if dirty > 0 { save_entity_cache(dir, &cache); }

        let mut linked = 0usize;
        for (slug, (display, doc_ids)) in entity_docs {
            // Une entité citée par UN seul doc ne crée aucun pont utile → ignorée
            // (évite aussi une explosion de nœuds-entités à une feuille).
            if doc_ids.len() < 2 { continue; }
            let entity_id = format!("arch-entity-{slug}");
            if !graph.nodes.iter().any(|c| c.id == entity_id) {
                if let Some(root_id) = &root_id {
                    write_pending_proposal(dir, &entity_id, "create", root_id, &display, "", "", &[])?;
                }
            }
            for doc_id in &doc_ids {
                // Lien déjà présent → ne pas re-proposer (idempotent entre passes).
                let exists = graph.edges.iter().any(|ed| ed.kind == "link"
                    && ((ed.source == *doc_id && ed.target == entity_id) || (ed.source == entity_id && ed.target == *doc_id)));
                if exists { continue; }
                let id = format!("arch-link-{n}");
                n += 1;
                write_link_proposal(dir, &id, doc_id, &entity_id, "client")?;
            }
            linked += 1;
        }
        if linked > 0 {
            report.push_str(&format!("→ {linked} société(s)/client(s) reliée(s) à leurs documents (wikilinks)\n"));
        }
    }
    } // fin ENTITY_LAYER_ENABLED

    // Pas de bloc "orphelines laissées de côté" ici : `result.orphans_unresolved`
    // est désormais toujours routé vers le bac "Non triable" plus haut, dans le
    // même passage (cf. commentaire sur `orphans_unresolved_ids`) — le laisser
    // aurait rapporté deux fois la même chose sous deux formulations différentes.

    // Une passe trouée ne doit pas se lire comme une passe réussie. Les documents
    // des lots en échec ne sont ni taggés ni mis en cache (ils repasseront), mais
    // sans cette ligne l'utilisateur ne voit qu'un rangement incomplet sans cause
    // — c'est exactement ce qui a rendu la fossilisation « Autre » invisible
    // pendant des semaines. Placé AVANT la garde « rapport jamais vide » : une
    // passe dont le seul événement est un échec ne doit pas annoncer « rien à ranger ».
    let failed_calls = ai::llama::failed_calls().saturating_sub(failed_calls_before);
    if failed_calls > 0 || domain_failures > 0 {
        report.push_str(&format!("⚠️ {failed_calls} appel(s) IA en échec pendant cette passe"));
        if domain_failures > 0 {
            report.push_str(&format!(
                " — {domain_failures} lot(s) de classement laissé(s) sans domaine, à retenter au prochain scan"
            ));
        }
        report.push_str(".\n");
    }

    // Le rapport ne doit JAMAIS être silencieux : sans ça, "rien à faire" et
    // "je n'ai pas encore tourné" sont indiscernables pour l'utilisateur
    // (retour de Liam le 2026-07-23 : impression que l'Archiviste ne fait rien).
    if report.trim().is_empty() {
        report.push_str(match &engine {
            Some(_) => "Rien à ranger : ton cerveau est déjà bien organisé.",
            None => "Rien à ranger côté script — IA locale indisponible, aucune fusion ambiguë tranchée.",
        });
    }

    // Métriques persistées : le rapport de diagnostic lit le cerveau, pas une
    // passe — sans ce fichier ces chiffres seraient invisibles après coup, donc
    // impossibles à comparer d'une configuration à l'autre.
    metrics.duration_secs = pass_started.elapsed().as_secs();
    metrics.llm_calls = ai::llama::total_calls().saturating_sub(total_calls_before);
    if let Ok(json) = serde_json::to_string_pretty(&metrics) {
        let _ = std::fs::write(pass_metrics_path(dir), json);
    }

    let _ = std::fs::remove_file(archivist_marker_path(dir));
    Ok(report)
}

#[cfg(test)]
mod archivist_orchestration_tests {
    use super::*;

    fn note(id: &str, parent: &str, label: &str) -> BrainNode {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": label, "kind": "note", "weight": 1, "parent_id": parent
        }))
        .unwrap()
    }

    /// Le cœur de la qualité d'ancrage (2026-08-03) : un document presque aussi
    /// proche de deux dossiers ne doit PAS être rangé. C'est cette marge qui fait
    /// passer les ancrages de 32 bons / 10 fautifs à 23 bons / 1 fautif sur les
    /// données réelles de Liam.
    #[test]
    fn lancrage_refuse_un_document_ambigu_mais_accepte_un_document_net() {
        let t = ArchivistTuning::default();
        // Deux dossiers du même domaine, à 90° l'un de l'autre.
        let targets = vec![
            ("arch-theme-a".to_string(), vec![1.0, 0.0], Some("Facturation".to_string())),
            ("arch-theme-b".to_string(), vec![0.0, 1.0], Some("Facturation".to_string())),
        ];
        // Pile entre les deux : similarité identique aux deux → ambigu, refusé.
        assert_eq!(best_anchor(&[1.0, 1.0], Some("Facturation"), &targets, &t), None,
            "à égalité de similarité, on ne range pas");
        // Franchement aligné sur le premier : accepté.
        assert_eq!(best_anchor(&[1.0, 0.02], Some("Facturation"), &targets, &t).map(|(f, _)| f).as_deref(),
            Some("arch-theme-a"));
        // Proche du premier mais sous le seuil absolu (≈0.83 < 0.88) : refusé.
        assert_eq!(best_anchor(&[1.0, 0.68], Some("Facturation"), &targets, &t), None,
            "sous le seuil absolu, la marge ne sauve rien");
        // Domaine différent : la garde coupe avant tout calcul.
        assert_eq!(best_anchor(&[1.0, 0.02], Some("Santé"), &targets, &t), None);
        // Un seul dossier dans ce domaine → aucune ambiguïté, la marge ne s'applique
        // pas (sinon un domaine à dossier unique n'ancrerait jamais rien).
        let solo = vec![("arch-theme-c".to_string(), vec![1.0, 0.0], Some("Santé".to_string()))];
        assert_eq!(best_anchor(&[1.0, 0.02], Some("Santé"), &solo, &t).map(|(f, _)| f).as_deref(),
            Some("arch-theme-c"));
    }

    /// Les flags existent pour MESURER, pas pour changer le comportement : par
    /// défaut, et sur un dossier sans fichier de réglages, la passe doit se
    /// comporter exactement comme avant leur introduction (gardes actives, seuil
    /// d'ancrage 0,84). Et un fichier PARTIEL ne doit jamais désactiver une garde
    /// par omission — c'est le piège le plus facile à créer ici.
    #[test]
    fn les_reglages_par_defaut_reproduisent_le_comportement_dorigine() {
        let d = ArchivistTuning::default();
        assert!(d.domain_guard_anchor, "garde d'ancrage active par défaut");
        assert!(d.domain_guard_cluster, "garde de clustering active par défaut");
        assert_eq!(d.anchor_sim_threshold, ANCHOR_SIM_THRESHOLD);
        assert_eq!(d.anchor_sim_threshold, 0.88, "seuil relevé le 2026-08-03 après mesure");
        assert_eq!(d.anchor_min_margin, 0.02, "marge introduite le 2026-08-03");
        // Seule exception au « défaut = comportement d'origine » : le regroupement
        // en dossiers parents est volontairement COUPÉ (décision Liam 2026-08-03),
        // les dossiers restent à plat.
        assert!(!d.taxonomy_grouping, "regroupement en parents coupé par défaut");

        let dir = std::env::temp_dir().join("lucid_test_tuning");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Dossier vierge → défauts, ET le fichier est créé pour être éditable.
        assert_eq!(load_tuning(&dir), d);
        assert!(tuning_path(&dir).exists(), "le fichier de réglages est créé pour pouvoir l'éditer");

        // Fichier partiel : seul le seuil est donné, les deux gardes restent ACTIVES.
        std::fs::write(tuning_path(&dir), r#"{"anchor_sim_threshold":0.7}"#).unwrap();
        let partial = load_tuning(&dir);
        assert!(partial.domain_guard_anchor && partial.domain_guard_cluster,
            "un réglage omis ne doit jamais désactiver une garde");
        assert_eq!(partial.anchor_sim_threshold, 0.7);

        // Fichier illisible → défauts, jamais un plantage de passe.
        std::fs::write(tuning_path(&dir), "pas du json").unwrap();
        assert_eq!(load_tuning(&dir), d);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Récupération des « Autre » fossilisés (2026-08-02) : un lot en échec les
    /// gravait en cache sous la signature courante des docs, qui n'étaient plus
    /// jamais reclassés. La purge doit retirer les « Autre » SANS toucher aux
    /// vrais domaines, et ne jamais retourner une seconde fois (sinon tout
    /// « Autre » légitime serait reclassé à chaque passe, pour rien).
    #[test]
    fn la_purge_des_autre_fossiles_ne_passe_quune_fois_et_epargne_les_vrais_domaines() {
        let dir = std::env::temp_dir().join("lucid_test_purge_autre");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let entry = |domain: &str| DomainEntry { sig: "sig".into(), domain: domain.into() };
        let cache: std::collections::HashMap<String, DomainEntry> = [
            ("doc-1".to_string(), entry("Autre")),
            ("doc-2".to_string(), entry("Facturation")),
            ("doc-3".to_string(), entry("Autre")),
        ].into_iter().collect();
        std::fs::write(domain_cache_path(&dir), serde_json::to_string(&cache).unwrap()).unwrap();

        purge_fossil_autre_once(&dir);
        let after: std::collections::HashMap<String, DomainEntry> =
            serde_json::from_str(&std::fs::read_to_string(domain_cache_path(&dir)).unwrap()).unwrap();
        assert_eq!(after.len(), 1, "les deux « Autre » doivent être partis");
        assert_eq!(after.get("doc-2").map(|d| d.domain.as_str()), Some("Facturation"),
            "un domaine réel n'est jamais purgé");

        // Deuxième passe : un « Autre » légitime réapparu doit SURVIVRE.
        let mut relu = after;
        relu.insert("doc-4".to_string(), entry("Autre"));
        std::fs::write(domain_cache_path(&dir), serde_json::to_string(&relu).unwrap()).unwrap();
        purge_fossil_autre_once(&dir);
        let final_cache: std::collections::HashMap<String, DomainEntry> =
            serde_json::from_str(&std::fs::read_to_string(domain_cache_path(&dir)).unwrap()).unwrap();
        assert!(final_cache.contains_key("doc-4"), "purge non rejouée : marqueur en place");
        assert_eq!(final_cache.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Régression du 2026-07-31 : Gemma renommait le même groupe à chaque passe,
    /// et comme l'id du dossier vient du nom, ça créait un dossier jumeau au lieu
    /// de réutiliser l'existant. Le nom doit tenir tant que le groupe est
    /// reconnaissable, et changer seulement quand c'est un autre groupe.
    #[test]
    fn le_nom_dun_theme_tient_entre_deux_passes_meme_si_le_groupe_bouge_un_peu() {
        let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut cache = Vec::new();
        let groupe = ids(&["a", "b", "c", "d", "e"]);
        remember_cluster_name(&mut cache, &groupe, "Factures Parabola");

        assert_eq!(recall_cluster_name(&cache, &groupe).as_deref(), Some("Factures Parabola"),
            "groupe identique → même nom, aucun appel Gemma");
        // Une facture de plus, une retirée : toujours le même dossier.
        assert_eq!(recall_cluster_name(&cache, &ids(&["a", "b", "c", "d", "f"])).as_deref(),
            Some("Factures Parabola"), "le groupe a bougé d'un document, pas d'identité");
        // Groupe sans rapport : il doit être nommé pour lui-même.
        assert_eq!(recall_cluster_name(&cache, &ids(&["x", "y", "z"])), None,
            "un autre groupe ne doit jamais hériter d'un nom existant");

        // Le groupe suit ses documents : après renommage de la composition, un
        // ancien membre isolé ne rappelle plus le nom.
        remember_cluster_name(&mut cache, &ids(&["a", "b", "c", "d", "f"]), "Factures Parabola");
        assert_eq!(cache.len(), 1, "un nom = une seule entrée, pas d'accumulation");
    }

    /// Régression du 2026-07-31 : `embed_sig` keyé sur `updated_at` invalidait
    /// TOUT le cache à chaque passe, parce que l'Archiviste déplace les docs
    /// qu'il range (nouveau parent → nouvelle estampille). Résultat : ~230
    /// embeddings + ~230 tags Gemma refaits à chaque run, ~6 min de GPU pour
    /// une seule note neuve. La signature doit suivre le TEXTE, rien d'autre.
    #[test]
    fn la_signature_dembedding_survit_a_un_deplacement_mais_pas_a_un_changement_de_texte() {
        let mut n = note("doc-1", "arch-non-triable", "Devis toiture");
        n.source_text = "Devis pour la réfection de la toiture.".into();
        let before = embed_sig(&n);

        // L'Archiviste le range ailleurs et le graphe le ré-estampille.
        n.parent_id = Some("arch-theme-travaux".into());
        n.updated_at = Some(n.updated_at.unwrap_or(0) + 9_999);
        assert_eq!(embed_sig(&n), before, "un déplacement ne doit pas invalider le vecteur");

        // Le contenu change réellement → le vecteur doit être refait.
        n.source_text.push_str(" Montant révisé : 12 000 €.");
        assert_ne!(embed_sig(&n), before, "un texte modifié doit invalider le vecteur");
    }

    // NOTE (2026-07-29, ADR-0019) : les anciens tests du clustering thématique
    // MÉCANIQUE (par mot partagé) ont été retirés — ce rangement est désormais fait
    // par EMBEDDINGS (voir `archivist::cluster_indices`, testé unitairement sans
    // moteur). Les tests E2E d'orchestration ne peuvent plus vérifier le
    // regroupement sans lancer un vrai serveur d'embedding, donc on ne garde ici
    // que le contrat testable SANS moteur : un document à ranger n'est jamais perdu
    // → il finit dans le bac « Non triable » (repli), cf. le test ci-dessous.

    /// Régression du bug remonté par Liam le 2026-07-23 : après un reset +
    /// premier scan (donc AUCUN bac "Non triable" encore présent), un seul
    /// passage de l'Archiviste créait le bac mais n'y rangeait rien — "un
    /// prochain passage" qui, en pratique, n'arrivait jamais tout seul. Perçu
    /// comme "l'Archiviste n'a rien fait". Le bac doit maintenant être créé ET
    /// peuplé dans le même passage.
    #[test]
    fn cree_le_bac_non_triable_et_y_range_les_orphelins_des_le_premier_passage() {
        let dir = std::env::temp_dir().join("lucid_test_archivist_catchall_same_pass");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let graph = BrainGraph {
            nodes: vec![
                serde_json::from_value(serde_json::json!({
                    "id": "root", "label": "Lucid", "kind": "root", "weight": 3
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "id": "projet-a", "label": "Projet Alpha", "kind": "container",
                    "weight": 2, "parent_id": "root"
                }))
                .unwrap(),
                // Orphelin direct sous la racine, sans candidat thématique
                // plausible — donc pas de bac "Non triable" présent nulle part.
                note("orphelin", "root", "Facture Ünique 42"),
            ],
            edges: vec![],
            markdown: String::new(),
            report: String::new(),
            generated_at: "t".into(),
        };
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&graph).unwrap()).unwrap();

        let report = run_archivist_scan_once_in(&dir).unwrap();
        assert!(!report.trim().is_empty());

        let proposals = load_proposals_in(&dir);
        assert!(
            proposals.iter().any(|p| p.action == "create" && p.id == archivist::CATCHALL_ID),
            "le bac « Non triable » doit être proposé à la création : {:?}",
            proposals.iter().map(|p| (&p.action, &p.id)).collect::<Vec<_>>()
        );
        assert!(
            proposals.iter().any(|p| p.action == "move"
                && p.target_id == "orphelin"
                && p.new_parent_id == archivist::CATCHALL_ID),
            "l'orphelin doit être rangé dans le bac DANS LE MÊME PASSAGE, pas laissé pour un suivant : {:?}",
            proposals.iter().map(|p| (&p.action, &p.target_id, &p.new_parent_id)).collect::<Vec<_>>()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

}

/// Une seule passe de l'Archiviste à la fois (même logique que `GEN_LOCK` pour
/// la génération) — évite deux déclencheurs concurrents (fin de scan + retour
/// MCP + bouton manuel) de se marcher dessus sur `mcp_pending/`.
static ARCHIVIST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Commande Tauri de l'Archiviste — déclenchée en fin de scan machine, après
/// qu'une IA (MCP) ait écrit dans le cerveau, ou manuellement depuis les
/// Réglages. Dépose ses propositions dans `mcp_pending/` comme le ferait un
/// MCP distant : le watcher existant (`start_mcp_pending_watcher`) prévient le
/// front, qui les résout avec le même circuit (auto si autonome, bulles sinon)
/// — aucune plomberie supplémentaire nécessaire pour l'appliquer.
#[tauri::command]
async fn run_archivist(app: tauri::AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _lock = ARCHIVIST_LOCK.try_lock().map_err(|_| "L'Archiviste tourne déjà.".to_string())?;
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        run_archivist_scan_once_in_progress(&dir, |current, total| {
            let _ = app.emit("archivist-progress", serde_json::json!({
                "current": current, "total": total,
            }));
        })
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Agrège toutes les sources connues.
fn load_all_conversations() -> Vec<Conversation> {
    let mut convs = connectors::claude_code::load_all_conversations();
    convs.extend(connectors::cowork::load_all_conversations());
    convs.extend(connectors::claude_ai::load_conversations());
    convs.extend(connectors::chatgpt::load_conversations());
    convs.extend(connectors::google_drive::load_conversations());
    convs.extend(connectors::obsidian::load_all_conversations());
    convs.extend(connectors::local_folder::load_conversations());
    convs.extend(connectors::apple_notes::load_conversations());
    convs
}

/// Indique si le modèle actif doit être téléchargé (absent du disque).
#[tauri::command]
fn ai_setup_needed() -> bool {
    ai::llama::model_needs_download()
}

/// Télécharge le modèle actif depuis le CDN (public, sans auth).
/// Émet des événements "download-progress" { downloaded_mb, total_mb, percent }.
#[tauri::command]
async fn download_model(app: tauri::AppHandle, model_id: String) -> Result<(), String> {
    ai::llama::select_model(&model_id)?;
    tauri::async_runtime::spawn_blocking(move || ai::llama::download_model(&app))
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Bootstrap IA au 1er lancement : télécharge AUTOMATIQUEMENT le modèle de
/// génération recommandé (chaîne de secours par RAM décroissante si un DL
/// échoue) puis le modèle d'embedding — zéro choix utilisateur (le choix manuel
/// reste dans les Réglages). Émet "bootstrap-step" { step, total, label } avant
/// chaque phase ; la barre de progression suit "download-progress".
#[tauri::command]
async fn ai_bootstrap(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || bootstrap_blocking(&app))
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

fn bootstrap_blocking(app: &tauri::AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    let catalog = ai::llama::load_catalog();
    let chain = ai::llama::recommended_chain(&catalog);
    if chain.is_empty() {
        return Err("Catalogue de modèles indisponible (connexion requise au 1er lancement).".into());
    }
    let name_of = |id: &str| catalog.iter().find(|m| m.id == id).map(|m| m.name.clone()).unwrap_or_else(|| id.to_string());

    // 1. Modèle de génération : on descend la chaîne jusqu'à un DL réussi.
    let mut gen_ok = false;
    let mut last_err = String::new();
    for id in &chain {
        let _ = app.emit("bootstrap-step", serde_json::json!({
            "step": 1, "total": 2, "label": format!("Modèle IA — {}", name_of(id)),
        }));
        if let Err(e) = ai::llama::select_model(id) { last_err = e; continue; }
        match ai::llama::download_model(app) {
            Ok(()) => { gen_ok = true; break; }
            Err(e) => { crate::elog!("⬇️ bootstrap: échec DL {id} ({e}), essai suivant."); last_err = e; }
        }
    }
    if !gen_ok {
        return Err(format!("Aucun modèle IA n'a pu être téléchargé : {last_err}"));
    }

    // 2. Modèle d'embedding : best-effort. Un échec ici NE bloque PAS l'app —
    //    l'Archiviste retombe sur le chemin Gemma (parité), le Diagnostic le signale.
    let _ = app.emit("bootstrap-step", serde_json::json!({
        "step": 2, "total": 2, "label": "Moteur de rangement (embeddings)",
    }));
    if let Err(e) = ai::llama::download_embed_model(app) {
        crate::elog!("⬇️ bootstrap: modèle d'embedding non téléchargé ({e}) — rangement en mode Gemma.");
    }
    Ok(())
}

/// Fallback : installe un fichier .gguf local déjà téléchargé.
/// Émet des événements "install-progress" (u8 %).
#[tauri::command]
async fn install_model_file(
    app: tauri::AppHandle,
    src_path: String,
    model_id: String,
) -> Result<(), String> {
    ai::llama::select_model(&model_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        ai::llama::install_from_path(&app, std::path::Path::new(&src_path))
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

#[derive(serde::Serialize)]
struct ModelInfo {
    id: String,
    name: String,
    file: String,
    size_gb: f32,
    min_ram_gb: f32,
    downloaded: bool,
    active: bool,
    recommended: bool,
}

/// Liste les modèles disponibles avec leur statut (téléchargé, actif, recommandé).
/// Async : load_catalog peut partir en réseau (15 s de timeout) et la lecture RAM
/// spawne powershell sous Windows — jamais sur le thread UI (ça gelait l'app).
#[tauri::command]
async fn list_models() -> Vec<ModelInfo> {
    tauri::async_runtime::spawn_blocking(list_models_blocking).await.unwrap_or_default()
}

fn list_models_blocking() -> Vec<ModelInfo> {
    let catalog = ai::llama::load_catalog();
    let active_id = ai::llama::active_model_stored().map(|m| m.id);
    let recommended_id = ai::llama::recommended_id(&catalog);
    catalog.into_iter().map(|m| {
        let downloaded = ai::llama::shared_data_dir()
            .map(|d| d.join("models").join(&m.file).is_file())
            .unwrap_or(false);
        let active = active_id.as_deref() == Some(m.id.as_str());
        let recommended = recommended_id.as_deref() == Some(m.id.as_str());
        ModelInfo {
            id: m.id,
            name: m.name,
            file: m.file,
            size_gb: m.size_gb,
            min_ram_gb: m.min_ram_gb,
            downloaded,
            active,
            recommended,
        }
    }).collect()
}

/// Change le modèle actif (persiste dans model_config.json avec l'URL).
/// Si le nouveau modèle n'est pas téléchargé, ai_setup_needed() retournera true.
#[tauri::command]
fn set_active_model(id: String) -> Result<(), String> {
    ai::llama::select_model(&id)
}

/// Liste toutes les conversations détectées (toutes sources confondues).
#[tauri::command]
fn list_conversations() -> Vec<ConversationSummary> {
    let mut all = connectors::claude_code::list_conversations();
    all.extend(connectors::claude_ai::load_conversations().into_iter().map(|c| c.summary));
    all.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));
    all
}

/// Charge le contenu complet d'une conversation.
/// `source` prend le dessus sur `project_slug` pour router vers le bon connecteur
/// (ex. : un PDF Drive a project_slug="cours-medecine", source="google-drive").
#[tauri::command]
fn load_conversation(project_slug: String, id: String, source: Option<String>) -> Option<Conversation> {
    match source.as_deref() {
        Some("cowork")       => connectors::cowork::load_by_id(&id),
        Some("claude-ai")    => connectors::claude_ai::load_by_id(&id),
        Some("chatgpt")      => connectors::chatgpt::load_by_id(&id),
        Some("google-drive") => connectors::google_drive::load_by_id(&id),
        Some("obsidian")     => connectors::obsidian::load_by_id(&id),
        Some("local-folder") => connectors::local_folder::load_by_id(&id),
        Some("apple-notes")  => connectors::apple_notes::load_by_id(&id),
        _                    => connectors::claude_code::load_conversation(&project_slug, &id),
    }
}

/// Indique si le connecteur Claude Code est disponible (dossier présent).
#[tauri::command]
fn claude_code_available() -> bool {
    connectors::claude_code::projects_dir().is_some()
}

/// Déconnecte Claude Code (flag local, réversible).
#[tauri::command]
fn claude_code_disconnect() {
    connectors::claude_code::disconnect();
}

/// Reconnecte Claude Code (supprime le flag).
#[tauri::command]
fn claude_code_reconnect() {
    connectors::claude_code::reconnect();
}

/// Indique si l'IA locale est prête (binaire llama.cpp + modèle présents).
#[tauri::command]
fn ai_ready() -> bool {
    LlamaEngine::is_ready()
}

/// Statut des connecteurs (actifs + dernière synchro).
#[tauri::command]
fn connectors_status() -> Vec<ConnectorStatus> {
    let cc_convs = connectors::claude_code::list_conversations();
    let cc_sync = cc_convs.first().and_then(|c| c.last_timestamp.clone());

    let ai_convs = connectors::claude_ai::load_conversations();
    let ai_connected = !ai_convs.is_empty();
    let ai_sync = ai_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max();

    let mut list = vec![
        ConnectorStatus {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            connected: connectors::claude_code::projects_dir().is_some(),
            last_sync: cc_sync,
            conversation_count: cc_convs.len(),
            needs_setup: false,
        },
        {
            // Cowork (Claude Desktop, mode agent local) : détecté tout seul comme Claude
            // Code — le dossier existe ou n'existe pas, il n'y a rien à connecter.
            let cw_convs = connectors::cowork::list_conversations();
            let cw_sync = cw_convs.iter().filter_map(|c| c.last_timestamp.clone()).max();
            // Le dossier existe mais aucune session lisible : le cas mérite une trace,
            // c'est le symptôme d'un changement de format côté Anthropic. On ne journalise
            // pas le cas nominal, `connectors_status` est appelé très souvent.
            if cw_convs.is_empty() && connectors::cowork::sessions_dir().is_some() {
                crate::elog!("🔎 cowork: dossier présent mais 0 session lisible");
            }
            ConnectorStatus {
                id: "cowork".into(),
                name: "Cowork".into(),
                connected: connectors::cowork::sessions_dir().is_some(),
                last_sync: cw_sync,
                conversation_count: cw_convs.len(),
                needs_setup: false,
            }
        },
        {
            let gd_convs = connectors::google_drive::load_conversations();
            let gd_sync = gd_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max();
            ConnectorStatus {
                id: "google-drive".into(),
                name: "Google Drive".into(),
                connected: connectors::google_drive::is_connected(),
                last_sync: gd_sync,
                conversation_count: gd_convs.len(),
                needs_setup: false,
            }
        },
        ConnectorStatus {
            id: "obsidian".into(),
            name: "Obsidian".into(),
            connected: connectors::obsidian::is_connected(),
            last_sync: None,
            conversation_count: connectors::obsidian::count_files(),
            needs_setup: !connectors::obsidian::is_connected(),
        },
        {
            let lf_convs = connectors::local_folder::load_conversations();
            let lf_sync = lf_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max();
            ConnectorStatus {
                id: "local-folder".into(),
                name: "Dossiers locaux".into(),
                connected: connectors::local_folder::is_connected(),
                last_sync: lf_sync,
                conversation_count: lf_convs.len(),
                needs_setup: !connectors::local_folder::is_connected(),
            }
        },
    ];

    // claude.ai et ChatGPT en STAND-BY comme sources (décision 2026-08-06) : ni OpenAI ni
    // Anthropic n'expose d'API pour lire l'historique d'un compte, et le seul chemin —
    // l'export ZIP à redemander manuellement à chaque mise à jour — sert l'ancien
    // positionnement « agrégateur de conversations IA ». ChatGPT reste dans le produit,
    // mais comme CLIENT du cerveau via le MCP. Le code des connecteurs reste en place :
    // si une API s'ouvre, il suffit de retirer la condition.
    //
    // La carte reste affichée à qui a DÉJÀ importé : masquer les cartes ferait disparaître
    // ses conversations de l'écran Sources sans un mot, alors qu'elles sont toujours dans
    // son cerveau — exactement l'échec silencieux qu'interdit l'ADR-0015.
    if ai_connected {
        list.push(ConnectorStatus {
            id: "claude-ai".into(),
            name: "Claude".into(),
            connected: true,
            last_sync: ai_sync,
            conversation_count: ai_convs.len(),
            needs_setup: false,
        });
    }
    let cg_convs = connectors::chatgpt::load_conversations();
    if !cg_convs.is_empty() {
        list.push(ConnectorStatus {
            id: "chatgpt".into(),
            name: "ChatGPT".into(),
            connected: true,
            last_sync: cg_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max(),
            conversation_count: cg_convs.len(),
            needs_setup: false,
        });
    }

    // Mac uniquement : Notes.app n'existe pas sur Windows, la carte n'a rien à
    // faire dans la grille là-bas plutôt que d'afficher un connecteur inerte.
    if connectors::apple_notes::available() {
        let an_convs = connectors::apple_notes::load_conversations();
        let an_sync = an_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max();
        list.push(ConnectorStatus {
            id: "apple-notes".into(),
            name: "Notes Apple".into(),
            connected: connectors::apple_notes::is_connected(),
            last_sync: an_sync,
            conversation_count: an_convs.len(),
            needs_setup: !connectors::apple_notes::is_connected(),
        });
    }

    list
}

/// Lance le flux OAuth loopback Google. Ouvre le navigateur, attend le redirect.
#[tauri::command]
async fn google_drive_connect(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (listener, auth_url, redirect_uri, verifier) =
            connectors::google_drive::prepare_connect()?;

        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(&auth_url, None::<&str>)
            .map_err(|e| format!("Impossible d'ouvrir le navigateur : {e}"))?;

        connectors::google_drive::finish_connect(listener, &redirect_uri, &verifier)
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Synchronise les fichiers Drive vers le cache local. Renvoie (nouveaux, total).
#[tauri::command]
async fn google_drive_sync() -> Result<(usize, usize), String> {
    tauri::async_runtime::spawn_blocking(connectors::google_drive::sync_docs)
        .await
        .map_err(|e| format!("Sync interrompue : {e}"))?
}

/// Enregistre les credentials OAuth Google (client_id + client_secret) dans l'app data.
#[tauri::command]
fn google_drive_save_credentials(client_id: String, client_secret: String) -> Result<(), String> {
    connectors::google_drive::save_creds(&client_id, &client_secret)
}

/// Vérifie si les credentials OAuth Google sont déjà configurés (fichier ou compile-time).
#[tauri::command]
fn google_drive_has_credentials() -> bool {
    connectors::google_drive::has_credentials()
}

/// Déconnecte Google Drive (supprime tokens + cache local).
#[tauri::command]
fn google_drive_disconnect() {
    connectors::google_drive::disconnect();
}

/// Arbre des dossiers Drive pour l'écran de sélection. Ne liste QUE les dossiers
/// (quelques secondes même sur un Drive de 1 To).
#[tauri::command]
async fn google_drive_folders() -> Result<Vec<connectors::google_drive::DriveFolder>, String> {
    tauri::async_runtime::spawn_blocking(connectors::google_drive::list_folders)
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Nombre de documents ingérables par dossier (clé "" = fichiers sans dossier).
/// Demande l'énumération complète → appelé en arrière-plan, jamais bloquant.
#[tauri::command]
async fn google_drive_folder_counts() -> Result<std::collections::HashMap<String, usize>, String> {
    tauri::async_runtime::spawn_blocking(connectors::google_drive::folder_doc_counts)
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Sélection actuelle. `folders` vide = tout le Drive.
#[tauri::command]
fn google_drive_selection() -> connectors::google_drive::Selection {
    connectors::google_drive::selection()
}

#[tauri::command]
fn google_drive_set_selection(folders: Vec<String>, include_orphans: bool) -> Result<(), String> {
    connectors::google_drive::set_selection(folders, include_orphans)
}

/// Importe un export ZIP claude.ai. Renvoie le nombre de conversations importées.
#[tauri::command]
fn import_claude_ai(path: String) -> Result<usize, String> {
    let convs = connectors::claude_ai::parse_zip(std::path::Path::new(&path))?;
    let count = convs.len();
    if count == 0 {
        return Err("Aucune conversation trouvée dans ce fichier.".to_string());
    }
    connectors::claude_ai::save_conversations(&convs)?;
    Ok(count)
}

/// Importe un export ZIP ChatGPT. Renvoie le nombre de conversations importées.
#[tauri::command]
fn import_chatgpt(path: String) -> Result<usize, String> {
    let convs = connectors::chatgpt::parse_zip(std::path::Path::new(&path))?;
    let count = convs.len();
    if count == 0 {
        return Err("Aucune conversation trouvée dans ce fichier.".to_string());
    }
    connectors::chatgpt::save_conversations(&convs)?;
    Ok(count)
}

/// Modèle IA actif + taille de sa fenêtre de contexte (pour l'UI des assistants).
#[derive(serde::Serialize)]
struct AiInfo {
    model: String,
    context_tokens: u32,
}

#[tauri::command]
fn ai_info() -> AiInfo {
    AiInfo {
        model: ai::llama::active_model_stored().map(|m| m.name).unwrap_or_else(|| "—".into()),
        context_tokens: ai::llama::CONTEXT_TOKENS,
    }
}

/// Diagnostic de la stack IA pour le retour bêta : quels binaires/modèles sont
/// présents + la fin de `lucid.log` (démarrages/échecs `llama-server`, bootstrap,
/// embeddings). RGPD-safe : les logs sont des COMPTEURS et messages d'infra, sans
/// contenu de document (les libellés ne sont jamais loggés, cf. sites `elog!`).
#[derive(serde::Serialize)]
struct AiDiagnostics {
    os: String,
    total_ram_gb: f32,
    completion_binary: bool,
    server_binary: bool,
    gen_model: Option<String>,
    gen_model_present: bool,
    embed_model_present: bool,
    log_tail: String,
    /// stderr du dernier démarrage du serveur de génération (raison d'un crash).
    gen_server_log: String,
    /// stderr du dernier démarrage du serveur d'embedding.
    embed_server_log: String,
}

/// Dernières `n` lignes d'un fichier du dossier machine (vide si absent).
fn tail_data_file(name: &str, n: usize) -> String {
    ai::llama::shared_data_dir()
        .map(|d| d.join(name))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| {
            let lines: Vec<&str> = s.lines().collect();
            lines[lines.len().saturating_sub(n)..].join("\n")
        })
        .unwrap_or_default()
}

#[tauri::command]
fn ai_diagnostics() -> AiDiagnostics {
    AiDiagnostics {
        os: std::env::consts::OS.to_string(),
        total_ram_gb: ai::llama::detected_ram_gb(),
        completion_binary: ai::llama::completion_binary_available(),
        server_binary: ai::llama::server_binary_available(),
        gen_model: ai::llama::active_model_stored().map(|m| m.name),
        gen_model_present: ai::llama::generation_model_available(),
        embed_model_present: ai::llama::embed_model_available(),
        log_tail: tail_data_file("lucid.log", 60),
        gen_server_log: tail_data_file(ai::llama::GEN_SERVER_LOG, 40),
        embed_server_log: tail_data_file(ai::llama::EMBED_SERVER_LOG, 40),
    }
}

/// Contexte BORNÉ pour ask_brain : aperçu compact (report) + pages les plus
/// pertinentes à la question (récupération par mots-clés). Injecter tout brain.md
/// dépasse la fenêtre du modèle (8192 tokens) → llama plante. Ici on borne à
/// `budget` octets, quelle que soit la taille du cerveau.
/// Minuscules + accents pliés, sans dépendance : « geographique » doit trouver
/// « géographique ». Table volontairement limitée au français (parité Windows :
/// aucun binaire, aucun crate en plus).
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            'ñ' => 'n',
            'ÿ' | 'ý' => 'y',
            other => other,
        })
        .collect()
}

/// Passages de `text` contenant les mots de la question, dans l'ordre du document,
/// sous `budget` caractères. `None` si aucun mot n'apparaît.
///
/// Miroir Rust de `relevantPassages` (MCP) et même raison d'être : injecter le
/// RÉSUMÉ d'une page ne fait jamais arriver un montant, une date ou une référence
/// jusqu'au modèle — l'assistant local répondait « ce n'est pas visible » sur des
/// informations bel et bien présentes dans le document (constaté le 2026-08-05).
fn relevant_passage(text: &str, terms: &[String], budget: usize) -> Option<String> {
    const BLOCK: usize = 250; // fenêtre courte : jusqu'à 40 pages partagent 16 000 car.
    // Plafond de balayage : le cerveau contient des documents de 3,2 MILLIONS de
    // caractères (exports CSV). Découper 40 pages de cette taille à chaque question
    // se paie cher, surtout en build debug (`tauri dev`), et une réponse qui vit
    // au-delà de 100 k caractères est rare. Le SCORE, lui, continue de voir tout le
    // document : on ne perd que l'extraction du passage, qui se rabat sur le résumé.
    const SCAN_CAP: usize = 100_000;
    if terms.is_empty() || text.trim().is_empty() { return None; }
    let text: &str = match text.char_indices().nth(SCAN_CAP) {
        Some((byte, _)) => &text[..byte],
        None => text,
    };

    // Blocs d'environ BLOCK caractères, coupés aux sauts de ligne quand c'est
    // possible ; une ligne géante (export CSV) est coupée en dur, sinon un tableur
    // entier ne formerait qu'un seul bloc.
    let mut blocks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for line in text.lines() {
        let chars: Vec<char> = line.chars().collect();
        for piece in chars.chunks(BLOCK) {
            let piece: String = piece.iter().collect();
            if !cur.is_empty() && cur.chars().count() + piece.chars().count() > BLOCK {
                blocks.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() { cur.push(' '); }
            cur.push_str(&piece);
        }
    }
    if !cur.trim().is_empty() { blocks.push(cur); }

    let mut scored: Vec<(usize, usize, &String)> = blocks.iter().enumerate()
        .map(|(i, b)| {
            let f = fold(b);
            (terms.iter().map(|t| f.matches(t.as_str()).count()).sum::<usize>(), i, b)
        })
        .filter(|(s, _, _)| *s > 0)
        .collect();
    if scored.is_empty() { return None; }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));

    let mut kept: Vec<(usize, &String)> = Vec::new();
    let mut used = 0usize;
    for (_, i, b) in scored {
        let n = b.chars().count();
        if !kept.is_empty() && used + n > budget { break; }
        kept.push((i, b));
        used += n;
    }
    kept.sort_by_key(|(i, _)| *i);

    let mut out = String::new();
    let mut prev: Option<usize> = None;
    for (i, b) in kept {
        if let Some(p) = prev { out.push_str(if i == p + 1 { " " } else { " […] " }); }
        out.push_str(b.trim());
        prev = Some(i);
    }
    Some(out)
}

fn ask_context(graph: &BrainGraph, question: &str, report: &str, semantic: &[String]) -> String {
    const BUDGET: usize = 16_000; // ~4000 tokens, laisse la place au report + réponse
    const DEEP: usize = 8; // pages qui reçoivent un passage complet
    // 3 places, et pas plus : à 5, les candidats vectoriels poussaient la queue du
    // classement mots-clés hors de la fenêtre de 40 et le score retombait de 10/12
    // à 9/12 (mesuré le 2026-08-05).
    const SEM_SLOTS: usize = 3;
    let terms: Vec<String> = {
        let mut v: Vec<String> = fold(question)
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.chars().count() >= 2)
            .map(str::to_string)
            .collect();
        v.sort();
        v.dedup();
        v
    };

    let label_of: std::collections::HashMap<&str, &str> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str())).collect();

    // Nombre d'enfants par nœud : sans lui, le modèle COMPTE CE QU'IL VOIT. Sur
    // « combien de factures Xano ai-je ? », Gemma a répondu « trois » (les trois
    // injectées) alors que le dossier en contient 16 — le montant était juste, le
    // compte faux, et rien dans le contexte ne pouvait le lui dire (2026-08-05).
    // `brain_overview` donne cette information côté MCP ; l'injection locale non.
    let child_count = graph.nodes.iter().fold(
        std::collections::HashMap::<&str, usize>::new(),
        |mut m, n| {
            if let Some(p) = n.parent_id.as_deref() { *m.entry(p).or_insert(0) += 1; }
            m
        },
    );

    // Champs pliés une fois pour toutes (deux passes : IDF, puis score).
    let docs: Vec<(&BrainNode, String, String, String, String)> = graph.nodes.iter()
        .filter(|n| n.kind != "root")
        .map(|n| (
            n,
            fold(&n.label),
            fold(&n.keywords.join(" ")),
            fold(&n.summary),
            fold(&format!("{} {}", n.content, n.source_text)),
        ))
        .collect();

    // Score BINAIRE par champ (titre 5 / mots-clés 3 / résumé 2 / corps 1), pondéré
    // par la RARETÉ du mot (IDF). Deux corrections mesurées le 2026-08-05 avec
    // `cargo run --example retrieval` :
    //  - compter les occurrences rendait le score proportionnel à la TAILLE du
    //    document (« sur » comptait 499 fois dans un export immobilier via
    //    « surface »/« assurance ») : les 4 plus gros documents gagnaient tout — 4/12 ;
    //  - sans IDF, un mot rare comme « vaccination » pesait autant que « document »,
    //    présent partout, et l'aiguille restait noyée dans les ex aequo — 9/12.
    // L'IDF rend aussi la liste de mots vides inutile : ils s'annulent d'eux-mêmes.
    let n_docs = docs.len().max(1) as f32;
    let idf: std::collections::HashMap<&str, f32> = terms.iter()
        .map(|t| {
            let df = docs.iter()
                .filter(|(_, ti, kw, su, bo)| ti.contains(t) || kw.contains(t) || su.contains(t) || bo.contains(t))
                .count();
            (t.as_str(), if df == 0 { 0.0 } else { (1.0 + n_docs / df as f32).ln() })
        })
        .collect();

    // ponytail: un bonus de « couverture » (favoriser les pages qui matchent
    // plusieurs mots de la question) a été essayé le 2026-08-05 et MESURÉ PIRE
    // — 8/12 contre 9/12. Ne pas le réintroduire sans le remesurer.
    let mut scored: Vec<(f32, &BrainNode)> = docs.iter()
        .map(|(n, title, kw, summary, body)| {
            let score: f32 = terms.iter().map(|t| {
                let w = idf.get(t.as_str()).copied().unwrap_or(0.0);
                if w == 0.0 { return 0.0; }
                let t = t.as_str();
                w * ((if title.contains(t) { 5.0 } else { 0.0 })
                    + (if kw.contains(t) { 3.0 } else { 0.0 })
                    + (if summary.contains(t) { 2.0 } else { 0.0 })
                    + (if body.contains(t) { 1.0 } else { 0.0 }))
            }).sum();
            (score, *n)
        })
        .filter(|(s, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.weight.cmp(&a.1.weight)));

    // Aucun match (question vague) → replie sur les nœuds les plus « lourds ».
    let mut selected: Vec<&BrainNode> = if scored.is_empty() {
        let mut v: Vec<&BrainNode> = graph.nodes.iter().filter(|n| n.kind != "root").collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight));
        v
    } else {
        scored.into_iter().map(|(_, n)| n).collect()
    };
    // HYBRIDE : les vecteurs ne font qu'AJOUTER des candidats que les mots-clés
    // n'ont pas vus, dans des places réservées de l'étage profond. Mesuré le
    // 2026-08-05 sur 12 questions : mots-clés 9/12, vecteurs 5/12, mais les deux
    // échouent sur des ensembles DISJOINTS (les vecteurs sont nuls sur les
    // identifiants — « Xano » en rang 173, « ADR-0019 » en rang 83 — et bons sur le
    // fossé de vocabulaire) ; l'union monte à 11/12.
    //
    // Les vecteurs ne décident JAMAIS d'une absence : la similarité du bruit pur
    // (0,690–0,747 sur les contrôles négatifs) dépasse celle des vraies réponses
    // (0,674–0,703), donc aucun seuil ne les sépare. Le « je ne trouve rien » reste
    // au classement par mots-clés, où il est gratuit et fiable.
    if !semantic.is_empty() && !selected.is_empty() {
        let known: std::collections::HashSet<&str> =
            selected.iter().take(40).map(|n| n.id.as_str()).collect();
        let fresh: Vec<&BrainNode> = semantic.iter()
            .filter(|id| !known.contains(id.as_str()))
            .filter_map(|id| graph.nodes.iter().find(|n| &n.id == id))
            .take(SEM_SLOTS)
            .collect();
        if !fresh.is_empty() {
            let cut = DEEP.saturating_sub(fresh.len()).min(selected.len());
            let tail = selected.split_off(cut);
            selected.extend(fresh);
            selected.extend(tail);
        }
    }
    // 48 et non 40 : le budget de caractères s'arrête de lui-même avant, donc ce
    // plafond n'est qu'une ceinture. À 40 pile, une page utile en queue de liste
    // (Q13, rang 40) sortait du contexte au moindre allongement des lignes.
    selected.truncate(48);

    // Injection en DEUX ÉTAGES sous le même budget : les premières pages du
    // classement portent un vrai passage (là où vivent les montants, dates et
    // références), la queue reste en une ligne pour garder de la largeur. Tout
    // donner à 500 caractères tombait à 28 pages injectées au lieu de 40 et
    // faisait sortir du contexte des pages utiles (mesuré le 2026-08-05).
    const PASSAGE: usize = 500;
    const SHALLOW: usize = 150;

    let mut ctx = format!("APERÇU :\n{report}\n\nPAGES PERTINENTES :\n");
    for (rank, n) in selected.iter().enumerate() {
        if ctx.len() > BUDGET { ctx.push_str("[…autres pages omises…]\n"); break; }
        let parent = n.parent_id.as_deref().and_then(|p| label_of.get(p)).copied().unwrap_or("");
        let budget = if rank < DEEP { PASSAGE } else { SHALLOW };
        let raw = if !n.content.trim().is_empty() { n.content.as_str() } else { n.source_text.as_str() };
        let body = relevant_passage(raw, &terms, budget).unwrap_or_else(|| {
            if !n.summary.trim().is_empty() {
                n.summary.trim().chars().take(budget).collect::<String>()
            } else {
                raw.chars().take(budget.min(300)).collect::<String>()
            }
        });
        // Annotations de comptage : « ce dossier contient N pages » et « cette page
        // vient d'un dossier qui en contient N » — de quoi répondre à un « combien »
        // sans avoir les N pages sous les yeux.
        // UN SEUL chiffre par ligne, sinon le modèle prend le mauvais : la première
        // version annonçait « dans Lucid, qui contient 37 pages · 16 sous-pages » et
        // Gemma a répondu 37. Un dossier annonce son propre compte ; une page
        // annonce celui de son dossier.
        let own = child_count.get(n.id.as_str()).copied();
        let ann = match (own, parent.is_empty()) {
            (Some(c), _) => format!(" (dossier de {c} pages)"),
            (None, false) => match n.parent_id.as_deref().and_then(|p| child_count.get(p)).copied() {
                Some(c) if c > 1 => format!(" (dans « {parent} », dossier de {c} pages)"),
                _ => format!(" (dans « {parent} »)"),
            },
            (None, true) => String::new(),
        };
        ctx.push_str(&format!("- {}{ann} : {body}\n", n.label));
    }
    ctx
}

/// Recherche VECTORIELLE expérimentale, pour comparer au classement par mots-clés
/// (`cargo run --example retrieval`). La question est embeddée avec BGE-M3, puis
/// comparée aux vecteurs DÉJÀ en cache : aucun document n'est ré-embeddé, une
/// mesure ne doit pas coûter 6 minutes de GPU à fond. Les pages sans vecteur sont
/// donc absentes du résultat — c'est justement une donnée de la mesure.
/// Renvoie `(similarité, id, label)` trié du plus proche au plus loin.
pub fn semantic_candidates(question: &str, top: usize) -> Result<Vec<(f32, String, String)>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph =
        backup::load_brain_cached(&dir).map_err(|_| "Génère d'abord ta mind map.".to_string())?;
    let cache: std::collections::HashMap<String, EmbedEntry> =
        std::fs::read_to_string(embed_cache_path(&dir)).ok()
            .and_then(|r| serde_json::from_str(&r).ok())
            .unwrap_or_default();
    if cache.is_empty() {
        return Err("aucun vecteur en cache — lance une passe de l'Archiviste d'abord".into());
    }
    let q = ai::llama::embed_texts(&[question.to_string()])?
        .pop()
        .ok_or("embedding de la question vide")?;
    let mut out: Vec<(f32, String, String)> = graph.nodes.iter()
        .filter(|n| n.kind != "root")
        .filter_map(|n| {
            let e = cache.get(&n.id)?;
            if e.vec.is_empty() { return None; }
            Some((ai::llama::cosine(&q, &e.vec), n.id.clone(), n.label.clone()))
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.truncate(top);
    Ok(out)
}

/// Le contexte RÉELLEMENT injecté dans le modèle local pour une question, sans
/// appeler le modèle. Exposé pour `cargo run --example retrieval` : ça permet de
/// scorer la récupération seule, sans qu'un modèle faible brouille le résultat.
pub fn ask_context_for(question: &str, hybrid: bool) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph =
        backup::load_brain_cached(&dir).map_err(|_| "Génère d'abord ta mind map.".to_string())?;
    let report = std::fs::read_to_string(dir.join("brain_report.md")).unwrap_or_default();
    let sem = if hybrid { semantic_ids(question) } else { Vec::new() };
    Ok(ask_context(&graph, question, &report, &sem))
}

/// Candidats vectoriels pour une question, en BEST-EFFORT : moteur d'embedding
/// absent, cache vide, modèle non téléchargé → liste vide, et la recherche par
/// mots-clés répond seule. L'assistant ne doit jamais échouer à cause de ça.
fn semantic_ids(question: &str) -> Vec<String> {
    const CANDIDATES: usize = 10;
    semantic_candidates(question, CANDIDATES)
        .map(|v| v.into_iter().map(|(_, id, _)| id).collect())
        .unwrap_or_default()
}

/// Corps de `ask_brain`, synchrone et exposé : chaque étape est chronométrée dans
/// `lucid.log`, et `cargo run --example ask -- "<question>"` rejoue exactement le
/// même chemin hors de l'app (un « réflexion » qui ne rend jamais la main n'est
/// diagnosticable qu'en voyant laquelle des étapes ne finit pas).
pub fn answer_question(question: &str) -> Result<String, String> {
    let t0 = std::time::Instant::now();
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph = backup::load_brain_cached(&dir)
        .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
    let report = std::fs::read_to_string(dir.join("brain_report.md")).unwrap_or_default();
    let engine = LlamaEngine::detect()?;
    elog!("💬 ask_brain : moteur prêt en {:.1}s", t0.elapsed().as_secs_f32());

    // Hybride mots-clés + vecteurs (best-effort : sans moteur d'embedding, les
    // mots-clés répondent seuls).
    let t_sem = std::time::Instant::now();
    let sem = semantic_ids(question);
    elog!("💬 ask_brain : {} candidat(s) vectoriel(s) en {:.1}s", sem.len(), t_sem.elapsed().as_secs_f32());

    let system = "Tu es l'assistant du second cerveau de l'utilisateur. Réponds en \
français, de façon concise, en te basant UNIQUEMENT sur le contexte fourni. Cite les pages \
par leur titre. Si l'information n'y figure pas, dis-le clairement.";
    let user = format!("CONTEXTE :\n{}\n\nQUESTION : {question}", ask_context(&graph, question, &report, &sem));
    elog!("💬 ask_brain : contexte de {} car., génération…", user.len());

    let t_gen = std::time::Instant::now();
    let out = engine.complete(Some(system), &user, 512);
    elog!("💬 ask_brain : génération {} en {:.1}s (total {:.1}s)",
        if out.is_ok() { "OK" } else { "ÉCHEC" }, t_gen.elapsed().as_secs_f32(), t0.elapsed().as_secs_f32());
    out
}

#[tauri::command]
async fn ask_brain(question: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || answer_question(&question))
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Chat contextuel sur une page : contexte = contenu de la page (+ sous-pages si demandé).
#[tauri::command]
async fn ask_node(node_id: String, question: String, include_children: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let graph: BrainGraph = backup::load_brain_cached(&dir)
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let node = graph.nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;

        let take = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        let mut ctx = format!("# {}\n", node.label);
        if !node.summary.is_empty() { ctx.push_str(&node.summary); ctx.push('\n'); }
        if !node.content.is_empty() { ctx.push_str(&take(&node.content, 4000)); ctx.push('\n'); }

        if include_children {
            let kids: Vec<&BrainNode> = graph.nodes.iter()
                .filter(|n| n.parent_id.as_deref() == Some(node_id.as_str()))
                .collect();
            if !kids.is_empty() {
                ctx.push_str("\n# Sous-pages\n");
                for k in kids {
                    ctx.push_str(&format!("\n## {}\n", k.label));
                    if !k.summary.is_empty() { ctx.push_str(&k.summary); ctx.push('\n'); }
                    if !k.content.is_empty() { ctx.push_str(&take(&k.content, 1200)); ctx.push('\n'); }
                    if ctx.len() > 9000 { ctx.push_str("\n[…contexte tronqué…]\n"); break; }
                }
            }
        }

        let engine = LlamaEngine::detect()?;
        let system = "Tu es l'assistant de cette page du second cerveau. Réponds en français, \
de façon concise et utile, en te basant sur le CONTEXTE fourni (la page courante et, si présentes, \
ses sous-pages). Si l'information n'y figure pas, dis-le clairement plutôt que d'inventer.";
        let user = format!("CONTEXTE :\n{ctx}\n\nQUESTION : {question}");
        engine.complete(Some(system), &user, 512)
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Génère du contenu markdown pour une page selon une consigne, contexte = page (+ sous-pages).
#[tauri::command]
async fn generate_content(node_id: String, instruction: String, include_children: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let graph: BrainGraph = backup::load_brain_cached(&dir)
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let node = graph.nodes.iter().find(|n| n.id == node_id)
            .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;

        let take = |s: &str, n: usize| s.chars().take(n).collect::<String>();
        let mut ctx = format!("# {}\n", node.label);
        if !node.summary.is_empty() { ctx.push_str(&node.summary); ctx.push('\n'); }
        if !node.content.is_empty() { ctx.push_str(&take(&node.content, 4000)); ctx.push('\n'); }

        if include_children {
            let kids: Vec<&BrainNode> = graph.nodes.iter()
                .filter(|n| n.parent_id.as_deref() == Some(node_id.as_str()))
                .collect();
            if !kids.is_empty() {
                ctx.push_str("\n# Sous-pages\n");
                for k in kids {
                    ctx.push_str(&format!("\n## {}\n", k.label));
                    if !k.summary.is_empty() { ctx.push_str(&k.summary); ctx.push('\n'); }
                    if !k.content.is_empty() { ctx.push_str(&take(&k.content, 1200)); ctx.push('\n'); }
                    if ctx.len() > 9000 { ctx.push_str("\n[…contexte tronqué…]\n"); break; }
                }
            }
        }

        let engine = LlamaEngine::detect()?;
        let system = "Tu rédiges du contenu markdown pour cette page du second cerveau, selon la \
CONSIGNE de l'utilisateur, en t'appuyant sur le CONTEXTE fourni (la page courante et, si présentes, \
ses sous-pages). Réponds UNIQUEMENT avec le contenu markdown demandé — pas de préambule, pas de \
phrase d'introduction, pas de bloc de code englobant.";
        let user = format!("CONTEXTE :\n{ctx}\n\nCONSIGNE : {instruction}");
        engine.complete(Some(system), &user, 1024)
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Lit le graphe.
/// Migre `brain.json` vers `brain.db` au premier appel après mise à jour de
/// l'app (one-shot, no-op ensuite — cf. Phase 2/4 du plan de migration SQLite).
/// Si `brain.md` est plus récent que l'état persisté (édition manuelle ou
/// écriture LLM), on repparse brain.md et on le persiste avant de retourner le graphe.
#[tauri::command]
fn read_brain_graph() -> Option<BrainGraph> {
    let dir = ai::llama::app_data_dir()?;
    if let Err(e) = storage::migrate_json_to_sqlite(&dir) {
        elog!("migration SQLite : {e}");
    }
    let db_path   = dir.join("brain.db");
    let json_path = dir.join("brain.json");
    let md_path   = dir.join("brain.md");
    let state_path = if db_path.exists() { &db_path } else { &json_path };

    let md_newer = md_path.exists() && {
        let jm = state_path.metadata().and_then(|m| m.modified()).ok();
        let mm = md_path.metadata().and_then(|m| m.modified()).ok();
        match (jm, mm) {
            (Some(j), Some(m)) => m > j,
            _ => !state_path.exists(),
        }
    };

    if md_newer {
        let content = std::fs::read_to_string(&md_path).ok()?;
        let mut graph = ai::brain_md::parse(&content);
        let _ = backup::write_brain(&dir, &mut graph);
        return Some(graph);
    }

    backup::load_brain_cached(&dir).ok()
}

/// Exporte un nœud + tous ses descendants en markdown standalone vers `path`.
#[tauri::command]
fn export_node_md(node_id: String, path: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph = backup::load_brain_cached(&dir)?;

    let mut queue: std::collections::VecDeque<(&models::BrainNode, usize)> =
        std::collections::VecDeque::new();
    if let Some(root) = graph.nodes.iter().find(|n| n.id == node_id) {
        queue.push_back((root, 0));
    } else {
        return Err(format!("Nœud {node_id} introuvable."));
    }
    let mut ordered: Vec<(&models::BrainNode, usize)> = Vec::new();
    while let Some((node, depth)) = queue.pop_front() {
        ordered.push((node, depth));
        let mut children: Vec<&models::BrainNode> = graph.nodes.iter()
            .filter(|n| n.parent_id.as_deref() == Some(node.id.as_str()))
            .collect();
        children.sort_by(|a, b| b.weight.cmp(&a.weight));
        for child in children { queue.push_back((child, depth + 1)); }
    }

    let mut md = String::new();
    for (node, depth) in ordered {
        ai::pipeline::emit_section(&mut md, node, depth + 1);
    }
    std::fs::write(&path, md).map_err(|e| e.to_string())
}

/// Charge le contenu d'un nœud feuille depuis le cache local (0 appel API).
/// Route selon connector : google-drive → cache JSON, claude-code → .jsonl, etc.
#[tauri::command]
fn load_node_content(node_id: String) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph = backup::load_brain_cached(&dir)?;

    let node = graph.nodes.iter().find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;

    let connector = node.connector.as_deref().unwrap_or("");
    let source_id = node.source_id.as_deref().unwrap_or("");
    let project_slug = node.source_project.as_deref().unwrap_or("");

    let conv = match connector {
        "google-drive" => connectors::google_drive::load_by_id(source_id),
        "claude-ai"    => connectors::claude_ai::load_by_id(source_id),
        "chatgpt"      => connectors::chatgpt::load_by_id(source_id),
        "claude-code"  => connectors::claude_code::load_conversation(project_slug, source_id),
        "obsidian"     => connectors::obsidian::load_by_id(source_id),
        "local-folder" => connectors::local_folder::load_by_id(source_id),
        "apple-notes"  => connectors::apple_notes::load_by_id(source_id),
        // Import local : le markdown converti vit dans node.content, pas de cache connecteur.
        "local-file"   => return Ok(node.content.clone()),
        _ => return Err(format!("Connecteur inconnu : {connector}")),
    };

    let conv = conv.ok_or_else(|| "Contenu absent du cache — relancez un Sync.".to_string())?;

    if conv.messages.len() == 1 {
        Ok(conv.messages[0].text.clone())
    } else {
        Ok(conv.messages.iter().map(|m| {
            let who = if m.role == "user" { "**Toi**" } else { "**IA**" };
            format!("{who}\n\n{}", m.text.trim())
        }).collect::<Vec<_>>().join("\n\n---\n\n"))
    }
}

// ── Historique par nœud ────────────────────────────────────────────────────

#[derive(serde::Serialize)]
struct NodeSnapshotInfo {
    id: String,        // "content_<ts>" (sans .md)
    created_at: u64,
    preview: String,   // premiers 150 chars
}

fn node_history_dir(dir: &std::path::Path, node_id: &str) -> std::path::PathBuf {
    // safe_component : les ids contiennent `:` (interdit sur Windows, os error 123).
    dir.join("node_history").join(backup::safe_component(node_id))
}

fn save_node_content_history(dir: &std::path::Path, node_id: &str, old_content: &str) {
    if old_content.trim().is_empty() { return; }
    let hdir = node_history_dir(dir, node_id);
    if std::fs::create_dir_all(&hdir).is_err() { return; }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::write(hdir.join(format!("content_{ts}.md")), old_content);
    // Garder les 10 dernières versions
    if let Ok(entries) = std::fs::read_dir(&hdir) {
        let mut files: Vec<_> = entries.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("content_"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        files.reverse();
        for old in files.into_iter().skip(10) { let _ = std::fs::remove_file(old.path()); }
    }
}

#[tauri::command]
fn list_node_snapshots(node_id: String) -> Vec<NodeSnapshotInfo> {
    let Some(dir) = ai::llama::app_data_dir() else { return vec![]; };
    let hdir = node_history_dir(&dir, &node_id);
    let mut infos: Vec<NodeSnapshotInfo> = std::fs::read_dir(&hdir)
        .into_iter().flatten().flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            if !s.starts_with("content_") || !s.ends_with(".md") { return None; }
            let ts_str = s.strip_prefix("content_")?.strip_suffix(".md")?;
            let created_at: u64 = ts_str.parse().ok()?;
            let content = std::fs::read_to_string(e.path()).unwrap_or_default();
            let preview: String = content.chars().take(150).collect();
            Some(NodeSnapshotInfo { id: format!("content_{ts_str}"), created_at, preview })
        })
        .collect();
    infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    infos
}

#[tauri::command]
fn get_node_snapshot(node_id: String, snapshot_id: String) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let path = node_history_dir(&dir, &node_id).join(format!("{snapshot_id}.md"));
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Sauvegarde le contenu markdown libre d'un nœud (jamais écrasé par l'IA).
#[tauri::command]
fn save_node_content(node_id: String, content: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    save_node_content_in(&dir, &node_id, &content)
}

/// Variante injectable (dossier explicite) — réutilisée par l'acceptation d'une
/// proposition MCP `update_node` (dispatch dans `resolve_proposal_in`).
fn save_node_content_in(dir: &std::path::Path, node_id: &str, content: &str) -> Result<(), String> {
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    save_node_content_on(dir, &mut graph, node_id, content)?;
    backup::write_brain_touching(dir, &mut graph, &[node_id.to_string()], &[])
}

/// Mutation en mémoire seule (pas de lecture/écriture de brain.json) — permet
/// à `resolve_all_pending_in` d'appliquer tout un lot de propositions avec UN
/// seul cycle lecture+écriture au lieu d'un par proposition (bug remonté par
/// Liam le 2026-07-21 : une arborescence de 17 pages faisait ~17 cycles de
/// lecture/écriture complets d'un brain.json de 50 Mo → app très lente).
fn save_node_content_on(dir: &std::path::Path, graph: &mut BrainGraph, node_id: &str, content: &str) -> Result<(), String> {
    // Détection de secrets obligatoire (autonome ET validation) — c'est ici que
    // passe tout contenu tapé/édité dans l'éditeur (insert_note_node_on ne couvre
    // que la création initiale : import, proposition MCP acceptée).
    let (content, masked) = secrets::mask_secrets(content);
    if masked { elog!("secrets: contenu masqué avant écriture (nœud {node_id})"); }
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    // Sauvegarde l'ancienne version avant d'écraser
    save_node_content_history(dir, node_id, &node.content);
    node.content = content;
    Ok(())
}

/// Crée un nœud « note » (prise de note utilisateur) rattaché à `parent_id`.
/// Persisté dans brain.json ; préservé lors des régénérations (kind == "note").
#[tauri::command]
fn create_note_node(parent_id: String, label: String) -> Result<BrainNode, String> {
    insert_note_node(parent_id, label, String::new(), None)
}

/// Insère un nœud note (avec contenu markdown éventuel) dans brain.json.
/// Cœur partagé entre `create_note_node` et `import_file`.
/// `source` = (connector, source_id) — ex. ("local-file", chemin d'origine)
/// pour que « Ouvrir l'original » retrouve le fichier.
fn insert_note_node(parent_id: String, label: String, content: String, source: Option<(&str, String)>) -> Result<BrainNode, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    insert_note_node_in(&dir, format!("note-{ts}"), parent_id, label, content, source)
}

/// Variante injectable (id + dossier de données explicites) — testable, et
/// utilisée par l'acceptation des propositions MCP (l'id de la proposition
/// devient l'id du nœud, ce qui garde valides les références parent en chaîne).
fn insert_note_node_in(dir: &std::path::Path, id: String, parent_id: String, label: String, content: String, source: Option<(&str, String)>) -> Result<BrainNode, String> {
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    let node = insert_note_node_on(&mut graph, id, parent_id, label, content, source)?;
    backup::write_brain_touching(dir, &mut graph, &[node.id.clone()], &[])?;
    Ok(node)
}

/// Mutation en mémoire seule — cf. `save_node_content_on`.
fn insert_note_node_on(graph: &mut BrainGraph, id: String, parent_id: String, label: String, content: String, source: Option<(&str, String)>) -> Result<BrainNode, String> {
    // Détection de secrets obligatoire (autonome ET validation) — point de passage
    // unique pour toute écriture de contenu note (humaine, import, ou acceptation MCP).
    let (content, masked) = secrets::mask_secrets(&content);
    if masked { elog!("secrets: contenu masqué avant écriture (nœud {id})"); }
    if !graph.nodes.iter().any(|n| n.id == parent_id) {
        return Err(format!("Nœud parent {parent_id} introuvable."));
    }
    if graph.nodes.iter().any(|n| n.id == id) {
        return Err(format!("Nœud {id} déjà présent."));
    }
    let node = BrainNode { updated_at: None,
        id: id.clone(),
        label: {
            let l = label.trim();
            if l.is_empty() { "Nouvelle note".into() } else { l.to_string() }
        },
        kind: "note".into(),
        weight: 0,
        summary: String::new(),
        keywords: vec![],
        decisions: vec![],
        patterns: vec![],
        community: 0,
        parent_id: Some(parent_id.clone()),
        synthesized_at: None,
        date: Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
        content,
        connector: source.as_ref().map(|(c, _)| c.to_string()),
        source_id: source.map(|(_, sid)| sid),
        source_project: None,
        source_text: String::new(),
    };
    graph.edges.push(BrainEdge {
        source: parent_id, target: id, kind: "contains".into(), relation: "contains".into(),
    });
    graph.nodes.push(node.clone());
    Ok(node)
}

/// Convertit un fichier local en markdown (PDF, DOC/DOCX/RTF, PPTX, XLSX, TXT/MD, CSV).
/// Partagé entre `import_file` et le connecteur « dossier local ».
/// Erreur = message honnête et actionnable (ADR-0015), jamais d'échec silencieux.
pub(crate) fn file_to_source_text(p: &std::path::Path) -> Result<String, String> {
    let label = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fichier").to_string();
    let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    let content = match ext.as_str() {
        "pdf" => connectors::google_drive::pdf_to_markdown(p, &label)
            .ok_or(if cfg!(windows) {
                "Extraction impossible — PDF scanné ? OCR requis : lance scripts/bundle-sidecars.ps1 (embarque poppler + tesseract)."
            } else {
                "Extraction PDF impossible (PDF scanné : OCR requis — brew install poppler tesseract tesseract-lang)."
            })?,
        "docx" => docx_to_text(p)?,
        "pptx" => connectors::google_drive::pptx_to_markdown(p)
            .ok_or("Extraction impossible : cette présentation ne contient pas de texte.")?,
        // .ppt legacy = format binaire propriétaire : réellement impossible sans
        // LibreOffice (ADR-0015) — on le dit.
        "ppt" => return Err("Format .ppt (ancien PowerPoint) non supporté — enregistre-le en .pptx ou PDF.".into()),
        "doc" | "rtf" => textutil_to_text(p)?,
        "txt" | "md" | "markdown" => read_lossy(p)?,
        "csv" => csv_to_markdown(&read_lossy(p)?),
        "xlsx" => xlsx_to_markdown(p)?,
        other => return Err(format!("Format non supporté : .{other}")),
    };
    if content.trim().is_empty() {
        return Err("Le fichier ne contient aucun texte exploitable.".into());
    }
    Ok(content)
}

/// Importe un fichier local : conversion en markdown puis création d'un nœud
/// note sous `parent_id`.
#[tauri::command]
fn import_file(path: String, parent_id: String) -> Result<BrainNode, String> {
    let p = std::path::Path::new(&path);
    let label = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fichier importé").to_string();
    let content = file_to_source_text(p)?;
    // Garde le chemin d'origine : « Ouvrir l'original » l'ouvrira avec l'app
    // par défaut (PowerPoint, Aperçu…) — le markdown reste la version cerveau.
    insert_note_node(parent_id, label, content, Some(("local-file", path)))
}

// ─── Propositions MCP (écriture validée par l'utilisateur) ─────────────────────
//
// Le serveur MCP distant (`supabase/functions/lucid-mcp`, seul serveur MCP —
// local et distant partagent la même URL depuis la décision du 2026-07-21) ne
// touche jamais brain.json directement : il dépose une proposition, rapatriée
// localement dans `mcp_pending/`. L'app les affiche (bulles fantômes/badges +
// panneau) ; seule l'acceptation écrit dans brain.json — un seul écrivain, pas
// de course. L'id de la proposition devient l'id du nœud accepté (action
// "create"), ce qui permet à l'IA de construire des arbres (parent_id = id d'une
// proposition précédente).

fn default_action() -> String { "create".into() }

/// Généralisée à 5 formes (`action`) plutôt qu'un système de bulles par action :
/// `create` (parent_id/label/content), `update` (target_id/content), `move`
/// (target_id/new_parent_id), `merge` (merge_ids/label optionnel du survivant),
/// `link` (target_id/link_target/relation). Rétrocompatible : les anciens
/// fichiers sans `action` sur disque se relisent comme `create`.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct McpProposal {
    id: String,
    #[serde(default = "default_action")]
    action: String,
    #[serde(default)]
    parent_id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    target_id: String,
    #[serde(default)]
    new_parent_id: String,
    #[serde(default)]
    merge_ids: Vec<String>,
    #[serde(default)]
    link_target: String,
    #[serde(default)]
    relation: String,
    #[serde(default)]
    created_at: String,
}

fn mcp_pending_dir(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("mcp_pending")
}

fn remove_pending_file(dir: &std::path::Path, id: &str) -> Result<(), String> {
    std::fs::remove_file(mcp_pending_dir(dir).join(format!("{id}.json"))).map_err(|e| e.to_string())
}

// ── Dédoublonnage des propositions résolues ─────────────────────────────────
// Le poll distant (App.tsx) rapatrie une proposition Supabase pendant 10 min
// après son dépôt (le temps qu'une IA puisse chaîner des sous-pages). Pour
// "create", une proposition déjà acceptée est détectable (son id est devenu un
// id de nœud réel) — mais pour update/move/merge/link, l'id de la proposition
// n'est JAMAIS un id de nœud, donc rien ne permettait de détecter un réimport.
// Chaque réimport relançait le cycle complet (snapshot + lecture + écriture
// des 50 Mo de brain.json), en boucle pendant 10 minutes après une action déjà
// appliquée — bug remonté par Liam le 2026-07-21 (lenteur persistante, IA à
// l'arrêt). Un petit registre horodaté, purgé après 24h, ferme ce trou.
fn resolved_proposals_path(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("mcp_resolved.json")
}

fn load_resolved_proposals(dir: &std::path::Path) -> std::collections::HashMap<String, u64> {
    std::fs::read_to_string(resolved_proposals_path(dir)).ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

fn proposal_already_resolved(dir: &std::path::Path, id: &str) -> bool {
    load_resolved_proposals(dir).contains_key(id)
}

fn mark_proposal_resolved(dir: &std::path::Path, id: &str) {
    let mut m = load_resolved_proposals(dir);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
    m.insert(id.to_string(), now);
    m.retain(|_, t| now.saturating_sub(*t) < 24 * 3600); // large marge vs. la fenêtre de 10 min côté serveur
    if let Ok(json) = serde_json::to_string(&m) {
        let _ = std::fs::write(resolved_proposals_path(dir), json);
    }
}

fn load_proposals_in(dir: &std::path::Path) -> Vec<McpProposal> {
    let mut out: Vec<McpProposal> = std::fs::read_dir(mcp_pending_dir(dir))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&std::fs::read_to_string(e.path()).ok()?).ok())
        .collect();
    out.sort_by(|a: &McpProposal, b: &McpProposal| a.id.cmp(&b.id));
    out
}

/// Accepte (insère dans brain.json, ancêtres pending d'abord) ou refuse
/// (supprime, descendants compris) une proposition. Renvoie le nombre de
/// propositions traitées.
/// Renvoie les ids réellement résolus (la chaîne d'ancêtres acceptés, ou le
/// sous-arbre refusé) — le front purge ces lignes côté Supabase (MCP distant).
fn resolve_proposal_in(dir: &std::path::Path, id: &str, accept: bool) -> Result<Vec<String>, String> {
    // Même verrou que la régénération complète : sans lui, une régénération en
    // tâche de fond (watcher Claude Code) et une acceptation MCP peuvent lire
    // brain.json au même instant puis écrire chacune leur propre version —
    // l'une écrase le travail de l'autre en mémoire (bug remonté par Liam le
    // 2026-07-21 : "le canvas s'est vidé, plus rien, ni les nouvelles pages").
    let _gen = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let props = load_proposals_in(dir);
    let target = props.iter().find(|p| p.id == id)
        .ok_or_else(|| format!("Proposition {id} introuvable."))?;

    if !accept {
        // Refus récursif : les descendants pending tombent avec le parent — ne
        // concerne que les chaînes create→create (parent_id) ; sans objet pour
        // les 4 autres actions (boucle sans effet, juste ce fichier est retiré).
        let mut doomed = vec![id.to_string()];
        let mut i = 0;
        while i < doomed.len() {
            for p in &props {
                if p.parent_id == doomed[i] && !doomed.contains(&p.id) {
                    doomed.push(p.id.clone());
                }
            }
            i += 1;
        }
        for rid in &doomed {
            let _ = std::fs::remove_file(mcp_pending_dir(dir).join(format!("{rid}.json")));
        }
        return Ok(doomed);
    }

    // Acceptation = écriture d'origine IA : filet de sécurité avant toute mutation
    // (identifiable ensuite dans le panneau Historique comme « Archiviste (MCP) »).
    save_snapshot_in(dir, "mcp_accept");

    // Zombie tolerance : le poll Supabase peut recréer localement une proposition
    // déjà appliquée (fenêtre de 10 min côté serveur) — une cible introuvable dans
    // ce cas n'est pas une vraie erreur, juste un nettoyage (même logique que
    // "déjà présent" pour `create` plus bas). Exclut EXPRÈS "Nœud parent … introuvable."
    // (message distinct de `set_node_parent_on`, cf. juste en dessous) : ce n'est
    // pas un zombie mais une vraie dépendance pas encore là (ex. accepter la bulle
    // "déplacer vers X" avant celle "créer X") — la tolérer aurait fait disparaître
    // le déplacement pour toujours (marqué résolu, fichier supprimé) sans jamais
    // l'appliquer ni le signaler (bug réel remonté par Liam le 2026-07-23, trouvé
    // via le même symptôme sur le lot automatique `resolve_all_pending_in`).
    fn tolerate_already_applied(r: Result<(), String>) -> Result<(), String> {
        match r {
            Err(e) if e.ends_with("introuvable.") && !e.starts_with("Nœud parent ") => Ok(()),
            other => other,
        }
    }

    match target.action.as_str() {
        "update" => {
            tolerate_already_applied(save_node_content_in(dir, &target.target_id, &target.content))?;
            remove_pending_file(dir, id)?;
            mark_proposal_resolved(dir, id);
            Ok(vec![target.target_id.clone()])
        }
        "move" => {
            tolerate_already_applied(set_node_parent_in(dir, &target.target_id, &target.new_parent_id))?;
            remove_pending_file(dir, id)?;
            mark_proposal_resolved(dir, id);
            Ok(vec![target.target_id.clone()])
        }
        "merge" => {
            let label = if target.label.trim().is_empty() { None } else { Some(target.label.clone()) };
            let ids = match merge_nodes_in(dir, &target.merge_ids, label) {
                Ok(survivor) => vec![survivor.id],
                Err(e) if e.ends_with("introuvable.") => target.merge_ids.clone(),
                Err(e) => return Err(e),
            };
            remove_pending_file(dir, id)?;
            mark_proposal_resolved(dir, id);
            Ok(ids)
        }
        "link" => {
            let relation = if target.relation.trim().is_empty() { None } else { Some(target.relation.clone()) };
            tolerate_already_applied(link_nodes_in(dir, &target.target_id, &target.link_target, relation))?;
            remove_pending_file(dir, id)?;
            mark_proposal_resolved(dir, id);
            Ok(vec![target.target_id.clone(), target.link_target.clone()])
        }
        _ => {
            // "create" : chaîne d'ancêtres encore pending (accepter un enfant accepte ses parents).
            let mut chain = vec![target.clone()];
            let mut cur_parent = target.parent_id.clone();
            while let Some(p) = props.iter().find(|p| p.id == cur_parent) {
                chain.push(p.clone());
                cur_parent = p.parent_id.clone();
            }
            chain.reverse();
            for p in &chain {
                match insert_note_node_in(dir, p.id.clone(), p.parent_id.clone(), p.label.clone(), p.content.clone(), None) {
                    Ok(_) => {}
                    // Réimport zombie (le poll Supabase a recréé une proposition déjà
                    // acceptée) : le nœud vit déjà dans le cerveau — on nettoie juste.
                    Err(e) if e.ends_with("déjà présent.") => {}
                    Err(e) => return Err(e),
                }
                remove_pending_file(dir, &p.id)?;
            }
            // Le nouveau nœud rejoint les spaces qui "voient" son parent : la vue d'un
            // space (filterGraphBySpace, front) montre node_ids ET leurs ANCÊTRES (pour
            // le fil d'ariane) — un nœud peut donc être visible dans un space sans être
            // un membre littéral de node_ids. Le check devait couvrir les deux cas ;
            // il ne couvrait que le littéral, laissant les propositions acceptées sous
            // un nœud "ancêtre seulement" invisibles alors même que brain.json les contient
            // (bug confirmé 2026-07-21 : compteur de nœuds à jour, bulle introuvable).
            let fresh_graph: BrainGraph = backup::load_brain_cached(dir)?;
            let parent_of: std::collections::HashMap<&str, Option<&str>> = fresh_graph.nodes.iter()
                .map(|n| (n.id.as_str(), n.parent_id.as_deref()))
                .collect();
            // `parent_id` est-il un membre littéral de `ids`, ou l'ancêtre d'un membre ?
            let space_can_see = |ids: &[String], parent_id: &str| -> bool {
                if ids.iter().any(|i| i == parent_id) { return true; }
                ids.iter().any(|member| {
                    let mut cur = parent_of.get(member.as_str()).copied().flatten();
                    while let Some(pid) = cur {
                        if pid == parent_id { return true; }
                        cur = parent_of.get(pid).copied().flatten();
                    }
                    false
                })
            };
            let mut spaces = load_spaces(dir);
            let mut touched = false;
            for p in &chain {
                for s in spaces.iter_mut() {
                    if let Some(ids) = s.node_ids.as_mut() {
                        if space_can_see(ids, &p.parent_id) && !ids.contains(&p.id) {
                            ids.push(p.id.clone());
                            touched = true;
                        }
                    }
                }
            }
            if touched { save_spaces(dir, &spaces); }
            for p in &chain { mark_proposal_resolved(dir, &p.id); }
            Ok(chain.iter().map(|p| p.id.clone()).collect())
        }
    }
}

/// Résout TOUTES les propositions en attente en UN seul cycle lecture+écriture
/// (au lieu d'un cycle complet par proposition) — une arborescence de N pages
/// (via brain_add_tree) faisait jusqu'ici N cycles de lecture/écriture d'un
/// brain.json qui peut peser plusieurs dizaines de Mo, rendant l'app très
/// lente/saccadée sur un gros lot (bug remonté par Liam le 2026-07-21, 17
/// pages d'un coup). Traite par vagues : tant qu'une proposition progresse, on
/// retente les suivantes (gère les chaînes create→create et les dépendances
/// internes au lot, ex. un `link` visant un `create` du même lot). Une
/// proposition dont la cible n'existe jamais reste en attente, visible dans
/// mcp_pending/ — jamais perdue en silence.
fn resolve_all_pending_in(dir: &std::path::Path) -> Result<Vec<String>, String> {
    // Même verrou que la régénération complète — cf. commentaire dans
    // `resolve_proposal_in`, même risque de course.
    let _gen = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let mut props = load_proposals_in(dir);
    if props.is_empty() { return Ok(vec![]); }
    // Snapshot différé après la boucle (cf. plus bas) : si rien ne progresse
    // (cible qui n'arrivera jamais), aucun snapshot ni écriture n'a lieu — sans
    // ça, un tick répété sur une proposition définitivement bloquée réécrirait
    // brain.json en boucle pour rien à chaque tentative.
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    let mut resolved: Vec<String> = Vec::new();
    let mut tombstoned: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut created_parent_of: Vec<(String, String)> = Vec::new();
    // Ids dont un CHAMP a réellement changé (pas juste une arête) — permet
    // l'écriture ciblée en fin de lot (write_brain_touching) sans deviner par
    // comparaison. "merge" reparente des enfants et redirige des wikilinks dans
    // un ensemble de nœuds non prévisible à l'avance : dès qu'il apparaît dans
    // le lot, on retombe sur le diff générique pour tout le lot (sûr, plus lent).
    let mut touched_for_write: Vec<String> = Vec::new();
    let mut used_merge = false;

    loop {
        let mut progressed = false;
        let mut remaining = Vec::new();
        for p in props {
            let ready = match p.action.as_str() {
                "update" => graph.nodes.iter().any(|n| n.id == p.target_id),
                // "link" a DEUX dépendances comme "move" : les deux extrémités
                // doivent exister. Sans vérifier `link_target`, un lien doc→entité
                // proposé dans le même lot que la CRÉATION de l'entité pouvait
                // s'appliquer avant elle → `link_nodes_on` renvoie "…introuvable.",
                // toléré plus bas comme zombie et la proposition DROPPÉE : le lien
                // ne se formait jamais (même piège que le bug "move" de 2026-07-23).
                "link" => graph.nodes.iter().any(|n| n.id == p.target_id)
                    && graph.nodes.iter().any(|n| n.id == p.link_target),
                // "move" a DEUX dépendances, pas juste la cible : la nouvelle
                // destination doit aussi déjà exister (ex. un thème que
                // l'Archiviste vient de créer dans LE MÊME lot). Sans vérifier
                // `new_parent_id` ici, `set_node_parent_on` échouait plus bas
                // avec "Nœud parent introuvable." — un message qui se termine
                // par "introuvable.", donc TOLÉRÉ PLUS BAS comme un doublon
                // déjà appliqué (zombie) au lieu d'être retenté au tour
                // suivant : le déplacement disparaissait silencieusement pour
                // toujours, le thème restant vide sans aucune erreur visible
                // (bug réel remonté par Liam le 2026-07-23).
                "move" => graph.nodes.iter().any(|n| n.id == p.target_id)
                    && graph.nodes.iter().any(|n| n.id == p.new_parent_id),
                "merge" => p.merge_ids.iter().all(|id| graph.nodes.iter().any(|n| &n.id == id)),
                _ => graph.nodes.iter().any(|n| n.id == p.parent_id), // "create"
            };
            if !ready { remaining.push(p); continue; }
            progressed = true;
            let outcome: Result<Vec<String>, String> = match p.action.as_str() {
                "update" => save_node_content_on(dir, &mut graph, &p.target_id, &p.content).map(|_| {
                    touched_for_write.push(p.target_id.clone());
                    vec![p.target_id.clone()]
                }),
                "move" => set_node_parent_on(&mut graph, &p.target_id, &p.new_parent_id).map(|_| {
                    touched_for_write.push(p.target_id.clone());
                    vec![p.target_id.clone()]
                }),
                "link" => {
                    let relation = if p.relation.trim().is_empty() { None } else { Some(p.relation.clone()) };
                    // link_nodes_on ne modifie que les arêtes (ni target_id ni link_target
                    // ne changent de champ) — rien à ajouter à touched_for_write.
                    link_nodes_on(&mut graph, &p.target_id, &p.link_target, relation)
                        .map(|_| vec![p.target_id.clone(), p.link_target.clone()])
                }
                "merge" => {
                    let label = if p.label.trim().is_empty() { None } else { Some(p.label.clone()) };
                    match merge_nodes_on(&mut graph, &p.merge_ids, label) {
                        Ok(survivor) => {
                            for m in &p.merge_ids[1..] { tombstoned.insert(m.clone()); }
                            used_merge = true;
                            Ok(vec![survivor.id])
                        }
                        Err(e) => Err(e),
                    }
                }
                _ => insert_note_node_on(&mut graph, p.id.clone(), p.parent_id.clone(), p.label.clone(), p.content.clone(), None)
                    .map(|n| {
                        created_parent_of.push((n.id.clone(), p.parent_id.clone()));
                        touched_for_write.push(n.id.clone());
                        vec![n.id]
                    }),
            };
            match outcome {
                Ok(ids) => {
                    resolved.extend(ids);
                    let _ = remove_pending_file(dir, &p.id);
                    mark_proposal_resolved(dir, &p.id);
                }
                // Zombie/no-op toléré (réimport distant d'une action déjà appliquée) :
                // nettoyage sans y voir une vraie erreur (même logique que le chemin
                // par-proposition dans `resolve_proposal_in`).
                Err(e) if e.ends_with("introuvable.") || e.ends_with("déjà présent.") => {
                    let _ = remove_pending_file(dir, &p.id);
                    mark_proposal_resolved(dir, &p.id);
                }
                // Erreur NON tolérée après avoir passé le contrôle de dépendances
                // (readiness) : l'opération est intrinsèquement invalide (fusion à
                // <2 ids, cycle, auto-lien…) → elle ne réussira JAMAIS. On la met en
                // quarantaine (retirée + loggée, donc pas « perdue en silence »)
                // plutôt que de la re-empiler : sinon chaque poll la rejoue en boucle
                // → flood du log + CPU à fond, l'app ne s'ouvre plus (bug remonté par
                // Liam le 2026-07-28 : `arch-merge-3` avec un seul id).
                Err(e) => {
                    elog!("mcp: proposition {} REJETÉE (invalide, retirée) : {e}", p.id);
                    let _ = remove_pending_file(dir, &p.id);
                    mark_proposal_resolved(dir, &p.id);
                }
            }
        }
        props = remaining;
        if !progressed || props.is_empty() { break; }
    }

    if resolved.is_empty() {
        return Ok(resolved); // rien n'a progressé (cibles bloquées) : ni snapshot ni écriture
    }
    save_snapshot_in(dir, "mcp_accept"); // un seul snapshot pour tout le lot, juste avant l'écriture

    // Les nouveaux nœuds rejoignent les spaces qui "voient" leur parent — même
    // logique que `resolve_proposal_in` (bug des ancêtres du 2026-07-21).
    if !created_parent_of.is_empty() {
        let parent_of: std::collections::HashMap<&str, Option<&str>> = graph.nodes.iter()
            .map(|n| (n.id.as_str(), n.parent_id.as_deref()))
            .collect();
        let space_can_see = |ids: &[String], parent_id: &str| -> bool {
            if ids.iter().any(|i| i == parent_id) { return true; }
            ids.iter().any(|member| {
                let mut cur = parent_of.get(member.as_str()).copied().flatten();
                while let Some(pid) = cur {
                    if pid == parent_id { return true; }
                    cur = parent_of.get(pid).copied().flatten();
                }
                false
            })
        };
        let mut spaces = load_spaces(dir);
        let mut touched = false;
        for (id, parent_id) in &created_parent_of {
            for s in spaces.iter_mut() {
                if let Some(ids) = s.node_ids.as_mut() {
                    if space_can_see(ids, parent_id) && !ids.contains(id) {
                        ids.push(id.clone());
                        touched = true;
                    }
                }
            }
        }
        if touched { save_spaces(dir, &spaces); }
    }

    if !tombstoned.is_empty() { add_tombstones(dir, &tombstoned); }
    if used_merge {
        backup::write_brain(dir, &mut graph)?;
    } else {
        backup::write_brain_touching(dir, &mut graph, &touched_for_write, &[])?;
    }
    let existing: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    purge_dead_space_ids(dir, &existing);
    Ok(resolved)
}

/// Version "tout accepter d'un coup" — utilisée par le mode autonome et par le
/// bouton "Tout accepter" du panneau de propositions.
#[tauri::command]
fn resolve_all_mcp_proposals() -> Result<Vec<String>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    resolve_all_pending_in(&dir)
}

/// Liste les propositions en attente déposées par le serveur MCP.
#[tauri::command]
fn list_mcp_proposals() -> Result<Vec<McpProposal>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    Ok(load_proposals_in(&dir))
}

/// Rapatrie une proposition MCP DISTANTE (table Supabase) dans le circuit local
/// `mcp_pending/` — même validation par bulles fantômes/badges que le MCP local.
/// `action` généralisé (voir `McpProposal`) : seuls les champs pertinents pour
/// l'action sont non-vides côté appelant (le reste passe en défaut).
/// Les 6 nouveaux champs sont `Option` : un appelant qui ne les connaît pas
/// encore (front pas encore mis à jour) continue de fonctionner en mode create.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn import_mcp_proposal(
    id: String,
    parent_id: String,
    label: String,
    content: String,
    action: Option<String>,
    target_id: Option<String>,
    new_parent_id: Option<String>,
    merge_ids: Option<Vec<String>>,
    link_target: Option<String>,
    relation: Option<String>,
) -> Result<(), String> {
    let target_id = target_id.unwrap_or_default();
    let new_parent_id = new_parent_id.unwrap_or_default();
    let merge_ids = merge_ids.unwrap_or_default();
    let link_target = link_target.unwrap_or_default();
    let relation = relation.unwrap_or_default();
    // L'id vient du réseau : charset strict (uuid) contre toute traversée de chemin.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("id de proposition invalide".into());
    }
    let action = action.filter(|a| !a.trim().is_empty()).unwrap_or_else(default_action);
    if action == "create" && label.trim().is_empty() {
        return Err("label vide".into());
    }
    let data_dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    // Déjà résolue (n'importe quelle action) → ne pas recréer le fichier pending.
    // Le poll distant rapatrie la même ligne Supabase pendant 10 min ; sans ce
    // garde-fou, chaque réimport relançait tout le cycle (snapshot + lecture +
    // écriture des 50 Mo de brain.json) même après une action déjà appliquée —
    // pour "update"/"move"/"merge"/"link" l'id n'est jamais un id de nœud donc
    // rien ne le détectait avant (bug remonté par Liam le 2026-07-21).
    if proposal_already_resolved(&data_dir, &id) {
        return Ok(());
    }
    // "create" seul : garde-fou historique redondant mais inoffensif (le nœud
    // vit déjà dans le cerveau si accepté avant que ce registre n'existe).
    if action == "create" {
        if let Ok(g) = backup::load_brain_cached(&data_dir) {
            if g.nodes.iter().any(|n| n.id == id) {
                return Ok(());
            }
        }
    }
    let dir = data_dir.join("mcp_pending");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let proposal = serde_json::json!({
        "id": id,
        "action": action,
        "parent_id": parent_id,
        "label": label.trim(),
        "content": content,
        "target_id": target_id,
        "new_parent_id": new_parent_id,
        "merge_ids": merge_ids,
        "link_target": link_target,
        "relation": relation,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(dir.join(format!("{id}.json")), proposal.to_string()).map_err(|e| e.to_string())
}

/// Fork V1 : copie un space partagé (payload publié) comme nouveau projet sous
/// la racine de MON cerveau — ids re-mintés (les ids du cerveau source
/// collisionneraient avec les nôtres), provenance `fork`, un seul write.
/// Renvoie le nœud projet créé. Spec : coffre « Copier un space partagé (fork) ».
#[tauri::command]
fn import_shared_space(payload: serde_json::Value, space_id: String) -> Result<BrainNode, String> {
    let title = payload.get("title").and_then(|v| v.as_str()).unwrap_or("Space importé").to_string();
    let src = payload.get("nodes").and_then(|v| v.as_array()).ok_or("Payload sans nodes.")?;
    if src.is_empty() {
        return Err("Ce space est vide.".into());
    }
    if serde_json::to_string(&payload).map(|s| s.len()).unwrap_or(0) > 10_000_000 {
        return Err("Space trop volumineux (> 10 Mo).".into());
    }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut graph: BrainGraph = backup::load_brain_cached(&dir)
        .map_err(|_| "Ton cerveau n'existe pas encore — termine l'onboarding avant d'importer un space.".to_string())?;
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone())
        .ok_or("Cerveau sans racine.")?;

    // Nom unique sous la racine : « Titre », puis « Titre (copie) », « Titre (copie 2) »…
    let taken: Vec<String> = graph.nodes.iter()
        .filter(|n| n.parent_id.as_deref() == Some(root_id.as_str()))
        .map(|n| n.label.clone())
        .collect();
    let mut name = title.clone();
    if taken.contains(&name) {
        name = format!("{title} (copie)");
        let mut k = 2;
        while taken.contains(&name) { name = format!("{title} (copie {k})"); k += 1; }
    }

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let proj_id = format!("fork-{nanos}");

    // Re-mint des ids ; les nœuds `root` du payload (la racine du cerveau source,
    // présente car le sous-graphe publié garde les ancêtres) sont sautés — leurs
    // enfants se rattachent directement au projet créé.
    let mut remap: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (i, sn) in src.iter().enumerate() {
        let old = sn.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if old.is_empty() || sn.get("kind").and_then(|v| v.as_str()) == Some("root") { continue; }
        remap.insert(old.to_string(), format!("fork-{nanos}-{i}"));
    }

    let proj = BrainNode { updated_at: None,
        id: proj_id.clone(),
        label: name,
        kind: "group".into(),
        weight: remap.len(),
        summary: String::new(),
        keywords: vec![],
        decisions: vec![],
        patterns: vec![],
        community: 0,
        parent_id: Some(root_id.clone()),
        synthesized_at: None,
        date: Some(today.clone()),
        content: String::new(),
        connector: Some("fork".into()),
        source_id: Some(space_id.clone()),
        source_project: None,
        source_text: String::new(),
    };
    graph.edges.push(BrainEdge {
        source: root_id, target: proj_id.clone(), kind: "contains".into(), relation: "contains".into(),
    });
    graph.nodes.push(proj.clone());

    for sn in src {
        let old = sn.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let Some(new_id) = remap.get(old) else { continue; };
        let parent_new = sn.get("parent_id").and_then(|v| v.as_str())
            .and_then(|p| remap.get(p)).cloned()
            .unwrap_or_else(|| proj_id.clone());
        let str_of = |k: &str| sn.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        graph.edges.push(BrainEdge {
            source: parent_new.clone(), target: new_id.clone(), kind: "contains".into(), relation: "contains".into(),
        });
        graph.nodes.push(BrainNode { updated_at: None,
            id: new_id.clone(),
            label: { let l = str_of("label"); if l.is_empty() { "Sans titre".into() } else { l } },
            kind: { let k = str_of("kind"); if k.is_empty() { "note".into() } else { k } },
            weight: sn.get("weight").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            summary: str_of("summary"),
            keywords: sn.get("keywords").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            decisions: vec![],
            patterns: vec![],
            community: 0,
            parent_id: Some(parent_new),
            synthesized_at: None,
            date: { let d = str_of("date"); Some(if d.is_empty() { today.clone() } else { d }) },
            content: str_of("content"),
            connector: Some("fork".into()),
            // `space:id-d-origine` — la clé d'un futur re-sync (V2 de la spec).
            source_id: Some(format!("{space_id}:{old}")),
            source_project: None,
            source_text: String::new(),
        });
    }

    // Ponts wikilinks du payload, remappés des deux côtés (sinon abandonnés).
    if let Some(edges) = payload.get("edges").and_then(|v| v.as_array()) {
        for e in edges {
            if e.get("kind").and_then(|v| v.as_str()) != Some("link") { continue; }
            let s = e.get("source").and_then(|v| v.as_str()).and_then(|x| remap.get(x));
            let t = e.get("target").and_then(|v| v.as_str()).and_then(|x| remap.get(x));
            if let (Some(s), Some(t)) = (s, t) {
                graph.edges.push(BrainEdge {
                    source: s.clone(), target: t.clone(), kind: "link".into(), relation: "wikilink".into(),
                });
            }
        }
    }

    backup::write_brain(&dir, &mut graph)?;
    Ok(proj)
}

/// Accepte ou refuse une proposition MCP. Renvoie les ids résolus (chaîne).
#[tauri::command]
fn resolve_mcp_proposal(id: String, accept: bool) -> Result<Vec<String>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    resolve_proposal_in(&dir, &id, accept)
}

/// Exporte le cerveau en zip (hors modèles) pour la sauvegarde/sync cloud.
/// Async + spawn_blocking : une commande sync tournerait sur le main thread et
/// gèlerait l'UI le temps du zip. Réponse IPC brute (pas de Vec<u8> sérialisé
/// en tableau JSON — plusieurs Mo d'octets = freeze du webview).
#[tauri::command]
async fn export_backup() -> Result<tauri::ipc::Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(|| {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.".to_string())?;
        backup::export_in(&dir)
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Restaure une sauvegarde (zip, payload IPC brut) dans le dossier de données.
/// Renvoie le nombre de fichiers restaurés. L'app doit recharger le graphe ensuite.
#[tauri::command]
async fn import_backup(request: tauri::ipc::Request<'_>) -> Result<usize, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Payload binaire attendu.".into());
    };
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.".to_string())?;
        backup::import_in(&dir, &bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Fusionne le zip de sync cloud dans les données locales (nœud par nœud, le
/// plus récent gagne — voir backup::merge_in). Contrairement à import_backup
/// (remplacement intégral, bouton « Restaurer »), rien n'est perdu.
/// Renvoie { files, local_extra } — local_extra = le local avait des choses en
/// plus, l'appelant doit repousser vers le cloud.
#[tauri::command]
async fn merge_backup(request: tauri::ipc::Request<'_>) -> Result<backup::MergeReport, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("Payload binaire attendu.".into());
    };
    let bytes = bytes.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.".to_string())?;
        backup::merge_in(&dir, &bytes)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Empreinte des données locales (mtime max du périmètre de backup) — le front
/// pousse la sync cloud quand elle change. Async : appelée toutes les 60 s, le
/// walk de node_history/snapshots ne doit jamais passer par le main thread.
#[tauri::command]
async fn sync_fingerprint() -> u64 {
    tauri::async_runtime::spawn_blocking(|| {
        ai::llama::app_data_dir().map(|d| backup::fingerprint_in(&d)).unwrap_or(0)
    })
    .await
    .unwrap_or(0)
}

/// Sauvegarde une image collée dans l'éditeur → `assets/img-{ts}.{ext}`.
/// Le markdown stocke le chemin relatif (`![](assets/…)`), l'affichage passe
/// par le protocole asset de Tauri.
#[tauri::command]
fn save_pasted_image(bytes: Vec<u8>, ext: String) -> Result<String, String> {
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err(format!("Format d'image non supporté : {ext}"));
    }
    if bytes.is_empty() { return Err("Image vide.".into()); }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let assets = dir.join("assets");
    std::fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let name = format!("img-{ts}.{ext}");
    std::fs::write(assets.join(&name), &bytes).map_err(|e| e.to_string())?;
    Ok(format!("assets/{name}"))
}

/// Lecture tolérante aux encodages non-UTF-8 (latin-1…).
/// Les octets nuls sont retirés : un fichier UTF-16 lu ici en sème un sur deux,
/// et Postgres refuse `\0` en jsonb (l'upload du space MCP échouait en bloc,
/// cf. `stripNul` côté front).
fn read_lossy(p: &std::path::Path) -> Result<String, String> {
    std::fs::read(p)
        .map(|b| String::from_utf8_lossy(&b).replace('\0', ""))
        .map_err(|e| e.to_string())
}

/// Extraction texte d'un .docx sans binaire externe (parité Mac/Windows, ADR-0015) :
/// un .docx est un zip ; le texte vit dans word/document.xml. Paragraphes → \n,
/// tags XML strippés, entités de base décodées. Suffisant pour alimenter brain.md.
fn docx_to_text(p: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let f = std::fs::File::open(p).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("docx invalide : {e}"))?;
    let mut doc = zip.by_name("word/document.xml")
        .map_err(|_| "docx invalide : word/document.xml absent.".to_string())?;
    let mut xml = String::new();
    doc.read_to_string(&mut xml).map_err(|e| e.to_string())?;

    let out = connectors::google_drive::xml_text(&xml).trim().to_string();
    if out.is_empty() { Err("Le document ne contient aucun texte.".into()) } else { Ok(out) }
}

// textutil (natif macOS) : .doc legacy et .rtf. Le .docx passe par docx_to_text (pur Rust).
#[cfg(target_os = "macos")]
fn textutil_to_text(p: &std::path::Path) -> Result<String, String> {
    let out = std::process::Command::new("textutil")
        .args(["-convert", "txt", "-stdout"])
        .arg(p)
        .output()
        .map_err(|e| format!("textutil : {e}"))?;
    if !out.status.success() {
        return Err(format!("textutil a échoué : {}", String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// Hors macOS : .doc legacy (format binaire propriétaire) et .rtf (v1) — réellement
// non supportés, on le dit honnêtement (ADR-0015).
#[cfg(not(target_os = "macos"))]
fn textutil_to_text(_p: &std::path::Path) -> Result<String, String> {
    Err("Import .doc/.rtf non supporté sur Windows — convertis le fichier en .docx ou PDF.".into())
}

#[cfg(test)]
mod docx_tests {
    use super::*;

    #[test]
    fn extrait_le_texte_dun_docx_minimal() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("lucid_test_docx");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.docx");
        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(
            b"<w:document><w:body>\
              <w:p><w:r><w:t>Bonjour &amp; bienvenue</w:t></w:r></w:p>\
              <w:p><w:r><w:t>Deuxi\xc3\xa8me ligne</w:t></w:r></w:p>\
              </w:body></w:document>",
        ).unwrap();
        zw.finish().unwrap();

        let text = docx_to_text(&path).unwrap();
        assert!(text.contains("Bonjour & bienvenue"), "entités décodées : {text}");
        assert!(text.contains("Deuxième ligne"), "paragraphe suivant sur sa ligne : {text}");
        assert!(text.lines().count() >= 2, "les </w:p> deviennent des sauts de ligne");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refuse_un_zip_sans_document_xml() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("lucid_test_docx");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vide.docx");
        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file("autre.txt", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"rien").unwrap();
        zw.finish().unwrap();

        assert!(docx_to_text(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod mcp_proposal_tests {
    use super::*;

    fn setup(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("brainlink_test_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(mcp_pending_dir(&dir)).unwrap();
        let graph = serde_json::json!({
            "nodes": [{ "id": "root", "label": "Lucid", "kind": "root", "weight": 0,
                        "summary": "", "keywords": [], "decisions": [], "patterns": [],
                        "community": 0, "parent_id": null, "synthesized_at": null,
                        "content": "", "connector": null, "source_id": null, "source_project": null }],
            "edges": [], "markdown": "", "report": "", "generated_at": ""
        });
        std::fs::write(dir.join("brain.json"), graph.to_string()).unwrap();
        dir
    }

    fn propose(dir: &std::path::Path, id: &str, parent: &str, label: &str) {
        let p = McpProposal {
            id: id.into(), action: default_action(), parent_id: parent.into(), label: label.into(),
            content: String::new(), target_id: String::new(), new_parent_id: String::new(),
            merge_ids: vec![], link_target: String::new(), relation: String::new(),
            created_at: String::new(),
        };
        std::fs::write(mcp_pending_dir(dir).join(format!("{id}.json")),
                       serde_json::to_string(&p).unwrap()).unwrap();
    }

    fn graph_ids(dir: &std::path::Path) -> Vec<String> {
        let g: BrainGraph = serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        g.nodes.iter().map(|n| n.id.clone()).collect()
    }

    fn load_graph(dir: &std::path::Path) -> BrainGraph {
        serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap()
    }

    /// Ajoute un nœud directement dans brain.json (hors circuit MCP), pour les
    /// tests qui ont besoin de nœuds déjà existants (update/move/merge/link).
    fn add_node(dir: &std::path::Path, id: &str, parent: &str, label: &str, content: &str) {
        let mut graph: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        graph["nodes"].as_array_mut().unwrap().push(serde_json::json!({
            "id": id, "label": label, "kind": "page", "weight": 0,
            "summary": "", "keywords": [], "decisions": [], "patterns": [],
            "community": 0, "parent_id": parent, "synthesized_at": null,
            "content": content, "connector": null, "source_id": null, "source_project": null
        }));
        std::fs::write(dir.join("brain.json"), graph.to_string()).unwrap();
    }

    fn propose_action(dir: &std::path::Path, p: McpProposal) {
        std::fs::write(mcp_pending_dir(dir).join(format!("{}.json", p.id)),
                       serde_json::to_string(&p).unwrap()).unwrap();
    }

    fn blank_proposal(id: &str, action: &str) -> McpProposal {
        McpProposal {
            id: id.into(), action: action.into(), parent_id: String::new(), label: String::new(),
            content: String::new(), target_id: String::new(), new_parent_id: String::new(),
            merge_ids: vec![], link_target: String::new(), relation: String::new(),
            created_at: String::new(),
        }
    }

    #[test]
    fn accepter_un_enfant_accepte_ses_ancetres_dans_l_ordre() {
        let dir = setup("chain");
        propose(&dir, "mcp-1", "root", "Parent");
        propose(&dir, "mcp-2", "mcp-1", "Enfant");
        propose(&dir, "mcp-3", "mcp-2", "Petit-enfant");
        // Accepter le petit-enfant doit insérer les 3, parents d'abord.
        assert_eq!(resolve_proposal_in(&dir, "mcp-3", true).unwrap().len(), 3);
        let ids = graph_ids(&dir);
        assert!(ids.contains(&"mcp-1".into()) && ids.contains(&"mcp-3".into()));
        assert!(load_proposals_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepter_un_zombie_deja_dans_le_cerveau_nettoie_sans_erreur() {
        let dir = setup("zombie");
        propose(&dir, "mcp-1", "root", "Note");
        assert_eq!(resolve_proposal_in(&dir, "mcp-1", true).unwrap().len(), 1);
        // Le poll Supabase recrée la proposition déjà acceptée (réimport zombie).
        propose(&dir, "mcp-1", "root", "Note");
        assert_eq!(resolve_proposal_in(&dir, "mcp-1", true).unwrap().len(), 1);
        assert!(load_proposals_in(&dir).is_empty());
        assert_eq!(graph_ids(&dir).iter().filter(|i| *i == "mcp-1").count(), 1); // pas de doublon
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuser_un_parent_supprime_ses_descendants() {
        let dir = setup("reject");
        propose(&dir, "mcp-1", "root", "Parent");
        propose(&dir, "mcp-2", "mcp-1", "Enfant");
        propose(&dir, "mcp-9", "root", "Autre");
        assert_eq!(resolve_proposal_in(&dir, "mcp-1", false).unwrap().len(), 2);
        let rest = load_proposals_in(&dir);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, "mcp-9");
        assert_eq!(graph_ids(&dir).len(), 1); // rien inséré
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn accepter_sous_un_noeud_visible_seulement_comme_ancetre_dans_un_space() {
        // Bug réel du 2026-07-21 : une proposition acceptée sous un nœud qui n'est
        // "vu" dans un space que comme ancêtre (fil d'ariane, filterGraphBySpace
        // remonte les parents) — pas comme membre littéral de node_ids — restait
        // invisible dans ce space alors même que brain.json la contenait.
        let dir = setup("space_ancestor");
        let mut graph: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        let mk = |id: &str, parent: &str| serde_json::json!({
            "id": id, "label": id, "kind": "page", "weight": 0,
            "summary": "", "keywords": [], "decisions": [], "patterns": [],
            "community": 0, "parent_id": parent, "synthesized_at": null,
            "content": "", "connector": null, "source_id": null, "source_project": null
        });
        graph["nodes"].as_array_mut().unwrap().push(mk("project", "root"));
        graph["nodes"].as_array_mut().unwrap().push(mk("leaf-x", "project"));
        std::fs::write(dir.join("brain.json"), graph.to_string()).unwrap();

        // Space scopé à "leaf-x" seul : "project" n'est PAS un membre littéral,
        // seulement un ancêtre affiché pour le contexte.
        let space = Space { id: "s1".into(), name: "Test".into(), node_ids: Some(vec!["leaf-x".into()]), updated_at: None };
        save_spaces(&dir, &[space]);

        // Propose et accepte une note sous "project" (l'ancêtre visible, pas membre).
        propose(&dir, "mcp-1", "project", "Nouvelle note");
        assert_eq!(resolve_proposal_in(&dir, "mcp-1", true).unwrap().len(), 1);

        let spaces = load_spaces(&dir);
        let s1 = spaces.iter().find(|s| s.id == "s1").unwrap();
        assert!(
            s1.node_ids.as_ref().unwrap().contains(&"mcp-1".to_string()),
            "le nouveau nœud doit rejoindre le space dont le parent n'était visible que comme ancêtre : {:?}",
            s1.node_ids
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_reparente_tombstone_et_redirige_le_wikilink() {
        let dir = setup("merge");
        add_node(&dir, "a", "root", "A", "");
        add_node(&dir, "b", "root", "B", "");
        add_node(&dir, "c", "b", "Enfant de B", ""); // doit être reparenté vers "a"
        add_node(&dir, "d", "root", "D", "voir [[B]] pour plus");

        let survivor = merge_nodes_in(&dir, &["a".to_string(), "b".to_string()], None).unwrap();
        assert_eq!(survivor.id, "a");

        let graph = load_graph(&dir);
        assert!(!graph.nodes.iter().any(|n| n.id == "b"), "b doit disparaître du graphe");
        let c = graph.nodes.iter().find(|n| n.id == "c").unwrap();
        assert_eq!(c.parent_id.as_deref(), Some("a"), "l'enfant de b doit être reparenté vers a");
        let d = graph.nodes.iter().find(|n| n.id == "d").unwrap();
        assert!(d.content.contains("[[A]]"), "le wikilink [[B]] doit être redirigé vers [[A]] : {}", d.content);
        assert!(!d.content.contains("[[B]]"));
        assert!(load_tombstones(&dir).contains("b"), "b doit être tombstoné");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn link_est_idempotent_et_refuse_lauto_lien() {
        let dir = setup("link");
        add_node(&dir, "x", "root", "X", "");
        add_node(&dir, "y", "root", "Y", "");
        link_nodes_in(&dir, "x", "y", None).unwrap();
        link_nodes_in(&dir, "y", "x", None).unwrap(); // même paire, sens inverse → no-op
        let graph = load_graph(&dir);
        assert_eq!(graph.edges.iter().filter(|e| e.kind == "link").count(), 1, "pas de doublon");
        assert!(link_nodes_in(&dir, "x", "x", None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_accepte_update_move_merge_link() {
        let dir = setup("dispatch_accept");
        add_node(&dir, "n1", "root", "Note 1", "vieux contenu");
        add_node(&dir, "n2", "root", "Note 2", "");
        add_node(&dir, "n3", "root", "Note 3", "");

        let mut p = blank_proposal("p-update", "update");
        p.target_id = "n1".into(); p.content = "nouveau contenu".into();
        propose_action(&dir, p);
        resolve_proposal_in(&dir, "p-update", true).unwrap();
        assert_eq!(load_graph(&dir).nodes.iter().find(|n| n.id == "n1").unwrap().content, "nouveau contenu");

        let mut p = blank_proposal("p-move", "move");
        p.target_id = "n2".into(); p.new_parent_id = "n1".into();
        propose_action(&dir, p);
        resolve_proposal_in(&dir, "p-move", true).unwrap();
        assert_eq!(load_graph(&dir).nodes.iter().find(|n| n.id == "n2").unwrap().parent_id.as_deref(), Some("n1"));

        let mut p = blank_proposal("p-link", "link");
        p.target_id = "n1".into(); p.link_target = "n3".into();
        propose_action(&dir, p);
        resolve_proposal_in(&dir, "p-link", true).unwrap();
        assert!(load_graph(&dir).edges.iter().any(|e| e.kind == "link" && (e.source == "n1" || e.target == "n1")));

        let mut p = blank_proposal("p-merge", "merge");
        p.merge_ids = vec!["n1".into(), "n3".into()];
        propose_action(&dir, p);
        resolve_proposal_in(&dir, "p-merge", true).unwrap();
        assert!(!load_graph(&dir).nodes.iter().any(|n| n.id == "n3"));

        assert!(load_proposals_in(&dir).is_empty(), "toutes les propositions doivent être consommées");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dispatch_refuse_est_un_simple_retrait_sans_mutation() {
        let dir = setup("dispatch_reject");
        add_node(&dir, "n1", "root", "Note 1", "contenu original");
        let mut p = blank_proposal("p-update", "update");
        p.target_id = "n1".into(); p.content = "ne doit jamais s'appliquer".into();
        propose_action(&dir, p);
        resolve_proposal_in(&dir, "p-update", false).unwrap();
        assert_eq!(load_graph(&dir).nodes.iter().find(|n| n.id == "n1").unwrap().content, "contenu original");
        assert!(load_proposals_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_all_traite_un_arbre_complet_en_un_seul_cycle() {
        // Reproduit le cas remonté par Liam le 2026-07-21 : une arborescence
        // (brain_add_tree) dépose plusieurs propositions "create" chaînées
        // d'un coup — resolve_all_pending_in doit toutes les appliquer avec
        // UNE seule lecture/écriture de brain.json (pas une par proposition).
        let dir = setup("resolve_all_tree");
        propose(&dir, "racine", "root", "Racine");
        propose(&dir, "enfant-a", "racine", "Enfant A");
        propose(&dir, "enfant-b", "racine", "Enfant B");
        propose(&dir, "petit-enfant", "enfant-a", "Petit-enfant");

        let snapshots_avant = std::fs::read_dir(dir.join("snapshots")).map(|d| d.count()).unwrap_or(0);
        let resolved = resolve_all_pending_in(&dir).unwrap();
        assert_eq!(resolved.len(), 4, "les 4 propositions doivent être résolues en un seul appel");

        let ids = graph_ids(&dir);
        for id in ["racine", "enfant-a", "enfant-b", "petit-enfant"] {
            assert!(ids.contains(&id.to_string()), "{id} doit exister : {ids:?}");
        }
        assert!(load_proposals_in(&dir).is_empty());

        let snapshots_apres = std::fs::read_dir(dir.join("snapshots")).map(|d| d.count()).unwrap_or(0);
        assert_eq!(snapshots_apres, snapshots_avant + 1, "un seul snapshot pour tout le lot, pas un par proposition");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Vérification Phase 4 (couche d'écriture SQLite) : reproduit le scénario
    /// mesuré le 2026-07-21 (lot de 44 créations via resolve_all_pending_in) sur
    /// le VRAI cerveau de Liam (415 nœuds, ~50 Mo en JSON) plutôt que la petite
    /// arborescence synthétique des autres tests — c'est la taille réelle qui
    /// causait le freeze. Sautée si la fixture n'est pas présente (gitignored).
    #[test]
    fn resolve_all_lot_de_44_creations_sur_le_vrai_cerveau_pas_de_freeze() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/brain.liam.json");
        if !fixture.exists() {
            eprintln!("fixture réelle absente — test sauté (normal hors machine de Liam)");
            return;
        }

        fn bench(dir: &std::path::Path, fixture: &std::path::Path, sqlite: bool) -> std::time::Duration {
            let _ = std::fs::remove_dir_all(dir);
            std::fs::create_dir_all(mcp_pending_dir(dir)).unwrap();
            std::fs::copy(fixture, dir.join("brain.json")).unwrap();
            if sqlite {
                crate::storage::migrate_json_to_sqlite(dir).unwrap();
                assert!(dir.join("brain.db").exists());
            }
            for i in 0..44 {
                propose(dir, &format!("bench-{i}"), "root", &format!("Bench {i}"));
            }
            let start = std::time::Instant::now();
            let resolved = resolve_all_pending_in(dir).unwrap();
            let elapsed = start.elapsed();
            assert_eq!(resolved.len(), 44, "les 44 créations doivent toutes être résolues en un cycle");
            elapsed
        }

        let dir_json = std::env::temp_dir().join("brainlink_test_bench_json");
        let dir_sqlite = std::env::temp_dir().join("brainlink_test_bench_sqlite");
        let t_json = bench(&dir_json, &fixture, false);
        let t_sqlite = bench(&dir_sqlite, &fixture, true);

        eprintln!("Phase 4 — lot de 44 créations sur brain réel (415 nœuds) : JSON = {t_json:?}, SQLite = {t_sqlite:?}");
        assert!(t_sqlite < std::time::Duration::from_secs(2), "lot de 44 créations trop lent côté SQLite : {t_sqlite:?}");

        let _ = std::fs::remove_dir_all(&dir_json);
        let _ = std::fs::remove_dir_all(&dir_sqlite);
    }

    #[test]
    fn resolve_all_laisse_en_attente_une_proposition_dont_la_cible_narrive_jamais() {
        let dir = setup("resolve_all_stuck");
        let mut p = blank_proposal("p-orphan", "update");
        p.target_id = "n-inexistant".into(); p.content = "peu importe".into();
        propose_action(&dir, p);
        let resolved = resolve_all_pending_in(&dir).unwrap();
        assert!(resolved.is_empty());
        assert_eq!(load_proposals_in(&dir).len(), 1, "reste visible, pas supprimée en silence");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod space_anchor_tests {
    use super::*;

    fn setup(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("brainlink_test_anchor_{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let graph = serde_json::json!({
            "nodes": [{ "id": "root", "label": "Lucid", "kind": "root", "weight": 0,
                        "summary": "", "keywords": [], "decisions": [], "patterns": [],
                        "community": 0, "parent_id": null, "synthesized_at": null,
                        "content": "", "connector": null, "source_id": null, "source_project": null }],
            "edges": [], "markdown": "", "report": "", "generated_at": ""
        });
        std::fs::write(dir.join("brain.json"), graph.to_string()).unwrap();
        dir
    }

    #[test]
    fn cree_une_ancre_et_lajoute_au_space() {
        let dir = setup("create");
        let mut space = Space { id: "s1".into(), name: "Perso".into(), node_ids: Some(vec![]), updated_at: None };
        ensure_space_anchor(&dir, &mut space);
        assert_eq!(space.node_ids.as_ref().unwrap(), &vec!["s1-vide".to_string()]);
        let graph: BrainGraph = serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        let anchor = graph.nodes.iter().find(|n| n.id == "s1-vide").expect("ancre absente de brain.json");
        assert_eq!(anchor.label, "Perso vide");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn idempotent_ne_duplique_pas_lancre() {
        let dir = setup("idempotent");
        let mut space = Space { id: "s1".into(), name: "Perso".into(), node_ids: Some(vec![]), updated_at: None };
        ensure_space_anchor(&dir, &mut space);
        ensure_space_anchor(&dir, &mut space);
        assert_eq!(space.node_ids.as_ref().unwrap().len(), 1);
        let graph: BrainGraph = serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        assert_eq!(graph.nodes.iter().filter(|n| n.id == "s1-vide").count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_backfille_un_space_existant_sans_ancre() {
        let dir = setup("migration");
        let space = Space { id: "old".into(), name: "Ancien space".into(), node_ids: Some(vec![]), updated_at: None };
        save_spaces(&dir, &[space]);
        ensure_all_space_anchors(&dir);
        let spaces = load_spaces(&dir);
        let s = spaces.iter().find(|s| s.id == "old").unwrap();
        assert!(s.node_ids.as_ref().unwrap().contains(&"old-vide".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod import_tests {
    use super::csv_to_markdown;

    #[test]
    fn csv_virgule() {
        let md = csv_to_markdown("nom,age\nAlice,30\nBob,25");
        assert!(md.starts_with("| nom | age |\n| --- | --- |\n"));
        assert!(md.contains("| Alice | 30 |"));
    }

    #[test]
    fn csv_point_virgule_et_pipe() {
        let md = csv_to_markdown("a;b\nx|y;\"z\"");
        assert!(md.contains("| a | b |"));
        assert!(md.contains("| x\\|y | z |")); // pipe échappé, quotes retirées
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn docx_via_textutil() {
        // Round-trip : txt → docx (textutil) → notre extraction. macOS only.
        let dir = std::env::temp_dir().join("brainlink_test_import");
        std::fs::create_dir_all(&dir).unwrap();
        let txt = dir.join("essai.txt");
        std::fs::write(&txt, "Bonjour LucidFlow").unwrap();
        let ok = std::process::Command::new("textutil")
            .args(["-convert", "docx", "-output"])
            .arg(dir.join("essai.docx"))
            .arg(&txt)
            .status().map(|s| s.success()).unwrap_or(false);
        assert!(ok, "textutil indisponible");
        let out = super::textutil_to_text(&dir.join("essai.docx")).unwrap();
        assert!(out.contains("Bonjour LucidFlow"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn csv_tronque_a_200_lignes() {
        let raw = std::iter::once("col".to_string())
            .chain((0..300).map(|i| i.to_string()))
            .collect::<Vec<_>>()
            .join("\n");
        let md = csv_to_markdown(&raw);
        assert!(md.contains("tronqué"));
        assert!(!md.contains("| 250 |"));
    }

    #[test]
    fn xlsx_route_bien_vers_le_parseur_calamine() {
        // Pas un vrai classeur : vérifie juste que .xlsx est routé vers
        // xlsx_to_markdown (et pas "Format non supporté"), et que l'échec
        // sur un fichier invalide reste un message honnête (ADR-0015).
        let dir = std::env::temp_dir().join("lucid_test_xlsx");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("faux.xlsx");
        std::fs::write(&path, b"pas un classeur excel").unwrap();
        let err = super::file_to_source_text(&path).unwrap_err();
        assert!(err.contains("Excel"), "message d'erreur attendu, reçu : {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// CSV → tableau markdown. Délimiteur `,` ou `;` auto-détecté, 200 lignes max.
// ponytail: split naïf — les champs quotés contenant le délimiteur seront mal découpés.
fn csv_to_markdown(raw: &str) -> String {
    let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return String::new() };
    let delim = if header.matches(';').count() > header.matches(',').count() { ';' } else { ',' };
    let cells = |l: &str| l.split(delim)
        .map(|c| c.trim().trim_matches('"').replace('|', "\\|"))
        .collect::<Vec<_>>()
        .join(" | ");
    let cols = header.split(delim).count();
    let mut md = format!("| {} |\n|{}\n", cells(header), " --- |".repeat(cols));
    let mut truncated = false;
    for (i, l) in lines.enumerate() {
        if i >= 200 { truncated = true; break; }
        md.push_str(&format!("| {} |\n", cells(l)));
    }
    if truncated { md.push_str("\n*… tronqué à 200 lignes.*\n"); }
    md
}

/// Extraction .xlsx pure Rust (calamine) — même logique de parité que docx/pptx :
/// pas de binaire externe, donc gratuit sur Windows. Une table markdown par feuille,
/// 200 lignes max chacune.
fn xlsx_to_markdown(p: &std::path::Path) -> Result<String, String> {
    use calamine::{open_workbook_auto, Data, Reader};
    let mut wb = open_workbook_auto(p).map_err(|e| format!("Classeur Excel illisible : {e}"))?;
    let sheet_names = wb.sheet_names().to_owned();
    let mut out = String::new();
    for name in &sheet_names {
        let Ok(range) = wb.worksheet_range(name) else { continue };
        let mut rows = range.rows();
        let Some(header) = rows.next() else { continue };
        let cell = |c: &Data| c.to_string().trim().replace('|', "\\|");
        let cells = |r: &[Data]| r.iter().map(cell).collect::<Vec<_>>().join(" | ");
        out.push_str(&format!("## {name}\n\n"));
        out.push_str(&format!("| {} |\n|{}\n", cells(header), " --- |".repeat(header.len())));
        let mut truncated = false;
        for (i, r) in rows.enumerate() {
            if i >= 200 { truncated = true; break; }
            out.push_str(&format!("| {} |\n", cells(r)));
        }
        if truncated { out.push_str("\n*… tronqué à 200 lignes.*\n"); }
        out.push('\n');
    }
    Ok(out)
}

/// Crée une arborescence de pages à partir d'une consigne en langage naturel
/// (ex. « une structure pour gérer un projet web »). Gemma propose l'arbre en JSON,
/// les nœuds sont créés en `kind: "note"` → préservés lors des régénérations.
/// Si `space_id` est fourni, les nœuds créés sont ajoutés à cet espace (sinon
/// ils seraient invisibles dans la vue filtrée). Retourne (label racine, nb créés).
#[tauri::command]
async fn create_structure(instruction: String, parent_id: Option<String>, space_id: Option<String>) -> Result<(String, usize), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = LlamaEngine::detect()?;
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let mut graph: BrainGraph = backup::load_brain_cached(&dir)
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;

        let parent = match parent_id {
            Some(p) if graph.nodes.iter().any(|n| n.id == p) => p,
            Some(p) => return Err(format!("Nœud parent {p} introuvable.")),
            None => graph.nodes.iter().find(|n| n.kind == "root")
                .map(|n| n.id.clone())
                .ok_or("Nœud racine introuvable.")?,
        };

        // Même technique que synthesize_node : le prompt se termine par l'ouverture
        // du JSON, le modèle ne peut que le compléter.
        let prompt = format!(
            "Tu organises un second cerveau. L'utilisateur demande :\n« {instruction} »\n\n\
Propose une arborescence de pages pour organiser ça. Format : un objet JSON avec \
\"label\" (titre court en français), \"summary\" (une phrase, optionnelle), \
\"content\" (corps markdown template de la page : 2 à 3 titres ## avec listes à puces, \
concis) et \"children\" (liste de sous-pages, même format, 2 niveaux maximum, \
4 à 6 pages par niveau).\n\n\
{{\"label\": \""
        );
        let completion = engine.complete(
            Some("Complete the JSON. Output only the JSON continuation, no other text."),
            &prompt,
            3000,
        )?;
        // Gemma continue parfois le préfixe, parfois régénère le JSON complet :
        // on tente les deux interprétations.
        let spec = ai::pipeline::parse_structure(&format!("{{\"label\": \"{completion}"))
            .or_else(|| ai::pipeline::parse_structure(&completion))
            .ok_or_else(|| format!("Réponse IA invalide : {completion}"))?;

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let mut counter = 0usize;
        fn insert(graph: &mut BrainGraph, parent: &str, spec: &ai::pipeline::StructureSpec, ts: u128, counter: &mut usize) {
            *counter += 1;
            let id = format!("note-{ts}-{counter}");
            graph.nodes.push(BrainNode { updated_at: None,
                id: id.clone(),
                label: spec.label.clone(),
                kind: "note".into(),
                weight: 0,
                summary: spec.summary.clone(),
                keywords: vec![],
                decisions: vec![],
                patterns: vec![],
                community: 0,
                parent_id: Some(parent.to_string()),
                synthesized_at: None,
                date: Some(chrono::Local::now().format("%Y-%m-%d").to_string()),
                content: spec.content.clone(),
                connector: None,
                source_id: None,
                source_project: None,
                source_text: String::new(),
            });
            graph.edges.push(BrainEdge {
                source: parent.to_string(), target: id.clone(),
                kind: "contains".into(), relation: "contains".into(),
            });
            for child in &spec.children {
                insert(graph, &id, child, ts, counter);
            }
        }
        insert(&mut graph, &parent, &spec, ts, &mut counter);

        backup::write_brain(&dir, &mut graph)?;

        // Rend les nœuds visibles dans l'espace actif (les ids sont déterministes :
        // note-{ts}-{1..=counter}).
        if let Some(sid) = space_id.filter(|s| s != "lucid") {
            let mut spaces = load_spaces(&dir);
            if let Some(space) = spaces.iter_mut().find(|s| s.id == sid) {
                let ids = space.node_ids.get_or_insert_with(Vec::new);
                ids.extend((1..=counter).map(|i| format!("note-{ts}-{i}")));
                save_spaces(&dir, &spaces);
            }
        }
        Ok((spec.label.clone(), counter))
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Re-rattache `node_id` sous `parent_id` (déplacement/lien dans la mind map).
/// Refuse un cycle (parent == descendant) qui figerait le layout.
#[tauri::command]
fn set_node_parent(node_id: String, parent_id: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    set_node_parent_in(&dir, &node_id, &parent_id)
}

/// Variante injectable (dossier explicite) — réutilisée par l'acceptation d'une
/// proposition MCP `move_node` (dispatch dans `resolve_proposal_in`).
fn set_node_parent_in(dir: &std::path::Path, node_id: &str, parent_id: &str) -> Result<(), String> {
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    set_node_parent_on(&mut graph, node_id, parent_id)?;
    backup::write_brain_touching(dir, &mut graph, &[node_id.to_string()], &[])
}

/// Mutation en mémoire seule — cf. `save_node_content_on`.
fn set_node_parent_on(graph: &mut BrainGraph, node_id: &str, parent_id: &str) -> Result<(), String> {
    if node_id == parent_id {
        return Err("Un nœud ne peut pas être son propre parent.".into());
    }
    if !graph.nodes.iter().any(|n| n.id == parent_id) {
        return Err(format!("Nœud parent {parent_id} introuvable."));
    }
    // Garde anti-cycle : on remonte les parents depuis parent_id ; si on croise node_id, refus.
    {
        let parent_of: std::collections::HashMap<&str, &str> = graph.nodes.iter()
            .filter_map(|n| n.parent_id.as_deref().map(|p| (n.id.as_str(), p)))
            .collect();
        let mut cur = Some(parent_id);
        // `seen` borne la remontée : le garde d'origine supposait le graphe
        // stocké acyclique. Or un lot de "move" peut créer un cycle en mémoire
        // (A→B puis B→A), écrit ensuite dans brain.json ; une remontée ultérieure
        // qui croise ce cycle bouclait alors À L'INFINI sur le thread principal —
        // app gelée sur le spinner à CHAQUE lancement (bug 2026-07-29, pile :
        // resolve_all_pending_in → set_node_parent_on → HashMap::get à 100% CPU).
        // Si on repasse par un nœud déjà vu, le cycle est PRÉEXISTANT (pas causé
        // par ce déplacement) : on refuse → l'appelant met la proposition en
        // quarantaine (cf. resolve_all_pending_in) et l'app démarre.
        let mut seen = std::collections::HashSet::new();
        while let Some(c) = cur {
            if c == node_id {
                return Err("Déplacement impossible : créerait une boucle.".into());
            }
            if !seen.insert(c) {
                return Err("Graphe incohérent : cycle de parenté préexistant.".into());
            }
            cur = parent_of.get(c).copied();
        }
    }
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    node.parent_id = Some(parent_id.to_string());
    // Maintient les arêtes de contenance cohérentes (utilisées par le filtrage par space).
    graph.edges.retain(|e| !(e.target == node_id && e.kind == "contains"));
    graph.edges.push(BrainEdge {
        source: parent_id.to_string(), target: node_id.to_string(), kind: "contains".into(), relation: "contains".into(),
    });
    Ok(())
}

#[cfg(test)]
mod set_parent_tests {
    use super::*;

    fn node(id: &str, parent: Option<&str>) -> BrainNode {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": id, "kind": "container", "weight": 0,
            "summary": "", "keywords": [], "decisions": [], "patterns": [],
            "community": 0, "parent_id": parent, "synthesized_at": null, "content": ""
        })).unwrap()
    }

    /// Régression 2026-07-29 : un cycle de parenté PRÉEXISTANT dans le graphe
    /// (a→b→a) faisait boucler la remontée anti-cycle à l'infini → thread
    /// principal gelé, app bloquée sur le spinner. Doit renvoyer une erreur, pas
    /// tourner sans fin (le test lui-même timeout si la boucle n'est pas bornée).
    #[test]
    fn cycle_preexistant_renvoie_erreur_sans_boucler() {
        let mut graph = BrainGraph {
            nodes: vec![node("root", None), node("a", Some("b")), node("b", Some("a")), node("x", Some("root"))],
            edges: vec![], markdown: String::new(), report: String::new(), generated_at: String::new(),
        };
        // Déplacer x sous a : la remontée depuis a traverse le cycle a↔b.
        let r = set_node_parent_on(&mut graph, "x", "a");
        assert!(r.is_err(), "un cycle préexistant doit être refusé, pas boucler : {r:?}");
    }
}

/// Ajoute un pont conceptuel entre deux nœuds existants (arête `link`, distincte
/// de l'arbre `contains`). Idempotent : pas de doublon si le lien existe déjà
/// dans un sens ou l'autre. Réutilisée par l'acceptation d'une proposition MCP
/// `link_nodes`.
fn link_nodes_in(dir: &std::path::Path, a: &str, b: &str, relation: Option<String>) -> Result<(), String> {
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    link_nodes_on(&mut graph, a, b, relation)?;
    // link_nodes_on ne modifie que les arêtes (ni a ni b ne changent de champ) —
    // aucun nœud à ré-estampiller, seul le diff d'arêtes de write_brain_touching joue.
    backup::write_brain_touching(dir, &mut graph, &[], &[])
}

/// Mutation en mémoire seule — cf. `save_node_content_on`.
fn link_nodes_on(graph: &mut BrainGraph, a: &str, b: &str, relation: Option<String>) -> Result<(), String> {
    if a == b {
        return Err("Un nœud ne peut pas être lié à lui-même.".into());
    }
    if !graph.nodes.iter().any(|n| n.id == a) { return Err(format!("Nœud {a} introuvable.")); }
    if !graph.nodes.iter().any(|n| n.id == b) { return Err(format!("Nœud {b} introuvable.")); }
    let already = graph.edges.iter().any(|e| e.kind == "link"
        && ((e.source == a && e.target == b) || (e.source == b && e.target == a)));
    if already { return Ok(()); }
    graph.edges.push(BrainEdge {
        source: a.to_string(), target: b.to_string(), kind: "link".into(),
        relation: relation.unwrap_or_else(|| "bridge".into()),
    });
    Ok(())
}

/// Fusionne 2+ nœuds en un seul (le premier id de `ids` survit) : reparente les
/// enfants des autres vers le survivant, redirige les arêtes (contains/link) et
/// les `[[wikilinks]]` textuels (résolus par label, cf. `src/App.tsx`), concatène
/// le contenu, tombstone les fusionnés (comme `delete_node`). Réutilisée par
/// l'acceptation d'une proposition MCP `merge_nodes`.
fn merge_nodes_in(dir: &std::path::Path, ids: &[String], label: Option<String>) -> Result<BrainNode, String> {
    save_snapshot_in(dir, "mcp_merge");
    let mut graph: BrainGraph = backup::load_brain_cached(dir)?;
    let result = merge_nodes_on(&mut graph, ids, label)?;
    add_tombstones(dir, &ids[1..].iter().cloned().collect());
    // Reste sur le diff générique (write_brain, pas write_brain_touching) :
    // merge_nodes_on reparente aussi les enfants des nœuds fusionnés et redirige
    // les [[wikilinks]] dans un nombre de nœuds non borné à l'avance — pas un
    // ensemble d'ids connu au moment de l'appel, contrairement aux 4 autres cas.
    backup::write_brain(dir, &mut graph)?;
    let existing: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    purge_dead_space_ids(dir, &existing);
    Ok(result)
}

/// Mutation en mémoire seule — cf. `save_node_content_on`. Ne gère PAS le
/// snapshot/tombstone/purge des spaces (dépendants du disque) : à charge de
/// l'appelant (`merge_nodes_in` pour un accept isolé, `resolve_all_pending_in`
/// pour un accept par lot).
fn merge_nodes_on(graph: &mut BrainGraph, ids: &[String], label: Option<String>) -> Result<BrainNode, String> {
    if ids.len() < 2 {
        return Err("merge_nodes demande au moins 2 ids.".into());
    }
    for id in ids {
        let node = graph.nodes.iter().find(|n| &n.id == id)
            .ok_or_else(|| format!("Nœud {id} introuvable."))?;
        if node.kind == "root" { return Err("La racine ne peut pas être fusionnée.".into()); }
    }
    let survivor_id = ids[0].clone();
    let merged_ids: std::collections::HashSet<String> = ids[1..].iter().cloned().collect();

    let bodies: Vec<String> = ids.iter()
        .filter_map(|id| graph.nodes.iter().find(|n| &n.id == id))
        .map(|n| if n.content.is_empty() { n.source_text.clone() } else { n.content.clone() })
        .filter(|c| !c.is_empty())
        .collect();
    let merged_content = bodies.join("\n\n---\n\n");
    let old_labels: Vec<String> = ids[1..].iter()
        .filter_map(|id| graph.nodes.iter().find(|n| &n.id == id))
        .map(|n| n.label.clone())
        .collect();
    let final_label = label.clone().unwrap_or_else(|| {
        graph.nodes.iter().find(|n| n.id == survivor_id).map(|n| n.label.clone()).unwrap_or_default()
    });

    // Reparente les enfants des fusionnés vers le survivant.
    for n in graph.nodes.iter_mut() {
        if let Some(p) = &n.parent_id {
            if merged_ids.contains(p) { n.parent_id = Some(survivor_id.clone()); }
        }
    }
    // Redirige toute arête (contains ou link) qui référence un nœud fusionné.
    for e in graph.edges.iter_mut() {
        if merged_ids.contains(&e.source) { e.source = survivor_id.clone(); }
        if merged_ids.contains(&e.target) { e.target = survivor_id.clone(); }
    }
    graph.edges.retain(|e| e.source != e.target); // pas d'auto-boucle après redirection
    // Dédoublonne (deux fusionnés pouvaient pointer vers la même cible).
    let mut seen = std::collections::HashSet::new();
    graph.edges.retain(|e| seen.insert((e.source.clone(), e.target.clone(), e.kind.clone())));

    // Redirige les [[wikilinks]] textuels (résolution par LABEL, jamais par id,
    // cf. src/App.tsx:graphWithGhosts) dans le contenu des nœuds restants.
    for old_label in &old_labels {
        if old_label == &final_label { continue; }
        let needle = format!("[[{old_label}]]");
        let replacement = format!("[[{final_label}]]");
        for n in graph.nodes.iter_mut() {
            if !merged_ids.contains(&n.id) && n.content.contains(&needle) {
                n.content = n.content.replace(&needle, &replacement);
            }
        }
    }

    graph.nodes.retain(|n| !merged_ids.contains(&n.id));
    let survivor = graph.nodes.iter_mut().find(|n| n.id == survivor_id)
        .ok_or("Nœud survivant introuvable après fusion.")?;
    survivor.content = merged_content;
    survivor.label = final_label;
    Ok(survivor.clone())
}

/// Renomme un nœud (change son `label`). Persisté dans brain.json.
#[tauri::command]
fn rename_node(node_id: String, label: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut graph: BrainGraph = backup::load_brain_cached(&dir)?;
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    let l = label.trim();
    node.label = if l.is_empty() { "Sans titre".into() } else { l.to_string() };
    backup::write_brain(&dir, &mut graph)
}

// ── Tombstones : une suppression doit survivre aux régénérations ─────────────
// Le pipeline reconstruit le graphe depuis les sources : sans mémoire des
// suppressions, tout nœud issu d'un connecteur renaîtrait à la génération
// suivante. Les ids supprimés (stables : `leaf:<conv>`, `p:<projet>`…) sont
// donc consignés dans `deleted_nodes.json` (embarqué dans la sync cloud).

fn load_tombstones(dir: &std::path::Path) -> std::collections::HashSet<String> {
    std::fs::read_to_string(dir.join("deleted_nodes.json")).ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn add_tombstones(dir: &std::path::Path, ids: &std::collections::HashSet<String>) {
    let mut all = load_tombstones(dir);
    all.extend(ids.iter().cloned());
    if let Ok(json) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(dir.join("deleted_nodes.json"), json);
    }
}

#[cfg(test)]
mod tombstone_tests {
    use super::*;

    #[test]
    fn add_cumule_et_load_retrouve_les_ids() {
        let dir = std::env::temp_dir().join("lucid_test_tombstones");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_tombstones(&dir).is_empty());
        add_tombstones(&dir, &["leaf:a".to_string()].into_iter().collect());
        add_tombstones(&dir, &["p:x".to_string(), "leaf:a".to_string()].into_iter().collect());
        let t = load_tombstones(&dir);
        assert_eq!(t.len(), 2, "cumul sans doublon : {t:?}");
        assert!(t.contains("leaf:a") && t.contains("p:x"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Phase 7 — snapshots via `VACUUM INTO` sur un compte migré SQLite. Avant ce
/// correctif, `save_snapshot_in` faisait `if !brain_path.exists() { return; }`
/// sur `brain.json` : une fois ce fichier renommé par la migration, plus AUCUN
/// snapshot n'était pris, silencieusement — trouvé le 2026-07-22.
#[cfg(test)]
mod snapshot_tests {
    use super::*;

    fn graph_with(ids: &[&str]) -> BrainGraph {
        BrainGraph {
            nodes: ids.iter().map(|id| serde_json::from_value(serde_json::json!({
                "id": id, "label": id, "kind": if *id == "root" { "root" } else { "leaf" }, "weight": 1
            })).unwrap()).collect(),
            edges: vec![],
            markdown: String::new(),
            report: String::new(),
            generated_at: "t".into(),
        }
    }

    #[test]
    fn save_snapshot_produit_un_db_sur_compte_migre_et_se_liste() {
        let dir = std::env::temp_dir().join("lucid_test_snapshot_db");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&graph_with(&["root", "A"])).unwrap()).unwrap();
        storage::migrate_json_to_sqlite(&dir).unwrap();
        assert!(dir.join("brain.db").exists());

        save_snapshot_in(&dir, "test_reason");

        let files: Vec<String> = std::fs::read_dir(dir.join("snapshots")).unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
            .collect();
        assert!(files.iter().any(|f| f.ends_with(".db")), "attendu un snapshot .db : {files:?}");

        let infos = list_snapshots_in(&dir);
        assert_eq!(infos.len(), 1);
        assert_eq!(infos[0].reason, "test_reason");
        assert_eq!(infos[0].node_count, 2, "le snapshot .db doit se lister avec le bon nombre de nœuds");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Régression du bug remonté par Liam le 2026-07-28 : une régénération (auto
    /// ou manuelle) détruisait la réorganisation de l'Archiviste — le pipeline
    /// re-rattachait chaque feuille à son conteneur d'origine (source), effaçant
    /// les déplacements. `reapply_overrides` doit préserver le parent choisi pour
    /// tout nœud modifié (updated_at → saved_parent), et laisser les autres tels
    /// que le pipeline les a placés.
    #[test]
    fn reapply_overrides_preserve_les_deplacements_de_larchiviste() {
        use std::collections::HashMap;
        let mut fresh: BrainGraph = serde_json::from_value(serde_json::json!({
            "nodes": [
                {"id":"leaf:x","label":"Doc","kind":"leaf","weight":1,"parent_id":"container:source"},
                {"id":"leaf:y","label":"Autre","kind":"leaf","weight":1,"parent_id":"container:source"}
            ],
            "edges": [], "markdown": "", "report": "", "generated_at": ""
        })).unwrap();
        // L'Archiviste avait déplacé leaf:x sous son conteneur (updated_at posé → saved_parent).
        let saved_parent = HashMap::from([("leaf:x".to_string(), Some("arch-theme-factures".to_string()))]);

        reapply_overrides(&mut fresh, &HashMap::new(), &saved_parent);

        assert_eq!(fresh.nodes[0].parent_id.as_deref(), Some("arch-theme-factures"),
            "le déplacement de l'Archiviste doit survivre à la régénération");
        assert_eq!(fresh.nodes[1].parent_id.as_deref(), Some("container:source"),
            "un nœud non modifié garde le placement du pipeline");
    }

    #[test]
    fn restore_snapshot_depuis_un_db_restaure_letat_sqlite_actif() {
        let dir = std::env::temp_dir().join("lucid_test_restore_db");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&graph_with(&["root", "A"])).unwrap()).unwrap();
        storage::migrate_json_to_sqlite(&dir).unwrap();

        save_snapshot_in(&dir, "avant_suppression");
        let infos = list_snapshots_in(&dir);
        let snapshot_id = infos[0].id.clone();

        // "A" disparaît après le snapshot (simule une suppression malencontreuse).
        let mut graph = backup::load_brain_cached(&dir).unwrap();
        graph.nodes.retain(|n| n.id != "A");
        backup::write_brain(&dir, &mut graph).unwrap();
        assert!(!backup::load_brain_cached(&dir).unwrap().nodes.iter().any(|n| n.id == "A"));

        let restored = restore_snapshot_in(&dir, &snapshot_id).unwrap();
        assert!(restored.nodes.iter().any(|n| n.id == "A"), "A doit revenir après restauration");

        // Persisté pour de vrai dans brain.db, pas juste dans la valeur retournée.
        let reloaded = backup::load_brain_cached(&dir).unwrap();
        assert!(reloaded.nodes.iter().any(|n| n.id == "A"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Supprime un nœud et toute sa descendance (snapshot avant coup, racine
/// intouchable). Purge aussi les ids morts des spaces. Renvoie le nombre
/// de nœuds supprimés.
#[tauri::command]
fn delete_node(node_id: String) -> Result<usize, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut graph: BrainGraph = backup::load_brain_cached(&dir)?;
    let node = graph.nodes.iter().find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    if node.kind == "root" { return Err("La racine ne peut pas être supprimée.".into()); }

    let doomed: std::collections::HashSet<String> =
        subtree_ids(&dir, &node_id).into_iter().collect();
    save_snapshot_in(&dir, "delete_node"); // destructif → filet (restaurable via les snapshots)
    add_tombstones(&dir, &doomed); // la suppression survivra aux régénérations
    graph.nodes.retain(|n| !doomed.contains(&n.id));
    graph.edges.retain(|e| !doomed.contains(&e.source) && !doomed.contains(&e.target));
    backup::write_brain(&dir, &mut graph)?;

    // Les spaces ne doivent pas garder d'ids morts.
    let existing: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    purge_dead_space_ids(&dir, &existing);
    Ok(doomed.len())
}

/// Re-synthétise un nœud unique à la demande (sources brutes + résumés enfants comme contexte).
/// Met à jour brain.json et retourne le nœud mis à jour.
#[tauri::command]
async fn synthesize_node(node_id: String) -> Result<BrainNode, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = LlamaEngine::detect()?;
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;

        let mut graph: BrainGraph = backup::load_brain_cached(&dir)?;

        let node_idx = graph.nodes.iter().position(|n| n.id == node_id)
            .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;

        let mut ctx = String::new();

        // Contenu propre du nœud (indispensable depuis que l'extraction IA
        // auto est désactivée : les summaries sont vides tant qu'on n'a pas
        // synthétisé à la main).
        let own = graph.nodes[node_idx].content.trim();
        if !own.is_empty() {
            ctx.push_str(own);
            ctx.push_str("\n\n");
        }

        // Résumés + décisions des enfants (fallback : début du contenu si pas de summary)
        let children: Vec<(String, String, Vec<String>)> = graph.nodes.iter()
            .filter(|n| n.parent_id.as_deref() == Some(node_id.as_str()))
            .map(|n| {
                let gist = if n.summary.trim().is_empty() {
                    n.content.chars().take(600).collect()
                } else {
                    n.summary.clone()
                };
                (n.label.clone(), gist, n.decisions.clone())
            })
            .collect();
        if !children.is_empty() {
            ctx.push_str("## Sous-espaces / pages\n");
            for (label, summary, decisions) in &children {
                if !summary.is_empty() {
                    ctx.push_str(&format!("### {label}\n{summary}\n"));
                }
                for d in decisions {
                    ctx.push_str(&format!("- {d}\n"));
                }
                ctx.push('\n');
            }
        }

        if ctx.trim().is_empty() {
            return Err("Aucun contenu disponible pour synthétiser ce nœud.".into());
        }
        // Tronqué comme le pipeline (évite les hallucinations sur contexte trop long)
        const MAX_CTX: usize = 5000;
        if ctx.len() > MAX_CTX {
            let mut end = MAX_CTX;
            while !ctx.is_char_boundary(end) { end -= 1; }
            ctx.truncate(end);
            ctx.push_str("\n[…tronqué]");
        }

        // Le prompt se termine par le début du JSON + premier champ ouvert.
        // Le modèle doit compléter le résumé depuis le contenu, impossible d'y échapper.
        let prompt = format!(
            "Voici un contenu. Complète le JSON en français, uniquement depuis ce contenu.\n\n\
CONTENU :\n{ctx}\n\n\
{{\"summary\": \""
        );
        let completion = engine.complete(
            Some("Complete the JSON. Use only information from the provided CONTENT. Output only the JSON continuation, no other text."),
            &prompt,
            600,
        )?;

        // Re-préfixe les caractères du prompt qui ouvraient le JSON
        let full = format!("{{\"summary\": \"{completion}");
        let json_str = ai::pipeline::extract_json(&full)
            .ok_or_else(|| format!("Réponse IA invalide : {completion}"))?;
        let v: serde_json::Value = serde_json::from_str(json_str)
            .map_err(|e| format!("JSON invalide : {e}"))?;

        let synthesized_at = chrono::Utc::now().to_rfc3339();

        let node = &mut graph.nodes[node_idx];
        if let Some(s) = v["summary"].as_str() { node.summary = s.to_string(); }
        node.keywords  = v["keywords"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        node.decisions = v["decisions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        node.patterns  = v["patterns"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        node.synthesized_at = Some(synthesized_at);

        let updated = node.clone();

        let _ = backup::write_brain(&dir, &mut graph);

        Ok(updated)
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

// ── Snapshots ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
struct SnapshotInfo {
    id: String,
    created_at: u64,
    node_count: usize,
    /// Origine du snapshot ("mcp_accept", "delete_node", "pre_restore",
    /// "regenerate", "manual" pour les anciens fichiers sans suffixe).
    reason: String,
}

/// `reason` identifie l'origine (ex. "mcp_accept") — sert au panneau Historique
/// à retrouver « la dernière action de l'Archiviste » sans mécanisme dédié.
/// Phase 7 : si `brain.db` existe, snapshot via `VACUUM INTO` (copie cohérente,
/// WAL inclus) — sinon copie JSON classique (comptes pas encore migrés).
/// Avant cette Phase 7, ce garde-fou ne prenait PLUS AUCUN snapshot une fois
/// `brain.json` renommé par la migration (le test d'existence renvoyait
/// silencieusement sans rien faire) : trouvé le 2026-07-22 en creusant le plan.
fn save_snapshot_in(dir: &std::path::Path, reason: &str) {
    let snap_dir = dir.join("snapshots");
    if std::fs::create_dir_all(&snap_dir).is_err() { return; }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if dir.join("brain.db").exists() {
        let _ = storage::snapshot_to(dir, &snap_dir.join(format!("brain_{ts}_{reason}.db")));
    } else {
        let brain_path = dir.join("brain.json");
        if !brain_path.exists() { return; }
        let _ = std::fs::copy(&brain_path, snap_dir.join(format!("brain_{ts}_{reason}.json")));
    }
    // Garder les 10 derniers (tous formats confondus — le nom trie chronologiquement).
    if let Ok(entries) = std::fs::read_dir(&snap_dir) {
        let mut files: Vec<_> = entries.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("brain_"))
            .collect();
        files.sort_by_key(|e| e.file_name());
        files.reverse();
        for old in files.into_iter().skip(10) { let _ = std::fs::remove_file(old.path()); }
    }
}

#[tauri::command]
fn list_snapshots() -> Vec<SnapshotInfo> {
    let Some(dir) = ai::llama::app_data_dir() else { return vec![]; };
    list_snapshots_in(&dir)
}

/// Variante injectable (dossier explicite) — testable sans dépendre de
/// `ai::llama::app_data_dir()`.
fn list_snapshots_in(dir: &std::path::Path) -> Vec<SnapshotInfo> {
    let snap_dir = dir.join("snapshots");
    let mut infos: Vec<SnapshotInfo> = std::fs::read_dir(&snap_dir)
        .into_iter().flatten().flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            // Phase 7 : les nouveaux snapshots sont des `.db` (VACUUM INTO) ; les
            // `.json` restent lisibles (snapshots pris avant la migration du compte).
            let is_db = s.ends_with(".db");
            if !s.starts_with("brain_") || !(is_db || s.ends_with(".json")) { return None; }
            let stem = s.strip_prefix("brain_")?.strip_suffix(if is_db { ".db" } else { ".json" })?;
            // Anciens snapshots : "brain_<ts>.json" (pas de raison) → "manual".
            let (ts_str, reason) = match stem.split_once('_') {
                Some((ts, reason)) => (ts, reason.to_string()),
                None => (stem, "manual".to_string()),
            };
            let created_at: u64 = ts_str.parse().ok()?;
            let node_count = if is_db {
                storage::load_brain_graph_from_file(&e.path()).map(|g| g.nodes.len()).unwrap_or(0)
            } else {
                std::fs::read_to_string(e.path()).ok()
                    .and_then(|r| serde_json::from_str::<BrainGraph>(&r).ok())
                    .map(|g| g.nodes.len()).unwrap_or(0)
            };
            Some(SnapshotInfo { id: format!("brain_{stem}"), created_at, node_count, reason })
        })
        .collect();
    infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    infos
}

#[tauri::command]
fn restore_snapshot(snapshot_id: String) -> Result<BrainGraph, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    restore_snapshot_in(&dir, &snapshot_id)
}

/// Variante injectable (dossier explicite) — testable sans dépendre de
/// `ai::llama::app_data_dir()`.
fn restore_snapshot_in(dir: &std::path::Path, snapshot_id: &str) -> Result<BrainGraph, String> {
    let snap_dir = dir.join("snapshots");
    let db_src = snap_dir.join(format!("{snapshot_id}.db"));
    let json_src = snap_dir.join(format!("{snapshot_id}.json"));
    save_snapshot_in(dir, "pre_restore"); // snapshot de l'état actuel avant restauration
    // write_brain (et pas une copie brute) : les nœuds qui changent par rapport à
    // l'état courant sont ré-estampillés, sinon la sync « annulerait » la restauration
    // en re-fusionnant l'état cloud plus récent par-dessus. Marche pareil pour un
    // snapshot .db (Phase 7) : on lit son contenu et on le repasse par write_brain,
    // qui écrit en JSON ou en SQLite selon ce que le compte actif utilise déjà.
    let mut graph: BrainGraph = if db_src.exists() {
        storage::load_brain_graph_from_file(&db_src)?
    } else {
        let raw = std::fs::read_to_string(&json_src).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())?
    };
    backup::write_brain(dir, &mut graph)?;
    // Un nœud créé après ce snapshot (ex. une note acceptée via MCP) peut encore
    // vivre dans un space : sans purge, le space affiche un nombre de nœuds sans
    // rien de visible sur le canvas (ids fantômes — bug remonté par Liam le 2026-07-21).
    let existing: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    purge_dead_space_ids(dir, &existing);
    Ok(graph)
}

// ── Spaces ─────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq)]
struct Space {
    id: String,
    name: String,
    node_ids: Option<Vec<String>>,
    /// Epoch (secondes) de la dernière modification — estampillé par save_spaces,
    /// sert à la fusion de la sync cloud (le plus récent gagne).
    #[serde(default)]
    updated_at: Option<u64>,
}

/// Retire des spaces tout id absent de `existing_ids` — sinon un space affiche un
/// nombre de nœuds sans rien de visible (ids fantômes après suppression/restauration/
/// régénération). Utilisé par `delete_node`, `restore_snapshot` et la régénération.
fn purge_dead_space_ids(dir: &std::path::Path, existing_ids: &std::collections::HashSet<String>) {
    let mut spaces = load_spaces(dir);
    let mut touched = false;
    for s in spaces.iter_mut() {
        if let Some(ids) = s.node_ids.as_mut() {
            let before = ids.len();
            ids.retain(|id| existing_ids.contains(id));
            touched |= ids.len() != before;
        }
    }
    if touched { save_spaces(dir, &spaces); }
}

fn load_spaces(dir: &std::path::Path) -> Vec<Space> {
    std::fs::read_to_string(dir.join("spaces.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_spaces(dir: &std::path::Path, spaces: &[Space]) {
    // Estampille les espaces nouveaux/modifiés (comparaison hors updated_at),
    // comme write_brain pour les nœuds — la fusion de sync s'appuie dessus.
    let old = load_spaces(dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut stamped: Vec<Space> = spaces.to_vec();
    for s in &mut stamped {
        s.updated_at = match old.iter().find(|o| o.id == s.id) {
            Some(o) if Space { updated_at: None, ..o.clone() } == Space { updated_at: None, ..s.clone() } => {
                o.updated_at.or(Some(now))
            }
            _ => Some(now),
        };
    }
    if let Ok(json) = serde_json::to_string_pretty(&stamped) {
        let _ = std::fs::write(dir.join("spaces.json"), json);
    }
}

#[tauri::command]
fn list_spaces() -> Vec<Space> {
    let lucid = Space { id: "lucid".into(), name: "Lucid".into(), node_ids: None, updated_at: None };
    let mut spaces = vec![lucid];
    if let Some(dir) = ai::llama::app_data_dir() {
        spaces.extend(load_spaces(&dir));
    }
    spaces
}

/// Nœud d'ancrage d'un space : point d'attache stable, jamais vide, jamais
/// dépendant du contenu réel du space. Sert de `parent_id` toujours valide pour
/// une proposition MCP — y compris quand un space PUBLIÉ est un instantané figé
/// (voir `supabase/functions/lucid-mcp`) qui référence des ids depuis disparus
/// du cerveau local (bug remonté par Liam le 2026-07-21 : proposition MCP visant
/// un id du space publié introuvable en local → acceptation impossible).
/// Idempotent : ne recrée rien si l'ancre existe déjà.
fn ensure_space_anchor(dir: &std::path::Path, space: &mut Space) {
    let anchor_id = format!("{}-vide", space.id);
    let already_member = space.node_ids.as_ref().is_some_and(|ids| ids.contains(&anchor_id));
    let Ok(graph) = backup::load_brain_cached(dir) else { return };
    let node_exists = graph.nodes.iter().any(|n| n.id == anchor_id);
    if already_member && node_exists { return; }
    if !node_exists {
        let Some(root_id) = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone()) else { return };
        let label = format!("{} vide", space.name);
        let content = format!(
            "Page d'ancrage de l'espace « {} » — sert de parent stable pour les propositions \
             MCP même si l'espace ne contient encore aucune autre page, ou si un space publié \
             pointe vers un instantané périmé. Ne la supprime pas.",
            space.name
        );
        let _ = insert_note_node_in(dir, anchor_id.clone(), root_id, label, content, None);
    }
    let ids = space.node_ids.get_or_insert_with(Vec::new);
    if !ids.contains(&anchor_id) { ids.push(anchor_id); }
}

/// Migration au démarrage : les spaces créés avant l'ajout du nœud d'ancrage
/// (ci-dessus) n'en ont pas encore — on la backfille une fois, silencieusement.
fn ensure_all_space_anchors(dir: &std::path::Path) {
    let mut spaces = load_spaces(dir);
    let before = spaces.clone();
    for s in spaces.iter_mut() { ensure_space_anchor(dir, s); }
    if spaces != before { save_spaces(dir, &spaces); }
}

#[tauri::command]
fn create_space(name: String) -> Result<Space, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut space = Space { id: format!("space_{ts}"), name, node_ids: Some(vec![]), updated_at: None };
    ensure_space_anchor(&dir, &mut space);
    let mut spaces = load_spaces(&dir);
    spaces.push(space.clone());
    save_spaces(&dir, &spaces);
    Ok(space)
}

#[tauri::command]
fn rename_space(id: String, name: String) -> Result<(), String> {
    if id == "lucid" { return Err("L'espace Lucid ne peut pas être renommé.".into()); }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let space = spaces.iter_mut().find(|s| s.id == id).ok_or("Espace introuvable.")?;
    space.name = name;
    save_spaces(&dir, &spaces);
    Ok(())
}

#[tauri::command]
fn delete_space(id: String) -> Result<(), String> {
    if id == "lucid" { return Err("L'espace Lucid ne peut pas être supprimé.".into()); }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let len_before = spaces.len();
    spaces.retain(|s| s.id != id);
    if spaces.len() == len_before { return Err("Espace introuvable.".into()); }
    save_spaces(&dir, &spaces);
    // Tombstone : la suppression doit tenir face à la fusion de sync (sinon
    // l'espace renaîtrait au prochain pull depuis l'autre machine).
    let mut dead: std::collections::HashSet<String> =
        std::fs::read_to_string(dir.join("deleted_spaces.json"))
            .ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    dead.insert(id);
    if let Ok(json) = serde_json::to_string_pretty(&dead) {
        let _ = std::fs::write(dir.join("deleted_spaces.json"), json);
    }
    Ok(())
}

#[tauri::command]
fn add_node_to_space(space_id: String, node_id: String) -> Result<(), String> {
    if space_id == "lucid" { return Err("L'espace Lucid est virtuel, pas de liste à modifier.".into()); }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let space = spaces.iter_mut().find(|s| s.id == space_id).ok_or("Espace introuvable.")?;
    let ids = space.node_ids.get_or_insert_with(Vec::new);
    // Un conteneur emmène toute sa descendance dans le space.
    for id in subtree_ids(&dir, &node_id) {
        if !ids.contains(&id) { ids.push(id); }
    }
    save_spaces(&dir, &spaces);
    Ok(())
}

#[tauri::command]
fn remove_node_from_space(space_id: String, node_id: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let space = spaces.iter_mut().find(|s| s.id == space_id).ok_or("Espace introuvable.")?;
    if let Some(ids) = &mut space.node_ids {
        let sub: std::collections::HashSet<String> = subtree_ids(&dir, &node_id).into_iter().collect();
        ids.retain(|id| !sub.contains(id));
    }
    save_spaces(&dir, &spaces);
    Ok(())
}

/// Ids du sous-arbre de `node_id` (lui inclus) d'après brain.json.
/// brain.json illisible → juste le nœud (dégradé, pas d'échec).
fn subtree_ids(dir: &std::path::Path, node_id: &str) -> Vec<String> {
    let graph: Option<BrainGraph> = backup::load_brain_cached(dir).ok();
    let Some(graph) = graph else { return vec![node_id.to_string()]; };
    let mut kids: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for n in &graph.nodes {
        if let Some(p) = &n.parent_id {
            kids.entry(p.as_str()).or_default().push(n.id.as_str());
        }
    }
    let mut out = vec![node_id.to_string()];
    let mut q = vec![node_id];
    while let Some(id) = q.pop() {
        for c in kids.get(id).into_iter().flatten() {
            out.push(c.to_string());
            q.push(c);
        }
    }
    out
}

#[tauri::command]
fn export_space_md(space_id: String) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let graph: BrainGraph = backup::load_brain_cached(&dir)?;

    let (space_name, node_ids): (String, Option<Vec<String>>) = if space_id == "lucid" {
        ("Lucid".into(), None)
    } else {
        let spaces = load_spaces(&dir);
        let s = spaces.into_iter().find(|s| s.id == space_id).ok_or("Espace introuvable.")?;
        let ids = s.node_ids.clone();
        (s.name, ids)
    };

    let nodes: Vec<&BrainNode> = match &node_ids {
        None => graph.nodes.iter().collect(),
        Some(ids) => graph.nodes.iter().filter(|n| ids.contains(&n.id)).collect(),
    };

    let mut md = format!("# {}\n\n", space_name);
    for node in nodes {
        md.push_str(&format!("## {}\n\n", node.label));
        if !node.summary.is_empty() { md.push_str(&format!("{}\n\n", node.summary)); }
        if !node.content.is_empty() { md.push_str(&format!("{}\n\n", node.content)); }
    }
    Ok(md)
}

// ─── Démo d'onboarding (explorer sans connecteur) ──────────────────────────────
//
// `seed_demo` écrit un brain.json + spaces.json factices pour explorer la carte
// sans source. Un fichier `demo.flag` marque ces données comme jetables :
// `reset_demo` n'efface QUE si le flag existe → jamais de vrai cerveau supprimé.

fn demo_leaf(id: &str, parent: &str, label: &str, content: &str) -> BrainNode {
    BrainNode { updated_at: None,
        id: id.into(), label: label.into(), kind: "leaf".into(), weight: 1,
        // Pas de summary : le bloc « Synthèse IA » ne doit pas mentir — l'user
        // pourra tester la vraie synthèse manuelle sur ces pages.
        summary: String::new(),
        keywords: vec![], decisions: vec![], patterns: vec![], community: 1,
        parent_id: Some(parent.into()), synthesized_at: None, date: None, content: content.into(),
        connector: None, source_id: None, source_project: None, source_text: String::new(),
    }
}

#[tauri::command]
fn seed_demo() -> Result<BrainGraph, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;

    let container = |id: &str, label: &str, weight: usize| BrainNode { updated_at: None,
        id: id.into(), label: label.into(), kind: "container".into(), weight,
        summary: String::new(), keywords: vec![], decisions: vec![], patterns: vec![],
        community: 1, parent_id: Some("root".into()), synthesized_at: None, date: None,
        content: String::new(), connector: None, source_id: None, source_project: None, source_text: String::new(),
    };

    let mut nodes = vec![
        BrainNode { updated_at: None,
            id: "root".into(), label: "Lucid".into(), kind: "root".into(), weight: 8,
            summary: "Contenu d'exemple — remplacé par tes vraies données au premier sync.".into(),
            keywords: vec![], decisions: vec![],
            patterns: vec![], community: 0, parent_id: None, synthesized_at: None, date: None,
            content: String::new(), connector: None, source_id: None, source_project: None, source_text: String::new(),
        },
        container("demo-guide", "Prise en main", 2),
        container("demo-projet", "Projet Alpha", 3),
        container("demo-cours", "Cours de chimie", 2),
        demo_leaf("demo-welcome", "demo-guide", "Bienvenue 👋",
"# Bienvenue dans Lucid

Ton second cerveau, **100 % local** — rien ne quitte cette machine.

## Explore

- Clique sur les bulles pour naviguer, ouvre une page, déplace-toi à la molette.
- ⌘K ouvre la recherche rapide.
- Regarde [[Plan de lancement]] : propriétés, tableau, tâches — une page peut tout porter.

## Crée

- `+` ou clic droit sur la carte → nouvelle page.
- Tape `[[` dans une page pour la lier à une autre : les liens deviennent des ponts sur la carte.
- **Glisse un PDF, Word ou CSV** directement sur la carte : il devient une page.

## Quand tu es prêt

Connecte une vraie source (voir [[Connecter tes sources]]) — ce contenu d'exemple s'effacera tout seul."),
        demo_leaf("demo-sources", "demo-guide", "Connecter tes sources",
"# Connecter tes sources

Lucid agrège tes outils en un seul cerveau : **Claude Code**, **Google Drive**, **Obsidian**…

1. Ouvre les Paramètres → Sources.
2. Connecte une source et lance un Sync.
3. Génère ton cerveau : la carte se reconstruit avec **tes** données.

Tout est analysé **en local** (llama.cpp). Ton cerveau est aussi consultable par tes IA via MCP."),
        demo_leaf("demo-plan", "demo-projet", "Plan de lancement",
"---
statut: En cours
échéance: 2026-09-15
tags: [produit, mvp]
---

# Plan de lancement

## Jalons

| Jalon | Responsable | Échéance | État |
| --- | --- | --- | --- |
| Prototype | Alex | Juin | ✅ Fait |
| Beta privée | Sam | Août | 🔶 En cours |
| Lancement public | Équipe | Septembre | ⬜ À venir |

## Tâches

- [x] Valider le concept avec 5 utilisateurs
- [ ] Rédiger la page d'accueil
- [ ] Préparer la démo produit

Contexte : voir [[Notes de réunion]] et [[Idées]]."),
        demo_leaf("demo-meeting", "demo-projet", "Notes de réunion",
"---
date: 2026-07-02
participants: [Alex, Sam]
---

# Réunion de lancement

- Objectif : valider le MVP avant la beta.
- Le [[Plan de lancement]] est la référence unique.

## Décisions

- Cible : indépendants et petites équipes.
- La beta privée passe avant toute nouvelle feature."),
        demo_leaf("demo-ideas", "demo-projet", "Idées",
"# Idées en vrac

- Mode présentation de la carte
- Export PDF par espace
- Raccourcis clavier personnalisables

À trier lors de la prochaine réunion — voir [[Notes de réunion]]."),
        demo_leaf("demo-atomes", "demo-cours", "Les atomes",
"# Les atomes

Un atome = noyau (protons + neutrons) + électrons.

| Élément | Symbole | Z |
| --- | --- | --- |
| Hydrogène | H | 1 |
| Carbone | C | 6 |
| Oxygène | O | 8 |

Le numéro atomique **Z** = nombre de protons."),
        demo_leaf("demo-reactions", "demo-cours", "Réactions chimiques",
"# Réactions chimiques

Une réaction conserve la masse (Lavoisier) : les atomes se réarrangent, rien ne se perd.

Exemple : combustion du méthane
`CH₄ + 2 O₂ → CO₂ + 2 H₂O`

Base : [[Les atomes]].

> 💡 Astuce : clique sur **Synthétiser** sur la bulle « Cours de chimie » pour voir l'IA locale résumer ces pages."),
    ];
    nodes.shrink_to_fit();

    let edge = |src: &str, tgt: &str| BrainEdge {
        source: src.into(), target: tgt.into(), kind: "contains".into(), relation: "contains".into(),
    };
    let edges = vec![
        edge("root", "demo-guide"), edge("root", "demo-projet"), edge("root", "demo-cours"),
        edge("demo-guide", "demo-welcome"), edge("demo-guide", "demo-sources"),
        edge("demo-projet", "demo-plan"), edge("demo-projet", "demo-meeting"), edge("demo-projet", "demo-ideas"),
        edge("demo-cours", "demo-atomes"), edge("demo-cours", "demo-reactions"),
    ];

    let mut graph = BrainGraph {
        nodes, edges,
        markdown: "# Lucid — contenu d'exemple\n\nGraphe starter pour la prise en main.".into(),
        report: String::new(),
        generated_at: "demo".into(),
    };

    backup::write_brain(&dir, &mut graph)?;
    save_spaces(&dir, &[Space {
        id: "space_demo".into(), name: "Projet Alpha".into(),
        node_ids: Some(vec!["demo-plan".into(), "demo-meeting".into(), "demo-ideas".into()]),
        updated_at: None,
    }]);
    std::fs::write(dir.join("demo.flag"), "1").map_err(|e| e.to_string())?;
    Ok(graph)
}

/// Efface les données de démo et remet à zéro — no-op si ce n'est pas une démo.
#[tauri::command]
fn reset_demo() -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    if !dir.join("demo.flag").exists() { return Ok(()); }
    for f in ["brain.json", "spaces.json", "demo.flag"] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    Ok(())
}

/// Remet l'environnement à zéro pour rejouer l'onboarding : vide le dossier de
/// données SAUF l'IA locale (llama.cpp, modèles ~2,3 Go) pour éviter le re-download.
#[tauri::command]
fn reset_environment() -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    const KEEP: [&str; 4] = ["llama.cpp", "models", "model_config.json", "model_catalog.json"];
    for entry in std::fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        if KEEP.iter().any(|k| name == *k) {
            continue;
        }
        let path = entry.path();
        let res = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        res.map_err(|e| format!("Impossible de supprimer {} : {e}", path.display()))?;
    }
    // Claude Code s'auto-détecte via ~/.claude/projects : sans ce flag il
    // reviendrait « connecté » après reset. On repart déconnecté (réactivable
    // dans Settings → Connecteurs), comme un utilisateur vierge.
    let _ = std::fs::write(dir.join("claude_code_disabled"), "");
    // Le process Tauri ne redémarre pas au reload du front : sans vider le
    // cache mémoire, une lecture juste après reconnexion renvoie l'ancien
    // cerveau depuis la RAM, disque vide ou pas.
    backup::clear_cache();
    Ok(())
}

// ── Isolation par compte ──────────────────────────────────────────────────────
// Les données utilisateur (brain, spaces, connecteurs, tokens) vivent dans
// `users/<uuid>/` ; les assets machine (modèles, llama.cpp) restent à la racine.
// Le front appelle cette commande au login (user_id) et au logout (null) AVANT
// de monter l'app — tous les accès disque passent par `app_data_dir()` qui lit
// le fichier `active_user`.

/// Fichiers/dossiers de la racine qui appartiennent à la machine, pas au compte.
const SHARED_ENTRIES: &[&str] = &[
    "llama.cpp", "models", "model_catalog.json", "model_config.json",
    "users", "active_user",
];

/// Renvoie `true` si le compte est neuf sur cette machine (dossier créé à
/// l'instant, hors migration legacy) — le front relance alors l'onboarding.
#[tauri::command]
fn set_active_user(user_id: Option<String>) -> Result<bool, String> {
    let root = ai::llama::shared_data_dir().ok_or("Dossier de données introuvable.")?;
    set_active_user_in(&root, user_id)
}

fn set_active_user_in(root: &std::path::Path, user_id: Option<String>) -> Result<bool, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let marker = root.join("active_user");

    let Some(id) = user_id.filter(|id| !id.trim().is_empty()) else {
        // Logout : plus d'utilisateur actif → le MCP et l'app ne voient plus le cerveau.
        return match std::fs::remove_file(&marker) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(false),
        };
    };
    // Le uuid vient de Supabase mais sert de nom de dossier : on refuse tout séparateur.
    let id = id.trim().to_string();
    if id.contains(['/', '\\', '.']) {
        return Err("Identifiant utilisateur invalide.".to_string());
    }

    let user_dir = root.join("users").join(&id);

    // Migration one-shot d'une install legacy : le premier compte qui se connecte
    // hérite du cerveau existant de la machine (sinon il serait perdu).
    let legacy = root.join("brain.json").exists() && !root.join("users").exists();
    let fresh = !user_dir.exists() && !legacy;
    if legacy && !user_dir.exists() {
        std::fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(root).map_err(|e| e.to_string())?.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            if SHARED_ENTRIES.iter().any(|s| name == std::ffi::OsStr::new(s)) { continue; }
            std::fs::rename(entry.path(), user_dir.join(&name))
                .map_err(|e| format!("Migration de {:?} impossible : {e}", name))?;
        }
    }

    std::fs::create_dir_all(&user_dir).map_err(|e| e.to_string())?;
    if fresh {
        // Compte neuf : les sources de la machine ne sont pas aspirées d'office.
        // Claude Code démarre déconnecté (opt-in dans Connexions) — sinon la
        // première génération ingère les conversations des autres comptes.
        let _ = std::fs::write(user_dir.join("claude_code_disabled"), "");
    }
    std::fs::write(&marker, &id).map_err(|e| e.to_string())?;
    Ok(fresh)
}

#[cfg(test)]
mod active_user_tests {
    use super::*;

    fn fresh_root(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn migre_le_legacy_vers_le_premier_compte_puis_isole_le_second() {
        let root = fresh_root("lucid_test_active_user");
        // Install legacy : cerveau + asset machine à la racine.
        std::fs::write(root.join("brain.json"), "{}").unwrap();
        std::fs::create_dir_all(root.join("models")).unwrap();
        std::fs::write(root.join("models/m.gguf"), "x").unwrap();

        let fresh = set_active_user_in(&root, Some("user-a".into())).unwrap();
        assert!(!fresh, "une migration legacy n'est pas un compte neuf");
        assert!(root.join("users/user-a/brain.json").exists(), "legacy migré vers le 1er compte");
        assert!(!root.join("brain.json").exists());
        assert!(root.join("models/m.gguf").exists(), "les assets machine restent partagés");
        assert!(!root.join("users/user-a/claude_code_disabled").exists(), "le compte migré garde ses connecteurs");
        assert_eq!(std::fs::read_to_string(root.join("active_user")).unwrap(), "user-a");

        // 2e compte : dossier vierge, pas de fuite du cerveau de user-a,
        // et les sources machine ne sont pas aspirées d'office.
        let fresh = set_active_user_in(&root, Some("user-b".into())).unwrap();
        assert!(fresh, "user-b est neuf sur cette machine");
        assert!(root.join("users/user-b").is_dir());
        assert!(!root.join("users/user-b/brain.json").exists(), "user-b ne voit pas le brain de user-a");
        assert!(root.join("users/user-b/claude_code_disabled").exists(), "compte neuf : Claude Code opt-in");

        // Logout : plus de marqueur → retombe sur la racine (vide de données user).
        set_active_user_in(&root, None).unwrap();
        assert!(!root.join("active_user").exists());

        // Re-login user-a : retrouve son cerveau, aucune re-migration, pas « neuf ».
        let fresh = set_active_user_in(&root, Some("user-a".into())).unwrap();
        assert!(!fresh);
        assert!(root.join("users/user-a/brain.json").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuse_un_user_id_avec_separateurs() {
        let root = fresh_root("lucid_test_active_user_bad_id");
        assert!(set_active_user_in(&root, Some("../evil".into())).is_err());
        assert!(!root.join("active_user").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

// ── Archiviste : autonome par défaut, validation manuelle en option ──────────
// Fichier-drapeau (comme la télémétrie) mais sémantique inversée : absent =
// autonome (aucun fichier à créer à l'installation, autonomie gratuite par
// défaut) ; présent = chaque écriture repasse par les bulles fantômes.

fn mcp_manual_validation_flag() -> Option<std::path::PathBuf> {
    ai::llama::app_data_dir().map(|d| d.join("mcp_manual_validation"))
}

#[tauri::command]
fn mcp_manual_validation_enabled() -> bool {
    mcp_manual_validation_flag().map(|p| p.exists()).unwrap_or(false)
}

#[tauri::command]
fn set_mcp_manual_validation(enabled: bool) -> Result<(), String> {
    let p = mcp_manual_validation_flag().ok_or("Dossier de données introuvable.")?;
    if enabled {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&p, "").map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

// ── Crash reporting (Sentry) — opt-in strict ─────────────────────────────────
// Promesse produit « 100 % local » : sans le flag de consentement ET un DSN,
// Sentry n'est jamais initialisé → zéro connexion sortante.

/// Fichier-drapeau : présent = l'utilisateur a accepté l'envoi des rapports de crash.
fn telemetry_flag() -> Option<std::path::PathBuf> {
    ai::llama::app_data_dir().map(|d| d.join("telemetry_enabled"))
}

#[tauri::command]
fn telemetry_enabled() -> bool {
    telemetry_flag().map(|p| p.exists()).unwrap_or(false)
}

/// Active/désactive l'envoi des rapports de crash (effet au redémarrage de l'app).
#[tauri::command]
fn set_telemetry(enabled: bool) -> Result<(), String> {
    let p = telemetry_flag().ok_or("Dossier de données introuvable.")?;
    if enabled {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(&p, "").map_err(|e| e.to_string())
    } else {
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}

/// Sentry réellement initialisé ce démarrage ? (≠ du flag : il faut aussi un DSN + redémarrage)
static SENTRY_ACTIVE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

#[tauri::command]
fn sentry_active() -> bool {
    *SENTRY_ACTIVE.get().unwrap_or(&false)
}

/// DSN : env runtime (`.env` en dev) puis compile-time (CI de release). Vide = désactivé.
fn sentry_dsn() -> Option<String> {
    std::env::var("SENTRY_DSN")
        .ok()
        .or_else(|| option_env!("SENTRY_DSN").map(str::to_string))
        .filter(|s| !s.trim().is_empty())
}

/// Anonymise un event avant envoi : le home dir (= username) devient `~`.
fn scrub_event(event: &mut sentry::protocol::Event<'static>) {
    // Le hostname contient souvent le nom de l'utilisateur (« MacBook-Pro-de-… »).
    event.server_name = None;
    // Chemins absolus des binaires chargés (/Users/<nom>/…) — sans utilité tant
    // qu'on n'upload pas de dSYM pour la symbolication serveur.
    event.debug_meta.to_mut().images.clear();

    let Some(home) = dirs::home_dir().map(|h| h.to_string_lossy().into_owned()) else {
        return;
    };
    let fix = |s: &mut String| {
        if s.contains(&home) {
            *s = s.replace(&home, "~");
        }
    };
    if let Some(m) = event.message.as_mut() {
        fix(m);
    }
    for exc in event.exception.values.iter_mut() {
        if let Some(v) = exc.value.as_mut() {
            fix(v);
        }
        if let Some(st) = exc.stacktrace.as_mut() {
            for f in st.frames.iter_mut() {
                if let Some(p) = f.abs_path.as_mut() {
                    fix(p);
                }
                if let Some(p) = f.filename.as_mut() {
                    fix(p);
                }
            }
        }
    }
    for b in event.breadcrumbs.values.iter_mut() {
        if let Some(m) = b.message.as_mut() {
            fix(m);
        }
    }
}

/// Panic volontaire pour tester la chaîne Sentry de bout en bout (dev uniquement).
/// Le message contient un chemin home réel → vérifie aussi le scrub (`~` attendu côté Sentry).
#[tauri::command]
fn crash_test() -> Result<(), String> {
    if cfg!(debug_assertions) {
        // Panic dans un thread secondaire : le hook Sentry capture pareil, mais
        // l'app survit (un panic sur le main thread aborterait le process avant
        // que le transport n'ait le temps d'envoyer l'event).
        std::thread::spawn(|| {
            let p = dirs::home_dir()
                .map(|h| h.join("note-secrète.md").display().to_string())
                .unwrap_or_default();
            panic!("Test Sentry : panic volontaire (scrub check : {p})");
        });
        return Ok(());
    }
    Err("Disponible uniquement en dev.".into())
}

/// Une seule génération à la fois (bouton Générer vs watch auto).
/// ponytail: lock global — suffisant tant qu'il n'y a qu'un cerveau par machine.
static GEN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Lance le pipeline IA : analyse toutes les conversations et construit le graphe.
#[tauri::command]
async fn generate_brain(app: tauri::AppHandle) -> Result<BrainGraph, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _gen = GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        run_generation(&app)
    })
    .await
    .map_err(|e| format!("Tâche d'analyse interrompue : {e}"))?
}

/// Cœur bloquant de la génération (commande Tauri + watch auto).
/// Un vrai cerveau existe-t-il pour ce compte ? `brain.db` après la migration
/// SQLite (Phase 8), `brain.json` avant. Les gardes des régénérations auto ne
/// testaient QUE `brain.json` → cassées en silence pour tout compte migré (la
/// migration renomme `brain.json` en `brain.json.migre-*`), donc watch auto ET
/// rattrapage démarrage ne se déclenchaient plus jamais (bug remonté par Liam le
/// 2026-07-28 : note + fichiers ajoutés jamais ingérés).
fn has_real_brain(dir: &std::path::Path) -> bool {
    dir.join("brain.db").exists() || dir.join("brain.json").exists()
}

/// Rafraîchit les caches des connecteurs à cache (Notes Apple, Drive, dossiers
/// locaux) — `run_generation` lit ces caches, PAS la source en direct. Sans ça,
/// un ajout/une modif hors-ligne n'apparaît jamais, même via « Générer » (bug
/// remonté par Liam le 2026-07-28). Best-effort, jamais bloquant : si un
/// connecteur échoue (permission révoquée, réseau coupé…) on log et on continue
/// avec les autres, jamais un échec silencieux (ADR-0015).
/// ponytail: re-sync à chaque génération ; incrémental donc bon marché (mtime
/// pour les fichiers, seulement les docs Drive modifiés). Si ça devient trop
/// lourd un jour, gater sur un fingerprint par connecteur.
fn refresh_connector_caches() {
    if connectors::apple_notes::is_connected() {
        if let Err(e) = connectors::apple_notes::sync() { crate::elog!("⚠️ sync Notes Apple : {e}"); }
    }
    if connectors::google_drive::is_connected() {
        if let Err(e) = connectors::google_drive::sync_docs() { crate::elog!("⚠️ sync Drive : {e}"); }
    }
    if connectors::local_folder::is_connected() {
        if let Err(e) = connectors::local_folder::sync(|_, _, _| {}) { crate::elog!("⚠️ sync dossiers locaux : {e}"); }
    }
}

/// Ré-applique par-dessus le graphe fraîchement régénéré les surcharges que le
/// pipeline (déterministe, dérivé des seules sources) ignore : contenu édité et
/// parent choisi par l'Archiviste/l'utilisateur (déplacements). Nœuds seulement —
/// les arêtes de contenance sont reconstruites ensuite depuis `parent_id`.
fn reapply_overrides(
    graph: &mut BrainGraph,
    saved_content: &std::collections::HashMap<String, String>,
    saved_parent: &std::collections::HashMap<String, Option<String>>,
) {
    for node in &mut graph.nodes {
        if node.content.is_empty() {
            if let Some(c) = saved_content.get(&node.id) { node.content = c.clone(); }
        }
        if let Some(p) = saved_parent.get(&node.id) { node.parent_id = p.clone(); }
    }
}

fn run_generation(app: &tauri::AppHandle) -> Result<BrainGraph, String> {
    {
        // Sources rafraîchies AVANT lecture des caches — sinon on régénère à partir
        // de données figées (note/fichier ajouté hors app jamais ingéré).
        refresh_connector_caches();
        // Sans IA locale la génération marche quand même (structure + texte source).
        let engine = LlamaEngine::detect().ok();
        let mut convs = load_all_conversations();
        // Conversations supprimées par l'utilisateur : exclues de l'ingestion,
        // sinon chaque régénération ressusciterait ce qu'il a effacé.
        let tombstones = ai::llama::app_data_dir()
            .map(|d| load_tombstones(&d))
            .unwrap_or_default();
        if !tombstones.is_empty() {
            convs.retain(|c| !tombstones.contains(&format!("leaf:{}", c.summary.id)));
        }
        if convs.is_empty() {
            return Err("Aucune conversation à analyser.".to_string());
        }

        // Une vraie génération n'est plus une démo : retire le flag pour qu'un
        // reset_demo ultérieur ne puisse jamais effacer ce cerveau réel.
        // Le contenu starter (et son espace de démo) est jetable : dès qu'une
        // donnée connecteur arrive, on repart de zéro — rien n'est préservé.
        let was_demo = ai::llama::app_data_dir()
            .map(|d| d.join("demo.flag").exists())
            .unwrap_or(false);
        if was_demo {
            if let Some(d) = ai::llama::app_data_dir() {
                let _ = std::fs::remove_file(d.join("demo.flag"));
                let _ = std::fs::remove_file(d.join("spaces.json"));
            }
        }

        // Préserve l'état existant avant que le pipeline écrase brain.json :
        //  - contenu édité (tout nœud) ;
        //  - TOUT nœud absent de la nouvelle génération (pass-through) : notes
        //    manuelles, imports, et surtout les nœuds venus d'une AUTRE machine
        //    via la sync — le pipeline ne connaît que les sources locales, il ne
        //    doit reconstruire que ce qui en vient. Une suppression = tombstone,
        //    jamais une absence de régénération.
        // Sauf en sortie de démo : le starter ne doit jamais fuiter dans le vrai cerveau.
        // Passe par load_brain_cached (lit brain.db si migré, brain.json sinon) —
        // une lecture directe de brain.json aurait silencieusement renvoyé "aucun
        // état existant" pour tout compte déjà migré vers SQLite (le fichier a été
        // renommé), effaçant notes manuelles et contenu édité à la régénération
        // suivante (trouvé le 2026-07-22 en creusant la Phase 6, jamais déclenché
        // en pratique mais aurait été un vrai bug silencieux).
        let prev_graph = if was_demo { None } else {
            ai::llama::app_data_dir().and_then(|d| backup::load_brain_cached(&d).ok())
        };
        let saved_content: std::collections::HashMap<String, String> = prev_graph
            .as_ref()
            .map(|g| g.nodes.iter()
                .filter(|n| !n.content.is_empty())
                .map(|n| (n.id.clone(), n.content.clone()))
                .collect())
            .unwrap_or_default();
        let carried: Vec<BrainNode> = prev_graph
            .as_ref()
            .map(|g| g.nodes.iter().filter(|n| n.kind != "root").cloned().collect())
            .unwrap_or_default();
        // Structure décidée par l'Archiviste / l'utilisateur : tout nœud
        // explicitement modifié (`updated_at` posé — un « move » MCP change
        // parent_id, donc write_brain le ré-estampille) garde SON parent à la
        // régénération. Sans ça le pipeline le re-rattache à son conteneur
        // d'origine (dérivé de la source) et efface tous les déplacements — la
        // régénération détruisait la réorganisation de l'Archiviste (bug remonté
        // par Liam le 2026-07-28).
        let saved_parent: std::collections::HashMap<String, Option<String>> = carried.iter()
            .filter(|n| n.updated_at.is_some())
            .map(|n| (n.id.clone(), n.parent_id.clone()))
            .collect();
        let prev_links: Vec<BrainEdge> = prev_graph
            .map(|g| g.edges.into_iter().filter(|e| e.kind != "contains").collect())
            .unwrap_or_default();

        let cache_path = ai::llama::app_data_dir().map(|d| d.join("brain_cache.json"));
        let mut graph = pipeline::generate_brain(
            engine.as_ref(),
            &convs,
            cache_path.as_deref(),
            |p| {
                let _ = app.emit(
                    "brain-progress",
                    serde_json::json!({
                        "current": p.current,
                        "total": p.total,
                        "label": p.label,
                    }),
                );
            },
            |label, idx, total| {
                let _ = app.emit(
                    "brain-node",
                    serde_json::json!({ "label": label, "index": idx, "total": total }),
                );
            },
            |nodes, edges| {
                // Graphe vivant : version allégée (sans source_text/content, trop
                // lourds pour l'IPC répété) — juste de quoi dessiner les bulles.
                let slim: Vec<serde_json::Value> = nodes.iter().map(|n| serde_json::json!({
                    "id": n.id, "label": n.label, "kind": n.kind, "weight": n.weight,
                    "summary": n.summary, "keywords": [], "decisions": [], "patterns": [],
                    "community": n.community, "parent_id": n.parent_id,
                })).collect();
                let _ = app.emit("brain-partial", serde_json::json!({ "nodes": slim, "edges": edges }));
            },
        )?;

        // Réinjecte le contenu utilisateur + la structure préservée (cf. saved_parent).
        reapply_overrides(&mut graph, &saved_content, &saved_parent);

        // Pass-through : ré-ajoute tel quel tout nœud absent de la nouvelle
        // génération (l'estampille updated_at reste intacte → la fusion de sync
        // ne les considère pas comme « modifiés ici »). Les wikilinks/ponts
        // précédents sont conservés ; l'arbre (contains) sera reconstruit depuis
        // parent_id, les parents disparus rattachés à la racine.
        let existing: std::collections::HashSet<String> =
            graph.nodes.iter().map(|n| n.id.clone()).collect();
        for node in carried {
            if !existing.contains(&node.id) { graph.nodes.push(node); }
        }
        graph.edges.extend(prev_links);

        // Filet : les nœuds dérivés (projets, concepts) tombstonés ne doivent pas
        // réapparaître même si leurs conversations restantes les régénèrent.
        if !tombstones.is_empty() {
            graph.nodes.retain(|n| !tombstones.contains(&n.id));
        }
        backup::rebuild_tree_edges(&mut graph);

        if let Some(dir) = ai::llama::app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            save_snapshot_in(&dir, "regenerate"); // snapshot avant écrasement
            let _ = std::fs::write(dir.join("brain.md"), &graph.markdown);
            let _ = std::fs::write(dir.join("brain_report.md"), &graph.report);
            let mut stamped = graph.clone();
            let _ = backup::write_brain(&dir, &mut stamped);
            let existing: std::collections::HashSet<String> = stamped.nodes.iter().map(|n| n.id.clone()).collect();
            purge_dead_space_ids(&dir, &existing);
        }
        Ok(graph)
    }
}

/// Configure le vault Obsidian (chemin local).
#[tauri::command]
fn obsidian_set_vault(path: String) -> Result<(), String> {
    connectors::obsidian::set_vault(&path)
}

/// Renvoie le chemin du vault Obsidian configuré (None si pas encore configuré).
#[tauri::command]
fn obsidian_vault_path() -> Option<String> {
    connectors::obsidian::vault_path()
}

/// Déconnecte Obsidian (supprime la config locale).
#[tauri::command]
fn obsidian_disconnect() {
    connectors::obsidian::disconnect();
}

/// Détecte le vault Obsidian le plus récent (lit la config d'Obsidian
/// lui-même) et le connecte automatiquement. Renvoie le chemin connecté,
/// `None` si Obsidian n'a jamais tourné sur cette machine.
#[tauri::command]
fn obsidian_auto_connect() -> Option<String> {
    connectors::obsidian::auto_connect()
}

/// Première connexion : synchronise les notes Apple (déclenche le prompt
/// d'autorisation macOS au premier appel) et marque comme connecté. Renvoie
/// le nombre de notes importées. Mac uniquement.
#[tauri::command]
async fn apple_notes_connect() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(connectors::apple_notes::connect)
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Resynchronise les notes Apple (réécrit le cache en entier).
#[tauri::command]
async fn apple_notes_sync() -> Result<usize, String> {
    tauri::async_runtime::spawn_blocking(connectors::apple_notes::sync)
        .await
        .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Déconnecte les notes Apple (supprime config + cache).
#[tauri::command]
fn apple_notes_disconnect() {
    connectors::apple_notes::disconnect();
}

/// Première connexion : ajoute automatiquement Bureau/Documents/Téléchargements
/// (ceux qui existent). Idempotent si déjà connecté. Renvoie la liste des dossiers.
#[tauri::command]
fn local_folder_connect() -> Result<Vec<String>, String> {
    connectors::local_folder::connect()
}

/// Renvoie les dossiers actuellement configurés (vide si pas encore connecté).
#[tauri::command]
fn local_folder_list() -> Vec<String> {
    connectors::local_folder::folders()
}

/// Ajoute un dossier supplémentaire à indexer. Renvoie la liste à jour.
#[tauri::command]
fn local_folder_add(path: String) -> Result<Vec<String>, String> {
    connectors::local_folder::add_folder(&path)
}

/// Retire un dossier de la liste (le disque n'est pas touché). Renvoie la liste à jour.
#[tauri::command]
fn local_folder_remove(path: String) -> Result<Vec<String>, String> {
    connectors::local_folder::remove_folder(&path)
}

/// Déconnecte tous les dossiers locaux (supprime config + cache).
#[tauri::command]
fn local_folder_disconnect() {
    connectors::local_folder::disconnect();
}

/// Synchronise tous les dossiers locaux configurés (extraction incrémentale).
/// Émet `local-folder-progress` (current/total/label) fichier par fichier —
/// affiché en direct à l'onboarding pendant le scan. L'extraction PDF/OCR peut
/// être longue → thread bloquant, pas le thread UI.
#[tauri::command]
async fn local_folder_sync(app: tauri::AppHandle) -> Result<connectors::local_folder::SyncReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        connectors::local_folder::sync(|current, total, label| {
            let _ = app.emit("local-folder-progress", serde_json::json!({
                "current": current, "total": total, "label": label,
            }));
        })
    })
    .await
    .map_err(|e| format!("Tâche de sync interrompue : {e}"))?
}

/// Charge les variables depuis `.env.local` / `.env` (CWD et dossiers parents).
/// Utilisé en dev : les apps macOS GUI n'héritent pas des env vars du shell.
/// Ne remplace pas une variable déjà définie.
fn load_env_local() {
    // Cherche dans le CWD puis ses parents (couvre src-tauri/ ET la racine du projet).
    let names = [".env.local", ".env"];
    let mut dir = std::env::current_dir().unwrap_or_default();
    loop {
        for name in &names {
            let path = dir.join(name);
            if let Ok(content) = std::fs::read_to_string(&path) {
                parse_env(&content);
                return;
            }
        }
        if !dir.pop() { break; }
    }
}

fn parse_env(content: &str) {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') { continue; }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let v = v.trim().trim_matches('"').trim_matches('\'');
            if std::env::var(k).is_err() {
                unsafe { std::env::set_var(k, v); }
            }
        }
    }
}

/// Watch auto : surveille `~/.claude/projects/` et régénère le cerveau tout seul
/// quand une session Claude Code change. Cross-platform (crate notify).
/// Watch du dossier de données : toute écriture dans le périmètre de backup
/// (brain.json, notes, snapshots…) émet `user-data-changed` (débouncé) — le
/// front pousse alors la sync cloud immédiatement, quelle que soit l'origine
/// de la modif (commande Tauri, watch auto, MCP, import). Une seule porte,
/// pas d'instrumentation par mutation.
fn start_data_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use notify::Watcher;
        use tauri::Emitter;
        let Some(root) = ai::llama::shared_data_dir() else { return };
        // Dans le périmètre de sync ? (exclut models/, llama.cpp/, fichiers témoins…)
        fn in_scope(p: &std::path::Path) -> bool {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if backup::FILES.contains(&name) { return true; }
            }
            p.components().any(|c| matches!(c.as_os_str().to_str(), Some(d) if backup::DIRS.contains(&d)))
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                if ev.paths.iter().any(|p| in_scope(p)) { let _ = tx.send(()); }
            }
        }) {
            Ok(w) => w,
            Err(e) => { crate::elog!("⚠️ watch données indisponible : {e}"); return; }
        };
        if let Err(e) = watcher.watch(&root, notify::RecursiveMode::Recursive) {
            crate::elog!("⚠️ watch données indisponible : {e}");
            return;
        }
        while rx.recv().is_ok() {
            // Debounce par sleep (pas un while-drain qui spinnerait à 100% CPU sous
            // une rafale d'écritures, cf. le fix de `start_watcher` le 2026-07-29).
            std::thread::sleep(std::time::Duration::from_secs(2));
            while rx.try_recv().is_ok() {}
            let _ = app.emit("user-data-changed", ());
        }
    });
}

/// Epoch (secondes) de la dernière régénération auto — borne la fréquence
/// ci-dessous, indépendamment du nombre de rafales détectées par le watcher.
static LAST_AUTO_REGEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Chemins fs surveillables MAINTENANT (source → chemin) — recalculé à chaque
/// tour plutôt qu'une seule fois au démarrage : un dossier ajouté/retiré dans
/// Réglages, ou Notes Apple connectée en cours de route, doit rejoindre le
/// watch sans relancer l'app. Le pipeline (`run_generation`) agrège déjà
/// toutes les sources d'un coup — peu importe QUI a déclenché, une seule
/// régénération suffit, jamais une par connecteur.
fn fs_watch_targets() -> Vec<(&'static str, std::path::PathBuf)> {
    let mut out = Vec::new();
    if let Some(d) = connectors::claude_code::projects_dir() { out.push(("claude-code", d)); }
    for f in connectors::local_folder::folders() { out.push(("local-folder", std::path::PathBuf::from(f))); }
    if let Some(v) = connectors::obsidian::vault_path() { out.push(("obsidian", std::path::PathBuf::from(v))); }
    // PAS de watch fs sur `NoteStore.sqlite` : Notes.app écrit dans `-wal` à
    // chaque ouverture de l'app, sync iCloud ou checkpoint SQLite, sans qu'une
    // seule note ait changé. Notes Apple passe uniquement par le sondage
    // (`changed_fingerprint`, plus bas), qui compare les notes réelles.
    out
}

/// Le chemin touché par un évènement fs vaut-il déclencheur ? Un filtre par
/// source (extension ou nom de fichier) pour ignorer le bruit — chaque
/// connecteur a son propre format de fichiers pertinents.
fn fs_event_relevant(p: &std::path::Path) -> bool {
    if p.extension().is_some_and(|e| e == "jsonl" || e == "md") { return true; }
    connectors::local_folder::EXTENSIONS.iter().any(|ext| p.extension().is_some_and(|e| e == *ext))
}

/// Le changement porte-t-il sur un contenu DOCUMENT (fichier à ranger) plutôt
/// qu'une conversation Claude Code ? Sert à ne réveiller l'Archiviste que sur du
/// vrai contenu à organiser — jamais sur une session Claude Code (`.jsonl` : le
/// seul format de conversation, et la cause de l'emballement du 2026-07-29 quand
/// on relançait l'Archiviste à chaque écriture de session). Obsidian (.md), les
/// fichiers locaux (pdf/docx…) et Notes Apple (NoteStore) sont des documents.
fn fs_event_is_document(p: &std::path::Path) -> bool {
    fs_event_relevant(p) && !p.extension().is_some_and(|e| e == "jsonl")
}

// ── Inbox : flux passif des fichiers récemment détectés ─────────────────────
// Visibilité pure (ajouté/modifié/supprimé) — PAS une étape de validation. En
// mémoire (ring buffer, reset au redémarrage) : suffisant pour "qu'est-ce qui
// vient de bouger", pas besoin de persistance.
#[derive(Clone, serde::Serialize)]
struct InboxEntry {
    name: String,   // nom de fichier (affiché) — le chemin complet reste local (clic pour ouvrir)
    path: String,   // chemin absolu — usage LOCAL uniquement (ouvrir le fichier), jamais exporté
    kind: String,   // "added" | "modified" | "deleted" (dernier événement)
    source: String, // "local" | "obsidian" | "apple-notes" | "claude-code"
    at: u64,        // epoch secondes (dernier événement)
    count: u32,     // nombre d'événements sur ce fichier (dédup → bulle ×N)
}
const INBOX_CAP: usize = 60;

fn inbox() -> &'static std::sync::Mutex<std::collections::VecDeque<InboxEntry>> {
    static I: std::sync::OnceLock<std::sync::Mutex<std::collections::VecDeque<InboxEntry>>> = std::sync::OnceLock::new();
    I.get_or_init(|| std::sync::Mutex::new(std::collections::VecDeque::new()))
}

fn inbox_record(path: &std::path::Path, kind: &str) {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else { return };
    if name.is_empty() { return; }
    let has_ext = |e: &str| path.extension().is_some_and(|x| x == e);
    let source = if has_ext("jsonl") { "claude-code" }
        else if has_ext("md") { "obsidian" }
        else { "local" };
    inbox_push(name, path.to_string_lossy().to_string(), source, kind);
}

/// Variante sans fichier sur le disque : une source qui n'expose pas de chemin
/// (Notes Apple, détectée par sondage). Le `path` reste vide — le front ne
/// propose alors pas « ouvrir le fichier ».
fn inbox_record_named(name: &str, source: &str, kind: &str) {
    if name.trim().is_empty() { return; }
    inbox_push(name.to_string(), String::new(), source, kind);
}

fn inbox_push(name: String, path_str: String, source: &str, kind: &str) {
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Dédup par chemin quand il y en a un, par nom sinon (sources sans fichier).
    let dedup_key = if path_str.is_empty() { name.clone() } else { path_str.clone() };
    let mut q = inbox().lock().unwrap_or_else(|p| p.into_inner());
    // Dédup par fichier : une seule ligne par chemin (dernière activité), avec un
    // compteur ×N repris de l'entrée précédente — une session Claude Code qui
    // réécrit en continu devient "×47" au lieu d'une rafale.
    let key_of = |e: &InboxEntry| if e.path.is_empty() { e.name.clone() } else { e.path.clone() };
    let prev_count = q.iter().find(|e| key_of(e) == dedup_key).map(|e| e.count).unwrap_or(0);
    q.retain(|e| key_of(e) != dedup_key);
    q.push_front(InboxEntry { name, path: path_str, kind: kind.into(), source: source.into(), at, count: prev_count + 1 });
    q.truncate(INBOX_CAP);
}

/// Flux des fichiers récemment détectés (plus récent d'abord).
#[tauri::command]
fn inbox_recent() -> Vec<InboxEntry> {
    inbox().lock().map(|q| q.iter().cloned().collect()).unwrap_or_default()
}

/// Entre deux tours : resynchronise les chemins fs surveillés (Réglages a pu
/// changer) ET sonde les sources sans signal fs (Drive, Notes Apple en
/// signal fs : Notes Apple, Google Drive).
const WATCH_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(300);

fn start_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use notify::Watcher;
        // Le bool = "un contenu DOCUMENT a changé" (→ réveille l'Archiviste).
        // false = seulement une source non-document (session Claude Code) : on
        // régénère mais on ne range pas.
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let tx_events = tx.clone();
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if let Ok(ev) = res {
                if ev.paths.iter().any(|p| fs_event_relevant(p)) {
                    // Inbox (visibilité) : enregistre chaque fichier pertinent avec son type d'événement.
                    let kind = match ev.kind {
                        notify::EventKind::Create(_) => Some("added"),
                        notify::EventKind::Modify(_) => Some("modified"),
                        notify::EventKind::Remove(_) => Some("deleted"),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        for p in ev.paths.iter().filter(|p| fs_event_relevant(p)) { inbox_record(p, k); }
                    }
                    let is_doc = ev.paths.iter().any(|p| fs_event_is_document(p));
                    let _ = tx_events.send(is_doc);
                }
            }
        }) {
            Ok(w) => w,
            Err(e) => { crate::elog!("⚠️ watch auto indisponible : {e}"); return; }
        };
        let mut watched: std::collections::HashMap<&'static str, std::path::PathBuf> = std::collections::HashMap::new();
        let mut fingerprints: std::collections::HashMap<&'static str, String> = std::collections::HashMap::new();
        let mut last_poll: Option<std::time::Instant> = None;

        loop {
            let targets = fs_watch_targets();
            let target_keys: std::collections::HashSet<&str> = targets.iter().map(|(k, _)| *k).collect();
            // Retire ce qui n'est plus une cible valide (dossier supprimé, connecteur coupé).
            let stale: Vec<&'static str> = watched.keys().filter(|k| !target_keys.contains(*k)).copied().collect();
            for k in stale { if let Some(p) = watched.remove(k) { let _ = watcher.unwatch(&p); } }
            // Ajoute les nouvelles cibles / change de chemin si reconfiguré.
            for (source, path) in &targets {
                if watched.get(source) == Some(path) { continue; }
                if let Some(old) = watched.get(source) { let _ = watcher.unwatch(old); }
                match watcher.watch(path, notify::RecursiveMode::Recursive) {
                    Ok(()) => { watched.insert(source, path.clone()); }
                    Err(e) => crate::elog!("⚠️ watch auto : {source} indisponible ({e}), sondage en secours si possible."),
                }
            }

            // Sondage des sources sans signal fs — une empreinte qui change vaut
            // déclencheur, au même titre qu'un évènement fs. Jamais au tout
            // premier tour (rien à comparer) : sinon chaque lancement d'app
            // déclencherait une régénération pour rien.
            //
            // CADENCÉ à WATCH_POLL_INTERVAL, pas une fois par tour de boucle : un
            // tour a lieu à CHAQUE évènement fs (une session Claude Code écrit en
            // continu → toutes les quelques secondes), et sonder revient à lister
            // tout Drive par le réseau + toute la bibliothèque Notes par osascript.
            // Sans ce garde, ces deux appels lourds partaient à la fréquence des
            // écritures de fichiers, pas toutes les 5 min (2026-07-31).
            let due = last_poll.is_none_or(|t: std::time::Instant| t.elapsed() >= WATCH_POLL_INTERVAL);
            if due {
                last_poll = Some(std::time::Instant::now());
                let mut polled = Vec::new();
                // Notes Apple : TOUJOURS par sondage (cf. `fs_watch_targets`) — le
                // `-wal` du NoteStore n'est pas une note. `label` = titre de la note
                // la plus récemment modifiée, pour l'Inbox.
                if let Some((fp, title)) = connectors::apple_notes::changed_fingerprint() {
                    polled.push(("apple-notes", fp, Some(title)));
                }
                if let Some(fp) = connectors::google_drive::changed_fingerprint() { polled.push(("google-drive", fp, None)); }
                for (source, fp, label) in polled {
                    match fingerprints.insert(source, fp.clone()) {
                        // Drive & Notes Apple = sources DOCUMENT → réveillent l'Archiviste.
                        Some(prev) if prev != fp => {
                            if let Some(title) = label { inbox_record_named(&title, source, "modified"); }
                            let _ = tx.send(true);
                        }
                        _ => {}
                    }
                }
            }

            // Attend soit un déclencheur (fs ou sondage), soit le prochain tour.
            // `doc_touched` : au moins un des déclencheurs de cette rafale portait
            // sur un contenu document (→ Archiviste), pas seulement Claude Code.
            let mut doc_touched = match rx.recv_timeout(WATCH_POLL_INTERVAL) {
                Ok(b) => b,
                Err(_) => continue,
            };
            // Debounce : une source qui écrit en RAFALE (session Claude Code en
            // cours, sync Drive en plusieurs lots…) → laisser l'accalmie. On DORT
            // un délai fixe puis on vide le canal d'un coup (non-bloquant) : un
            // `while recv_timeout().is_ok()` bouclait à 100% CPU tant que les
            // évènements affluaient sans jamais 3 s de silence (bug révélé le
            // 2026-07-29 quand une session Claude Code écrivait en continu dans
            // ~/.claude/projects — le watcher spinnait à fond).
            std::thread::sleep(std::time::Duration::from_secs(3));
            while let Ok(b) = rx.try_recv() { doc_touched = doc_touched || b; }
            // Inbox : rafraîchir le flux PROMPTEMENT (indépendant du cooldown de
            // régé 5 min) — un fichier détecté apparaît dans l'Inbox en ~3 s.
            { use tauri::Emitter; let _ = app.emit("inbox-updated", ()); }
            // Pas encore de cerveau, ou contenu démo → on ne touche à rien.
            let Some(data) = ai::llama::app_data_dir() else { continue; };
            if !has_real_brain(&data) || data.join("demo.flag").exists() { continue; }
            // Garde-fou : une source active (ex. CETTE session Claude Code, qui
            // écrit dans ~/.claude/projects/) peut redéclencher toutes les
            // quelques secondes — sans ce plafond, chaque accalmie relance une
            // régénération complète (~50 Mo réécrits à chaque fois). Bug remonté
            // par Liam le 2026-07-21 (app ralentie, 7 régénérations en 4 min).
            const AUTO_REGEN_COOLDOWN_SECS: u64 = 300;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
            let last = LAST_AUTO_REGEN.load(std::sync::atomic::Ordering::Relaxed);
            if now.saturating_sub(last) < AUTO_REGEN_COOLDOWN_SECS { continue; }
            // Une génération manuelle tourne → skip, le prochain déclencheur relancera.
            let Ok(_gen) = GEN_LOCK.try_lock() else { continue; };
            LAST_AUTO_REGEN.store(now, std::sync::atomic::Ordering::Relaxed);
            // run_generation rafraîchit lui-même les caches connecteurs (cf.
            // refresh_connector_caches) — le déclencheur ne fait que lancer.
            match run_generation(&app) {
                Ok(_) => {
                    let _ = app.emit("brain-updated", ());
                    // Déclencheur Archiviste ÉVÉNEMENTIEL (jamais un timer) : seulement
                    // si un contenu DOCUMENT a changé (local/Drive/Obsidian/Notes) —
                    // jamais sur une session Claude Code (conversations), qui était la
                    // cause de l'emballement du 2026-07-29. Déjà rate-limité : l'auto-
                    // régé est plafonnée à 1×/5 min (AUTO_REGEN_COOLDOWN_SECS), donc
                    // l'Archiviste tourne au plus 1×/5 min, uniquement sur du vrai neuf.
                    if doc_touched { let _ = app.emit("archiviste-auto", ()); }
                }
                Err(e) => crate::elog!("⚠️ watch auto : régénération échouée : {e}"),
            }
        }
    });
}

/// Watch `mcp_pending/` : prévient le front dès qu'une proposition MCP locale
/// apparaît/disparaît, via un évènement Tauri — au lieu de sonder à l'aveugle
/// toutes les 2s (coût continu pour rien, remonté par Liam le 2026-07-21).
fn start_mcp_pending_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use notify::Watcher;
        let dir = loop {
            if let Some(d) = ai::llama::app_data_dir() { break d; }
            std::thread::sleep(std::time::Duration::from_secs(5));
        };
        let pending_dir = mcp_pending_dir(&dir);
        if std::fs::create_dir_all(&pending_dir).is_err() { return; }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            if res.is_ok() { let _ = tx.send(()); }
        }) {
            Ok(w) => w,
            Err(e) => { crate::elog!("⚠️ watch mcp_pending indisponible : {e}"); return; }
        };
        if let Err(e) = watcher.watch(&pending_dir, notify::RecursiveMode::NonRecursive) {
            crate::elog!("⚠️ watch mcp_pending indisponible : {e}");
            return;
        }
        while rx.recv().is_ok() {
            // Léger débounce : une acceptation écrit/supprime plusieurs fichiers d'affilée.
            while rx.recv_timeout(std::time::Duration::from_millis(200)).is_ok() {}
            let _ = app.emit("mcp-proposal-changed", ());
        }
    });
}

/// Signature bon marché de toutes les sources (id + date de modif de chaque
/// conversation) — permet au rattrapage de démarrage de ne régénérer QUE si
/// quelque chose a bougé pendant que l'app était fermée, sans réécrire le cerveau
/// (ni re-déclencher la sync cloud) à chaque lancement pour rien.
fn sources_fingerprint(convs: &[Conversation]) -> String {
    use sha2::{Digest, Sha256};
    let mut keys: Vec<String> = convs.iter()
        .map(|c| format!("{}:{}", c.summary.id, c.summary.last_timestamp.as_deref().unwrap_or("")))
        .collect();
    keys.sort();
    let mut h = Sha256::new();
    h.update(keys.join("\n").as_bytes());
    format!("{}:{:x}", keys.len(), h.finalize())
}

/// Rattrapage au démarrage : une note/un fichier ajouté PENDANT que l'app était
/// fermée ne produit aucun évènement fs et échappe au watcher (qui n'observe que
/// le temps où l'app tourne) — sans ça il faut un « Générer » manuel. On rafraîchit
/// les caches connecteurs et, si les sources ont changé depuis la dernière
/// génération, on régénère une fois. Bon marché : le pipeline cache l'extraction
/// Gemma par conversation (cf. `ai::pipeline`), donc une régénération sans
/// nouveauté = 0 appel LLM. Gated sur le fingerprint pour ne pas réécrire le
/// cerveau — ni faire churner la sync cloud — quand rien n'a bougé.
fn startup_catchup(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        use tauri::Emitter;
        let Some(data) = ai::llama::app_data_dir() else { return };
        // Pas encore de vrai cerveau, ou contenu démo → l'onboarding s'en charge.
        if !has_real_brain(&data) || data.join("demo.flag").exists() { return; }

        refresh_connector_caches();
        let fp = sources_fingerprint(&load_all_conversations());
        let fp_path = data.join("sources.fingerprint");
        if std::fs::read_to_string(&fp_path).unwrap_or_default() == fp { return; } // rien de neuf hors ligne

        // Une génération manuelle tourne déjà → on la laisse faire.
        let Ok(_gen) = GEN_LOCK.try_lock() else { return };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        LAST_AUTO_REGEN.store(now, std::sync::atomic::Ordering::Relaxed);
        match run_generation(&app) {
            Ok(_) => { let _ = std::fs::write(&fp_path, &fp); let _ = app.emit("brain-updated", ()); }
            Err(e) => crate::elog!("⚠️ rattrapage démarrage : {e}"),
        }
    });
}

/// Icône barre de menu macOS (tray). Template image (`icon_as_template`) → suit
/// le thème clair/sombre du système. Menu minimal Afficher / Quitter. Câblée
/// côté Rust (pas l'API JS) pour exister dès le `setup()`. L'icône est embarquée
/// au build (`include_bytes!`) plutôt que résolue par chemin au runtime — pas de
/// dépendance à l'emplacement du bundle, marche en `dev` comme en `build`.
/// API vérifiée contre docs.rs/tauri/2.11.3 (`show_menu_on_left_click`, renommé
/// depuis `menu_on_left_click` en v2.2).
/// Handle du tray, gardé pour le mettre à jour au runtime (indication
/// « Archiviste au travail » même quand l'app est réduite, cf. `set_archivist_active`).
static TRAY: std::sync::OnceLock<tauri::tray::TrayIcon> = std::sync::OnceLock::new();

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::TrayIconBuilder;
    use tauri::Manager;

    let show = MenuItem::with_id(app, "show", "Afficher", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("Lucid")
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    let _ = TRAY.set(tray);
    Ok(())
}

/// Reflète l'état de l'Archiviste sur l'icône barre de menu — visible même app
/// réduite (demandé par Liam). Actif : un point « • » à côté de l'icône (macOS)
/// + tooltip explicite ; repos : tout s'efface. Le tooltip est le repli
/// cross-plateforme (le titre de tray ne s'affiche que sur macOS).
#[tauri::command]
fn set_archivist_active(active: bool) {
    let Some(tray) = TRAY.get() else { return };
    let _ = tray.set_tooltip(Some(if active {
        "Lucid — l'Archiviste range tes pages…"
    } else {
        "Lucid"
    }));
    if active {
        let _ = tray.set_title(Some("•"));
    } else {
        let _ = tray.set_title(None::<&str>);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_env_local();

    // Sentry seulement si consentement (flag) + DSN — sinon zéro réseau.
    let sentry_client = if telemetry_enabled() {
        sentry_dsn().map(|dsn| {
            sentry::init((
                dsn,
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    send_default_pii: false,
                    before_send: Some(std::sync::Arc::new(|mut event| {
                        scrub_event(&mut event);
                        Some(event)
                    })),
                    ..Default::default()
                },
            ))
        })
    } else {
        None
    };
    let _ = SENTRY_ACTIVE.set(sentry_client.is_some());

    let mut builder = tauri::Builder::default();
    if let Some(client) = &sentry_client {
        builder = builder.plugin(tauri_plugin_sentry::init(client));
    }
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            if let Some(dir) = ai::llama::app_data_dir() {
                ensure_all_space_anchors(&dir);
            }
            start_watcher(app.handle().clone());
            start_data_watcher(app.handle().clone());
            start_mcp_pending_watcher(app.handle().clone());
            // Rattrape les ajouts/modifs faits pendant que l'app était fermée
            // (le watcher, lui, ne voit que le temps où l'app tourne).
            startup_catchup(app.handle().clone());
            // Tray non bloquant : s'il échoue, l'app tourne quand même (jamais
            // d'échec silencieux — cf. ADR-0015, on log).
            if let Err(e) = setup_tray(app) { crate::elog!("⚠️ icône barre de menu indisponible : {e}"); }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_conversations,
            load_conversation,
            claude_code_available,
            claude_code_disconnect,
            claude_code_reconnect,
            ai_ready,
            connectors_status,
            import_claude_ai,
            google_drive_save_credentials,
            google_drive_has_credentials,
            google_drive_connect,
            google_drive_sync,
            google_drive_disconnect,
            google_drive_folders,
            google_drive_folder_counts,
            google_drive_selection,
            google_drive_set_selection,
            ask_brain,
            ask_node,
            generate_content,
            read_brain_graph,
            generate_brain,
            export_node_md,
            ai_setup_needed,
            download_model,
            ai_bootstrap,
            install_model_file,
            list_models,
            set_active_model,
            synthesize_node,
            save_node_content,
            load_node_content,
            create_note_node,
            create_structure,
            import_file,
            list_mcp_proposals,
            resolve_mcp_proposal,
            resolve_all_mcp_proposals,
            import_mcp_proposal,
            import_shared_space,
            save_pasted_image,
            export_backup,
            import_backup,
            merge_backup,
            sync_fingerprint,
            set_node_parent,
            rename_node,
            delete_node,
            obsidian_set_vault,
            obsidian_vault_path,
            obsidian_disconnect,
            obsidian_auto_connect,
            apple_notes_connect,
            apple_notes_sync,
            apple_notes_disconnect,
            set_archivist_active,
            import_chatgpt,
            local_folder_connect,
            local_folder_list,
            local_folder_add,
            local_folder_remove,
            local_folder_disconnect,
            local_folder_sync,
            list_snapshots,
            restore_snapshot,
            list_node_snapshots,
            get_node_snapshot,
            list_spaces,
            create_space,
            rename_space,
            delete_space,
            add_node_to_space,
            remove_node_from_space,
            export_space_md,
            seed_demo,
            reset_demo,
            reset_environment,
            set_active_user,
            mcp_manual_validation_enabled,
            set_mcp_manual_validation,
            telemetry_enabled,
            set_telemetry,
            sentry_active,
            crash_test,
            ai_info,
            ai_diagnostics,
            inbox_recent,
            run_archivist,
            archivist_was_interrupted,
            archivist_diagnostic
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|_app_handle, event| {
            // Le serveur d'inférence persistant (cf. ai/llama.rs) doit mourir
            // avec l'app — sinon un process GPU-actif traîne après la
            // fermeture de Lucid.
            if let tauri::RunEvent::Exit = event {
                ai::llama::shutdown_server();
            }
        });
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    #[test]
    fn relevant_passage_renvoie_le_passage_utile_pas_le_debut_de_page() {
        let text = format!("{}\nLe montant du loyer est de 850 euros.\n{}", "bla ".repeat(200), "fin ".repeat(200));
        let terms = vec!["loyer".to_string(), "montant".to_string()];
        let p = relevant_passage(&text, &terms, 500).expect("un passage attendu");
        assert!(p.contains("850 euros"), "le passage doit porter la réponse : {p}");
        assert!(p.chars().count() <= 800, "budget dépassé : {} car.", p.chars().count());
        // Accents pliés dans les deux sens, et rien à trouver → None (l'appelant
        // se rabat alors sur le résumé).
        assert!(relevant_passage("Le relevé géographique.", &["geographique".to_string()], 500).is_some());
        assert!(relevant_passage(&text, &["zzzz".to_string()], 500).is_none());
        // Ligne géante sans saut de ligne (export CSV) : la valeur cherchée est
        // quand même isolée au lieu de noyer tout le tableur.
        let csv = format!("{};PLOMBERIE;1240", (0..300).map(|i| format!("{i},client{i}")).collect::<Vec<_>>().join(";"));
        let p = relevant_passage(&csv, &["plomberie".to_string()], 500).expect("un passage attendu");
        assert!(p.contains("PLOMBERIE"));
        assert!(p.chars().count() <= 600, "{} car.", p.chars().count());
    }

    #[test]
    fn ask_context_selectionne_les_pages_pertinentes_et_reste_borne() {
        let mut nodes = vec![
            BrainNode { updated_at: None,
                id: "root".into(), label: "Lucid".into(), kind: "root".into(), weight: 3,
                summary: String::new(), keywords: vec![], decisions: vec![], patterns: vec![],
                community: 0, parent_id: None, synthesized_at: None, date: None, content: String::new(),
                connector: None, source_id: None, source_project: None, source_text: String::new(),
            },
            demo_leaf("p1", "root", "Notes Jaon", "Réunion avec Jaon sur le projet."),
            demo_leaf("p2", "root", "Recette", "Cuisine et macros."),
        ];
        // Bruit volumineux : sans le bornage, injecterait ~300k octets.
        for i in 0..500 {
            nodes.push(demo_leaf(&format!("n{i}"), "root", "Divers", &"lorem ipsum ".repeat(50)));
        }
        let graph = BrainGraph {
            nodes, edges: vec![], markdown: String::new(),
            report: String::new(), generated_at: String::new(),
        };

        let ctx = ask_context(&graph, "Quelles pages parlent de Jaon ?", "APERCU", &[]);
        assert!(ctx.contains("Notes Jaon"), "la page pertinente doit être incluse");
        assert!(!ctx.contains("Recette"), "les pages hors sujet sont exclues");
        assert!(ctx.len() < 20_000, "le contexte doit rester borné, pas tout le cerveau");
    }
}
