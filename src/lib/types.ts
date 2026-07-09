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

/** Graphe de connaissances généré par l'IA (mind map à bulles).
 *  `pending` = proposition MCP pas encore validée (bulle fantôme, jamais persistée). */
export type NodeKind = "root" | "container" | "leaf" | "group" | "espace" | "page" | "source" | "note" | "pending";

/** Client IA local (Claude Desktop/Code, Cursor…) connectable au MCP Lucid. */
export interface AiClientStatus {
  id: string;
  name: string;
  installed: boolean;
  connected: boolean;
  /** false = le client refuse les serveurs MCP locaux (ex. ChatGPT). */
  supported: boolean;
}

/** Rapport de sync du connecteur « dossier local ». */
export interface LocalFolderSyncReport {
  new: number;
  total: number;
  /** Fichiers illisibles : "chemin — raison". */
  skipped: string[];
}

/** Proposition de création déposée par le serveur MCP, en attente de validation. */
export interface McpProposal {
  id: string;
  parent_id: string;
  label: string;
  content: string;
  created_at: string;
}

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

export interface SnapshotInfo {
  id: string;
  created_at: number; // unix seconds
  node_count: number;
}

export interface NodeSnapshotInfo {
  id: string;        // "content_<ts>"
  created_at: number;
  preview: string;   // premiers 150 chars
}

export interface ConnectorStatus {
  id: string;
  name: string;
  connected: boolean;
  last_sync: string | null;
  conversation_count: number;
  needs_setup?: boolean;
}

export interface Space {
  id: string;
  name: string;
  node_ids: string[] | null; // null = "Lucid" (tous les nœuds)
}
