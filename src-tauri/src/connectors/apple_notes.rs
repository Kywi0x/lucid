//! Connecteur « Notes Apple » — macOS uniquement (pas d'équivalent Windows,
//! Notes.app n'existe pas là-bas). Lecture via l'automatisation sanctionnée
//! par Apple (`osascript -l JavaScript`), jamais la base SQLite interne de
//! Notes (contenu stocké en protobuf gzip non documenté, casse à chaque maj
//! macOS). Lecture seule. Le premier accès déclenche le prompt système natif
//! ("Lucid veut contrôler Notes.app" — Réglages → Confidentialité →
//! Automatisation) ; si refusé, l'erreur d'osascript est traduite en message
//! actionnable (ADR-0015), jamais un échec silencieux.

use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const SOURCE: &str = "apple-notes";

fn config_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("apple_notes_config.json"))
}

fn cache_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("apple_notes_conversations.json"))
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    connected: bool,
}

fn save_config(cfg: &Config) -> Result<(), String> {
    let p = config_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(p, serde_json::to_string(cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn is_connected() -> bool {
    let Some(path) = config_path() else { return false };
    std::fs::read_to_string(path).ok()
        .and_then(|r| serde_json::from_str::<Config>(&r).ok())
        .map(|c| c.connected)
        .unwrap_or(false)
}

pub fn disconnect() {
    if let Some(p) = config_path() { let _ = std::fs::remove_file(p); }
    if let Some(c) = cache_path() { let _ = std::fs::remove_file(c); }
}

#[cfg(target_os = "macos")]
pub fn available() -> bool { true }
#[cfg(not(target_os = "macos"))]
pub fn available() -> bool { false }

#[derive(Deserialize)]
struct RawNote {
    id: String,
    name: String,
    body: String,
    #[serde(rename = "modDate")]
    mod_date: String,
    folder: String,
}

#[cfg(target_os = "macos")]
fn fetch_notes() -> Result<Vec<RawNote>, String> {
    // JXA plutôt qu'AppleScript classique : sortie JSON directe, un seul
    // appel osascript pour toute la bibliothèque (pas un process par note).
    const SCRIPT: &str = r#"
    const Notes = Application('Notes');
    Notes.includeStandardAdditions = true;
    const notes = Notes.notes();
    const out = notes.map(n => {
      let folder = "";
      try { folder = n.container().name(); } catch (e) {}
      return { id: n.id(), name: n.name(), body: n.body(), modDate: n.modificationDate().toISOString(), folder };
    });
    JSON.stringify(out);
    "#;
    let out = std::process::Command::new("osascript")
        .args(["-l", "JavaScript", "-e", SCRIPT])
        .output()
        .map_err(|e| format!("osascript indisponible : {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("Not authorized") || err.contains("-1743") {
            return Err("Autorisation refusée — Réglages Système → Confidentialité et sécurité → Automatisation → autorise Lucid à contrôler Notes.".into());
        }
        return Err(format!("Lecture des notes impossible : {}", err.trim()));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(&stdout).map_err(|e| format!("Réponse inattendue de Notes : {e}"))
}

#[cfg(not(target_os = "macos"))]
fn fetch_notes() -> Result<Vec<RawNote>, String> {
    Err("Notes Apple n'est disponible que sur Mac.".into())
}

/// HTML (format natif du corps d'une note Apple) → texte lisible : tags
/// strippés, entités de base décodées. Même esprit que `google_drive::xml_text`
/// (pas un vrai parseur DOM — suffisant pour l'indexation, pas pour un rendu fidèle).
fn html_to_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&quot;", "\"");
    out.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}

fn write_cache(notes: &[RawNote]) -> Result<(), String> {
    let convs: Vec<Conversation> = notes.iter().map(|n| Conversation {
        summary: ConversationSummary {
            id: n.id.clone(),
            title: if n.name.trim().is_empty() { "Sans titre".into() } else { n.name.clone() },
            project: "Notes Apple".into(),
            project_slug: SOURCE.into(),
            source: SOURCE.into(),
            container_path: vec![
                "Notes Apple".into(),
                if n.folder.trim().is_empty() { "Notes".into() } else { n.folder.clone() },
            ],
            message_count: 1,
            first_timestamp: Some(n.mod_date.clone()),
            last_timestamp: Some(n.mod_date.clone()),
        },
        messages: vec![Message {
            role: "assistant".into(),
            text: html_to_text(&n.body),
            timestamp: Some(n.mod_date.clone()),
        }],
    }).collect();

    let path = cache_path().ok_or("Dossier de données introuvable.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, serde_json::to_string(&convs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Synchronise (réécrit le cache en entier — une bibliothèque de notes reste
/// petite comparée à des PDF, pas besoin d'incrémental). Renvoie le nombre de notes.
pub fn sync() -> Result<usize, String> {
    let notes = fetch_notes()?;
    let n = notes.len();
    write_cache(&notes)?;
    Ok(n)
}

/// Première connexion : synchronise et marque comme connecté si ça réussit
/// (l'autorisation macOS vient d'être accordée). Renvoie le nombre de notes.
pub fn connect() -> Result<usize, String> {
    let n = sync()?;
    save_config(&Config { connected: true })?;
    Ok(n)
}

pub fn load_conversations() -> Vec<Conversation> {
    let Some(path) = cache_path() else { return vec![] };
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

    #[test]
    fn html_to_text_strip_les_tags_et_decode_les_entites() {
        let html = "<div><b>Courses</b><br>Lait &amp; pain &nbsp;&nbsp;</div><div>Fin</div>";
        let text = html_to_text(html);
        assert!(text.contains("Courses"));
        assert!(text.contains("Lait & pain"), "entités décodées : {text}");
        assert!(!text.contains('<'), "tags strippés : {text}");
    }
}
