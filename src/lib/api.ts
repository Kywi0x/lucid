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
  source?: string,
): Promise<Conversation | null> {
  return invoke("load_conversation", { projectSlug, id, source });
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

/** Exporte un nœud + tous ses enfants récursivement en markdown vers `path`. */
export function exportNodeMd(nodeId: string, path: string): Promise<void> {
  return invoke("export_node_md", { nodeId, path });
}

/** Charge le contenu d'un nœud feuille depuis le cache local (0 appel API). */
export function loadNodeContent(nodeId: string): Promise<string> {
  return invoke("load_node_content", { nodeId });
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

/** Enregistre les credentials OAuth Google (client_id + client_secret) en app data. */
export function googleDriveSaveCredentials(clientId: string, clientSecret: string): Promise<void> {
  return invoke("google_drive_save_credentials", { clientId, clientSecret });
}

/** Vérifie si les credentials OAuth Google sont déjà configurés. */
export function googleDriveHasCredentials(): Promise<boolean> {
  return invoke("google_drive_has_credentials");
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

/** Enregistre le token Notion (secret_xxx). */
export function notionConnect(token: string): Promise<void> {
  return invoke("notion_connect", { token });
}

/** Synchronise les pages Notion. Renvoie [nouvelles, total]. */
export function notionSync(): Promise<[number, number]> {
  return invoke("notion_sync");
}

/** Déconnecte Notion (supprime token + cache). */
export function notionDisconnect(): Promise<void> {
  return invoke("notion_disconnect");
}

/** Fetch le contenu markdown d'une page Notion par ID (liens notion:ID). */
export function notionLoadPage(id: string): Promise<string> {
  return invoke("notion_load_page", { id });
}

/** Chat local sur le brain.md (réponse générée par Gemma). */
export function askBrain(question: string): Promise<string> {
  return invoke("ask_brain", { question });
}

/** Indique si le modèle actif doit être installé (fichier absent). */
export function aiSetupNeeded(): Promise<boolean> {
  return invoke("ai_setup_needed");
}

/** Télécharge le modèle depuis le CDN GPT4All (public, sans auth).
 *  Émet des événements "download-progress" { downloaded_mb, total_mb, percent }. */
export function downloadModel(modelId: string): Promise<void> {
  return invoke("download_model", { modelId });
}

/** Fallback : copie un .gguf local déjà téléchargé (émet "install-progress" en %). */
export function installModelFile(srcPath: string, modelId: string): Promise<void> {
  return invoke("install_model_file", { srcPath, modelId });
}

export interface ModelInfo {
  id: string;
  name: string;
  file: string;
  size_gb: number;
  min_ram_gb: number;
  downloaded: boolean;
  active: boolean;
  recommended: boolean;
}

/** Liste les modèles disponibles avec leur statut. */
export function listModels(): Promise<ModelInfo[]> {
  return invoke("list_models");
}

/** Change le modèle actif (si non téléchargé, aiSetupNeeded() passera à true). */
export function setActiveModel(id: string): Promise<void> {
  return invoke("set_active_model", { id });
}

/** Re-synthétise un nœud à la demande (sources + enfants comme contexte). Retourne le nœud mis à jour. */
export function synthesizeNode(nodeId: string): Promise<import("./types").BrainNode> {
  return invoke("synthesize_node", { nodeId });
}

/** Sauvegarde le contenu markdown libre d'un nœud (jamais écrasé par l'IA). */
export function saveNodeContent(nodeId: string, content: string): Promise<void> {
  return invoke("save_node_content", { nodeId, content });
}

/** Configure le vault Obsidian (chemin local). */
export function obsidianSetVault(path: string): Promise<void> {
  return invoke("obsidian_set_vault", { path });
}

/** Renvoie le chemin du vault Obsidian configuré (null si pas encore configuré). */
export function obsidianVaultPath(): Promise<string | null> {
  return invoke("obsidian_vault_path");
}

/** Déconnecte Obsidian (supprime la config locale). */
export function obsidianDisconnect(): Promise<void> {
  return invoke("obsidian_disconnect");
}
