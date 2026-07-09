//! Connecteur « Dossier local » : indexe un dossier de fichiers (PDF, Word,
//! PowerPoint, Markdown, texte, CSV). Sync explicite → extraction markdown
//! (via `crate::file_to_markdown`) mise en cache ; incrémental par mtime.
//! Les fichiers illisibles sont listés dans le rapport — jamais d'échec
//! silencieux (ADR-0015).

use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "local-folder";

/// Extensions parcourues. .doc/.rtf/.ppt sont inclus exprès : l'extraction
/// peut échouer (Windows, format legacy) mais l'échec est alors expliqué
/// dans le rapport plutôt que le fichier ignoré sans un mot.
const EXTENSIONS: &[&str] = &["md", "markdown", "txt", "pdf", "docx", "pptx", "csv", "doc", "rtf", "ppt"];

// ─── Config ──────────────────────────────────────────────────────────────────

fn config_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("local_folder_config.json"))
}

fn cache_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("local_folder_conversations.json"))
}

#[derive(Serialize, Deserialize)]
struct Config {
    folder_path: String,
}

pub fn set_folder(path: &str) -> Result<(), String> {
    if !Path::new(path).is_dir() {
        return Err(format!("Dossier introuvable : {path}"));
    }
    let p = config_path().ok_or("Dossier de données introuvable.")?;
    // Changement de dossier → le cache (chemins relatifs) ne vaut plus rien.
    if let Some(c) = cache_path() { let _ = std::fs::remove_file(c); }
    std::fs::write(
        p,
        serde_json::to_string(&Config { folder_path: path.to_string() }).map_err(|e| e.to_string())?,
    ).map_err(|e| e.to_string())
}

pub fn folder_path() -> Option<String> {
    let raw = std::fs::read_to_string(config_path()?).ok()?;
    serde_json::from_str::<Config>(&raw).ok().map(|c| c.folder_path)
}

pub fn is_connected() -> bool {
    folder_path().map(|p| Path::new(&p).is_dir()).unwrap_or(false)
}

pub fn disconnect() {
    if let Some(p) = config_path() { let _ = std::fs::remove_file(p); }
    if let Some(c) = cache_path() { let _ = std::fs::remove_file(c); }
}

// ─── Walk ─────────────────────────────────────────────────────────────────────

fn walk(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out);
    out
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') { continue; }
        if path.is_dir() {
            walk_dir(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
        {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
}

// ─── Sync ─────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SyncReport {
    /// Fichiers extraits (nouveaux ou modifiés) pendant ce sync.
    pub new: usize,
    /// Fichiers indexés au total après le sync.
    pub total: usize,
    /// Fichiers illisibles : "chemin — raison".
    pub skipped: Vec<String>,
}

fn file_to_conversation(root_str: &str, rel: &str, abs: &Path) -> Result<Conversation, String> {
    let text = crate::file_to_markdown(abs)?;
    let (sub_path, title) = super::obsidian::rel_to_parts(rel);
    let folder_name = Path::new(root_str)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Dossier local".to_string());
    let mut container_path = vec![folder_name];
    container_path.extend(sub_path);

    let ts = super::obsidian::mtime_iso(abs);
    Ok(Conversation {
        summary: ConversationSummary {
            id: rel.to_string(),
            title,
            project: "Dossier local".into(),
            project_slug: SOURCE.into(),
            source: SOURCE.into(),
            container_path,
            message_count: 1,
            first_timestamp: ts.clone(),
            last_timestamp: ts,
        },
        messages: vec![Message { role: "assistant".into(), text, timestamp: super::obsidian::mtime_iso(abs) }],
    })
}

/// Parcourt le dossier configuré, extrait les fichiers nouveaux/modifiés
/// (comparaison mtime vs cache) et réécrit le cache. Les fichiers disparus
/// sortent du cache d'eux-mêmes (on ne garde que ce qui est sur disque).
pub fn sync() -> Result<SyncReport, String> {
    let root_str = folder_path().ok_or("Aucun dossier configuré.")?;
    let root = Path::new(&root_str);
    if !root.is_dir() {
        return Err(format!("Le dossier n'existe plus : {root_str}"));
    }

    let cached: std::collections::HashMap<String, Conversation> = load_conversations()
        .into_iter()
        .map(|c| (c.summary.id.clone(), c))
        .collect();

    let mut out = Vec::new();
    let mut new_count = 0usize;
    let mut skipped = Vec::new();

    for (rel, abs) in walk(root) {
        let mtime = super::obsidian::mtime_iso(&abs);
        if let Some(prev) = cached.get(&rel) {
            if prev.summary.last_timestamp == mtime {
                out.push(prev.clone());
                continue;
            }
        }
        if abs.metadata().map(|m| m.len() > 100 * 1024 * 1024).unwrap_or(false) {
            skipped.push(format!("{rel} — fichier > 100 Mo, ignoré"));
            continue;
        }
        match file_to_conversation(&root_str, &rel, &abs) {
            Ok(c) => { new_count += 1; out.push(c); }
            Err(e) => skipped.push(format!("{rel} — {e}")),
        }
    }

    let path = cache_path().ok_or("Dossier de données introuvable.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string(&out).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    Ok(SyncReport { new: new_count, total: out.len(), skipped })
}

// ─── Lecture ──────────────────────────────────────────────────────────────────

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
    fn walk_filtre_extensions_et_dossiers_caches() {
        let dir = std::env::temp_dir().join("lucid_test_local_folder");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cours/.git")).unwrap();
        std::fs::write(dir.join("cours/anatomie.md"), "# Cours").unwrap();
        std::fs::write(dir.join("cours/photo.png"), "x").unwrap();
        std::fs::write(dir.join("cours/.git/notes.md"), "x").unwrap();
        std::fs::write(dir.join("Notes.TXT"), "x").unwrap();

        let files = walk(&dir);
        let rels: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert!(rels.contains(&"cours/anatomie.md"));
        assert!(rels.contains(&"Notes.TXT"), "extension insensible à la casse");
        assert_eq!(rels.len(), 2, "png et dossiers cachés exclus : {rels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
