import { useState, useEffect } from "react";
import {
  Plug,
  MessageCircle,
  Send,
  Loader2,
  Settings,
  X,
  RefreshCw,
  LogOut,
  CheckCircle2,
  Circle,
  Clock,
  FileText,
  Cpu,
} from "lucide-react";
import claudeLogo from "@/assets/claude-logo.png";
import googleDriveLogo from "@/assets/google_drive.svg.png";
import { open } from "@tauri-apps/plugin-dialog";
import {
  askBrain,
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
  type ModelInfo,
} from "@/lib/api";
import type { ConnectorStatus } from "@/lib/types";
import { cn, relativeDate } from "@/lib/utils";

interface Props {
  connectors: ConnectorStatus[];
  onRefresh: () => void;
  onSyncDone: () => void;
  onClose?: () => void;
}

type Tab = "connectors" | "assistant";

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

function ConnectorLogo({ id }: { id: string }) {
  if (id === "claude-code")  return <LogoClaudeCode />;
  if (id === "claude-ai")    return <LogoClaudeAi />;
  if (id === "google-drive") return <LogoGoogleDrive />;
  if (id === "notion")       return <LogoNotion />;
  if (id === "obsidian")     return <LogoObsidian />;
  if (id === "cowork")       return <LogoCowork />;
  return (
    <div className="flex size-9 items-center justify-center rounded-xl bg-[var(--color-surface-2)]">
      <Plug className="size-4 text-[var(--color-muted)]" />
    </div>
  );
}

// ── Status badge ──────────────────────────────────────────────────────────────

function StatusBadge({ connected, soon }: { connected: boolean; soon?: boolean }) {
  if (soon) return (
    <span className="flex items-center gap-1 text-[11px] text-[var(--color-muted)]">
      <Circle className="size-2.5" /> Bientôt
    </span>
  );
  return connected ? (
    <span className="flex items-center gap-1 text-[11px] text-emerald-500">
      <CheckCircle2 className="size-2.5" /> Configuré
    </span>
  ) : (
    <span className="flex items-center gap-1 text-[11px] text-amber-500">
      <Circle className="size-2.5 fill-amber-500" /> Non configuré
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
      className="absolute inset-0 z-50 flex flex-col"
      style={{ background: "rgba(0,0,0,0.18)", backdropFilter: "blur(2px)" }}
    >
      <div
        className="m-3 mt-auto flex flex-col gap-0 overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]"
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
      <div className={cn("size-1.5 rounded-full shrink-0", ok ? "bg-emerald-500" : "bg-red-400")} />
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
            ? "border border-red-400/40 text-red-400 hover:bg-red-400/10"
            : "border border-[var(--color-border)] text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)]",
      )}
    >
      {busy ? <Loader2 className="size-3.5 animate-spin" /> : icon}
      {children}
    </button>
  );
}

// ── ModelSection ──────────────────────────────────────────────────────────────

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

  if (models.length === 0) return null;

  return (
    <div className="mt-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] p-2.5">
      <div className="mb-2 flex items-center gap-1.5">
        <Cpu className="size-3 text-[var(--color-muted)]" />
        <span className="text-[11px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
          Modèle IA
        </span>
      </div>
      <div className="flex flex-col gap-1">
        {models.map((m) => (
          <button
            key={m.id}
            onClick={() => !m.active && !switching && handleSwitch(m.id)}
            disabled={switching}
            className={cn(
              "flex items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition-colors disabled:opacity-60",
              m.active
                ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
            )}
          >
            <span className="flex-1 font-medium">{m.name}</span>
            <span className="shrink-0 text-[10px] opacity-70">{m.size_gb} Go</span>
            {m.recommended && !m.active && (
              <span className="shrink-0 rounded-full bg-amber-500/20 px-1.5 py-0.5 text-[9px] text-amber-500">
                Recommandé
              </span>
            )}
            {!m.downloaded && (
              <span className="shrink-0 text-[9px] text-red-400">À dl</span>
            )}
          </button>
        ))}
      </div>
      {models.some((m) => m.active && !m.downloaded) && (
        <p className="mt-1.5 text-[10px] text-amber-500">
          Modèle non téléchargé — redémarrez l'app pour le télécharger.
        </p>
      )}
    </div>
  );
}

// ── ConnectorsTab ─────────────────────────────────────────────────────────────

export function LeftSidebar({ connectors, onRefresh, onSyncDone, onClose }: Props) {
  const [tab, setTab] = useState<Tab>("connectors");

  return (
    <div className="flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-1 border-b border-[var(--color-border)] p-2">
        <TabBtn active={tab === "connectors"} onClick={() => setTab("connectors")}>
          <Plug className="size-4" /> Connecteurs
        </TabBtn>
        <TabBtn active={tab === "assistant"} onClick={() => setTab("assistant")}>
          <MessageCircle className="size-4" /> Assistant
        </TabBtn>
        {onClose && (
          <button
            onClick={onClose}
            className="ml-auto rounded-md p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
          >
            <X className="size-4" />
          </button>
        )}
      </div>

      {tab === "connectors" ? (
        <ConnectorsTab connectors={connectors} onRefresh={onRefresh} onSyncDone={onSyncDone} />
      ) : (
        <AssistantTab />
      )}
    </div>
  );
}

function ConnectorsTab({
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
    <div className="relative flex-1 overflow-hidden">
      <div className="h-full space-y-2 overflow-y-auto p-3">
        {connectors.map((c) => {
          const isSoon = c.id === "cowork";
          return (
            <div
              key={c.id}
              className={cn(
                "flex items-center gap-2.5 rounded-lg border px-2.5 py-2 transition-colors",
                c.connected
                  ? "border-emerald-500/25 bg-emerald-500/5"
                  : "border-[var(--color-border)] bg-[var(--color-bg)]",
                isSoon && "opacity-40",
              )}
            >
              <ConnectorLogo id={c.id} />
              <div className="flex-1 min-w-0">
                <div className="text-xs font-medium leading-tight truncate">{c.name}</div>
                <StatusBadge connected={c.connected} soon={isSoon} />
              </div>
              {!isSoon && (
                <button
                  onClick={() => setModalId(c.id)}
                  className="shrink-0 rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                  title="Paramètres"
                >
                  <Settings className="size-3.5" />
                </button>
              )}
            </div>
          );
        })}

        <p className="px-1 pt-2 text-[11px] leading-relaxed text-[var(--color-muted)]">
          Chaque source connectée alimente le même graphe.
        </p>

        <ModelSection />
      </div>

      {/* Modal overlay */}
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

// ── UI helpers ────────────────────────────────────────────────────────────────

function TabBtn({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex flex-1 items-center justify-center gap-1.5 rounded-lg px-2 py-1.5 text-xs font-medium transition-colors",
        active ? "bg-[var(--color-accent)] text-white" : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

// ── AssistantTab (inchangé) ───────────────────────────────────────────────────

interface ChatMsg { role: "user" | "assistant"; text: string; }

function AssistantTab() {
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);

  async function send() {
    const q = input.trim();
    if (!q || loading) return;
    setInput("");
    setMessages((m) => [...m, { role: "user", text: q }]);
    setLoading(true);
    try {
      const answer = await askBrain(q);
      setMessages((m) => [...m, { role: "assistant", text: answer }]);
    } catch (e) {
      setMessages((m) => [...m, { role: "assistant", text: `⚠️ ${e}` }]);
    } finally { setLoading(false); }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex-1 space-y-2 overflow-y-auto p-3">
        {messages.length === 0 && (
          <p className="px-1 pt-2 text-xs leading-relaxed text-[var(--color-muted)]">
            Pose une question sur ton second cerveau — l'IA locale répond à partir
            de ton <code>brain.md</code>.
          </p>
        )}
        {messages.map((m, i) => (
          <div
            key={i}
            className={cn(
              "rounded-lg px-3 py-2 text-xs leading-relaxed",
              m.role === "user"
                ? "bg-[var(--color-accent-soft)]"
                : "border border-[var(--color-border)] bg-[var(--color-bg)]",
            )}
          >
            <span className="whitespace-pre-wrap break-words">{m.text}</span>
          </div>
        ))}
        {loading && (
          <div className="flex items-center gap-2 px-1 text-xs text-[var(--color-muted)]">
            <Loader2 className="size-3.5 animate-spin" /> réflexion…
          </div>
        )}
      </div>
      <div className="shrink-0 border-t border-[var(--color-border)] p-2">
        <div className="flex items-end gap-1.5">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
            rows={1}
            placeholder="Demander…"
            className="max-h-24 min-h-9 flex-1 resize-none rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-2 text-xs outline-none placeholder:text-[var(--color-muted)] focus:border-[var(--color-accent)]"
          />
          <button
            onClick={send}
            disabled={loading || !input.trim()}
            className="flex size-9 items-center justify-center rounded-md bg-[var(--color-accent)] text-white disabled:opacity-50"
          >
            <Send className="size-4" />
          </button>
        </div>
      </div>
    </div>
  );
}
