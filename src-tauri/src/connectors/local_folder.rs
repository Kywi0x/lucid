//! Connecteur « Dossiers locaux » : indexe un ou plusieurs dossiers de
//! documents bureautiques (PDF, Word, PowerPoint, Excel, CSV). Volontairement
//! restreint à ces formats — pas de .md/.txt : les dossiers par défaut
//! (Bureau/Documents…) contiennent souvent des projets de code (README.md,
//! logs…) qu'on ne veut pas aspirer dans le cerveau. À la connexion, les
//! dossiers standards de l'utilisateur (Bureau/Documents/Téléchargements —
//! ceux qui existent) sont ajoutés automatiquement ; l'utilisateur peut en
//! ajouter d'autres ou en retirer. Sync explicite → extraction markdown (via
//! `crate::file_to_markdown`) mise en cache ; incrémental par mtime. Les
//! fichiers illisibles sont listés dans le rapport — jamais d'échec silencieux
//! (ADR-0015). `dirs` (déjà une dépendance) résout Bureau/Documents/Téléchargements
//! de façon identique sur Mac et Windows — aucun code spécifique par OS ici.

use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SOURCE: &str = "local-folder";

/// Extensions parcourues — documents bureautiques uniquement (décision Liam,
/// 2026-07-21 : le scan par défaut ramenait les README/logs des projets de
/// code rangés dans Documents). .doc/.ppt sont inclus exprès : l'extraction
/// peut échouer (Windows, format legacy) mais l'échec est alors expliqué
/// dans le rapport plutôt que le fichier ignoré sans un mot.
const EXTENSIONS: &[&str] = &["pdf", "docx", "doc", "pptx", "ppt", "xlsx", "csv"];

/// Dossiers de dépendances/build jamais descendus, même s'ils ne sont pas
/// cachés — un `node_modules` contient des centaines de sous-dossiers, jamais
/// des documents utilisateur. Filet indépendant de la liste d'extensions :
/// même si un format bureautique traînait dedans (fixture de test…), on ne
/// veut jamais y descendre.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out",
    ".venv", "venv", "__pycache__", ".next", "vendor", "Pods",
];

// ─── Config ──────────────────────────────────────────────────────────────────

fn config_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("local_folder_config.json"))
}

fn cache_path() -> Option<PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("local_folder_conversations.json"))
}

#[derive(Serialize, Deserialize, Default)]
struct Config {
    folders: Vec<String>,
}

/// Ancien format (un seul dossier, choisi à la main) — lu en fallback pour ne
/// pas perdre la config des utilisateurs déjà connectés avant ce changement.
#[derive(Deserialize)]
struct LegacyConfig {
    folder_path: String,
}

fn load_config() -> Config {
    let Some(path) = config_path() else { return Config::default() };
    let Ok(raw) = std::fs::read_to_string(path) else { return Config::default() };
    if let Ok(c) = serde_json::from_str::<Config>(&raw) { return c; }
    match serde_json::from_str::<LegacyConfig>(&raw) {
        Ok(old) => Config { folders: vec![old.folder_path] },
        Err(_) => Config::default(),
    }
}

fn save_config(cfg: &Config) -> Result<(), String> {
    let p = config_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(p, serde_json::to_string(cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Bureau / Documents / Téléchargements qui existent réellement sur cette machine.
fn default_folders() -> Vec<String> {
    [dirs::desktop_dir(), dirs::document_dir(), dirs::download_dir()]
        .into_iter()
        .flatten()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

pub fn folders() -> Vec<String> {
    load_config().folders
}

pub fn is_connected() -> bool {
    !folders().is_empty()
}

/// Première connexion : ajoute les dossiers standards existants. Idempotent —
/// si déjà connecté, renvoie simplement la liste actuelle sans y toucher.
pub fn connect() -> Result<Vec<String>, String> {
    let existing = folders();
    if !existing.is_empty() {
        return Ok(existing);
    }
    let defaults = default_folders();
    if defaults.is_empty() {
        return Err("Aucun dossier standard trouvé (Bureau/Documents/Téléchargements) — ajoute un dossier manuellement.".into());
    }
    save_config(&Config { folders: defaults.clone() })?;
    Ok(defaults)
}

pub fn add_folder(path: &str) -> Result<Vec<String>, String> {
    if !Path::new(path).is_dir() {
        return Err(format!("Dossier introuvable : {path}"));
    }
    let mut cfg = load_config();
    if !cfg.folders.iter().any(|f| f == path) {
        cfg.folders.push(path.to_string());
    }
    save_config(&cfg)?;
    Ok(cfg.folders)
}

/// Retire un dossier de la liste (le disque n'est pas touché). Les fichiers
/// qu'il contenait sortent du cache tout seuls au prochain sync (il n'est
/// plus parcouru).
pub fn remove_folder(path: &str) -> Result<Vec<String>, String> {
    let mut cfg = load_config();
    cfg.folders.retain(|f| f != path);
    save_config(&cfg)?;
    Ok(cfg.folders)
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
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        if path.is_dir() {
            if SKIP_DIRS.contains(&name.as_str()) { continue; }
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
    /// Fichiers illisibles ou dossiers disparus : "chemin — raison".
    pub skipped: Vec<String>,
}

fn folder_name(root_str: &str) -> String {
    Path::new(root_str)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Dossier local".to_string())
}

fn file_to_conversation(root_str: &str, rel: &str, abs: &Path) -> Result<Conversation, String> {
    let text = crate::file_to_markdown(abs)?;
    let (sub_path, title) = super::obsidian::rel_to_parts(rel);
    let folder = folder_name(root_str);
    // Préfixé par le nom du dossier racine : deux dossiers différents peuvent
    // contenir un fichier de même chemin relatif, l'id doit rester unique.
    let id = format!("{folder}::{rel}");
    let mut container_path = vec![folder];
    container_path.extend(sub_path);

    let ts = super::obsidian::mtime_iso(abs);
    Ok(Conversation {
        summary: ConversationSummary {
            id,
            title,
            project: "Dossiers locaux".into(),
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

/// Parcourt tous les dossiers configurés, extrait les fichiers nouveaux/modifiés
/// (comparaison mtime vs cache) et réécrit le cache. Les fichiers disparus (ou
/// un dossier retiré de la config) sortent du cache d'eux-mêmes : on ne garde
/// que ce que le parcours actuel trouve.
///
/// `on_progress(index_1_based, total, label)` est appelé pour CHAQUE fichier
/// trouvé, avant son traitement — utilisé par l'appelant (commande Tauri) pour
/// afficher une progression en direct (scan Bureau/Documents/Téléchargements
/// à l'onboarding). Le parcours des dossiers se fait d'abord en entier pour
/// connaître `total` à l'avance (lister des noms de fichiers est quasi gratuit
/// comparé à l'extraction qui suit).
pub fn sync(mut on_progress: impl FnMut(usize, usize, &str)) -> Result<SyncReport, String> {
    let roots = folders();
    if roots.is_empty() {
        return Err("Aucun dossier configuré.".into());
    }

    let mut skipped = Vec::new();
    let mut all_files: Vec<(String, String, PathBuf)> = Vec::new(); // (root_str, rel, abs)
    for root_str in &roots {
        let root = Path::new(root_str);
        if !root.is_dir() {
            skipped.push(format!("{root_str} — dossier introuvable, ignoré"));
            continue;
        }
        for (rel, abs) in walk(root) {
            all_files.push((root_str.clone(), rel, abs));
        }
    }
    let total = all_files.len();

    let cached: std::collections::HashMap<String, Conversation> = load_conversations()
        .into_iter()
        .map(|c| (c.summary.id.clone(), c))
        .collect();

    let mut out = Vec::new();
    let mut new_count = 0usize;

    for (i, (root_str, rel, abs)) in all_files.into_iter().enumerate() {
        on_progress(i + 1, total, &rel);
        let folder = folder_name(&root_str);
        let id = format!("{folder}::{rel}");
        let mtime = super::obsidian::mtime_iso(&abs);
        if let Some(prev) = cached.get(&id) {
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
        std::fs::write(dir.join("cours/rapport.pdf"), "x").unwrap();
        std::fs::write(dir.join("cours/photo.png"), "x").unwrap();
        // README d'un projet de code rangé dans le même dossier — exclu (pas
        // un document bureautique, ne doit pas polluer le scan par défaut).
        std::fs::write(dir.join("cours/README.md"), "# Cours").unwrap();
        std::fs::write(dir.join("cours/.git/config.pdf"), "x").unwrap();
        std::fs::write(dir.join("Facture.CSV"), "x").unwrap();

        let files = walk(&dir);
        let rels: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert!(rels.contains(&"cours/rapport.pdf"));
        assert!(rels.contains(&"Facture.CSV"), "extension insensible à la casse");
        assert_eq!(rels.len(), 2, "png, .md et dossiers cachés exclus : {rels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn walk_ne_descend_jamais_dans_node_modules() {
        let dir = std::env::temp_dir().join("lucid_test_local_folder_node_modules");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mon-projet/node_modules/une-lib")).unwrap();
        std::fs::write(dir.join("mon-projet/rapport.pdf"), "x").unwrap();
        // Un vrai document bureautique planqué dans node_modules ne doit
        // quand même jamais remonter : on ne descend pas dedans, point.
        std::fs::write(dir.join("mon-projet/node_modules/une-lib/fixture.pdf"), "x").unwrap();

        let files = walk(&dir);
        let rels: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert!(rels.contains(&"mon-projet/rapport.pdf"));
        assert_eq!(rels.len(), 1, "node_modules jamais parcouru : {rels:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_migre_l_ancien_format_mono_dossier() {
        let dir = std::env::temp_dir().join("lucid_test_local_folder_legacy");
        std::fs::create_dir_all(&dir).unwrap();
        // Simule un fichier de config écrit par l'ancienne version (un seul dossier).
        let raw = format!(r#"{{"folder_path":"{}"}}"#, dir.to_string_lossy().replace('\\', "\\\\"));
        let legacy: LegacyConfig = serde_json::from_str(&raw).unwrap();
        let migrated = Config { folders: vec![legacy.folder_path.clone()] };
        assert_eq!(migrated.folders, vec![dir.to_string_lossy().to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
