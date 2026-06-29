//! Structures de données partagées entre les connecteurs, le pipeline IA et le frontend.
//! Toutes sont sérialisables en JSON pour passer la frontière Tauri (Rust → React).

use serde::{Deserialize, Serialize};

/// Niveau de confiance sur une arête du graphe (inspiré de Graphify).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Extracted,      // relation explicite dans le texte
    #[default]
    Inferred,       // déduite du contexte
    Ambiguous,      // faible signal, à valider
}

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

/// Référence vers une source ayant contribué à un nœud (traçabilité / drill-down).
/// Agnostique du connecteur → prêt pour Claude Code, Cowork, Drive, etc.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceRef {
    /// Identifiant du connecteur d'origine : "claude-code", "google-drive"…
    pub connector: String,
    /// Titre lisible (conversation, fichier…).
    pub title: String,
    /// Identifiant de la source (id de conversation, id de fichier…).
    pub id: String,
    /// Clé de rechargement côté connecteur (slug projet pour Claude Code).
    pub project_slug: String,
    /// Lien associé : chemin local ou URL (cwd, URL Drive…), si disponible.
    pub link: Option<String>,
    pub timestamp: Option<String>,
}

/// Un nœud du graphe : le cerveau central, un projet, ou un concept.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrainNode {
    pub id: String,
    pub label: String,
    /// "root" | "project" | "concept"
    pub kind: String,
    /// Importance relative (nb de conversations pour un projet, nb d'occurrences
    /// pour un concept) — pilote la taille de la bulle.
    pub weight: usize,
    /// Résumé synthétique (objectif d'un projet ; description d'un concept).
    #[serde(default)]
    pub summary: String,
    /// Mots-clés / thématiques.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Décisions clés (projets).
    #[serde(default)]
    pub decisions: Vec<String>,
    /// Patterns récurrents (projets).
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Sources ayant servi à générer ce nœud (drill-down + liens fichiers).
    #[serde(default)]
    pub sources: Vec<SourceRef>,
    /// Communauté détectée (0 = global/pont, 1..n = projet). Optimisation LLM-context.
    #[serde(default)]
    pub community: u32,
}

/// Une arête : appartenance (cerveau→projet, projet→concept) ou pont.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct BrainEdge {
    pub source: String,
    pub target: String,
    /// "project" | "concept" — conservé pour la rétrocompatibilité frontend.
    pub kind: String,
    /// Relation sémantique : "contains" | "uses" | "bridges".
    #[serde(default = "default_relation")]
    pub relation: String,
    #[serde(default)]
    pub confidence: Confidence,
    #[serde(default = "default_score")]
    pub confidence_score: f32,
}

fn default_relation() -> String { "uses".into() }
fn default_score() -> f32 { 0.8 }

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
    /// Markdown compact optimisé injection LLM (brain_report.md, ~10x moins de tokens).
    #[serde(default)]
    pub report: String,
    pub generated_at: String,
}
