//! Connecteur ChatGPT : import de l'export ZIP (chatgpt.com → Settings →
//! Data controls → Export data). Contrairement à claude.ai, chaque conversation
//! est un ARBRE (`mapping` id → nœud, liens parent/children, branches créées
//! par les régénérations) : on suit la branche active en remontant depuis
//! `current_node`.

use crate::models::{Conversation, ConversationSummary, Message};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

pub const SOURCE: &str = "chatgpt";

#[derive(Deserialize)]
struct RawConv {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    create_time: Option<f64>,
    #[serde(default)]
    update_time: Option<f64>,
    #[serde(default)]
    mapping: HashMap<String, RawNode>,
    #[serde(default)]
    current_node: Option<String>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
}

#[derive(Deserialize)]
struct RawNode {
    #[serde(default)]
    message: Option<RawMsg>,
    #[serde(default)]
    parent: Option<String>,
}

#[derive(Deserialize)]
struct RawMsg {
    author: RawAuthor,
    #[serde(default)]
    create_time: Option<f64>,
    #[serde(default)]
    content: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RawAuthor {
    role: String,
}

fn epoch_iso(t: Option<f64>) -> Option<String> {
    chrono::DateTime::from_timestamp(t? as i64, 0)
        .map(|d: chrono::DateTime<chrono::Utc>| d.to_rfc3339())
}

/// Texte d'un message : `content.parts` (text / multimodal, on ne garde que
/// les chaînes) ou `content.text` (content_type "code").
fn msg_text(content: &Option<serde_json::Value>) -> String {
    let Some(v) = content else { return String::new() };
    if let Some(parts) = v.get("parts").and_then(|p| p.as_array()) {
        return parts.iter().filter_map(|p| p.as_str()).collect::<Vec<_>>().join("\n");
    }
    v.get("text").and_then(|t| t.as_str()).unwrap_or_default().to_string()
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        return s;
    }
    format!("{}…", s.chars().take(max).collect::<String>().trim_end())
}

fn map_conv(raw: RawConv) -> Option<Conversation> {
    // Branche active : current_node → racine, puis inversion.
    let mut ordered: Vec<&RawMsg> = Vec::new();
    let mut cur = raw.current_node.clone();
    let mut guard = 0;
    while let Some(id) = cur {
        let Some(node) = raw.mapping.get(&id) else { break };
        if let Some(m) = &node.message {
            ordered.push(m);
        }
        cur = node.parent.clone();
        guard += 1;
        if guard > 100_000 { break; } // mapping cyclique = export corrompu
    }
    ordered.reverse();

    // Fallback (pas de current_node exploitable) : tous les nœuds par date.
    if ordered.is_empty() {
        ordered = raw.mapping.values().filter_map(|n| n.message.as_ref()).collect();
        ordered.sort_by(|a, b| {
            a.create_time.partial_cmp(&b.create_time).unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    let messages: Vec<Message> = ordered
        .iter()
        .filter_map(|m| {
            // system, tool… n'apportent rien au cerveau.
            let role = match m.author.role.as_str() {
                "user" => "user",
                "assistant" => "assistant",
                _ => return None,
            };
            let text = msg_text(&m.content).trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(Message { role: role.into(), text, timestamp: epoch_iso(m.create_time) })
        })
        .collect();

    if messages.is_empty() {
        return None;
    }

    let id = raw.conversation_id.or(raw.id)?;
    let title = raw
        .title
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
        .or_else(|| epoch_iso(raw.create_time));
    let last_ts = messages
        .last()
        .and_then(|m| m.timestamp.clone())
        .or_else(|| epoch_iso(raw.update_time));

    Some(Conversation {
        summary: ConversationSummary {
            id,
            title,
            project: "ChatGPT".to_string(),
            project_slug: SOURCE.to_string(),
            source: SOURCE.to_string(),
            container_path: vec![],
            message_count: messages.len(),
            first_timestamp: first_ts,
            last_timestamp: last_ts,
        },
        messages,
    })
}

/// Parse un export ZIP ChatGPT → `Vec<Conversation>`.
pub fn parse_zip(path: &Path) -> Result<Vec<Conversation>, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("Impossible d'ouvrir le fichier : {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Fichier ZIP invalide : {e}"))?;

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
    .ok_or("conversations.json introuvable dans le ZIP — est-ce bien un export ChatGPT ?")?;

    let mut entry = archive.by_index(idx).map_err(|e| e.to_string())?;
    let mut contents = String::new();
    entry.read_to_string(&mut contents).map_err(|e| e.to_string())?;

    let raw: Vec<RawConv> =
        serde_json::from_str(&contents).map_err(|e| format!("JSON invalide : {e}"))?;

    Ok(raw.into_iter().filter_map(map_conv).collect())
}

fn data_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("chatgpt_conversations.json"))
}

pub fn save_conversations(convs: &[Conversation]) -> Result<(), String> {
    let path = data_path().ok_or("Dossier de données introuvable.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string(convs).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

pub fn load_conversations() -> Vec<Conversation> {
    let Some(path) = data_path() else { return vec![] };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

pub fn load_by_id(id: &str) -> Option<Conversation> {
    load_conversations().into_iter().find(|c| c.summary.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mini-export : racine système → question → deux branches de réponse
    /// (régénération). current_node pointe la branche B : seule B doit sortir.
    #[test]
    fn suit_la_branche_active_du_mapping() {
        let json = r#"[{
            "title": "Anatomie",
            "conversation_id": "conv-1",
            "create_time": 1751000000.5,
            "mapping": {
                "root": {"message": null, "parent": null, "children": ["m1"]},
                "m1": {"message": {"author": {"role": "user"}, "create_time": 1751000001.0,
                        "content": {"content_type": "text", "parts": ["Explique le fémur"]}},
                       "parent": "root", "children": ["a", "b"]},
                "a": {"message": {"author": {"role": "assistant"},
                        "content": {"content_type": "text", "parts": ["Réponse A (abandonnée)"]}},
                      "parent": "m1", "children": []},
                "b": {"message": {"author": {"role": "assistant"},
                        "content": {"content_type": "text", "parts": ["Réponse B (active)"]}},
                      "parent": "m1", "children": []}
            },
            "current_node": "b"
        }]"#;
        let raw: Vec<RawConv> = serde_json::from_str(json).unwrap();
        let conv = map_conv(raw.into_iter().next().unwrap()).unwrap();

        assert_eq!(conv.summary.id, "conv-1");
        assert_eq!(conv.summary.source, "chatgpt");
        assert_eq!(conv.messages.len(), 2, "system/racine vide exclus");
        assert_eq!(conv.messages[0].role, "user");
        assert!(conv.messages[1].text.contains("Réponse B"), "la branche active gagne");
        assert!(!conv.messages.iter().any(|m| m.text.contains("Réponse A")));
        assert!(conv.messages[0].timestamp.as_deref().unwrap().starts_with("2025"), "epoch → ISO");
    }

    #[test]
    fn content_type_code_via_champ_text() {
        let v: Option<serde_json::Value> =
            Some(serde_json::json!({"content_type": "code", "text": "print(1)"}));
        assert_eq!(msg_text(&v), "print(1)");
    }
}
