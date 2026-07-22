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

/** Rapport de sync du connecteur « dossier local ». */
export interface LocalFolderSyncReport {
  new: number;
  total: number;
  /** Fichiers illisibles : "chemin — raison". */
  skipped: string[];
}

/** Proposition MCP en attente de validation — 5 formes (`action`), un seul
 *  circuit. "create" : parent_id/label/content. "update" : target_id/content.
 *  "move" : target_id/new_parent_id. "merge" : merge_ids (+ label optionnel du
 *  survivant). "link" : target_id/link_target/relation. */
export interface McpProposal {
  id: string;
  action: "create" | "update" | "move" | "merge" | "link";
  parent_id: string;
  label: string;
  content: string;
  target_id: string;
  new_parent_id: string;
  merge_ids: string[];
  link_target: string;
  relation: string;
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
  /** Date (YYYY-MM-DD) d'apparition — feuilles/notes ; absente = toujours visible (timeline). */
  date?: string | null;
  content?: string;
  /** Texte source embarqué par le pipeline (fichier, conversation…). */
  source_text?: string;
  // Provenance (nœuds feuilles uniquement)
  connector?: string;
  source_id?: string;
  source_project?: string;
  /** Front-only : proposition MCP en attente ciblant ce nœud (update/move/merge/link) —
   *  jamais persisté, calculé depuis `proposals` (cf. App.tsx:graphWithGhosts). */
  pendingAction?: "update" | "move" | "merge" | "link";
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
  /** Origine : "mcp_accept" | "delete_node" | "pre_restore" | "regenerate" | "manual". */
  reason: string;
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
