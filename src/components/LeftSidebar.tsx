import { useState } from "react";
import {
  Plug,
  MessageCircle,
  Send,
  Loader2,
  Terminal,
  Sparkles,
  Cloud,
  Check,
  Upload,
} from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  askBrain,
  googleDriveConnect,
  googleDriveDisconnect,
  googleDriveSync,
  importClaudeAi,
} from "@/lib/api";
import type { ConnectorStatus } from "@/lib/types";
import { cn, relativeDate } from "@/lib/utils";

interface Props {
  connectors: ConnectorStatus[];
  onRefresh: () => void;
  onSyncDone: () => void;
}

type Tab = "connectors" | "assistant";

const CONNECTOR_ICON: Record<string, typeof Terminal> = {
  "claude-code": Terminal,
  "claude-ai": Sparkles,
  cowork: MessageCircle,
  "google-drive": Cloud,
};

export function LeftSidebar({ connectors, onRefresh, onSyncDone }: Props) {
  const [tab, setTab] = useState<Tab>("connectors");

  return (
    <div className="pointer-events-auto flex w-[300px] flex-col overflow-hidden rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]">
      <div className="flex shrink-0 border-b border-[var(--color-border)] p-1">
        <TabBtn active={tab === "connectors"} onClick={() => setTab("connectors")}>
          <Plug className="size-4" /> Connecteurs
        </TabBtn>
        <TabBtn active={tab === "assistant"} onClick={() => setTab("assistant")}>
          <MessageCircle className="size-4" /> Assistant
        </TabBtn>
      </div>

      {tab === "connectors" ? (
        <ConnectorsTab connectors={connectors} onRefresh={onRefresh} onSyncDone={onSyncDone} />
      ) : (
        <AssistantTab />
      )}
    </div>
  );
}

function TabBtn({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex flex-1 items-center justify-center gap-1.5 rounded-lg px-2 py-1.5 text-xs font-medium transition-colors",
        active
          ? "bg-[var(--color-accent)] text-white"
          : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
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

  function setConnectorBusy(id: string, v: boolean) {
    setBusy((b) => ({ ...b, [id]: v }));
  }
  function setConnectorMsg(id: string, msg: string) {
    setMsgs((m) => ({ ...m, [id]: msg }));
  }

  async function handleImportClaudeAi() {
    const selected = await open({
      filters: [{ name: "Export claude.ai", extensions: ["zip"] }],
      multiple: false,
    });
    if (!selected || typeof selected !== "string") return;
    setConnectorBusy("claude-ai", true);
    setConnectorMsg("claude-ai", "");
    try {
      const count = await importClaudeAi(selected);
      setConnectorMsg("claude-ai", `${count} conversations importées`);
      onRefresh();
    } catch (e) {
      setConnectorMsg("claude-ai", `Erreur : ${e}`);
    } finally {
      setConnectorBusy("claude-ai", false);
    }
  }



  async function handleGoogleConnect() {
    setConnectorBusy("google-drive", true);
    setConnectorMsg("google-drive", "En attente du navigateur…");
    try {
      await googleDriveConnect();
      setConnectorMsg("google-drive", "Connecté !");
      onRefresh();
    } catch (e) {
      setConnectorMsg("google-drive", `Erreur : ${e}`);
    } finally {
      setConnectorBusy("google-drive", false);
    }
  }

  async function handleGoogleSync() {
    setConnectorBusy("google-drive", true);
    setConnectorMsg("google-drive", "Synchronisation…");
    try {
      const [newFiles, total] = await googleDriveSync();
      const label = newFiles > 0
        ? `${newFiles} nouveau(x) sur ${total} — génération en cours…`
        : `${total} fichiers en cache — mise à jour du graphe…`;
      setConnectorMsg("google-drive", label);
      onRefresh();
      // Régénère si des fichiers Drive existent (même inchangés : le brain.json
      // peut ne pas les inclure encore, et le pipeline-cache évite les LLM inutiles).
      if (total > 0) onSyncDone();
    } catch (e) {
      setConnectorMsg("google-drive", `Erreur : ${e}`);
    } finally {
      setConnectorBusy("google-drive", false);
    }
  }

  async function handleGoogleDisconnect() {
    setConnectorBusy("google-drive", true);
    try {
      await googleDriveDisconnect();
      setConnectorMsg("google-drive", "Déconnecté");
      onRefresh();
    } catch (e) {
      setConnectorMsg("google-drive", `Erreur : ${e}`);
    } finally {
      setConnectorBusy("google-drive", false);
    }
  }

  return (
    <div className="flex-1 space-y-2 overflow-y-auto p-3">
      {connectors.map((c) => {
        const Icon = CONNECTOR_ICON[c.id] ?? Plug;
        const isBusy = busy[c.id] ?? false;
        const msg = msgs[c.id] ?? "";

        return (
          <div
            key={c.id}
            className="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] p-3"
          >
            <div className="flex items-center gap-2">
              <Icon className="size-4 text-[var(--color-accent)]" />
              <span className="text-sm font-medium">{c.name}</span>
              {c.connected ? (
                <span className="ml-auto flex items-center gap-1 text-[11px] text-emerald-500">
                  <Check className="size-3" /> Actif
                </span>
              ) : c.id === "claude-ai" || c.id === "google-drive" ? null : (
                <span className="ml-auto rounded-full bg-[var(--color-surface-2)] px-2 py-0.5 text-[10px] text-[var(--color-muted)]">
                  Bientôt
                </span>
              )}
            </div>

            {c.connected && (
              <div className="mt-1.5 flex items-center justify-between text-[11px] text-[var(--color-muted)]">
                <span>{c.conversation_count} éléments</span>
                <span>Sync {relativeDate(c.last_sync)}</span>
              </div>
            )}

            {/* Actions spécifiques par connecteur */}
            {c.id === "claude-ai" && (
              <ConnectorAction
                label={c.connected ? "Réimporter un ZIP" : "Importer export ZIP"}
                busy={isBusy}
                msg={msg}
                onClick={handleImportClaudeAi}
              />
            )}

            {c.id === "google-drive" && !c.connected && (
              <ConnectorAction
                label="Connecter Google Drive"
                busy={isBusy}
                msg={msg || "Ouvrira ton navigateur pour OAuth"}
                onClick={handleGoogleConnect}
              />
            )}

            {c.id === "google-drive" && c.connected && (
              <div className="flex flex-col gap-1">
                <ConnectorAction
                  label="Synchroniser les fichiers"
                  busy={isBusy}
                  msg={msg}
                  onClick={handleGoogleSync}
                />
                <button
                  onClick={handleGoogleDisconnect}
                  disabled={isBusy}
                  className="w-full rounded px-2 py-1 text-[11px] text-[var(--color-muted)] hover:text-red-400 transition-colors disabled:opacity-40"
                >
                  Déconnecter / changer de compte
                </button>
              </div>
            )}
          </div>
        );
      })}
      <p className="px-1 pt-1 text-[11px] leading-relaxed text-[var(--color-muted)]">
        Chaque source connectée alimente le même graphe : les concepts partagés
        deviennent des ponts entre tes outils.
      </p>
    </div>
  );
}

function ConnectorAction({
  label,
  busy,
  msg,
  onClick,
}: {
  label: string;
  busy: boolean;
  msg: string;
  onClick: () => void;
}) {
  return (
    <div className="mt-2">
      <button
        onClick={onClick}
        disabled={busy}
        className="flex w-full items-center justify-center gap-1.5 rounded-md border border-[var(--color-border)] px-2 py-1.5 text-[11px] text-[var(--color-muted)] transition-colors hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] disabled:opacity-50"
      >
        {busy ? <Loader2 className="size-3 animate-spin" /> : <Upload className="size-3" />}
        {label}
      </button>
      {msg && (
        <p className="mt-1.5 text-center text-[10px] text-[var(--color-muted)]">{msg}</p>
      )}
    </div>
  );
}

interface ChatMsg {
  role: "user" | "assistant";
  text: string;
}

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
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex-1 space-y-2 overflow-y-auto p-3">
        {messages.length === 0 && (
          <p className="px-1 pt-2 text-xs leading-relaxed text-[var(--color-muted)]">
            Pose une question sur ton second cerveau — l'IA locale répond à partir
            de ton <code>brain.md</code>. Ex. « Quels projets utilisent Supabase ? »
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
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send();
              }
            }}
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
