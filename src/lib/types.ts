/** Miroir TypeScript des structures Rust (src-tauri/src/models.rs). */

export interface Message {
  role: "user" | "assistant" | string;
  text: string;
  timestamp: string | null;
}

export interface ConversationSummary {
  id: string;
  title: string;
  project: string;
  project_slug: string;
  source: string;
  container_path: string[];
  message_count: number;
  first_timestamp: string | null;
  last_timestamp: string | null;
}

export interface Conversation extends ConversationSummary {
  messages: Message[];
}

/** Graphe de connaissances généré par l'IA (mind map à bulles). */
export type NodeKind = "root" | "container" | "leaf" | "group" | "espace" | "page" | "source";

export interface BrainNode {
  id: string;
  label: string;
  kind: NodeKind;
  weight: number;
  summary: string;
  keywords: string[];
  decisions: string[];
  patterns: string[];
  community?: number;
  parent_id?: string | null;
  synthesized_at?: string | null;
  content?: string;
  // Provenance (nœuds feuilles uniquement)
  connector?: string;
  source_id?: string;
  source_project?: string;
}

export interface BrainEdge {
  source: string;
  target: string;
  kind: string;
  relation?: string;
}

export interface BrainGraph {
  nodes: BrainNode[];
  edges: BrainEdge[];
  markdown: string;
  report?: string;
  generated_at: string;
}

export interface ConnectorStatus {
  id: string;
  name: string;
  connected: boolean;
  last_sync: string | null;
  conversation_count: number;
  needs_setup?: boolean;
}
