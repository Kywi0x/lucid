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

/** Chat contextuel sur une page (contenu de la page + sous-pages si includeChildren). */
export function askNode(nodeId: string, question: string, includeChildren: boolean): Promise<string> {
  return invoke("ask_node", { nodeId, question, includeChildren });
}

/** Génère du contenu markdown pour une page selon une consigne (contexte page + sous-pages). */
export function generateContent(nodeId: string, instruction: string, includeChildren: boolean): Promise<string> {
  return invoke("generate_content", { nodeId, instruction, includeChildren });
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

/** Crée un nœud « note » (prise de note) rattaché à `parentId`. Renvoie le nœud créé. */
export function createNoteNode(parentId: string, label: string): Promise<import("./types").BrainNode> {
  return invoke("create_note_node", { parentId, label });
}

/** Déplace un nœud sous un nouveau parent (refuse les cycles). */
export function setNodeParent(nodeId: string, parentId: string): Promise<void> {
  return invoke("set_node_parent", { nodeId, parentId });
}

/** Renomme un nœud (change son label). */
export function renameNode(nodeId: string, label: string): Promise<void> {
  return invoke("rename_node", { nodeId, label });
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

/** Liste les snapshots disponibles (triés du plus récent au plus ancien). */
export function listSnapshots(): Promise<import("./types").SnapshotInfo[]> {
  return invoke("list_snapshots");
}

/** Restaure un snapshot par ID. Sauvegarde l'état courant avant de restaurer. */
export function restoreSnapshot(snapshotId: string): Promise<BrainGraph> {
  return invoke("restore_snapshot", { snapshotId });
}

/** Liste les versions d'un nœud spécifique (plus récent en premier). */
export function listNodeSnapshots(nodeId: string): Promise<import("./types").NodeSnapshotInfo[]> {
  return invoke("list_node_snapshots", { nodeId });
}

/** Récupère le contenu complet d'une version de nœud. */
export function getNodeSnapshot(nodeId: string, snapshotId: string): Promise<string> {
  return invoke("get_node_snapshot", { nodeId, snapshotId });
}

/** Liste les espaces (Lucid en premier, puis les espaces personnalisés). */
export function listSpaces(): Promise<import("./types").Space[]> {
  return invoke("list_spaces");
}

/** Crée un nouvel espace personnalisé. */
export function createSpace(name: string): Promise<import("./types").Space> {
  return invoke("create_space", { name });
}

/** Renomme un espace. */
export function renameSpace(id: string, name: string): Promise<void> {
  return invoke("rename_space", { id, name });
}

/** Supprime un espace (impossible pour "lucid"). */
export function deleteSpace(id: string): Promise<void> {
  return invoke("delete_space", { id });
}

/** Ajoute un nœud à un espace. */
export function addNodeToSpace(spaceId: string, nodeId: string): Promise<void> {
  return invoke("add_node_to_space", { spaceId, nodeId });
}

/** Retire un nœud d'un espace. */
export function removeNodeFromSpace(spaceId: string, nodeId: string): Promise<void> {
  return invoke("remove_node_from_space", { spaceId, nodeId });
}

/** Exporte un espace en markdown. */
export function exportSpaceMd(spaceId: string): Promise<string> {
  return invoke("export_space_md", { spaceId });
}
