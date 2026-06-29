/** Pont typé vers les commandes Tauri (Rust). */
import { invoke } from "@tauri-apps/api/core";
import type {
  BrainGraph,
  ConnectorStatus,
  Conversation,
  ConversationSummary,
} from "./types";

export function listConversations(): Promise<ConversationSummary[]> {
  return invoke("list_conversations");
}

export function loadConversation(
  projectSlug: string,
  id: string,
): Promise<Conversation | null> {
  return invoke("load_conversation", { projectSlug, id });
}

export function claudeCodeAvailable(): Promise<boolean> {
  return invoke("claude_code_available");
}

/** L'IA locale est-elle prête (binaire llama.cpp + modèle présents) ? */
export function aiReady(): Promise<boolean> {
  return invoke("ai_ready");
}

/** Lit le graphe brain.json déjà généré, s'il existe. */
export function readBrainGraph(): Promise<BrainGraph | null> {
  return invoke("read_brain_graph");
}

/** Lance le pipeline IA et renvoie le graphe de connaissances généré. */
export function generateBrain(): Promise<BrainGraph> {
  return invoke("generate_brain");
}

export interface BrainProgress {
  current: number;
  total: number;
  label: string;
}

/** Statut des connecteurs (actifs + dernière synchro). */
export function connectorsStatus(): Promise<ConnectorStatus[]> {
  return invoke("connectors_status");
}

/** Importe un export ZIP claude.ai. Renvoie le nb de conversations importées. */
export function importClaudeAi(path: string): Promise<number> {
  return invoke("import_claude_ai", { path });
}

/** Lance le flux OAuth Google (ouvre le navigateur, attend le redirect). */
export function googleDriveConnect(): Promise<void> {
  return invoke("google_drive_connect");
}

/** Déconnecte Google Drive (supprime tokens + cache). */
export function googleDriveDisconnect(): Promise<void> {
  return invoke("google_drive_disconnect");
}

/** Synchronise les fichiers Drive. Renvoie [ingérés, total]. */
export function googleDriveSync(): Promise<[number, number]> {
  return invoke("google_drive_sync");
}

/** Chat local sur le brain.md (réponse générée par Gemma). */
export function askBrain(question: string): Promise<string> {
  return invoke("ask_brain", { question });
}
