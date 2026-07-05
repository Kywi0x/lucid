import { useState, useEffect, useCallback } from "react";
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
  Cpu,
  Layers,
  Pencil,
  Trash2,
  Plus,
  Bot,
  User,
} from "lucide-react";
import claudeLogo from "@/assets/claude-logo.png";
import googleDriveLogo from "@/assets/google_drive.svg.png";
import cursorLogo from "@/assets/cursor.svg";
import openaiLogo from "@/assets/openai.svg";
import { open } from "@tauri-apps/plugin-dialog";
import {
  googleDriveConnect,
  googleDriveDisconnect,
  googleDriveSync,
  importClaudeAi,
  notionConnect,
  notionDisconnect,
  notionSync,
  obsidianSetVault,
  obsidianVaultPath,
  obsidianDisconnect,
  listModels,
  setActiveModel,
  aiClientsStatus,
  connectAiClient,
  disconnectAiClient,
  exportBackup,
  importBackup,
  type ModelInfo,
} from "@/lib/api";
import type { AiClientStatus } from "@/lib/types";
import { supabase, BACKUP_BUCKET } from "@/lib/supabase";
import type { ConnectorStatus, Space } from "@/lib/types";
import { cn, relativeDate } from "@/lib/utils";

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

function LogoNotion() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-[var(--color-border)] bg-white">
      <svg viewBox="0 0 24 24" className="size-5" aria-hidden fill="none">
        <path d="M4 4.5C4 3.67 4.67 3 5.5 3H17l3 3v13.5c0 .83-.67 1.5-1.5 1.5h-13C4.67 21 4 20.33 4 19.5V4.5z" fill="#fff" stroke="#e5e5e5" strokeWidth="1"/>
        <path d="M7 8h10M7 12h7M7 16h5" stroke="#37352f" strokeWidth="1.5" strokeLinecap="round"/>
        <path d="M17 3v3h3" stroke="#e5e5e5" strokeWidth="1"/>
      </svg>
    </div>
  );
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

function LogoCowork() {
  return (
    <div className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-[#0EA5E9]">
      <MessageCircle className="size-3.5 text-white" />
    </div>
  );
}

export function ConnectorLogo({ id }: { id: string }) {
  if (id === "claude-code")  return <LogoClaudeCode />;
  if (id === "claude-ai")    return <LogoClaudeAi />;
  if (id === "google-drive") return <LogoGoogleDrive />;
  if (id === "notion")       return <LogoNotion />;
  if (id === "obsidian")     return <LogoObsidian />;
  if (id === "cowork")       return <LogoCowork />;
  return (
    <div className="flex size-7 items-center justify-center rounded-lg bg-[var(--color-surface-2)]">
      <Plug className="size-4 text-[var(--color-muted)]" />
    </div>
  );
}

// ── Statut : point 6px + label mono ──────────────────────────────────────────

function StatusLine({ c, soon }: { c: ConnectorStatus; soon?: boolean }) {
  const label = soon
    ? "bientôt"
    : c.connected
      ? c.last_sync ? `sync ${relativeDate(c.last_sync)}` : "connecté"
      : "non lié";
  return (
    <span className="ml-auto flex shrink-0 items-center gap-2 font-mono text-[10px] uppercase tracking-wider text-[var(--color-muted)]">
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
  notionToken?: string;
  onNotionTokenChange?: (v: string) => void;
  obsidianVault?: string | null;
  onClose: () => void;
  onConnect: () => void;
  onSync: () => void;
  onDisconnect: () => void;
  onImport: () => void;
}

function ConnectorModal({ c, busy, msg, notionToken, onNotionTokenChange, obsidianVault, onClose, onConnect, onSync, onDisconnect, onImport }: ModalProps) {
  const isSoon = c.id === "cowork";

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
              {c.id === "claude-code" ? "~/.claude/projects/" : c.id === "claude-ai" ? "Import ZIP" : "Connecté"}
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

            {c.id === "claude-ai" && (
              <ActionBtn busy={busy} icon={<FileText className="size-3.5" />} onClick={onImport}>
                {c.connected ? "Réimporter un ZIP" : "Importer export ZIP"}
              </ActionBtn>
            )}

            {c.id === "notion" && !c.connected && (
              <div className="flex flex-col gap-2">
                <input
                  type="password"
                  value={notionToken ?? ""}
                  onChange={(e) => onNotionTokenChange?.(e.target.value)}
                  placeholder="secret_xxxxxxxxxxxx"
                  className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-1.5 text-xs outline-none placeholder:text-[var(--color-muted)] focus:border-[var(--color-accent)]"
                />
                <p className="text-[10px] text-[var(--color-muted)]">
                  Crée un token sur <span className="text-[var(--color-accent)]">notion.so/my-integrations</span> puis colle-le ici.
                </p>
                <ActionBtn busy={busy} icon={<CheckCircle2 className="size-3.5" />} onClick={onConnect} primary>
                  Connecter Notion
                </ActionBtn>
              </div>
            )}

            {c.id === "google-drive" && !c.connected && (
              <ActionBtn busy={busy} icon={<CheckCircle2 className="size-3.5" />} onClick={onConnect} primary>
                Connecter Google Drive
              </ActionBtn>
            )}

            {c.id === "notion" && c.connected && (
              <>
                <ActionBtn busy={busy} icon={<RefreshCw className="size-3.5" />} onClick={onSync} primary>
                  Synchroniser les pages
                </ActionBtn>
                <ActionBtn busy={busy} icon={<LogOut className="size-3.5" />} onClick={onDisconnect} danger>
                  Déconnecter
                </ActionBtn>
              </>
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

  const [notionToken, setNotionToken] = useState("");

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

  async function handleNotionConnect() {
    set("notion", true); msg("notion", "");
    try {
      await notionConnect(notionToken.trim());
      msg("notion", "Connecté !");
      setNotionToken("");
      onRefresh();
    } catch (e) { msg("notion", `Erreur : ${e}`); }
    finally { set("notion", false); }
  }

  async function handleNotionSync() {
    set("notion", true); msg("notion", "Synchronisation…");
    try {
      const [newPages, total] = await notionSync();
      msg("notion", newPages > 0 ? `${newPages} nouvelles pages sur ${total}` : `${total} pages synchronisées`);
      onRefresh();
      onSyncDone(); // toujours régénérer si sync réussie
    } catch (e) { msg("notion", `${e}`); }
    finally { set("notion", false); }
  }

  async function handleNotionDisconnect() {
    set("notion", true);
    try {
      await notionDisconnect();
      msg("notion", "Déconnecté");
      onRefresh();
    } catch (e) { msg("notion", `Erreur : ${e}`); }
    finally { set("notion", false); }
  }

  const openModal = modalId ? connectors.find((c) => c.id === modalId) : null;

  return (
    <div className="relative flex h-full flex-col">
      <div className="flex-1 overflow-y-auto px-5 py-4">
        <p className="mb-3 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
          Connecteurs · {connectors.filter((c) => c.connected).length}/{connectors.length}
        </p>
        {connectors.map((c) => {
          const isSoon = c.id === "cowork";
          return (
            <button
              key={c.id}
              onClick={() => !isSoon && setModalId(c.id)}
              disabled={isSoon}
              className={cn(
                "flex w-full items-center gap-2.5 border-b border-[var(--color-border)] px-1 py-2.5 text-left transition-colors last:border-b-0",
                isSoon ? "opacity-40" : "hover:bg-[var(--color-surface-2)]",
              )}
            >
              <ConnectorLogo id={c.id} />
              <span className="min-w-0 truncate text-sm font-medium">{c.name}</span>
              {msgs[c.id] && !isSoon && (
                <span className="min-w-0 truncate text-[10px] text-[var(--color-muted)]">{msgs[c.id]}</span>
              )}
              <StatusLine c={c} soon={isSoon} />
            </button>
          );
        })}
        <p className="px-1 pt-3 text-[11px] leading-relaxed text-[var(--color-muted)]">
          Chaque source connectée alimente le même graphe.
        </p>
      </div>

      {openModal && (
        <ConnectorModal
          c={openModal}
          busy={busy[openModal.id] ?? false}
          msg={msgs[openModal.id] ?? ""}
          notionToken={notionToken}
          onNotionTokenChange={setNotionToken}
          obsidianVault={obsidianVault}
          onClose={() => setModalId(null)}
          onConnect={
            openModal.id === "notion" ? handleNotionConnect :
            openModal.id === "obsidian" ? handleObsidianConnect :
            handleGoogleConnect
          }
          onSync={openModal.id === "notion" ? handleNotionSync : handleGoogleSync}
          onDisconnect={
            openModal.id === "notion" ? handleNotionDisconnect :
            openModal.id === "obsidian" ? handleObsidianDisconnect :
            handleGoogleDisconnect
          }
          onImport={handleImportClaudeAi}
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
      <p className="mb-3 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
        Modèle IA local
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
              <span className="shrink-0 text-[9px] text-[var(--color-err)]">À dl</span>
            )}
          </button>
        ))}
      </div>
      {models.some((m) => m.active && !m.downloaded) && (
        <p className="mt-2 text-[10px] text-[var(--color-warn)]">
          Modèle non téléchargé — redémarrez l'app pour le télécharger.
        </p>
      )}
    </div>
  );
}

// ── SettingsModal ─────────────────────────────────────────────────────────────

type Section = "connectors" | "ai-clients" | "spaces" | "model" | "account";

// ── Section « Compte » : auth Supabase + sauvegarde cloud du cerveau ─────────
function AccountSection({ onRestored }: { onRestored?: () => void }) {
  const [session, setSession] = useState<import("@supabase/supabase-js").Session | null>(null);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [backups, setBackups] = useState<{ name: string; created_at: string; size: number }[]>([]);

  useEffect(() => {
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
      setBackups(data.map((f) => ({
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
      <p className="mb-1 text-sm font-semibold text-[var(--color-text)]">Compte</p>
      <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
        Optionnel — l'app fonctionne entièrement sans compte. Le compte sert à
        <strong> sauvegarder ton cerveau dans le cloud</strong> (~2 Mo chiffrés au repos)
        et à le retrouver sur un autre Mac.
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

          <button onClick={handleBackup} disabled={busy !== null}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40">
            {busy === "backup" ? <Loader2 className="size-4 animate-spin" /> : <RefreshCw className="size-4" />}
            Sauvegarder mon cerveau maintenant
          </button>

          {backups.length > 0 && (
            <div>
              <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
                Sauvegardes ({backups.length})
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
    </div>
  );
}

// ── Section « Mes IA » : connexion one-click du serveur MCP Lucid ────────────

function AiClientLogo({ id }: { id: string }) {
  switch (id) {
    case "claude-desktop": return <LogoImg src={claudeLogo} alt="Claude Desktop" bg="bg-[#1a1a1a]" />;
    case "claude-code":    return <LogoClaudeCode />;
    case "cursor":         return <LogoImg src={cursorLogo} alt="Cursor" />;
    case "codex":          return <LogoImg src={openaiLogo} alt="Codex (OpenAI)" />;
    default:               return <LogoImg src={claudeLogo} alt={id} />;
  }
}

export function AiClientsSection() {
  const [clients, setClients] = useState<AiClientStatus[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [msgs, setMsgs] = useState<Record<string, string>>({});

  const refresh = () => aiClientsStatus().then(setClients).catch(console.error);
  useEffect(() => { refresh(); }, []);

  async function handle(id: string, connect: boolean) {
    setBusy(id);
    try {
      if (connect) {
        const m = await connectAiClient(id);
        setMsgs((s) => ({ ...s, [id]: m }));
      } else {
        await disconnectAiClient(id);
        setMsgs((s) => ({ ...s, [id]: "Déconnecté." }));
      }
      await refresh();
    } catch (e) {
      setMsgs((s) => ({ ...s, [id]: String(e) }));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="h-full overflow-y-auto p-5">
      <p className="mb-1 text-sm font-semibold text-[var(--color-text)]">Connecter mes IA</p>
      <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
        Branche ton second cerveau à tes IA en un clic : elles pourront le consulter
        (recherche, lecture) et proposer des pages — que tu valides dans Lucid.
        100 % local, aucune donnée n'est envoyée en ligne.
      </p>
      <div className="space-y-2">
        {clients.map((c) => (
          <div key={c.id} className={cn(
            "rounded-xl border border-[var(--color-border)] px-4 py-3 transition-opacity",
            !c.installed && "opacity-50",
          )}>
            <div className="flex items-center gap-3">
              <AiClientLogo id={c.id} />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium text-[var(--color-text)]">{c.name}</p>
                <p className="flex items-center gap-1.5 text-[11px] text-[var(--color-muted)]">
                  <span className={
                    "size-1.5 shrink-0 rounded-full " +
                    (c.connected ? "bg-[var(--color-ok)]" : c.installed ? "bg-[#e0a33c]" : "bg-[var(--color-border)]")
                  } />
                  {c.connected ? "Connecté au cerveau Lucid" : c.installed ? "Détecté — pas encore connecté" : "Non détecté sur ce Mac"}
                </p>
              </div>
              {c.connected ? (
                <button
                  onClick={() => handle(c.id, false)}
                  disabled={busy === c.id}
                  className="rounded-lg px-3 py-1.5 text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] disabled:opacity-40"
                >
                  {busy === c.id ? <Loader2 className="size-3.5 animate-spin" /> : "Déconnecter"}
                </button>
              ) : (
                <button
                  onClick={() => handle(c.id, true)}
                  disabled={busy === c.id || !c.installed}
                  className="rounded-lg bg-[var(--color-accent)] px-3 py-1.5 text-xs text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40"
                >
                  {busy === c.id ? <Loader2 className="size-3.5 animate-spin" /> : "Connecter"}
                </button>
              )}
            </div>
            {msgs[c.id] && (
              <p className="mt-2 text-[11px] text-[var(--color-muted)]">{msgs[c.id]}</p>
            )}
          </div>
        ))}
      </div>
      <p className="mt-4 text-[11px] leading-relaxed text-[var(--color-muted)]">
        Après connexion, demande par exemple : « Qu'est-ce qu'il y a dans mon second
        cerveau sur … ? » ou « Crée-moi une structure de révision dans Lucid ».
      </p>
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
  const [section, setSection] = useState<Section>("connectors");

  const NAV: { id: Section; label: string; desc: string; icon: React.ReactNode }[] = [
    { id: "connectors", label: "Sources",  desc: "D'où vient ton savoir",     icon: <Plug className="size-3.5" /> },
    { id: "ai-clients", label: "Mes IA",   desc: "Qui consulte ton cerveau",  icon: <Bot className="size-3.5" /> },
    { id: "spaces",     label: "Spaces",   desc: "Tes vues du graphe",        icon: <Layers className="size-3.5" /> },
    { id: "model",      label: "IA locale", desc: "Le moteur d'analyse",      icon: <Cpu className="size-3.5" /> },
    { id: "account",    label: "Compte",   desc: "Sauvegarde cloud",          icon: <User className="size-3.5" /> },
  ];

  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-black/40" onMouseDown={onClose}>
      <div
        className="panel flex h-[520px] w-[720px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        {/* Nav gauche */}
        <div className="flex w-44 shrink-0 flex-col gap-0.5 border-r border-[var(--color-border)] p-3">
          <p className="px-2 pb-2 pt-1 font-mono text-[10px] uppercase tracking-widest text-[var(--color-muted)]">
            Paramètres
          </p>
          {NAV.map((n) => (
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
          <button
            onClick={onClose}
            className="mt-auto flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-left text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
          >
            <X className="size-3.5" /> Fermer
          </button>
        </div>

        {/* Contenu */}
        <div className="relative min-w-0 flex-1">
          {section === "connectors" && (
            <ConnectorsSection connectors={connectors} onRefresh={onRefresh} onSyncDone={onSyncDone} />
          )}
          {section === "ai-clients" && <AiClientsSection />}
          {section === "spaces" && (
            <SpacesSection spaces={spaces} onCreate={onSpaceCreate} onRename={onSpaceRename} onDelete={onSpaceDelete} />
          )}
          {section === "model" && <ModelSection />}
          {section === "account" && <AccountSection onRestored={onRestored} />}
        </div>
      </div>
    </div>
  );
}
