/** Miroir TypeScript des structures Rust (src-tauri/src/models.rs). */

export interface Message {
  role: "user" | "assistant" | string;
  text: string;
  timestamp: string | null;
}

export interface ConversationSummary {
  id: string;
  title: string;
  /** Affichage seulement — ne plus utiliser comme clé de groupe. */
  project: string;
  project_slug: string;
  source: string;
  /** Hiérarchie native du connecteur (ex. ["Clients","Béaux Électricité"]). Vide = Non classé. */
  container_path: string[];
  message_count: number;
  first_timestamp: string | null;
  last_timestamp: string | null;
}

export interface Conversation extends ConversationSummary {
  messages: Message[];
}

/** Graphe de connaissances généré par l'IA (mind map à bulles). */
export type NodeKind = "root" | "project" | "concept";

/** Provenance d'un nœud (traçabilité / drill-down, multi-connecteurs). */
export interface SourceRef {
  connector: string; // "claude-code", "google-drive", ...
  title: string;
  id: string;
  project_slug: string;
  link: string | null;
  timestamp: string | null;
}

export type Confidence = "extracted" | "inferred" | "ambiguous";

export interface BrainNode {
  id: string;
  label: string;
  kind: NodeKind;
  weight: number;
  summary: string;
  keywords: string[];
  decisions: string[];
  patterns: string[];
  sources: SourceRef[];
  community?: number; // 0 = global/pont, 1..n = projet
}

export interface BrainEdge {
  source: string;
  target: string;
  kind: "project" | "concept";
  relation?: "contains" | "uses" | "bridges";
  confidence?: Confidence;
  confidence_score?: number;
}

export interface BrainGraph {
  nodes: BrainNode[];
  edges: BrainEdge[];
  markdown: string;
  report?: string; // compact, optimisé injection LLM
  generated_at: string;
}

export interface ConnectorStatus {
  id: string;
  name: string;
  connected: boolean;
  last_sync: string | null;
  conversation_count: number;
  /** true = credentials manquants, l'UI propose de les importer */
  needs_setup?: boolean;
}
