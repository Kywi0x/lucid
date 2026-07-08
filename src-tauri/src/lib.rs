//! Second Brain — point d'entrée de l'application Tauri.
//! Expose les commandes appelables depuis le frontend React.

mod ai;
mod backup;
mod connectors;
mod mcp_clients;
mod models;

use ai::{pipeline, LlamaEngine};
use models::{BrainEdge, BrainGraph, BrainNode, ConnectorStatus, Conversation, ConversationSummary};
use tauri::Emitter;

/// Accès public au scan (utilisé par les exemples / tests d'intégration).
pub fn list_conversations_pub() -> Vec<ConversationSummary> {
    connectors::claude_code::list_conversations()
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
        eprintln!("[{}/{}] {}", p.current, p.total, p.label);
    }, |_, _, _| {})?;
    if let Some(dir) = ai::llama::app_data_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("brain.md"), &graph.markdown);
        let _ = std::fs::write(dir.join("brain_report.md"), &graph.report);
        if let Ok(json) = serde_json::to_string_pretty(&graph) {
            let _ = std::fs::write(dir.join("brain.json"), json);
        }
    }
    Ok(graph)
}

/// Agrège toutes les sources connues.
fn load_all_conversations() -> Vec<Conversation> {
    let mut convs = connectors::claude_code::load_all_conversations();
    convs.extend(connectors::claude_ai::load_conversations());
    convs.extend(connectors::google_drive::load_conversations());
    convs.extend(connectors::notion::load_conversations());
    convs.extend(connectors::obsidian::load_all_conversations());
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
#[tauri::command]
fn list_models() -> Vec<ModelInfo> {
    let catalog = ai::llama::load_catalog();
    let active_id = ai::llama::active_model_stored().map(|m| m.id);
    let recommended_id = ai::llama::recommended_id(&catalog);
    catalog.into_iter().map(|m| {
        let downloaded = ai::llama::app_data_dir()
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
        Some("claude-ai")    => connectors::claude_ai::load_by_id(&id),
        Some("google-drive") => connectors::google_drive::load_by_id(&id),
        Some("notion")       => connectors::notion::load_by_id(&id),
        Some("obsidian")     => connectors::obsidian::load_by_id(&id),
        _                    => connectors::claude_code::load_conversation(&project_slug, &id),
    }
}

/// Indique si le connecteur Claude Code est disponible (dossier présent).
#[tauri::command]
fn claude_code_available() -> bool {
    connectors::claude_code::projects_dir().is_some()
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

    vec![
        ConnectorStatus {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            connected: connectors::claude_code::projects_dir().is_some(),
            last_sync: cc_sync,
            conversation_count: cc_convs.len(),
            needs_setup: false,
        },
        ConnectorStatus {
            id: "claude-ai".into(),
            name: "Claude".into(),
            connected: ai_connected,
            last_sync: ai_sync,
            conversation_count: ai_convs.len(),
            needs_setup: false,
        },
        ConnectorStatus {
            id: "cowork".into(),
            name: "Cowork".into(),
            connected: false,
            last_sync: None,
            conversation_count: 0,
            needs_setup: false,
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
        {
            let n_convs = connectors::notion::load_conversations();
            let n_sync  = n_convs.iter().filter_map(|c| c.summary.last_timestamp.clone()).max();
            ConnectorStatus {
                id: "notion".into(),
                name: "Notion".into(),
                connected: connectors::notion::is_connected(),
                last_sync: n_sync,
                conversation_count: n_convs.len(),
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
    ]
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

/// Enregistre le token Notion et valide la connexion.
#[tauri::command]
async fn notion_connect(token: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if !token.starts_with("secret_") && !token.starts_with("ntn_") {
            return Err("Token invalide — il doit commencer par « secret_ » ou « ntn_ ».".into());
        }
        connectors::notion::validate_token(&token)?;
        connectors::notion::save_token(&token)
    })
    .await
    .map_err(|e| format!("Erreur : {e}"))?
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

/// Synchronise les pages Notion. Renvoie (nouvelles, total).
#[tauri::command]
async fn notion_sync() -> Result<(usize, usize), String> {
    tauri::async_runtime::spawn_blocking(|| {
        let result = connectors::notion::sync()?;
        // Invalide le cache de synthèse pour que le pipeline régénère
        // les nœuds Notion avec les nouvelles pages.
        if let Some(dir) = ai::llama::app_data_dir() {
            let _ = std::fs::remove_file(dir.join("brain_synth_cache.json"));
        }
        Ok(result)
    })
    .await
    .map_err(|e| format!("Sync interrompue : {e}"))?
}

/// Déconnecte Notion (supprime token + cache).
#[tauri::command]
fn notion_disconnect() {
    connectors::notion::disconnect();
}

/// Lit le contenu d'une page Notion depuis le cache local (0 appel API).
#[tauri::command]
fn notion_load_page(id: String) -> Result<String, String> {
    let conv = connectors::notion::load_by_id(&id)
        .ok_or_else(|| "Page absente du cache — relancez un Sync Notion.".to_string())?;
    Ok(conv.messages.into_iter().map(|m| m.text).collect::<Vec<_>>().join("\n\n"))
}

/// Contexte BORNÉ pour ask_brain : aperçu compact (report) + pages les plus
/// pertinentes à la question (récupération par mots-clés). Injecter tout brain.md
/// dépasse la fenêtre du modèle (8192 tokens) → llama plante. Ici on borne à
/// `budget` octets, quelle que soit la taille du cerveau.
fn ask_context(graph: &BrainGraph, question: &str, report: &str) -> String {
    const BUDGET: usize = 16_000; // ~4000 tokens, laisse la place au report + réponse
    let terms: Vec<String> = question
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(str::to_string)
        .collect();

    let label_of: std::collections::HashMap<&str, &str> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str())).collect();

    // Score = correspondances des mots de la question (titre pondéré fort).
    let mut scored: Vec<(usize, &BrainNode)> = graph.nodes.iter()
        .filter(|n| n.kind != "root")
        .map(|n| {
            let title = n.label.to_lowercase();
            let hay = format!("{} {} {} {}", n.summary, n.keywords.join(" "), n.content, n.source_text).to_lowercase();
            let score = terms.iter().map(|t|
                if title.contains(t.as_str()) { 5 } else { 0 } + hay.matches(t.as_str()).count()
            ).sum();
            (score, n)
        })
        .filter(|(s, _)| *s > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.weight.cmp(&a.1.weight)));

    // Aucun match (question vague) → replie sur les nœuds les plus « lourds ».
    let mut selected: Vec<&BrainNode> = if scored.is_empty() {
        let mut v: Vec<&BrainNode> = graph.nodes.iter().filter(|n| n.kind != "root").collect();
        v.sort_by(|a, b| b.weight.cmp(&a.weight));
        v
    } else {
        scored.into_iter().map(|(_, n)| n).collect()
    };
    selected.truncate(40);

    let mut ctx = format!("APERÇU :\n{report}\n\nPAGES PERTINENTES :\n");
    for n in selected {
        if ctx.len() > BUDGET { ctx.push_str("[…autres pages omises…]\n"); break; }
        let parent = n.parent_id.as_deref().and_then(|p| label_of.get(p)).copied().unwrap_or("");
        let body = if !n.summary.trim().is_empty() {
            n.summary.trim().to_string()
        } else if !n.content.trim().is_empty() {
            n.content.chars().take(300).collect::<String>()
        } else {
            n.source_text.chars().take(300).collect::<String>()
        };
        if parent.is_empty() {
            ctx.push_str(&format!("- {} : {body}\n", n.label));
        } else {
            ctx.push_str(&format!("- {} (dans {parent}) : {body}\n", n.label));
        }
    }
    ctx
}

#[tauri::command]
async fn ask_brain(question: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let raw = std::fs::read_to_string(dir.join("brain.json"))
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let report = std::fs::read_to_string(dir.join("brain_report.md")).unwrap_or_default();
        let engine = LlamaEngine::detect()?;
        let system = "Tu es l'assistant du second cerveau de l'utilisateur. Réponds en \
français, de façon concise, en te basant UNIQUEMENT sur le contexte fourni. Cite les pages \
par leur titre. Si l'information n'y figure pas, dis-le clairement.";
        let user = format!("CONTEXTE :\n{}\n\nQUESTION : {question}", ask_context(&graph, &question, &report));
        engine.complete(Some(system), &user, 512)
    })
    .await
    .map_err(|e| format!("Tâche interrompue : {e}"))?
}

/// Chat contextuel sur une page : contexte = contenu de la page (+ sous-pages si demandé).
#[tauri::command]
async fn ask_node(node_id: String, question: String, include_children: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let raw = std::fs::read_to_string(dir.join("brain.json"))
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
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
        let raw = std::fs::read_to_string(dir.join("brain.json"))
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
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
/// Si `brain.md` est plus récent que `brain.json` (édition manuelle ou écriture LLM),
/// on repparse brain.md et on met brain.json à jour avant de retourner le graphe.
#[tauri::command]
fn read_brain_graph() -> Option<BrainGraph> {
    let dir = ai::llama::app_data_dir()?;
    let json_path = dir.join("brain.json");
    let md_path   = dir.join("brain.md");

    let md_newer = md_path.exists() && {
        let jm = json_path.metadata().and_then(|m| m.modified()).ok();
        let mm = md_path.metadata().and_then(|m| m.modified()).ok();
        match (jm, mm) {
            (Some(j), Some(m)) => m > j,
            _ => !json_path.exists(),
        }
    };

    if md_newer {
        let content = std::fs::read_to_string(&md_path).ok()?;
        let graph = ai::brain_md::parse(&content);
        if let Ok(json) = serde_json::to_string(&graph) {
            let _ = std::fs::write(&json_path, json);
        }
        return Some(graph);
    }

    std::fs::read_to_string(&json_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// Exporte un nœud + tous ses descendants en markdown standalone vers `path`.
#[tauri::command]
fn export_node_md(node_id: String, path: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

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
/// Route selon connector : notion → notion_cache.json, claude-code → .jsonl, etc.
#[tauri::command]
fn load_node_content(node_id: String) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let node = graph.nodes.iter().find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;

    let connector = node.connector.as_deref().unwrap_or("");
    let source_id = node.source_id.as_deref().unwrap_or("");
    let project_slug = node.source_project.as_deref().unwrap_or("");

    let conv = match connector {
        "notion"       => connectors::notion::load_by_id(source_id),
        "google-drive" => connectors::google_drive::load_by_id(source_id),
        "claude-ai"    => connectors::claude_ai::load_by_id(source_id),
        "claude-code"  => connectors::claude_code::load_conversation(project_slug, source_id),
        "obsidian"     => connectors::obsidian::load_by_id(source_id),
        _ => return Err(format!("Connecteur inconnu : {connector}")),
    };

    let conv = conv.ok_or_else(|| "Contenu absent du cache — relancez un Sync.".to_string())?;

    if conv.messages.len() == 1 {
        Ok(conv.messages[0].text.clone())
    } else {
        Ok(conv.messages.iter().map(|m| {
            let who = if m.role == "user" { "**Toi**" } else { "**Claude**" };
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
    dir.join("node_history").join(node_id)
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
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    // Sauvegarde l'ancienne version avant d'écraser
    save_node_content_history(&dir, &node_id, &node.content);
    node.content = content;
    std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Crée un nœud « note » (prise de note utilisateur) rattaché à `parent_id`.
/// Persisté dans brain.json ; préservé lors des régénérations (kind == "note").
#[tauri::command]
fn create_note_node(parent_id: String, label: String) -> Result<BrainNode, String> {
    insert_note_node(parent_id, label, String::new())
}

/// Insère un nœud note (avec contenu markdown éventuel) dans brain.json.
/// Cœur partagé entre `create_note_node` et `import_file`.
fn insert_note_node(parent_id: String, label: String, content: String) -> Result<BrainNode, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    insert_note_node_in(&dir, format!("note-{ts}"), parent_id, label, content)
}

/// Variante injectable (id + dossier de données explicites) — testable, et
/// utilisée par l'acceptation des propositions MCP (l'id de la proposition
/// devient l'id du nœud, ce qui garde valides les références parent en chaîne).
fn insert_note_node_in(dir: &std::path::Path, id: String, parent_id: String, label: String, content: String) -> Result<BrainNode, String> {
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if !graph.nodes.iter().any(|n| n.id == parent_id) {
        return Err(format!("Nœud parent {parent_id} introuvable."));
    }
    if graph.nodes.iter().any(|n| n.id == id) {
        return Err(format!("Nœud {id} déjà présent."));
    }
    let node = BrainNode {
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
        content,
        connector: None,
        source_id: None,
        source_project: None,
        source_text: String::new(),
    };
    graph.edges.push(BrainEdge {
        source: parent_id, target: id, kind: "contains".into(), relation: "contains".into(),
    });
    graph.nodes.push(node.clone());
    std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(node)
}

/// Importe un fichier local (PDF, DOC/DOCX/RTF, TXT/MD, CSV) : conversion en
/// markdown puis création d'un nœud note sous `parent_id`.
#[tauri::command]
fn import_file(path: String, parent_id: String) -> Result<BrainNode, String> {
    let p = std::path::Path::new(&path);
    let label = p.file_stem().and_then(|s| s.to_str()).unwrap_or("Fichier importé").to_string();
    let ext = p.extension().and_then(|s| s.to_str()).map(|s| s.to_lowercase()).unwrap_or_default();
    let content = match ext.as_str() {
        "pdf" => connectors::google_drive::pdf_to_markdown(p, &label)
            .ok_or(if cfg!(windows) {
                "Extraction PDF impossible : poppler non embarqué sur cette build Windows."
            } else {
                "Extraction PDF impossible (pdftotext/tesseract requis — brew install poppler tesseract)."
            })?,
        "doc" | "docx" | "rtf" => textutil_to_text(p)?,
        "txt" | "md" | "markdown" => read_lossy(p)?,
        "csv" => csv_to_markdown(&read_lossy(p)?),
        other => return Err(format!("Format non supporté : .{other}")),
    };
    if content.trim().is_empty() {
        return Err("Le fichier ne contient aucun texte exploitable.".into());
    }
    insert_note_node(parent_id, label, content)
}

// ─── Propositions MCP (écriture validée par l'utilisateur) ─────────────────────
//
// Le serveur MCP (`lucid_mcp`) ne touche jamais brain.json : il dépose une
// proposition par fichier dans `mcp_pending/`. L'app les affiche (bulles
// fantômes + panneau) ; seule l'acceptation écrit dans brain.json — un seul
// écrivain, pas de course. L'id de la proposition devient l'id du nœud accepté,
// ce qui permet à l'IA de construire des arbres (parent_id = id d'une
// proposition précédente).

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct McpProposal {
    id: String,
    parent_id: String,
    label: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    created_at: String,
}

fn mcp_pending_dir(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("mcp_pending")
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
fn resolve_proposal_in(dir: &std::path::Path, id: &str, accept: bool) -> Result<usize, String> {
    let props = load_proposals_in(dir);
    let target = props.iter().find(|p| p.id == id)
        .ok_or_else(|| format!("Proposition {id} introuvable."))?;

    if accept {
        // Chaîne d'ancêtres encore pending (accepter un enfant accepte ses parents).
        let mut chain = vec![target.clone()];
        let mut cur_parent = target.parent_id.clone();
        while let Some(p) = props.iter().find(|p| p.id == cur_parent) {
            chain.push(p.clone());
            cur_parent = p.parent_id.clone();
        }
        chain.reverse();
        for p in &chain {
            insert_note_node_in(dir, p.id.clone(), p.parent_id.clone(), p.label.clone(), p.content.clone())?;
            std::fs::remove_file(mcp_pending_dir(dir).join(format!("{}.json", p.id)))
                .map_err(|e| e.to_string())?;
        }
        Ok(chain.len())
    } else {
        // Refus récursif : les descendants pending tombent avec le parent.
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
        Ok(doomed.len())
    }
}

/// Liste les propositions en attente déposées par le serveur MCP.
#[tauri::command]
fn list_mcp_proposals() -> Result<Vec<McpProposal>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    Ok(load_proposals_in(&dir))
}

/// Accepte ou refuse une proposition MCP.
#[tauri::command]
fn resolve_mcp_proposal(id: String, accept: bool) -> Result<usize, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    resolve_proposal_in(&dir, &id, accept)
}

/// Exporte le cerveau en zip (~2 Mo, hors modèles) pour la sauvegarde cloud.
#[tauri::command]
fn export_backup() -> Result<Vec<u8>, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    backup::export_in(&dir)
}

/// Restaure une sauvegarde (zip) dans le dossier de données. Renvoie le nombre
/// de fichiers restaurés. L'app doit recharger le graphe ensuite.
#[tauri::command]
fn import_backup(bytes: Vec<u8>) -> Result<usize, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    backup::import_in(&dir, &bytes)
}

/// Statut des clients IA (Claude Desktop/Code, Cursor) : installés ? connectés au MCP Lucid ?
#[tauri::command]
fn ai_clients_status() -> Vec<mcp_clients::AiClientStatus> {
    mcp_clients::status()
}

/// Connexion one-click : écrit `mcpServers.lucid` dans la config du client (avec backup).
#[tauri::command]
fn connect_ai_client(id: String) -> Result<String, String> {
    mcp_clients::connect(&id)
}

/// Retire l'entrée MCP Lucid de la config du client.
#[tauri::command]
fn disconnect_ai_client(id: String) -> Result<(), String> {
    mcp_clients::disconnect(&id)
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
fn read_lossy(p: &std::path::Path) -> Result<String, String> {
    std::fs::read(p)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .map_err(|e| e.to_string())
}

// ponytail: textutil = natif macOS ; passer à une crate docx le jour du port Windows.
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

// Hors macOS : import .doc/.rtf/.docx non supporté en v1 (textutil est natif macOS).
#[cfg(not(target_os = "macos"))]
fn textutil_to_text(_p: &std::path::Path) -> Result<String, String> {
    Err("Import .doc/.rtf/.docx non supporté sur cette plateforme (macOS uniquement en v1).".into())
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
        let p = McpProposal { id: id.into(), parent_id: parent.into(), label: label.into(),
                              content: String::new(), created_at: String::new() };
        std::fs::write(mcp_pending_dir(dir).join(format!("{id}.json")),
                       serde_json::to_string(&p).unwrap()).unwrap();
    }

    fn graph_ids(dir: &std::path::Path) -> Vec<String> {
        let g: BrainGraph = serde_json::from_str(&std::fs::read_to_string(dir.join("brain.json")).unwrap()).unwrap();
        g.nodes.iter().map(|n| n.id.clone()).collect()
    }

    #[test]
    fn accepter_un_enfant_accepte_ses_ancetres_dans_l_ordre() {
        let dir = setup("chain");
        propose(&dir, "mcp-1", "root", "Parent");
        propose(&dir, "mcp-2", "mcp-1", "Enfant");
        propose(&dir, "mcp-3", "mcp-2", "Petit-enfant");
        // Accepter le petit-enfant doit insérer les 3, parents d'abord.
        assert_eq!(resolve_proposal_in(&dir, "mcp-3", true).unwrap(), 3);
        let ids = graph_ids(&dir);
        assert!(ids.contains(&"mcp-1".into()) && ids.contains(&"mcp-3".into()));
        assert!(load_proposals_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuser_un_parent_supprime_ses_descendants() {
        let dir = setup("reject");
        propose(&dir, "mcp-1", "root", "Parent");
        propose(&dir, "mcp-2", "mcp-1", "Enfant");
        propose(&dir, "mcp-9", "root", "Autre");
        assert_eq!(resolve_proposal_in(&dir, "mcp-1", false).unwrap(), 2);
        let rest = load_proposals_in(&dir);
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].id, "mcp-9");
        assert_eq!(graph_ids(&dir).len(), 1); // rien inséré
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
        let raw = std::fs::read_to_string(dir.join("brain.json"))
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

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
            graph.nodes.push(BrainNode {
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

        std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;

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
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
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
        let mut cur = Some(parent_id.as_str());
        while let Some(c) = cur {
            if c == node_id {
                return Err("Déplacement impossible : créerait une boucle.".into());
            }
            cur = parent_of.get(c).copied();
        }
    }
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    node.parent_id = Some(parent_id.clone());
    // Maintient les arêtes de contenance cohérentes (utilisées par le filtrage par space).
    graph.edges.retain(|e| !(e.target == node_id && e.kind == "contains"));
    graph.edges.push(BrainEdge {
        source: parent_id, target: node_id, kind: "contains".into(), relation: "contains".into(),
    });
    std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Renomme un nœud (change son `label`). Persisté dans brain.json.
#[tauri::command]
fn rename_node(node_id: String, label: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let node = graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?;
    let l = label.trim();
    node.label = if l.is_empty() { "Sans titre".into() } else { l.to_string() };
    std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Re-synthétise un nœud unique à la demande (sources brutes + résumés enfants comme contexte).
/// Met à jour brain.json et retourne le nœud mis à jour.
#[tauri::command]
async fn synthesize_node(node_id: String) -> Result<BrainNode, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = LlamaEngine::detect()?;
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;

        let raw_graph = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
        let mut graph: BrainGraph = serde_json::from_str(&raw_graph).map_err(|e| e.to_string())?;

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

        if let Ok(json) = serde_json::to_string_pretty(&graph) {
            let _ = std::fs::write(dir.join("brain.json"), json);
        }

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
}

fn save_snapshot_in(dir: &std::path::Path) {
    let brain_path = dir.join("brain.json");
    if !brain_path.exists() { return; }
    let snap_dir = dir.join("snapshots");
    if std::fs::create_dir_all(&snap_dir).is_err() { return; }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::copy(&brain_path, snap_dir.join(format!("brain_{ts}.json")));
    // Garder les 10 derniers
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
    let snap_dir = dir.join("snapshots");
    let mut infos: Vec<SnapshotInfo> = std::fs::read_dir(&snap_dir)
        .into_iter().flatten().flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            if !s.starts_with("brain_") || !s.ends_with(".json") { return None; }
            let ts_str = s.strip_prefix("brain_")?.strip_suffix(".json")?;
            let created_at: u64 = ts_str.parse().ok()?;
            let node_count = std::fs::read_to_string(e.path()).ok()
                .and_then(|r| serde_json::from_str::<BrainGraph>(&r).ok())
                .map(|g| g.nodes.len()).unwrap_or(0);
            Some(SnapshotInfo { id: format!("brain_{ts_str}"), created_at, node_count })
        })
        .collect();
    infos.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    infos
}

#[tauri::command]
fn restore_snapshot(snapshot_id: String) -> Result<BrainGraph, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let src = dir.join("snapshots").join(format!("{snapshot_id}.json"));
    let dest = dir.join("brain.json");
    save_snapshot_in(&dir); // snapshot de l'état actuel avant restauration
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    let raw = std::fs::read_to_string(&dest).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

// ── Spaces ─────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct Space {
    id: String,
    name: String,
    node_ids: Option<Vec<String>>,
}

fn load_spaces(dir: &std::path::Path) -> Vec<Space> {
    std::fs::read_to_string(dir.join("spaces.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_spaces(dir: &std::path::Path, spaces: &[Space]) {
    if let Ok(json) = serde_json::to_string_pretty(spaces) {
        let _ = std::fs::write(dir.join("spaces.json"), json);
    }
}

#[tauri::command]
fn list_spaces() -> Vec<Space> {
    let lucid = Space { id: "lucid".into(), name: "Lucid".into(), node_ids: None };
    let mut spaces = vec![lucid];
    if let Some(dir) = ai::llama::app_data_dir() {
        spaces.extend(load_spaces(&dir));
    }
    spaces
}

#[tauri::command]
fn create_space(name: String) -> Result<Space, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let space = Space { id: format!("space_{ts}"), name, node_ids: Some(vec![]) };
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
    Ok(())
}

#[tauri::command]
fn add_node_to_space(space_id: String, node_id: String) -> Result<(), String> {
    if space_id == "lucid" { return Err("L'espace Lucid est virtuel, pas de liste à modifier.".into()); }
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let space = spaces.iter_mut().find(|s| s.id == space_id).ok_or("Espace introuvable.")?;
    let ids = space.node_ids.get_or_insert_with(Vec::new);
    if !ids.contains(&node_id) { ids.push(node_id); }
    save_spaces(&dir, &spaces);
    Ok(())
}

#[tauri::command]
fn remove_node_from_space(space_id: String, node_id: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let mut spaces = load_spaces(&dir);
    let space = spaces.iter_mut().find(|s| s.id == space_id).ok_or("Espace introuvable.")?;
    if let Some(ids) = &mut space.node_ids { ids.retain(|id| id != &node_id); }
    save_spaces(&dir, &spaces);
    Ok(())
}

#[tauri::command]
fn export_space_md(space_id: String) -> Result<String, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

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
    BrainNode {
        id: id.into(), label: label.into(), kind: "leaf".into(), weight: 1,
        // Pas de summary : le bloc « Synthèse IA » ne doit pas mentir — l'user
        // pourra tester la vraie synthèse manuelle sur ces pages.
        summary: String::new(),
        keywords: vec![], decisions: vec![], patterns: vec![], community: 1,
        parent_id: Some(parent.into()), synthesized_at: None, content: content.into(),
        connector: None, source_id: None, source_project: None, source_text: String::new(),
    }
}

#[tauri::command]
fn seed_demo() -> Result<BrainGraph, String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;

    let container = |id: &str, label: &str, weight: usize| BrainNode {
        id: id.into(), label: label.into(), kind: "container".into(), weight,
        summary: String::new(), keywords: vec![], decisions: vec![], patterns: vec![],
        community: 1, parent_id: Some("root".into()), synthesized_at: None,
        content: String::new(), connector: None, source_id: None, source_project: None, source_text: String::new(),
    };

    let mut nodes = vec![
        BrainNode {
            id: "root".into(), label: "Lucid".into(), kind: "root".into(), weight: 8,
            summary: "Contenu d'exemple — remplacé par tes vraies données au premier sync.".into(),
            keywords: vec![], decisions: vec![],
            patterns: vec![], community: 0, parent_id: None, synthesized_at: None,
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

Lucid agrège tes outils en un seul cerveau : **Claude Code**, **Notion**, **Google Drive**, **Obsidian**…

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

    let graph = BrainGraph {
        nodes, edges,
        markdown: "# Lucid — contenu d'exemple\n\nGraphe starter pour la prise en main.".into(),
        report: String::new(),
        generated_at: "demo".into(),
    };

    std::fs::write(dir.join("brain.json"), serde_json::to_string_pretty(&graph).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    save_spaces(&dir, &[Space {
        id: "space_demo".into(), name: "Projet Alpha".into(),
        node_ids: Some(vec!["demo-plan".into(), "demo-meeting".into(), "demo-ideas".into()]),
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

/// Lance le pipeline IA : analyse toutes les conversations et construit le graphe.
#[tauri::command]
async fn generate_brain(app: tauri::AppHandle) -> Result<BrainGraph, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Sans IA locale la génération marche quand même (structure + texte source).
        let engine = LlamaEngine::detect().ok();
        let convs = load_all_conversations();
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

        // Préserve l'état utilisateur avant que le pipeline écrase brain.json :
        //  - contenu édité (tout nœud) ;
        //  - nœuds « note » créés à la main (absents des conversations, sinon perdus).
        // Sauf en sortie de démo : le starter ne doit jamais fuiter dans le vrai cerveau.
        let prev_graph = if was_demo { None } else {
            ai::llama::app_data_dir()
                .and_then(|d| std::fs::read_to_string(d.join("brain.json")).ok())
                .and_then(|raw| serde_json::from_str::<BrainGraph>(&raw).ok())
        };
        let saved_content: std::collections::HashMap<String, String> = prev_graph
            .as_ref()
            .map(|g| g.nodes.iter()
                .filter(|n| !n.content.is_empty())
                .map(|n| (n.id.clone(), n.content.clone()))
                .collect())
            .unwrap_or_default();
        let user_notes: Vec<BrainNode> = prev_graph
            .map(|g| g.nodes.into_iter().filter(|n| n.kind == "note").collect())
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
        )?;

        // Réinjecte le contenu utilisateur sur les nœuds correspondants
        for node in &mut graph.nodes {
            if node.content.is_empty() {
                if let Some(c) = saved_content.get(&node.id) {
                    node.content = c.clone();
                }
            }
        }

        // Ré-ajoute les notes utilisateur ; réattache à la racine si leur parent a disparu.
        let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());
        for mut note in user_notes {
            if graph.nodes.iter().any(|n| n.id == note.id) { continue; }
            let parent_ok = note.parent_id.as_ref()
                .map(|p| graph.nodes.iter().any(|n| &n.id == p))
                .unwrap_or(false);
            if !parent_ok { note.parent_id = root_id.clone(); }
            if let Some(p) = note.parent_id.clone() {
                graph.edges.push(BrainEdge {
                    source: p, target: note.id.clone(), kind: "contains".into(), relation: "contains".into(),
                });
            }
            graph.nodes.push(note);
        }

        if let Some(dir) = ai::llama::app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
            save_snapshot_in(&dir); // snapshot avant écrasement
            let _ = std::fs::write(dir.join("brain.md"), &graph.markdown);
            let _ = std::fs::write(dir.join("brain_report.md"), &graph.report);
            if let Ok(json) = serde_json::to_string_pretty(&graph) {
                let _ = std::fs::write(dir.join("brain.json"), json);
            }
        }
        Ok(graph)
    })
    .await
    .map_err(|e| format!("Tâche d'analyse interrompue : {e}"))?
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    load_env_local();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            list_conversations,
            load_conversation,
            claude_code_available,
            ai_ready,
            connectors_status,
            import_claude_ai,
            google_drive_save_credentials,
            google_drive_has_credentials,
            google_drive_connect,
            google_drive_sync,
            google_drive_disconnect,
            notion_connect,
            notion_sync,
            notion_disconnect,
            notion_load_page,
            ask_brain,
            ask_node,
            generate_content,
            read_brain_graph,
            generate_brain,
            export_node_md,
            ai_setup_needed,
            download_model,
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
            save_pasted_image,
            ai_clients_status,
            connect_ai_client,
            disconnect_ai_client,
            export_backup,
            import_backup,
            set_node_parent,
            rename_node,
            obsidian_set_vault,
            obsidian_vault_path,
            obsidian_disconnect,
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
            ai_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod ask_tests {
    use super::*;

    #[test]
    fn ask_context_selectionne_les_pages_pertinentes_et_reste_borne() {
        let mut nodes = vec![
            BrainNode {
                id: "root".into(), label: "Lucid".into(), kind: "root".into(), weight: 3,
                summary: String::new(), keywords: vec![], decisions: vec![], patterns: vec![],
                community: 0, parent_id: None, synthesized_at: None, content: String::new(),
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

        let ctx = ask_context(&graph, "Quelles pages parlent de Jaon ?", "APERCU");
        assert!(ctx.contains("Notes Jaon"), "la page pertinente doit être incluse");
        assert!(!ctx.contains("Recette"), "les pages hors sujet sont exclues");
        assert!(ctx.len() < 20_000, "le contexte doit rester borné, pas tout le cerveau");
    }
}
