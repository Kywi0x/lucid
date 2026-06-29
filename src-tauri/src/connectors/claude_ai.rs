use crate::models::{Conversation, ConversationSummary, Message};
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

pub const SOURCE: &str = "claude-ai";

#[derive(Deserialize)]
struct RawConv {
    uuid: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    chat_messages: Vec<RawMsg>,
}

#[derive(Deserialize)]
struct RawMsg {
    sender: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    created_at: Option<String>,
}

fn msg_text(m: &RawMsg) -> String {
    if let Some(t) = &m.text {
        if !t.trim().is_empty() {
            return t.clone();
        }
    }
    match &m.content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn map_conv(raw: RawConv) -> Option<Conversation> {
    let messages: Vec<Message> = raw
        .chat_messages
        .iter()
        .filter_map(|m| {
            let text = msg_text(m).trim().to_string();
            if text.is_empty() {
                return None;
            }
            let role = if m.sender == "human" { "user" } else { "assistant" }.to_string();
            Some(Message { role, text, timestamp: m.created_at.clone() })
        })
        .collect();

    if messages.is_empty() {
        return None;
    }

    let title = raw
        .name
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| truncate(&m.text, 80))
                .unwrap_or_else(|| "Conversation".to_string())
        });

    let first_ts = messages
        .first()
        .and_then(|m| m.timestamp.clone())
        .or(raw.created_at.clone());
    let last_ts = messages
        .last()
        .and_then(|m| m.timestamp.clone())
        .or(raw.updated_at);

    Some(Conversation {
        summary: ConversationSummary {
            id: raw.uuid,
            title,
            project: "claude.ai".to_string(),
            project_slug: "claude-ai".to_string(),
            source: SOURCE.to_string(),
            container_path: vec![], // TODO chantier 5 doc : dériver depuis le projet claude.ai
            message_count: messages.len(),
            first_timestamp: first_ts,
            last_timestamp: last_ts,
        },
        messages,
    })
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        return s;
    }
    format!("{}…", s.chars().take(max).collect::<String>().trim_end())
}

/// Parse un export ZIP claude.ai → `Vec<Conversation>`.
pub fn parse_zip(path: &Path) -> Result<Vec<Conversation>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Impossible d'ouvrir le fichier : {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Fichier ZIP invalide : {e}"))?;

    // Cherche conversations.json n'importe où dans le ZIP.
    let idx = {
        let mut found = None;
        for i in 0..archive.len() {
            if let Ok(f) = archive.by_index(i) {
                if f.name().ends_with("conversations.json") {
                    found = Some(i);
                    break;
                }
            }
        }
        found
    }
    .ok_or("conversations.json introuvable dans le ZIP.")?;

    let mut entry = archive.by_index(idx).map_err(|e| e.to_string())?;
    let mut contents = String::new();
    entry.read_to_string(&mut contents).map_err(|e| e.to_string())?;

    let raw: Vec<RawConv> =
        serde_json::from_str(&contents).map_err(|e| format!("JSON invalide : {e}"))?;

    Ok(raw.into_iter().filter_map(map_conv).collect())
}

fn data_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("claude_ai_conversations.json"))
}

/// Persiste les conversations importées dans le dossier de données applicatif.
pub fn save_conversations(convs: &[Conversation]) -> Result<(), String> {
    let path = data_path().ok_or("Dossier de données introuvable.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string(convs).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Charge toutes les conversations claude.ai précédemment importées.
pub fn load_conversations() -> Vec<Conversation> {
    let path = match data_path() {
        Some(p) => p,
        None => return vec![],
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

/// Charge une conversation par son id (uuid).
pub fn load_by_id(id: &str) -> Option<Conversation> {
    load_conversations().into_iter().find(|c| c.summary.id == id)
}
