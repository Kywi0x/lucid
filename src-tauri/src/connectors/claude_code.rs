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

/// Fichier-drapeau : présent = connecteur désactivé par l'utilisateur.
fn disabled_flag() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("claude_code_disabled"))
}

/// Désactive le connecteur (réversible via `reconnect`).
pub fn disconnect() {
    if let Some(p) = disabled_flag() {
        let _ = fs::write(p, "");
    }
}

/// Réactive le connecteur.
pub fn reconnect() {
    if let Some(p) = disabled_flag() {
        let _ = fs::remove_file(p);
    }
}

/// Renvoie `~/.claude/projects` si le dossier existe et que le connecteur est actif.
pub fn projects_dir() -> Option<PathBuf> {
    if disabled_flag().map(|p| p.exists()).unwrap_or(false) {
        return None;
    }
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
/// - `text`     → tel quel (Markdown natif des réponses Claude)
/// - `tool_use` → ligne de résumé compacte (outil + argument clé)
/// - `tool_result` → ignoré (souvent du contenu brut verbeux)
fn extract_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            let mut parts: Vec<String> = Vec::new();
            for block in blocks {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            let t = t.trim();
                            if !t.is_empty() { parts.push(t.to_string()); }
                        }
                    }
                    Some("tool_use") => {
                        if let Some(name) = block.get("name").and_then(Value::as_str) {
                            let arg = tool_summary(name, block.get("input"));
                            let line = if arg.is_empty() {
                                format!("> `{name}`")
                            } else {
                                format!("> `{name}` {arg}")
                            };
                            parts.push(line);
                        }
                    }
                    _ => {}
                }
            }
            parts.join("\n\n")
        }
        _ => String::new(),
    }
}

fn char_truncate(s: &str, max: usize) -> String {
    let mut indices = s.char_indices();
    match indices.nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// Résume l'argument principal d'un appel d'outil en une ligne courte.
fn tool_summary(name: &str, input: Option<&Value>) -> String {
    let Some(inp) = input else { return String::new(); };
    // Argument principal selon le type d'outil
    let key = match name {
        "Read" | "Edit" | "Write" | "MultiEdit" => "file_path",
        "Bash"                                   => "command",
        "Agent"                                  => "prompt",
        "WebFetch" | "WebSearch"                 => "url",
        _                                        => "",
    };
    if !key.is_empty() {
        if let Some(val) = inp.get(key).and_then(Value::as_str) {
            let short = char_truncate(val, 80);
            return format!("`{short}`");
        }
    }
    // Fallback : premier champ string de l'input
    if let Some(obj) = inp.as_object() {
        if let Some((_, v)) = obj.iter().find(|(_, v)| v.is_string()) {
            if let Some(s) = v.as_str() {
                return format!("`{}`", char_truncate(s, 60));
            }
        }
    }
    String::new()
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
    fn extracts_text_and_formats_tool_use() {
        let v: Value = serde_json::json!([
            {"type": "text", "text": "première"},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "src/main.rs"}},
            {"type": "tool_result", "content": "…file content…"},
            {"type": "text", "text": "seconde"}
        ]);
        let result = extract_text(&v);
        assert!(result.contains("première"), "text block missing");
        assert!(result.contains("> `Read`"), "tool_use line missing");
        assert!(result.contains("src/main.rs"), "tool arg missing");
        assert!(result.contains("seconde"), "second text block missing");
        assert!(!result.contains("file content"), "tool_result should be ignored");
    }

    #[test]
    fn truncate_respects_max() {
        assert_eq!(truncate("court", 80), "court");
        assert!(truncate(&"a".repeat(200), 80).chars().count() <= 81);
    }
}
