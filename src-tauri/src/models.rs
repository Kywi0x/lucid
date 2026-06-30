//! Structures de données partagées entre les connecteurs, le pipeline IA et le frontend.
//! Toutes sont sérialisables en JSON pour passer la frontière Tauri (Rust → React).

use serde::{Deserialize, Serialize};

/// Un message unitaire au sein d'une conversation, déjà nettoyé du bruit
/// (thinking, tool_use, tool_result) — il ne reste que le texte porteur de sens.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Message {
    /// "user" | "assistant"
    pub role: String,
    pub text: String,
    /// ISO 8601, tel que stocké par Claude Code.
    pub timestamp: Option<String>,
}

/// Vue légère d'une conversation pour l'affichage en liste (sans les messages).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    /// Chemin projet lisible (déduit du `cwd` ou du nom de dossier).
    pub project: String,
    /// Nom de dossier brut côté Claude Code (sert de clé de rechargement).
    pub project_slug: String,
    /// Identifiant du connecteur d'origine, ex. "claude-code".
    pub source: String,
    /// Hiérarchie native du connecteur, ex. ["Marketing","Q3"]. Vide = "Non classé".
    #[serde(default)]
    pub container_path: Vec<String>,
    pub message_count: usize,
    pub first_timestamp: Option<String>,
    pub last_timestamp: Option<String>,
}

/// Conversation complète, messages inclus — chargée à la demande.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Conversation {
    #[serde(flatten)]
    pub summary: ConversationSummary,
    pub messages: Vec<Message>,
}

// ─── Graphe de connaissances (mind map à bulles, généré par l'IA) ────────────

/// Un nœud du graphe : le cerveau central, un conteneur, ou une page feuille.
/// Tout est un nœud — documents inclus (feuilles avec connector + source_id).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrainNode {
    pub id: String,
    pub label: String,
    /// "root" | "container" | "leaf" — indicatif seulement, l'UI dérive l'affichage de children.length.
    pub kind: String,
    /// Importance relative (nb de feuilles dans le sous-arbre).
    pub weight: usize,
    /// Résumé synthétique généré par l'IA.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Communauté détectée (0 = global, 1..n = projet).
    #[serde(default)]
    pub community: u32,
    /// Id du nœud parent direct.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Timestamp ISO de la dernière synthèse IA manuelle.
    #[serde(default)]
    pub synthesized_at: Option<String>,
    /// Corps markdown édité par l'utilisateur (jamais écrasé par l'IA).
    #[serde(default)]
    pub content: String,
    // ─── Provenance (nœuds feuilles uniquement) ────────────────────────────
    /// Connecteur d'origine : "notion" | "claude-code" | "google-drive".
    #[serde(default)]
    pub connector: Option<String>,
    /// Identifiant de la source (page_id Notion, session_id Claude Code, file_id Drive).
    #[serde(default)]
    pub source_id: Option<String>,
    /// Slug projet (utilisé uniquement par Claude Code pour localiser le fichier JSONL).
    #[serde(default)]
    pub source_project: Option<String>,
}

/// Une arête : relation de contenance dans l'arbre.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrainEdge {
    pub source: String,
    pub target: String,
    /// Toujours "contains".
    #[serde(default = "default_contains")]
    pub kind: String,
    #[serde(default = "default_relation")]
    pub relation: String,
}

fn default_contains() -> String { "contains".into() }
fn default_relation() -> String { "contains".into() }

/// Statut d'un connecteur pour l'onglet « Connecteurs ».
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ConnectorStatus {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub last_sync: Option<String>,
    pub conversation_count: usize,
    /// true = credentials manquants, l'UI propose de les importer.
    #[serde(default)]
    pub needs_setup: bool,
}

/// Le graphe complet + ses sérialisations Markdown.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrainGraph {
    pub nodes: Vec<BrainNode>,
    pub edges: Vec<BrainEdge>,
    /// Markdown narratif complet (brain.md).
    pub markdown: String,
    /// Markdown compact optimisé injection LLM.
    #[serde(default)]
    pub report: String,
    pub generated_at: String,
}
