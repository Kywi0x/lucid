//! Connecteur Cowork (Claude Desktop, mode agent local).
//!
//! Cowork écrit ses sessions en JSONL **au même format que Claude Code** — vérifié le
//! 2026-08-06 : mêmes types de lignes (`user`, `assistant`, `ai-title`), mêmes champs
//! (`message.role`, `timestamp`, `cwd`, `uuid`). On réutilise donc `claude_code::parse_file`
//! tel quel plutôt que d'écrire un second parseur qui divergerait.
//!
//! Arborescence réelle (relevée le 2026-08-06 ; macOS `~/Library/Application Support/Claude/`,
//! Windows `%APPDATA%\Claude\` — `dirs::config_dir()` résout les deux) :
//!
//! ```text
//! local-agent-mode-sessions/
//!   <groupe>/<sous-id>/local_<uuid>/               ← répertoire de travail de la session
//!       .claude/projects/<slug>/<sessionId>.jsonl  ← LE TRANSCRIPT, ce qu'on lit
//!       audit.jsonl, uploads/, outputs/            ← journal d'exécution et fichiers, ignorés
//!   <groupe>/<sous-id>/local_<uuid>.json           ← métadonnées, ignorées (voir plus bas)
//! ```
//!
//! Autrement dit **Cowork fait tourner un Claude Code dont le home est le répertoire de
//! travail de la session**. D'où la règle de sélection, qui tient en une ligne : un
//! transcript est un `.jsonl` situé sous un `.claude/projects/`. Elle écarte d'elle-même
//! les `audit.jsonl`, posés à la racine du répertoire de travail — pas de liste noire à
//! maintenir, et elle survit à un changement de profondeur.
//!
//! **Pourquoi on n'ouvre PAS `local_<uuid>.json`** alors qu'il porte un joli `title` : il
//! contient aussi `accountName` et `emailAddress`. Le transcript suffit — `parse_file` en
//! tire déjà le `cwd` et un titre (`ai-title`, sinon le premier prompt). Ne pas lire le
//! fichier est la façon la plus sûre de ne pas faire entrer ces données dans le cerveau.

use crate::models::{Conversation, ConversationSummary};
use std::fs;
use std::path::PathBuf;

/// Profondeur maximale explorée sous `local-agent-mode-sessions` (le réel fait 6).
const MAX_DEPTH: usize = 8;

/// Conteneur unique sous lequel toutes les sessions Cowork sont regroupées.
const CONTAINER: &str = "Cowork";

/// Dossier des sessions Cowork, s'il existe et que le connecteur est actif.
/// `dirs::config_dir()` = `~/Library/Application Support` (macOS) et `%APPDATA%` (Windows) :
/// une seule ligne couvre les deux plateformes.
pub fn sessions_dir() -> Option<PathBuf> {
    // ponytail: pas de drapeau de désactivation ici tant qu'aucun bouton ne l'appelle —
    // `claude_code` en a un, à copier le jour où l'UI propose de couper la source.
    let dir = dirs::config_dir()?
        .join("Claude")
        .join("local-agent-mode-sessions");
    dir.is_dir().then_some(dir)
}

/// Chemins des transcripts, avec le dossier de projet comme `project_slug`.
///
/// **La règle** : un transcript est un `.jsonl` dont le grand-parent s'appelle `projects`
/// — c'est-à-dire un fichier de `…/.claude/projects/<slug>/`. Elle écarte d'elle-même les
/// `audit.jsonl` (posés à la racine du répertoire de travail) sans liste noire, et elle
/// tient si Anthropic change la profondeur d'imbrication des sessions.
fn transcripts(root: &std::path::Path) -> Vec<(PathBuf, String)> {
    fn walk(dir: &std::path::Path, depth: usize, out: &mut Vec<(PathBuf, String)>) {
        if depth > MAX_DEPTH {
            return; // garde-fou : l'arborescence réelle fait 6 niveaux
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, depth + 1, out);
            } else if p.extension().and_then(|x| x.to_str()) == Some("jsonl")
                && p.parent()
                    .and_then(|d| d.parent())
                    .is_some_and(|g| g.file_name().is_some_and(|n| n == "projects"))
            {
                // Le slug est le dossier de projet, comme chez Claude Code. `parse_file`
                // ne s'en sert qu'en repli : le `cwd` du transcript prime.
                let slug = p
                    .parent()
                    .and_then(|d| d.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                out.push((p, slug));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

fn parse_all() -> Vec<Conversation> {
    let Some(root) = sessions_dir() else {
        return Vec::new();
    };
    let mut out: Vec<Conversation> = transcripts(&root)
        .iter()
        .filter_map(|(p, slug)| super::claude_code::parse_file(p, slug, super::SOURCE_COWORK))
        .map(|mut c| {
            // Chez Claude Code, le `cwd` est le dépôt sur lequel on travaille : c'est un
            // nom de conteneur qui a du sens. Chez Cowork, c'est le répertoire de travail
            // JETABLE de la session — le graphe afficherait une bulle « local_3b99c123-… »
            // par session. On regroupe donc tout sous un conteneur unique et lisible ; le
            // titre de la session, lui, reste porté par chaque conversation.
            c.summary.project = CONTAINER.to_string();
            c.summary.container_path = vec![CONTAINER.to_string()];
            c
        })
        .collect();
    out.sort_by(|a, b| b.summary.last_timestamp.cmp(&a.summary.last_timestamp));
    out
}

/// Vue légère, sans messages.
pub fn list_conversations() -> Vec<ConversationSummary> {
    parse_all().into_iter().map(|c| c.summary).collect()
}

/// Conversations complètes — pour le pipeline IA.
pub fn load_all_conversations() -> Vec<Conversation> {
    parse_all()
}

/// Charge une conversation par son id (nom de fichier sans extension).
pub fn load_by_id(id: &str) -> Option<Conversation> {
    let root = sessions_dir()?;
    transcripts(&root)
        .into_iter()
        .find(|(p, _)| p.file_stem().is_some_and(|s| s == id))
        .and_then(|(p, slug)| super::claude_code::parse_file(&p, &slug, super::SOURCE_COWORK))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reproduit l'arborescence RÉELLE relevée le 2026-08-06 — transcripts six niveaux
    /// plus bas, sous le `.claude/projects/` du répertoire de travail de la session. Une
    /// fixture plate laisserait passer le bug qui a rendu 0 conversation au premier essai.
    /// (`std::env::temp_dir()` comme les tests de `local_folder`, pas de dépendance.)
    fn fixture(nom: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(nom);
        let _ = fs::remove_dir_all(&dir);
        let travail = dir.join("groupe").join("sous-id").join("local_abc");
        let projet = travail.join(".claude").join("projects").join("-Users-x-projet");
        fs::create_dir_all(&projet).unwrap();
        fs::write(projet.join("session-1.jsonl"), "").unwrap();
        fs::write(projet.join("session-2.jsonl"), "").unwrap();
        // Leurres : tout ce qui n'est pas sous `.claude/projects/` doit être ignoré.
        fs::write(travail.join("audit.jsonl"), "").unwrap();
        fs::create_dir_all(travail.join("uploads")).unwrap();
        fs::write(travail.join("uploads").join("export.jsonl"), "").unwrap();
        fs::write(dir.join("groupe").join("sous-id").join("local_abc.json"), "{}").unwrap();
        dir
    }

    /// Le point qui casserait en silence : remonter comme conversations les `audit.jsonl`
    /// (journaux d'exécution) ou un `.jsonl` déposé par l'utilisateur dans `uploads/`.
    #[test]
    fn ne_remonte_que_les_transcripts_sous_claude_projects() {
        let dir = fixture("lucid_test_cowork_transcripts");
        let noms: Vec<String> = transcripts(&dir)
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(noms.len(), 2, "seuls les 2 transcripts, ni audit.jsonl ni uploads/");
        assert!(noms.contains(&"session-1.jsonl".to_string()));
        assert!(!noms.contains(&"audit.jsonl".to_string()));
        assert!(!noms.contains(&"export.jsonl".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn le_slug_est_le_dossier_de_projet() {
        let dir = fixture("lucid_test_cowork_slug");
        assert_eq!(transcripts(&dir)[0].1, "-Users-x-projet");
        let _ = fs::remove_dir_all(&dir);
    }
}
