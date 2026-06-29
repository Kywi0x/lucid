//! Connecteur Claude Code.
//!
//! Claude Code stocke chaque session dans `~/.claude/projects/<slug>/<sessionId>.jsonl`,
//! où `<slug>` est le chemin de travail encodé (les `/` remplacés par des `-`).
//! Chaque ligne du fichier est un objet JSON typé (`type`: user, assistant, system,
//! ai-title, ...). On ne conserve que le signal porteur de sens : les prompts
//! utilisateur et les réponses texte de l'assistant.

use crate::models::{Conversation, ConversationSummary, Message};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Renvoie `~/.claude/projects` si le dossier existe.
pub fn projects_dir() -> Option<PathBuf> {
    let dir = dirs::home_dir()?.join(".claude").join("projects");
    if dir.is_dir() {
        Some(dir)
    } else {
        None
    }
}

/// Décode un slug de dossier (`-Users-liam-GitHub-brainlink`) en chemin lisible.
/// Heuristique de repli : on ne s'en sert que si aucun `cwd` n'est présent dans le
/// fichier. Le `cwd` reste la source de vérité.
fn decode_slug(slug: &str) -> String {
    // Claude Code encode `/` en `-`. On ne peut pas distinguer un vrai `-` d'un
    // séparateur, donc on reconstruit un chemin et on garde le nom final lisible.
    let path = slug.replacen('-', "/", 1).replace('-', "/");
    path
}

/// Extrait le texte exploitable d'un champ `content` (string ou tableau de blocs).
/// - string  → tel quel
/// - tableau → concatène uniquement les blocs `{ "type": "text" }`
fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            let mut out = String::new();
            for block in blocks {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Parse un fichier `.jsonl` en une conversation complète.
/// Tolérant : les lignes vides ou invalides sont ignorées.
fn parse_file(path: &Path, project_slug: &str) -> Option<Conversation> {
    let raw = fs::read_to_string(path).ok()?;
    let id = path.file_stem()?.to_string_lossy().to_string();

    let mut messages: Vec<Message> = Vec::new();
    let mut ai_title: Option<String> = None;
    let mut custom_title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;

    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue; // ligne corrompue → on saute
        };
        let line_type = v.get("type").and_then(Value::as_str).unwrap_or("");

        match line_type {
            "ai-title" => {
                ai_title = v.get("aiTitle").and_then(Value::as_str).map(str::to_string);
            }
            "custom-title" => {
                custom_title = v
                    .get("customTitle")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "user" | "assistant" => {
                if cwd.is_none() {
                    cwd = v.get("cwd").and_then(Value::as_str).map(str::to_string);
                }
                let ts = v.get("timestamp").and_then(Value::as_str).map(str::to_string);
                let role = v
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(Value::as_str)
                    .unwrap_or(line_type)
                    .to_string();
                let content = v.get("message").and_then(|m| m.get("content"));
                let text = content.map(extract_text).unwrap_or_default();
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue; // messages purement outillage (tool_result/use)
                }
                if first_ts.is_none() {
                    first_ts = ts.clone();
                }
                if ts.is_some() {
                    last_ts = ts.clone();
                }
                messages.push(Message { role, text, timestamp: ts });
            }
            _ => {}
        }
    }

    if messages.is_empty() {
        return None; // session sans contenu exploitable
    }

    // Titre : custom > ai > premier prompt utilisateur tronqué.
    let title = custom_title
        .or(ai_title)
        .or_else(|| {
            messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| truncate(&m.text, 80))
        })
        .unwrap_or_else(|| "Conversation sans titre".to_string());

    let project = cwd.unwrap_or_else(|| decode_slug(project_slug));
    let repo_name = project
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(&project)
        .to_string();

    let summary = ConversationSummary {
        id,
        title,
        project,
        project_slug: project_slug.to_string(),
        source: super::SOURCE_CLAUDE_CODE.to_string(),
        container_path: vec![repo_name],
        message_count: messages.len(),
        first_timestamp: first_ts,
        last_timestamp: last_ts,
    };

    Some(Conversation { summary, messages })
}

fn truncate(s: &str, max: usize) -> String {
    let s = s.replace('\n', " ");
    if s.chars().count() <= max {
        s
    } else {
        let truncated: String = s.chars().take(max).collect();
        format!("{}…", truncated.trim_end())
    }
}

/// Liste toutes les conversations (vue légère, sans messages) de tous les projets.
pub fn list_conversations() -> Vec<ConversationSummary> {
    let Some(root) = projects_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();

    let Ok(projects) = fs::read_dir(&root) else {
        return out;
    };
    for project in projects.flatten() {
        if !project.path().is_dir() {
            continue;
        }
        let slug = project.file_name().to_string_lossy().to_string();
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let p = file.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(conv) = parse_file(&p, &slug) {
                out.push(conv.summary);
            }
        }
    }

    // Plus récentes d'abord.
    out.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));
    out
}

/// Charge une conversation complète (messages inclus) par projet + id.
pub fn load_conversation(project_slug: &str, id: &str) -> Option<Conversation> {
    let root = projects_dir()?;
    let path = root.join(project_slug).join(format!("{id}.jsonl"));
    parse_file(&path, project_slug)
}

/// Charge toutes les conversations complètes (messages inclus) — pour le pipeline IA.
/// Triées des plus récentes aux plus anciennes.
pub fn load_all_conversations() -> Vec<Conversation> {
    let Some(root) = projects_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Ok(projects) = fs::read_dir(&root) else {
        return out;
    };
    for project in projects.flatten() {
        if !project.path().is_dir() {
            continue;
        }
        let slug = project.file_name().to_string_lossy().to_string();
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let p = file.path();
            if p.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(conv) = parse_file(&p, &slug) {
                out.push(conv);
            }
        }
    }
    out.sort_by(|a, b| b.summary.last_timestamp.cmp(&a.summary.last_timestamp));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_string_content() {
        let v: Value = serde_json::json!("bonjour");
        assert_eq!(extract_text(&v), "bonjour");
    }

    #[test]
    fn extracts_text_blocks_only() {
        let v: Value = serde_json::json!([
            {"type": "text", "text": "première"},
            {"type": "tool_use", "name": "x"},
            {"type": "text", "text": "seconde"}
        ]);
        assert_eq!(extract_text(&v), "première\nseconde");
    }

    #[test]
    fn truncate_respects_max() {
        assert_eq!(truncate("court", 80), "court");
        assert!(truncate(&"a".repeat(200), 80).chars().count() <= 81);
    }
}
