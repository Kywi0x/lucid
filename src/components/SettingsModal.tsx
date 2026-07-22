import { useState, useEffect, useCallback } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { syncNow, useSyncStatus, SYNC_FILE } from "@/lib/sync";
import {
  Plug,
  MessageCircle,
  Loader2,
  X,
  RefreshCw,
  LogOut,
  CheckCircle2,
  Circle,
  Clock,
  FileText,
  Folder,
  Layers,
  Pencil,
  Trash2,
  Plus,
  User,
  StickyNote,
  Copy,
  Check,
} from "lucide-react";
import claudeLogo from "@/assets/claude-logo.png";
import googleDriveLogo from "@/assets/google_drive.svg.png";
import openaiLogo from "@/assets/openai.svg";
import { open } from "@tauri-apps/plugin-dialog";
import {
  googleDriveConnect,
  googleDriveDisconnect,
  googleDriveSync,
  importClaudeAi,
  importChatGpt,
  localFolderConnect,
  localFolderList,
  localFolderAdd,
  localFolderRemove,
  localFolderDisconnect,
  localFolderSync,
  obsidianSetVault,
  obsidianVaultPath,
  obsidianDisconnect,
  appleNotesConnect,
  appleNotesSync,
  appleNotesDisconnect,
  claudeCodeAvailable,
  claudeCodeDisconnect,
  claudeCodeReconnect,
  listModels,
  setActiveModel,
  mcpManualValidationEnabled,
  setMcpManualValidation,
  exportBackup,
  importBackup,
  resetEnvironment,
  telemetryEnabled,
  setTelemetry,
  sentryActive,
  crashTest,
  type ModelInfo,
} from "@/lib/api";
import { supabase, BACKUP_BUCKET } from "@/lib/supabase";
import type { ConnectorStatus, Space } from "@/lib/types";
import { cn, relativeDate, copyText } from "@/lib/utils";
import { McpConnectGuide } from "@/components/McpConnectGuide";
import { ensurePersonalMcpUrl } from "@/lib/share";

// ── Brand logos ───────────────────────────────────────────────────────────────

function LogoImg({ src, alt, bg = "bg-white" }: { src: string; alt: string; bg?: string }) {
  return (
    <div className={cn("flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-[var(--color-border)]", bg)}>
      <img src={src} alt={alt} className="size-5 object-contain" />
    </div>
  );
}

function LogoClaudeCode() {
  return <LogoImg src={claudeLogo} alt="Claude Code" bg="bg-[#1a1a1a]" />;
}

function LogoClaudeAi() {
  return <LogoImg src={claudeLogo} alt="Claude.ai" bg="bg-[#1a1a1a]" />;
}

function LogoGoogleDrive() {
  return <LogoImg src={googleDriveLogo} alt="Google Drive" bg="bg-white" />;
}

function LogoObsidian() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-[var(--color-border)] bg-[#7C3AED]">
      <svg viewBox="0 0 24 24" className="size-4" fill="white" aria-hidden>
        <path d="M12 2C8.5 2 5.5 4.5 5 8c-.3 2 .5 4 2 5.5L5.5 22l3-1.5 1.5 1.5 2-4 2 4 1.5-1.5 3 1.5-1.5-8.5C18 12 18.3 10 18 8c-.5-3.5-3.5-6-6-6z"/>
      </svg>
    </div>
  );
}

function LogoChatGpt() {
  return <LogoImg src={openaiLogo} alt="ChatGPT" />;
}

function LogoLocalFolder() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)]">
      <Folder className="size-4 text-[var(--color-accent)]" />
    </div>
  );
}

function LogoCowork() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-[#0EA5E9]">
      <MessageCircle className="size-3.5 text-white" />
    </div>
  );
}

function LogoAppleNotes() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center rounded-lg border border-[var(--color-border)] bg-[#FFD60A]">
      <StickyNote className="size-4 text-[#7a5c00]" />
    </div>
  );
}

export function ConnectorLogo({ id }: { id: string }) {
  if (id === "claude-code")  return <LogoClaudeCode />;
  if (id === "claude-ai")    return <LogoClaudeAi />;
  if (id === "chatgpt")      return <LogoChatGpt />;
  if (id === "google-drive") return <LogoGoogleDrive />;
  if (id === "obsidian")     return <LogoObsidian />;
  if (id === "local-folder") return <LogoLocalFolder />;
  if (id === "cowork")       return <LogoCowork />;
  if (id === "apple-notes")  return <LogoAppleNotes />;
  return (
    <div className="flex size-7 items-center justify-center rounded-lg bg-[var(--color-surface-2)]">
      <Plug className="size-4 text-[var(--color-muted)]" />
    </div>
  );
}

// ── Statut : point 6px + label mono ──────────────────────────────────────────

function StatusLine({ c, soon, className }: { c: ConnectorStatus; soon?: boolean; className?: string }) {
  const label = soon
    ? "bientôt"
    : c.connected
      ? c.last_sync ? `sync ${relativeDate(c.last_sync)}` : "connecté"
      : "à connecter";
  return (
    <span className={cn(
      "flex shrink-0 items-center gap-2 font-mono text-[10px] uppercase tracking-wider text-[var(--color-muted)]",
      className ?? "ml-auto",
    )}>
      <span
        className={cn(
          "size-1.5 rounded-full",
          !soon && c.connected
            ? "bg-[var(--color-ok)]"
            : "border border-[var(--color-muted)]",
        )}
      />
      {label}
    </span>
  );
}

function StatusBadge({ connected, soon }: { connected: boolean; soon?: boolean }) {
  if (soon) return (
    <span className="flex items-center gap-1 text-[11px] text-[var(--color-muted)]">
      <Circle className="size-2.5" /> Bientôt
    </span>
  );
  return connected ? (
    <span className="flex items-center gap-1 text-[11px] text-[var(--color-ok)]">
      <CheckCircle2 className="size-2.5" /> Configuré
    </span>
  ) : (
    <span className="flex items-center gap-1 text-[11px] text-[var(--color-warn)]">
      <Circle className="size-2.5" /> Non configuré
    </span>
  );
}

// ── Modal par connecteur ──────────────────────────────────────────────────────

interface ModalProps {
  c: ConnectorStatus;
  busy: boolean;
  msg: string;
  obsidianVault?: string | null;
  localFolders?: string[];
  onAddFolder?: () => void;
  onRemoveFolder?: (path: string) => void;
  onClose: () => void;
  onConnect: () => void;
  onSync: () => void;
  onDisconnect: () => void;
  onImport: () => void;
}

function ConnectorModal({ c, busy, msg, obsidianVault, localFolders, onAddFolder, onRemoveFolder, onClose, onConnect, onSync, onDisconnect, onImport }: ModalProps) {
  const isSoon = c.id === "cowork";

  // Échap ferme cette sous-modale AVANT la modale Paramètres : écouteur en phase
  // capture + stopPropagation pour court-circuiter celui de SettingsModal.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); onClose(); }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [onClose]);

  return (
    <div
      className="absolute inset-0 z-10 flex flex-col justify-end bg-black/20"
      onClick={onClose}
    >
      <div
        className="m-3 flex flex-col gap-0 overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 border-b border-[var(--color-border)] px-4 py-3">
          <ConnectorLogo id={c.id} />
          <div className="flex-1">
            <div className="text-sm font-semibold">{c.name}</div>
            <StatusBadge connected={c.connected} soon={isSoon} />
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] transition-colors"
          >
            <X className="size-4" />
          </button>
        </div>

        {/* Compte associé */}
        <div className="px-4 py-3 border-b border-[var(--color-border)]">
          <p className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
            Compte associé
          </p>
          {c.connected ? (
            <p className="text-xs text-[var(--color-text)]">
              {c.id === "claude-code" ? "~/.claude/projects/" : c.id === "claude-ai" || c.id === "chatgpt" ? "Import ZIP" : "Connecté"}
            </p>
          ) : (
            <p className="text-xs text-[var(--color-muted)] italic">Aucun compte connecté</p>
          )}
        </div>

        {/* Historique sync */}
        <div className="px-4 py-3 border-b border-[var(--color-border)]">
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
            Historique des synchronisations
          </p>
          {c.connected && c.last_sync ? (
            <div className="space-y-1.5">
              <SyncEntry
                label={`${c.conversation_count} éléments indexés`}
                date={c.last_sync}
                ok
              />
            </div>
          ) : (
            <p className="text-xs text-[var(--color-muted)] italic">Aucune synchronisation</p>
          )}
        </div>

        {/* Actions */}
        {!isSoon && (
          <div className="flex flex-col gap-1.5 px-4 py-3">
            {msg && (
              <p className="text-center text-[11px] text-[var(--color-muted)]">{msg}</p>
            )}

            {c.id === "claude-code" && (
              c.connected ? (
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              ) : (
                <ActionBtn busy={busy} icon={<CheckCircle2 className="size-3.5" />} onClick={onConnect} primary>
                  Connecter Claude Code
                </ActionBtn>
              )
            )}

            {(c.id === "claude-ai" || c.id === "chatgpt") && (
              <>
                {c.id === "chatgpt" && !c.connected && (
                  <p className="text-[10px] leading-relaxed text-[var(--color-muted)]">
                    chatgpt.com → Réglages → Contrôle des données → Exporter les données, puis importe le ZIP reçu par mail.
                  </p>
                )}
                <ActionBtn busy={busy} icon={<FileText className="size-3.5" />} onClick={onImport}>
                  {c.connected ? "Réimporter un ZIP" : "Importer export ZIP"}
                </ActionBtn>
              </>
            )}

            {c.id === "google-drive" && !c.connected && (
              <ActionBtn busy={busy} icon={<CheckCircle2 className="size-3.5" />} onClick={onConnect} primary>
                Connecter Google Drive
              </ActionBtn>
            )}

            {c.id === "google-drive" && c.connected && (
              <>
                <ActionBtn busy={busy} icon={<RefreshCw className="size-3.5" />} onClick={onSync} primary>
                  Synchroniser
                </ActionBtn>
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              </>
            )}

            {c.id === "obsidian" && !c.connected && (
              <ActionBtn busy={busy} icon={<FileText className="size-3.5" />} onClick={onConnect} primary>
                Choisir le dossier vault
              </ActionBtn>
            )}

            {c.id === "obsidian" && c.connected && (
              <>
                {obsidianVault && (
                  <p className="truncate rounded-lg bg-[var(--color-surface-2)] px-2.5 py-1.5 text-[10px] text-[var(--color-muted)]" title={obsidianVault}>
                    {obsidianVault}
                  </p>
                )}
                <ActionBtn busy={busy} icon={<FileText className="size-3.5" />} onClick={onConnect}>
                  Changer de vault
                </ActionBtn>
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              </>
            )}

            {c.id === "local-folder" && !c.connected && (
              <>
                <p className="text-[10px] leading-relaxed text-[var(--color-muted)]">
                  Indexe automatiquement ton Bureau, tes Documents et tes Téléchargements
                  (PDF, Word, PowerPoint, Excel, CSV — 100 % en local). Les fichiers
                  de code/projets ne sont jamais ramassés. Tu pourras ajouter ou
                  retirer des dossiers ensuite.
                </p>
                <ActionBtn busy={busy} icon={<Folder className="size-3.5" />} onClick={onConnect} primary>
                  Connecter
                </ActionBtn>
              </>
            )}

            {c.id === "local-folder" && c.connected && (
              <>
                {localFolders && localFolders.length > 0 && (
                  <ul className="flex flex-col gap-1">
                    {localFolders.map((f) => (
                      <li key={f} className="flex items-center gap-1.5 rounded-lg bg-[var(--color-surface-2)] px-2.5 py-1.5">
                        <span className="min-w-0 flex-1 truncate text-[10px] text-[var(--color-muted)]" title={f}>{f}</span>
                        <button
                          onClick={() => onRemoveFolder?.(f)}
                          title="Retirer ce dossier"
                          className="shrink-0 rounded p-0.5 text-[var(--color-muted)] hover:text-[var(--color-err)] transition-colors"
                        >
                          <X className="size-3" />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
                <ActionBtn busy={busy} icon={<Folder className="size-3.5" />} onClick={() => onAddFolder?.()}>
                  Ajouter un dossier
                </ActionBtn>
                <ActionBtn busy={busy} icon={<RefreshCw className="size-3.5" />} onClick={onSync} primary>
                  Synchroniser
                </ActionBtn>
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              </>
            )}

            {c.id === "apple-notes" && !c.connected && (
              <>
                <p className="text-[10px] leading-relaxed text-[var(--color-muted)]">
                  Lecture seule via l'automatisation native de macOS — jamais la base
                  interne de Notes. macOS te demandera d'autoriser Lucid à contrôler Notes.
                </p>
                <ActionBtn busy={busy} icon={<CheckCircle2 className="size-3.5" />} onClick={onConnect} primary>
                  Connecter Notes Apple
                </ActionBtn>
              </>
            )}

            {c.id === "apple-notes" && c.connected && (
              <>
                <ActionBtn busy={busy} icon={<RefreshCw className="size-3.5" />} onClick={onSync} primary>
                  Synchroniser
                </ActionBtn>
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function SyncEntry({ label, date, ok }: { label: string; date: string; ok: boolean }) {
  return (
    <div className="flex items-center gap-2">
      <div className={cn("size-1.5 rounded-full shrink-0", ok ? "bg-[var(--color-ok)]" : "bg-[var(--color-err)]")} />
      <span className="flex-1 text-[11px] text-[var(--color-text)]">{label}</span>
      <span className="flex items-center gap-0.5 text-[10px] text-[var(--color-muted)]">
        <Clock className="size-2.5" /> {relativeDate(date)}
      </span>
    </div>
  );
}

function ActionBtn({
  busy, icon, onClick, primary, danger, children,
}: {
  busy: boolean; icon: React.ReactNode; onClick: () => void;
  primary?: boolean; danger?: boolean; children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={busy}
      className={cn(
        "flex w-full items-center justify-center gap-2 rounded-lg px-3 py-2 text-xs font-medium transition-colors disabled:opacity-50",
        primary
          ? "bg-[var(--color-accent)] text-white hover:bg-[var(--color-accent-hover)]"
          : danger
            ? "border border-[var(--color-err)]/40 text-[var(--color-err)] hover:bg-[var(--color-err)]/10"
            : "border border-[var(--color-border)] text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]",
      )}
    >
      {busy ? <Loader2 className="size-3.5 animate-spin" /> : icon}
      {children}
    </button>
  );
}

// ── Section Connecteurs ───────────────────────────────────────────────────────

function ConnectorsSection({
  connectors,
  onRefresh,
  onSyncDone,
}: {
  connectors: ConnectorStatus[];
  onRefresh: () => void;
  onSyncDone: () => void;
}) {
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [msgs, setMsgs] = useState<Record<string, string>>({});
  const [modalId, setModalId] = useState<string | null>(null);

  function set(id: string, v: boolean) { setBusy((b) => ({ ...b, [id]: v })); }
  function msg(id: string, m: string)  { setMsgs((s) => ({ ...s, [id]: m })); }

  async function handleImportClaudeAi() {
    const selected = await open({ filters: [{ name: "Export claude.ai", extensions: ["zip"] }], multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("claude-ai", true); msg("claude-ai", "");
    try {
      const count = await importClaudeAi(selected);
      msg("claude-ai", `${count} conversations importées`);
      onRefresh();
    } catch (e) { msg("claude-ai", `Erreur : ${e}`); }
    finally { set("claude-ai", false); }
  }

  async function handleImportChatGpt() {
    const selected = await open({ filters: [{ name: "Export ChatGPT", extensions: ["zip"] }], multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("chatgpt", true); msg("chatgpt", "");
    try {
      const count = await importChatGpt(selected);
      msg("chatgpt", `${count} conversations importées`);
      onRefresh();
    } catch (e) { msg("chatgpt", `Erreur : ${e}`); }
    finally { set("chatgpt", false); }
  }

  async function handleGoogleConnect() {
    set("google-drive", true); msg("google-drive", "En attente du navigateur…");
    try {
      await googleDriveConnect();
      msg("google-drive", "Connecté !");
      onRefresh();
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  async function handleGoogleSync() {
    set("google-drive", true); msg("google-drive", "Synchronisation…");
    try {
      const [newFiles, total] = await googleDriveSync();
      msg("google-drive", newFiles > 0 ? `${newFiles} nouveaux sur ${total}` : `${total} fichiers en cache`);
      onRefresh();
      if (total > 0) onSyncDone();
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  async function handleGoogleDisconnect() {
    set("google-drive", true);
    try {
      await googleDriveDisconnect();
      msg("google-drive", "Déconnecté");
      onRefresh();
    } catch (e) { msg("google-drive", `Erreur : ${e}`); }
    finally { set("google-drive", false); }
  }

  const [obsidianVault, setObsidianVault] = useState<string | null>(null);
  useEffect(() => { obsidianVaultPath().then(setObsidianVault); }, []);

  async function handleObsidianConnect() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("obsidian", true); msg("obsidian", "");
    try {
      await obsidianSetVault(selected);
      setObsidianVault(selected);
      msg("obsidian", "Vault configuré !");
      onRefresh();
    } catch (e) { msg("obsidian", `Erreur : ${e}`); }
    finally { set("obsidian", false); }
  }

  async function handleObsidianDisconnect() {
    set("obsidian", true);
    try {
      await obsidianDisconnect();
      setObsidianVault(null);
      msg("obsidian", "Déconnecté");
      onRefresh();
    } catch (e) { msg("obsidian", `Erreur : ${e}`); }
    finally { set("obsidian", false); }
  }

  const [localFolders, setLocalFolders] = useState<string[]>([]);
  useEffect(() => { localFolderList().then(setLocalFolders); }, []);

  async function handleLocalFolderConnect() {
    set("local-folder", true); msg("local-folder", "");
    try {
      const folders = await localFolderConnect();
      setLocalFolders(folders);
      msg("local-folder", `${folders.length} dossier${folders.length > 1 ? "s" : ""} détecté${folders.length > 1 ? "s" : ""} — lance une synchronisation.`);
      onRefresh();
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderAddFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (!selected || typeof selected !== "string") return;
    set("local-folder", true); msg("local-folder", "");
    try {
      const folders = await localFolderAdd(selected);
      setLocalFolders(folders);
      msg("local-folder", "Dossier ajouté — lance une synchronisation.");
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderRemoveFolder(path: string) {
    set("local-folder", true);
    try {
      const folders = await localFolderRemove(path);
      setLocalFolders(folders);
      msg("local-folder", "Dossier retiré.");
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderSync() {
    set("local-folder", true); msg("local-folder", "Synchronisation… (l'extraction PDF peut être longue)");
    try {
      const r = await localFolderSync();
      const skipped = r.skipped.length ? ` — ${r.skipped.length} illisibles` : "";
      msg("local-folder", (r.new > 0 ? `${r.new} nouveaux sur ${r.total}` : `${r.total} fichiers indexés`) + skipped);
      if (r.skipped.length) console.warn("Dossiers locaux — fichiers ignorés :", r.skipped);
      onRefresh();
      if (r.total > 0) onSyncDone();
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleLocalFolderDisconnect() {
    set("local-folder", true);
    try {
      await localFolderDisconnect();
      setLocalFolders([]);
      msg("local-folder", "Déconnecté");
      onRefresh();
    } catch (e) { msg("local-folder", `Erreur : ${e}`); }
    finally { set("local-folder", false); }
  }

  async function handleAppleNotesConnect() {
    set("apple-notes", true); msg("apple-notes", "");
    try {
      const n = await appleNotesConnect();
      msg("apple-notes", `${n} note${n > 1 ? "s" : ""} importée${n > 1 ? "s" : ""}`);
      onRefresh();
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleAppleNotesSync() {
    set("apple-notes", true); msg("apple-notes", "Synchronisation…");
    try {
      const n = await appleNotesSync();
      msg("apple-notes", `${n} note${n > 1 ? "s" : ""} synchronisée${n > 1 ? "s" : ""}`);
      onRefresh();
      onSyncDone();
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleAppleNotesDisconnect() {
    set("apple-notes", true);
    try {
      await appleNotesDisconnect();
      msg("apple-notes", "Déconnecté");
      onRefresh();
    } catch (e) { msg("apple-notes", `Erreur : ${e}`); }
    finally { set("apple-notes", false); }
  }

  async function handleClaudeCodeDisconnect() {
    set("claude-code", true);
    try {
      await claudeCodeDisconnect();
      msg("claude-code", "Déconnecté");
      onRefresh();
    } catch (e) { msg("claude-code", `Erreur : ${e}`); }
    finally { set("claude-code", false); }
  }

  async function handleClaudeCodeConnect() {
    set("claude-code", true);
    try {
      await claudeCodeReconnect();
      const ok = await claudeCodeAvailable();
      msg("claude-code", ok ? "Connecté !" : "Dossier ~/.claude/projects introuvable — Claude Code est-il installé ?");
      onRefresh();
    } catch (e) { msg("claude-code", `Erreur : ${e}`); }
    finally { set("claude-code", false); }
  }

  const openModal = modalId ? connectors.find((c) => c.id === modalId) : null;

  return (
    <div className="relative flex h-full flex-col">
      <div className="flex-1 overflow-y-auto px-5 py-4">
        <div className="grid grid-cols-2 gap-2">
          {connectors.map((c) => {
            const isSoon = c.id === "cowork";
            return (
              <button
                key={c.id}
                onClick={() => !isSoon && setModalId(c.id)}
                disabled={isSoon}
                className={cn(
                  "flex items-center gap-3 rounded-xl border border-[var(--color-border)] px-3.5 py-3 text-left transition-colors",
                  isSoon ? "opacity-40" : "hover:bg-[var(--color-surface-2)]",
                )}
              >
                <ConnectorLogo id={c.id} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium">{c.name}</span>
                  <StatusLine c={c} soon={isSoon} className="mt-0.5" />
                </span>
              </button>
            );
          })}
        </div>
        <p className="px-1 pt-3 text-[11px] leading-relaxed text-[var(--color-muted)]">
          Toutes les sources alimentent le même graphe — clique sur l'une d'elles
          pour la connecter, la synchroniser ou la retirer.
        </p>
      </div>

      {openModal && (
        <ConnectorModal
          c={openModal}
          busy={busy[openModal.id] ?? false}
          msg={msgs[openModal.id] ?? ""}
          obsidianVault={obsidianVault}
          localFolders={localFolders}
          onAddFolder={handleLocalFolderAddFolder}
          onRemoveFolder={handleLocalFolderRemoveFolder}
          onClose={() => setModalId(null)}
          onConnect={
            openModal.id === "claude-code" ? handleClaudeCodeConnect :
            openModal.id === "obsidian" ? handleObsidianConnect :
            openModal.id === "local-folder" ? handleLocalFolderConnect :
            openModal.id === "apple-notes" ? handleAppleNotesConnect :
            handleGoogleConnect
          }
          onSync={
            openModal.id === "local-folder" ? handleLocalFolderSync :
            openModal.id === "apple-notes" ? handleAppleNotesSync :
            handleGoogleSync
          }
          onDisconnect={
            openModal.id === "claude-code" ? handleClaudeCodeDisconnect :
            openModal.id === "obsidian" ? handleObsidianDisconnect :
            openModal.id === "local-folder" ? handleLocalFolderDisconnect :
            openModal.id === "apple-notes" ? handleAppleNotesDisconnect :
            handleGoogleDisconnect
          }
          onImport={openModal.id === "chatgpt" ? handleImportChatGpt : handleImportClaudeAi}
        />
      )}
    </div>
  );
}

// ── Section Spaces (gestion : créer / renommer / supprimer) ──────────────────

function SpacesSection({
  spaces,
  onCreate,
  onRename,
  onDelete,
}: {
  spaces: Space[];
  onCreate: (name: string) => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  function commitRename() {
    const name = draft.trim();
    if (editingId && name) onRename(editingId, name);
    setEditingId(null);
  }

  function commitCreate() {
    const name = newName.trim();
    if (name) onCreate(name);
    setNewName("");
    setCreating(false);
  }

  return (
    <div className="h-full overflow-y-auto px-5 py-4">
      <p className="mb-3 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
        Espaces · {spaces.length}
      </p>
      {spaces.length <= 1 && (
        <p className="mb-3 text-xs leading-relaxed text-[var(--color-muted)]">
          Un espace est une vue filtrée du graphe (un projet, des révisions, un
          client…) — partageable en lecture via le bouton Partager.
        </p>
      )}
      {spaces.map((s) => {
        const isLucid = s.id === "lucid";
        return (
          <div
            key={s.id}
            className="group flex items-center gap-2.5 border-b border-[var(--color-border)] px-1 py-2.5 last:border-b-0"
          >
            {editingId === s.id ? (
              <input
                autoFocus
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") commitRename();
                  if (e.key === "Escape") { e.stopPropagation(); setEditingId(null); }
                }}
                onBlur={commitRename}
                className="flex-1 rounded-lg border border-[var(--color-accent)] bg-[var(--color-bg)] px-2 py-1 text-sm outline-none"
              />
            ) : (
              <span className="min-w-0 flex-1 truncate text-sm font-medium">{s.name}</span>
            )}
            <span className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-[var(--color-muted)]">
              {s.node_ids ? `${s.node_ids.length} nœuds` : "tout le graphe"}
            </span>
            {!isLucid && editingId !== s.id && (
              <span className="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-hover:opacity-100">
                <button
                  onClick={() => { setEditingId(s.id); setDraft(s.name); }}
                  title="Renommer"
                  className="rounded p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                >
                  <Pencil className="size-3" />
                </button>
                <button
                  onClick={() => onDelete(s.id)}
                  title="Supprimer"
                  className="rounded p-1 text-[var(--color-muted)] hover:text-[var(--color-err)] transition-colors"
                >
                  <Trash2 className="size-3" />
                </button>
              </span>
            )}
          </div>
        );
      })}

      {creating ? (
        <div className="mt-3 flex items-center gap-1.5">
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitCreate();
              if (e.key === "Escape") { e.stopPropagation(); setCreating(false); setNewName(""); }
            }}
            placeholder="Nom de l'espace…"
            className="flex-1 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-xs outline-none placeholder:text-[var(--color-muted)] focus:border-[var(--color-accent)]"
          />
          <button
            onClick={commitCreate}
            disabled={!newName.trim()}
            className="rounded-lg bg-[var(--color-accent)] px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50 hover:bg-[var(--color-accent-hover)] transition-colors"
          >
            Créer
          </button>
        </div>
      ) : (
        <button
          onClick={() => setCreating(true)}
          className="mt-3 flex items-center gap-1.5 rounded-lg border border-dashed border-[var(--color-border)] px-2.5 py-2 text-xs text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
        >
          <Plus className="size-3.5" /> Nouvel espace
        </button>
      )}
    </div>
  );
}

// ── Section Modèle IA ─────────────────────────────────────────────────────────

function ModelSection() {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [switching, setSwitching] = useState(false);

  useEffect(() => { listModels().then(setModels); }, []);

  async function handleSwitch(id: string) {
    setSwitching(true);
    try {
      await setActiveModel(id);
      setModels((prev) => prev.map((m) => ({ ...m, active: m.id === id })));
    } finally { setSwitching(false); }
  }

  return (
    <div className="h-full overflow-y-auto px-5 py-4">
      <p className="mb-3 text-xs leading-relaxed text-[var(--color-muted)]">
        Le modèle tourne entièrement sur ta machine. Le changement est immédiat,
        la prochaine génération l'utilise.
      </p>
      <div className="flex flex-col gap-1">
        {models.map((m) => (
          <button
            key={m.id}
            onClick={() => !m.active && !switching && handleSwitch(m.id)}
            disabled={switching}
            className={cn(
              "flex items-center gap-2 rounded-lg px-2.5 py-2 text-left text-xs transition-colors disabled:opacity-60",
              m.active
                ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
            )}
          >
            <span className="flex-1 font-medium">{m.name}</span>
            <span className="shrink-0 font-mono text-[10px] opacity-70">{m.size_gb} Go</span>
            {m.recommended && !m.active && (
              <span className="shrink-0 rounded-full border border-[var(--color-warn)]/40 px-1.5 py-0.5 text-[9px] text-[var(--color-warn)]">
                Recommandé
              </span>
            )}
            {!m.downloaded && (
              <span className="shrink-0 font-mono text-[9px] uppercase tracking-wide text-[var(--color-warn)]">à télécharger</span>
            )}
          </button>
        ))}
      </div>
      {models.some((m) => m.active && !m.downloaded) && (
        <div className="mt-2 flex items-center gap-2">
          <p className="flex-1 text-[10px] text-[var(--color-warn)]">
            Modèle non téléchargé — les features IA sont désactivées.
          </p>
          <button
            onClick={() => { localStorage.removeItem("lucid.ai.skipped"); location.reload(); }}
            className="shrink-0 rounded-lg bg-[var(--color-accent)] px-2.5 py-1 text-[10px] font-medium text-white hover:opacity-90"
          >
            Installer maintenant
          </button>
        </div>
      )}
    </div>
  );
}

// ── Section Connexions : Connecteurs / MCP / IA locale sous une même page ────

type ConnTab = "connectors" | "mcp" | "model";

function Pill({ active, onClick, children }: {
  active: boolean; onClick: () => void; children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "rounded-full px-3 py-1 text-xs font-medium transition-colors",
        active
          ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
          : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

function ConnectionsSection({
  connectors, onRefresh, onSyncDone,
}: {
  connectors: ConnectorStatus[];
  onRefresh: () => void;
  onSyncDone: () => void;
}) {
  const [tab, setTab] = useState<ConnTab>("connectors");
  const connected = connectors.filter((c) => c.connected).length;

  const TABS: { id: ConnTab; label: string }[] = [
    { id: "connectors", label: `Connecteurs · ${connected}/${connectors.length}` },
    { id: "mcp",        label: "MCP" },
    { id: "model",      label: "IA locale" },
  ];

  return (
    <div className="flex h-full flex-col">
      <div className="shrink-0 px-5 pt-4">
        <p className="mb-2.5 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
          Connexions · ce qui nourrit et consulte ton cerveau
        </p>
        <div className="flex gap-1 border-b border-[var(--color-border)] pb-2.5">
          {TABS.map((t) => (
            <Pill key={t.id} active={tab === t.id} onClick={() => setTab(t.id)}>{t.label}</Pill>
          ))}
        </div>
      </div>
      <div className="min-h-0 flex-1">
        {tab === "connectors" && (
          <ConnectorsSection connectors={connectors} onRefresh={onRefresh} onSyncDone={onSyncDone} />
        )}
        {tab === "mcp" && <AiClientsSection />}
        {tab === "model" && <ModelSection />}
      </div>
    </div>
  );
}

// ── SettingsModal ─────────────────────────────────────────────────────────────

type Section = "connections" | "spaces" | "account";

// ── Section « Compte » : auth Supabase + sauvegarde cloud du cerveau ─────────
function AccountSection({ onRestored }: { onRestored?: () => void }) {
  const sync = useSyncStatus();
  const [session, setSession] = useState<import("@supabase/supabase-js").Session | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [backups, setBackups] = useState<{ name: string; created_at: string; size: number }[]>([]);
  const [telemetry, setTelemetryState] = useState(false);
  const [sentryOn, setSentryOn] = useState(false);

  useEffect(() => {
    telemetryEnabled().then(setTelemetryState).catch(() => {});
    sentryActive().then(setSentryOn).catch(() => {});
    if (!supabase) return;
    supabase.auth.getSession().then(({ data }) => setSession(data.session));
    const { data: sub } = supabase.auth.onAuthStateChange((_e, s) => setSession(s));
    return () => sub.subscription.unsubscribe();
  }, []);

  const uid = session?.user.id;

  const refreshBackups = useCallback(async () => {
    if (!supabase || !uid) return;
    const { data, error } = await supabase.storage.from(BACKUP_BUCKET)
      .list(uid, { sortBy: { column: "created_at", order: "desc" }, limit: 20 });
    if (!error && data) {
      // sync.zip = la sync automatique, affichée dans son propre bloc — seules
      // les sauvegardes manuelles datées vont dans la liste « Restaurer ».
      setBackups(data.filter((f) => f.name !== SYNC_FILE).map((f) => ({
        name: f.name,
        created_at: f.created_at ?? "",
        size: (f.metadata as { size?: number } | null)?.size ?? 0,
      })));
    }
  }, [uid]);
  useEffect(() => { refreshBackups(); }, [refreshBackups]);

  async function auth(mode: "signin" | "signup") {
    if (!supabase || !email.trim() || password.length < 6) {
      setMsg("Email + mot de passe (6 caractères min).");
      return;
    }
    setBusy(mode); setMsg(null);
    try {
      const { error, data } = mode === "signin"
        ? await supabase.auth.signInWithPassword({ email: email.trim(), password })
        : await supabase.auth.signUp({ email: email.trim(), password });
      if (error) throw error;
      if (mode === "signup" && !data.session) {
        setMsg("Compte créé — vérifie ta boîte mail pour confirmer, puis connecte-toi.");
      }
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  async function handleBackup() {
    if (!supabase || !uid) return;
    setBusy("backup"); setMsg(null);
    try {
      const bytes = await exportBackup();
      const name = `${new Date().toISOString().replace(/[:.]/g, "-")}.zip`;
      const { error } = await supabase.storage.from(BACKUP_BUCKET)
        .upload(`${uid}/${name}`, new Blob([bytes.buffer as ArrayBuffer], { type: "application/zip" }));
      if (error) throw error;
      setMsg(`Sauvegardé ✓ (${(bytes.length / 1024 / 1024).toFixed(1)} Mo)`);
      await refreshBackups();
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  async function handleRestore(name: string) {
    if (!supabase || !uid) return;
    if (!confirm("Restaurer cette sauvegarde ? Le cerveau actuel sera remplacé (une copie locale de brain.json est gardée).")) return;
    setBusy(name); setMsg(null);
    try {
      const { data, error } = await supabase.storage.from(BACKUP_BUCKET).download(`${uid}/${name}`);
      if (error || !data) throw error ?? new Error("Téléchargement vide");
      const n = await importBackup(new Uint8Array(await data.arrayBuffer()));
      setMsg(`${n} fichiers restaurés ✓`);
      onRestored?.();
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  if (!supabase) {
    return (
      <div className="p-5">
        <p className="mb-1 text-sm font-semibold text-[var(--color-text)]">Compte</p>
        <p className="text-xs leading-relaxed text-[var(--color-muted)]">
          Supabase n'est pas configuré : remplis <code>VITE_SUPABASE_URL</code> et{" "}
          <code>VITE_SUPABASE_ANON_KEY</code> dans <code>.env</code> puis relance l'app.
        </p>
      </div>
    );
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <p className="mb-3 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
        Compte · accès, cloud & vie privée
      </p>
      <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
        Ton compte donne accès à l'app et permet de
        <strong> sauvegarder ton cerveau dans le cloud</strong> (~2 Mo chiffrés au repos)
        et de le retrouver sur une autre machine.
      </p>

      {!session ? (
        <div className="space-y-2">
          <input
            type="email" value={email} onChange={(e) => setEmail(e.target.value)}
            placeholder="email@exemple.fr"
            className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
          />
          <input
            type="password" value={password} onChange={(e) => setPassword(e.target.value)}
            placeholder="Mot de passe (6 caractères min)"
            className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
          />
          <div className="flex gap-2">
            <button onClick={() => auth("signin")} disabled={busy !== null}
              className="flex-1 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40">
              {busy === "signin" ? <Loader2 className="mx-auto size-4 animate-spin" /> : "Se connecter"}
            </button>
            <button onClick={() => auth("signup")} disabled={busy !== null}
              className="flex-1 rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text)] hover:bg-[var(--color-surface-2)] disabled:opacity-40">
              {busy === "signup" ? <Loader2 className="mx-auto size-4 animate-spin" /> : "Créer un compte"}
            </button>
          </div>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="flex items-center gap-3 rounded-xl border border-[var(--color-border)] px-3.5 py-2.5">
            <span className="size-2 shrink-0 rounded-full bg-[var(--color-ok)]" />
            <p className="min-w-0 flex-1 truncate text-sm text-[var(--color-text)]">{session.user.email}</p>
            <button onClick={() => supabase!.auth.signOut()}
              className="shrink-0 rounded-lg px-2.5 py-1 text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">
              Se déconnecter
            </button>
          </div>

          {/* Sync automatique : état visible + déclenchement manuel — jamais d'échec silencieux. */}
          <div className="rounded-xl border border-[var(--color-border)] px-3.5 py-2.5">
            <div className="flex items-center justify-between gap-2">
              <p className="flex items-center gap-2 text-sm text-[var(--color-text)]">
                <span className={cn(
                  "size-2 shrink-0 rounded-full",
                  sync.phase === "error" ? "bg-[var(--color-err)]"
                    : sync.phase === "ok" ? "bg-[var(--color-ok)]"
                    : "bg-[var(--color-muted)]",
                )} />
                Synchronisation automatique
              </p>
              <button onClick={() => void syncNow()} disabled={sync.phase === "syncing"}
                className="shrink-0 rounded-lg px-2.5 py-1 text-xs font-medium text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] disabled:opacity-40">
                {sync.phase === "syncing"
                  ? <Loader2 className="size-3.5 animate-spin" />
                  : "Synchroniser maintenant"}
              </button>
            </div>
            <p className={cn(
              "mt-1 text-xs leading-relaxed",
              sync.phase === "error" ? "text-[var(--color-err)]" : "text-[var(--color-muted)]",
            )}>
              {sync.phase === "error"
                ? `Erreur : ${sync.detail} — nouvel essai automatique sous 1 min.`
                : sync.phase === "ok" && sync.at
                  ? `À jour — dernière vérification ${relativeDate(new Date(sync.at).toISOString())}.`
                  : "Ton cerveau suit ton compte sur toutes tes machines, en continu."}
            </p>
          </div>

          <button onClick={handleBackup} disabled={busy !== null}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40">
            {busy === "backup" ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            Créer une sauvegarde manuelle
          </button>

          {backups.length > 0 && (
            <div>
              <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
                Sauvegardes manuelles ({backups.length})
              </p>
              <ul className="space-y-1">
                {backups.map((b) => (
                  <li key={b.name} className="flex items-center justify-between gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2">
                    <div className="min-w-0">
                      <p className="truncate text-xs text-[var(--color-text)]">
                        {b.created_at ? relativeDate(b.created_at) : b.name}
                      </p>
                      <p className="text-[10px] text-[var(--color-muted)]">{(b.size / 1024 / 1024).toFixed(1)} Mo</p>
                    </div>
                    <button onClick={() => handleRestore(b.name)} disabled={busy !== null}
                      className="shrink-0 rounded-lg px-2 py-1 text-[11px] font-medium text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] disabled:opacity-40">
                      {busy === b.name ? <Loader2 className="size-3 animate-spin" /> : "Restaurer"}
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}

      {msg && <p className="mt-3 text-xs text-[var(--color-muted)]">{msg}</p>}

      <div className="mt-6">
        <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
          Confidentialité
        </p>
        <label className="flex cursor-pointer items-start gap-3 rounded-xl border border-[var(--color-border)] px-3.5 py-2.5">
          <input
            type="checkbox"
            checked={telemetry}
            onChange={async (e) => {
              const on = e.target.checked;
              setTelemetryState(on);
              try { await setTelemetry(on); } catch { setTelemetryState(!on); }
            }}
            className="mt-0.5 accent-[var(--color-accent)]"
          />
          <span className="min-w-0">
            <span className="block text-sm text-[var(--color-text)]">Envoyer les rapports de crash</span>
            <span className="block text-xs leading-relaxed text-[var(--color-muted)]">
              Anonymes (aucun contenu de tes notes, chemins masqués). Désactivé par défaut :
              rien ne sort de ta machine. Prend effet au redémarrage de l'app.
            </span>
            <span className="mt-1 flex items-center gap-1.5 text-[11px]">
              <span className={cn("size-1.5 rounded-full", sentryOn ? "bg-[var(--color-ok)]" : "bg-[var(--color-muted)]")} />
              <span className="text-[var(--color-muted)]">
                {sentryOn
                  ? "Actif sur cette session"
                  : telemetry
                    ? "Inactif — redémarre l'app (ou DSN absent du build)"
                    : "Inactif"}
              </span>
            </span>
          </span>
        </label>
      </div>

      {import.meta.env.DEV && (
        <div className="mt-6 rounded-xl border border-dashed border-[var(--color-err)]/40 p-3">
          <p className="mb-2 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
            Dev uniquement
          </p>
          <button
            onClick={async () => {
              if (!confirm("Reset complet : cerveau, connecteurs, session et onboarding effacés (modèles IA gardés). Continuer ?")) return;
              setBusy("reset"); setMsg(null);
              try {
                await resetEnvironment();
                localStorage.clear(); // flags lucid.* + session Supabase → onboarding complet
                location.reload();
              } catch (e) {
                setMsg(String((e as Error).message ?? e));
                setBusy(null);
              }
            }}
            disabled={busy !== null}
            className="flex w-full items-center justify-center gap-2 rounded-lg border border-[var(--color-err)]/50 px-3 py-2 text-sm text-[var(--color-err)] hover:bg-[var(--color-err)]/10 disabled:opacity-40"
          >
            {busy === "reset" ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
            Tout réinitialiser (rejouer l'onboarding)
          </button>
          <button
            onClick={() => { crashTest().catch((e) => setMsg(String(e))); }}
            className="mt-2 flex w-full items-center justify-center gap-2 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]"
          >
            Tester Sentry (panic Rust volontaire)
          </button>
        </div>
      )}
    </div>
  );
}

// ── Section « Mes IA » : une URL MCP unique, pour n'importe quelle IA ────────
// Locale (Claude Desktop/Code sur cette machine) ou distante (claude.ai…) :
// même serveur, même URL, mêmes fonctionnalités (décision 2026-07-21) — plus
// de connexion par client, une seule chose à copier-coller.

export function AiClientsSection() {
  const [manualValidation, setManualValidationState] = useState(false);
  const [mcpUrl, setMcpUrl] = useState<string | null>(null);
  const [loadingUrl, setLoadingUrl] = useState(false);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    mcpManualValidationEnabled().then(setManualValidationState).catch(() => {});
  }, []);

  async function copyUrl(u: string) {
    if (await copyText(u)) { setCopied(true); setTimeout(() => setCopied(false), 2000); }
  }

  async function handleGetUrl() {
    setLoadingUrl(true);
    setError(null);
    try {
      const url = await ensurePersonalMcpUrl();
      if (!url) {
        setError("Indisponible — connecte-toi à ton compte Lucid, ou applique les migrations docs/supabase-mcp-personal.sql et docs/supabase-mcp-proposals-v2.sql dans Supabase.");
        return;
      }
      setMcpUrl(url);
      localStorage.setItem("lucid.mcp.connected", "1"); // checklist onboarding (App.tsx)
      await copyUrl(url);
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setLoadingUrl(false);
    }
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
        Une seule URL branche n'importe quelle IA à ton cerveau — Claude Desktop,
        Claude Code, claude.ai en ligne… Elles peuvent le consulter et proposer des
        changements, que tu valides dans Lucid (ou qui s'appliquent aussitôt en
        mode autonome, réglable ci-dessous).
      </p>

      <div className="rounded-xl border border-[var(--color-border)] px-4 py-3">
        {mcpUrl ? (
          <div className="flex items-center gap-2.5">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-[var(--color-text)]">URL MCP personnelle</p>
              <p className="truncate font-mono text-[10.5px] text-[var(--color-muted)]">{mcpUrl}</p>
            </div>
            <button
              onClick={() => copyUrl(mcpUrl)}
              title="Copier"
              className="shrink-0 rounded-md p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
            >
              {copied ? <Check className="size-3.5 text-[var(--color-ok)]" /> : <Copy className="size-3.5" />}
            </button>
          </div>
        ) : (
          <button
            onClick={handleGetUrl}
            disabled={loadingUrl}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40"
          >
            {loadingUrl ? <Loader2 className="size-4 animate-spin" /> : <Copy className="size-4" />}
            Copier l'URL MCP personnelle
          </button>
        )}
        {error && <p className="mt-2 text-[11px] text-[var(--color-err)]">{error}</p>}
        <McpConnectGuide url={mcpUrl} />
      </div>

      <p className="mt-4 text-[11px] leading-relaxed text-[var(--color-muted)]">
        Après connexion, demande par exemple : « Qu'est-ce qu'il y a dans mon second
        cerveau sur … ? » ou « Crée-moi une structure de révision dans Lucid ».
      </p>

      {/* Archiviste : autonomie par défaut, filet de sécurité toujours actif
          (snapshot avant chaque action, annulable depuis l'Historique). */}
      <div className="mt-5 border-t border-[var(--color-border)] pt-4">
        <p className="mb-1.5 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
          Archiviste
        </p>
        <label className="flex cursor-pointer items-start gap-3 rounded-xl border border-[var(--color-border)] px-3.5 py-2.5">
          <input
            type="checkbox"
            checked={manualValidation}
            onChange={async (e) => {
              const on = e.target.checked;
              setManualValidationState(on);
              try { await setMcpManualValidation(on); } catch { setManualValidationState(!on); }
            }}
            className="mt-0.5 accent-[var(--color-accent)]"
          />
          <span className="min-w-0">
            <span className="block text-sm text-[var(--color-text)]">Valider manuellement les actions de l'Archiviste</span>
            <span className="block text-xs leading-relaxed text-[var(--color-muted)]">
              Décoché (défaut) : l'Archiviste écrit directement dans ton cerveau. Coché :
              chaque action attend ta validation (bulle ou badge à accepter/refuser). Dans
              les deux cas, un snapshot est pris avant chaque action et reste annulable
              depuis l'icône Historique en haut de l'écran. S'applique pareil, que l'IA
              soit locale ou distante — même mécanisme, un seul réglage.
            </span>
          </span>
        </label>
      </div>
    </div>
  );
}

interface Props {
  connectors: ConnectorStatus[];
  spaces: Space[];
  onRefresh: () => void;
  onSyncDone: () => void;
  onClose: () => void;
  onSpaceCreate: (name: string) => void;
  onSpaceRename: (id: string, name: string) => void;
  onSpaceDelete: (id: string) => void;
  /** Appelé après restauration d'une sauvegarde (recharge le graphe). */
  onRestored?: () => void;
}

export function SettingsModal({
  connectors, spaces, onRefresh, onSyncDone, onClose,
  onSpaceCreate, onSpaceRename, onSpaceDelete, onRestored,
}: Props) {
  const [section, setSection] = useState<Section>("connections");
  const [version, setVersion] = useState("");
  useEffect(() => { getVersion().then(setVersion).catch(() => {}); }, []);

  // Échap ferme la modale (comportement attendu de toute surface modale).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  type NavItem = { id: Section; label: string; desc: string; icon: React.ReactNode };
  const NAV_GROUPS: { label: string; items: NavItem[] }[] = [
    {
      label: "Compte",
      items: [
        { id: "account", label: "Compte", desc: "Accès, sync & vie privée", icon: <User className="size-3.5" /> },
      ],
    },
    {
      label: "Cerveau",
      items: [
        { id: "connections", label: "Connexions", desc: "Connecteurs, MCP & IA locale", icon: <Plug className="size-3.5" /> },
        { id: "spaces",      label: "Espaces",    desc: "Tes vues du graphe",           icon: <Layers className="size-3.5" /> },
      ],
    },
  ];

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40" onMouseDown={onClose}>
      <div
        className="panel flex h-[520px] w-[720px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* Nav gauche */}
        <div className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-[var(--color-border)] p-3">
          <p className="px-2 pb-1 pt-1 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
            Paramètres
          </p>
          {NAV_GROUPS.map((g) => (
            <div key={g.label} className="flex flex-col gap-0.5">
              <p className="px-2 pb-1 pt-2.5 text-[10px] font-medium uppercase tracking-wide text-[var(--color-muted)]/70">
                {g.label}
              </p>
              {g.items.map((n) => (
                <button
                  key={n.id}
                  onClick={() => setSection(n.id)}
                  className={cn(
                    "flex items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors",
                    section === n.id
                      ? "bg-[var(--color-accent-soft)]"
                      : "hover:bg-[var(--color-surface-2)]",
                  )}
                >
                  <span className={cn("mt-0.5", section === n.id ? "text-[var(--color-accent)]" : "text-[var(--color-muted)]")}>
                    {n.icon}
                  </span>
                  <span className="min-w-0">
                    <span className={cn(
                      "block text-xs font-medium",
                      section === n.id ? "text-[var(--color-accent)]" : "text-[var(--color-text)]",
                    )}>
                      {n.label}
                    </span>
                    <span className="block text-[10px] leading-tight text-[var(--color-muted)]">{n.desc}</span>
                  </span>
                </button>
              ))}
            </div>
          ))}
          <p className="mt-auto px-2 pb-1 font-mono text-[9px] uppercase tracking-[0.16em] text-[var(--color-muted)]/70">
            Lucid{version ? ` · v${version}` : ""}
          </p>
        </div>

        {/* Contenu */}
        <div className="relative min-w-0 flex-1">
          <button
            onClick={onClose}
            title="Fermer (Échap)"
            className="absolute right-3 top-3 z-10 rounded-lg p-1.5 text-[var(--color-muted)] transition-colors hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
          >
            <X className="size-4" />
          </button>
          {section === "connections" && (
            <ConnectionsSection connectors={connectors} onRefresh={onRefresh} onSyncDone={onSyncDone} />
          )}
          {section === "spaces" && (
            <SpacesSection spaces={spaces} onCreate={onSpaceCreate} onRename={onSpaceRename} onDelete={onSpaceDelete} />
          )}
          {section === "account" && <AccountSection onRestored={onRestored} />}
        </div>
      </div>
    </div>
  );
}
