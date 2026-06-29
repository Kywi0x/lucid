//! Second Brain — point d'entrée de l'application Tauri.
//! Expose les commandes appelables depuis le frontend React.

mod ai;
mod connectors;
mod models;

use ai::{pipeline, LlamaEngine};
use models::{BrainGraph, ConnectorStatus, Conversation, ConversationSummary};
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
    })?;
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
    convs
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
#[tauri::command]
fn load_conversation(project_slug: String, id: String) -> Option<Conversation> {
    match project_slug.as_str() {
        "claude-ai" => connectors::claude_ai::load_by_id(&id),
        "google-drive" => connectors::google_drive::load_by_id(&id),
        slug => connectors::claude_code::load_conversation(slug, &id),
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

/// Lit le graphe `brain.json` déjà généré, s'il existe.
#[tauri::command]
fn read_brain_graph() -> Option<BrainGraph> {
    let path = ai::llama::app_data_dir()?.join("brain.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
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
        let graph = pipeline::generate_brain(&engine, &convs, cache_path.as_deref(), |p| {
            let _ = app.emit(
                "brain-progress",
                serde_json::json!({
                    "current": p.current,
                    "total": p.total,
                    "label": p.label,
                }),
            );
        })?;

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            google_drive_connect,
            google_drive_sync,
            google_drive_disconnect,
            ask_brain,
            read_brain_graph,
            generate_brain
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
