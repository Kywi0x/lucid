//! Connecteur Obsidian.
//! Lit les fichiers .md d'un vault local. Lecture directe, pas de cache.

use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ─── Config ──────────────────────────────────────────────────────────────────

fn config_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("obsidian_config.json"))
}

#[derive(Serialize, Deserialize)]
struct Config {
    vault_path: String,
}

pub fn set_vault(path: &str) -> Result<(), String> {
    if !Path::new(path).is_dir() {
        return Err(format!("Dossier introuvable : {path}"));
    }
    let p = config_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(
        p,
        serde_json::to_string(&Config { vault_path: path.to_string() }).map_err(|e| e.to_string())?,
    ).map_err(|e| e.to_string())
}

fn load_vault() -> Option<String> {
    let raw = std::fs::read_to_string(config_path()?).ok()?;
    serde_json::from_str::<Config>(&raw).ok().map(|c| c.vault_path)
}

pub fn is_connected() -> bool {
    load_vault().map(|p| Path::new(&p).is_dir()).unwrap_or(false)
}

pub fn disconnect() {
    if let Some(p) = config_path() { let _ = std::fs::remove_file(p); }
}

pub fn vault_path() -> Option<String> {
    load_vault()
}

// ─── Auto-détection ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ObsidianVaultEntry {
    path: String,
    #[serde(default)]
    ts: u64,
}

#[derive(Deserialize)]
struct ObsidianAppConfig {
    #[serde(default)]
    vaults: std::collections::HashMap<String, ObsidianVaultEntry>,
}

/// Fichier de config d'Obsidian lui-même (liste les vaults déjà ouverts sur
/// cette machine) — même chemin sur Mac et Windows via `dirs::config_dir()`
/// (`~/Library/Application Support` / `%APPDATA%`).
fn obsidian_app_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("obsidian").join("obsidian.json"))
}

/// Détecte le vault Obsidian le plus récemment utilisé et le connecte
/// automatiquement si aucun vault n'est déjà configuré. Renvoie le chemin
/// connecté (existant ou nouvellement détecté), `None` si Obsidian n'a jamais
/// tourné sur cette machine — absent, pas une erreur.
pub fn auto_connect() -> Option<String> {
    if let Some(existing) = load_vault() {
        return Some(existing);
    }
    let raw = std::fs::read_to_string(obsidian_app_config_path()?).ok()?;
    let cfg: ObsidianAppConfig = serde_json::from_str(&raw).ok()?;
    let best = cfg.vaults.values()
        .filter(|v| Path::new(&v.path).is_dir())
        .max_by_key(|v| v.ts)?;
    set_vault(&best.path).ok()?;
    Some(best.path.clone())
}

// ─── Walk ─────────────────────────────────────────────────────────────────────

fn walk_vault(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    walk_dir(root, root, &mut out);
    out
}

fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip hidden dirs (.obsidian, .git, etc.)
        if entry.file_name().to_string_lossy().starts_with('.') { continue; }
        if path.is_dir() {
            walk_dir(root, &path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            if let Ok(rel) = path.strip_prefix(root) {
                out.push((rel.to_string_lossy().replace('\\', "/"), path));
            }
        }
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn mtime_iso(path: &Path) -> Option<String> {
    use std::time::UNIX_EPOCH;
    let secs = path.metadata().ok()?.modified().ok()?
        .duration_since(UNIX_EPOCH).ok()?.as_secs();
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt: chrono::DateTime<chrono::Utc>| dt.to_rfc3339())
}

/// Splits "04 Stack/ADR/note.md" → (["04 Stack", "ADR"], "note")
pub(crate) fn rel_to_parts(rel: &str) -> (Vec<String>, String) {
    let parts: Vec<&str> = rel.split('/').collect();
    let title = parts.last()
        .map(|f| Path::new(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| f.to_string()))
        .unwrap_or_else(|| rel.to_string());
    let container = parts[..parts.len().saturating_sub(1)]
        .iter().map(|s| s.to_string()).collect();
    (container, title)
}

fn file_to_conversation(vault_root: &str, rel: &str, abs: &Path) -> Option<Conversation> {
    let (sub_path, title) = rel_to_parts(rel);
    let vault_name = Path::new(vault_root)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Obsidian".to_string());
    // Tout le contenu Obsidian est groupé sous le nœud vault_name.
    let mut container_path = vec![vault_name];
    container_path.extend(sub_path);

    let text = std::fs::read_to_string(abs).ok()?;
    if text.trim().is_empty() { return None; }
    let ts = mtime_iso(abs);
    Some(Conversation {
        summary: ConversationSummary {
            id: rel.to_string(),
            title,
            project: "obsidian".into(),
            project_slug: "obsidian".into(),
            source: "obsidian".into(),
            container_path,
            message_count: 1,
            first_timestamp: ts.clone(),
            last_timestamp: ts,
        },
        messages: vec![Message {
            role: "assistant".into(),
            text,
            timestamp: mtime_iso(abs),
        }],
    })
}

// ─── Public API ───────────────────────────────────────────────────────────────

pub fn load_all_conversations() -> Vec<Conversation> {
    let Some(root_str) = load_vault() else { return vec![] };
    let root = Path::new(&root_str);
    if !root.is_dir() { return vec![]; }
    walk_vault(root)
        .into_iter()
        .filter_map(|(rel, abs)| file_to_conversation(&root_str, &rel, &abs))
        .collect()
}

pub fn load_by_id(id: &str) -> Option<Conversation> {
    let root_str = load_vault()?;
    let abs = Path::new(&root_str).join(id);
    file_to_conversation(&root_str, id, &abs)
}

pub fn count_files() -> usize {
    let Some(root_str) = load_vault() else { return 0 };
    let root = Path::new(&root_str);
    if !root.is_dir() { return 0; }
    walk_vault(root).len()
}
