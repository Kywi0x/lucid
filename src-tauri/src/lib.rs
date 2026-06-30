//! Second Brain — point d'entrée de l'application Tauri.
//! Expose les commandes appelables depuis le frontend React.

mod ai;
mod connectors;
mod models;

use ai::{pipeline, LlamaEngine};
use models::{BrainGraph, BrainNode, ConnectorStatus, Conversation, ConversationSummary};
use tauri::Emitter;

/// Accès public au scan (utilisé par les exemples / tests d'intégration).
pub fn list_conversations_pub() -> Vec<ConversationSummary> {
    connectors::claude_code::list_conversations()
}

/// Démo en ligne de commande du pipeline complet (utilisé par `examples/brain.rs`).
/// Persiste le graphe (`brain.json` + `brain.md`) comme le ferait la commande Tauri.
pub fn run_pipeline_demo(limit: usize) -> Result<BrainGraph, String> {
    let engine = LlamaEngine::detect()?;
    let mut convs = load_all_conversations();
    if limit > 0 {
        convs.truncate(limit);
    }
    let cache_path = ai::llama::app_data_dir().map(|d| d.join("brain_cache.json"));
    let graph = pipeline::generate_brain(&engine, &convs, cache_path.as_deref(), |p| {
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
        let (listener, auth_url, redirect_uri) =
            connectors::google_drive::prepare_connect()?;

        use tauri_plugin_opener::OpenerExt;
        app.opener()
            .open_url(&auth_url, None::<&str>)
            .map_err(|e| format!("Impossible d'ouvrir le navigateur : {e}"))?;

        connectors::google_drive::finish_connect(listener, &redirect_uri)
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

/// Chat local sur le `brain.md`.
#[tauri::command]
async fn ask_brain(question: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
        let brain = std::fs::read_to_string(dir.join("brain.md"))
            .map_err(|_| "Génère d'abord ta mind map.".to_string())?;
        let engine = LlamaEngine::detect()?;
        let system = "Tu es l'assistant du second cerveau de l'utilisateur. Réponds en \
français, de façon concise, en te basant UNIQUEMENT sur le contexte fourni (son brain.md). \
Si l'information n'y figure pas, dis-le clairement.";
        let user = format!("CONTEXTE (brain.md) :\n{brain}\n\nQUESTION : {question}");
        engine.complete(Some(system), &user, 512)
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

/// Sauvegarde le contenu markdown libre d'un nœud (jamais écrasé par l'IA).
#[tauri::command]
fn save_node_content(node_id: String, content: String) -> Result<(), String> {
    let dir = ai::llama::app_data_dir().ok_or("Dossier de données introuvable.")?;
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let mut graph: BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    graph.nodes.iter_mut()
        .find(|n| n.id == node_id)
        .ok_or_else(|| format!("Nœud {node_id} introuvable."))?
        .content = content;
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

        // Résumés + décisions des enfants
        let children: Vec<(String, String, Vec<String>)> = graph.nodes.iter()
            .filter(|n| n.parent_id.as_deref() == Some(node_id.as_str()))
            .map(|n| (n.label.clone(), n.summary.clone(), n.decisions.clone()))
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

/// Lance le pipeline IA : analyse toutes les conversations et construit le graphe.
#[tauri::command]
async fn generate_brain(app: tauri::AppHandle) -> Result<BrainGraph, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = LlamaEngine::detect()?;
        let convs = load_all_conversations();
        if convs.is_empty() {
            return Err("Aucune conversation à analyser.".to_string());
        }

        let cache_path = ai::llama::app_data_dir().map(|d| d.join("brain_cache.json"));
        let graph = pipeline::generate_brain(
            &engine,
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

        if let Some(dir) = ai::llama::app_data_dir() {
            let _ = std::fs::create_dir_all(&dir);
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
            obsidian_set_vault,
            obsidian_vault_path,
            obsidian_disconnect
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
