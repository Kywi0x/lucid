import { useState } from "react";
import { Layers, MessageCircle, Send, Loader2, X } from "lucide-react";
import { askBrain } from "@/lib/api";
import type { Space } from "@/lib/types";
import { cn } from "@/lib/utils";

// Panneaux ouverts par le dock de widgets (bord gauche). La gestion des
// connecteurs / spaces / modèle vit dans SettingsModal — ici : usage quotidien.

function PanelHeader({ icon, title, onClose }: { icon: React.ReactNode; title: string; onClose?: () => void }) {
  return (
    <div className="flex shrink-0 items-center gap-2 border-b border-[var(--color-border)] px-3.5 py-2.5">
      <span className="text-[var(--color-accent)]">{icon}</span>
      <span className="flex-1 text-xs font-semibold">{title}</span>
      {onClose && (
        <button
          onClick={onClose}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
        >
          <X className="size-4" />
        </button>
      )}
    </div>
  );
}

// ── SpacesPanel : sélection de l'espace actif (filtre du graphe) ─────────────

const LUCID_SPACE: Space = { id: "lucid", name: "Lucid", node_ids: null };

export function SpacesPanel({
  spaces,
  activeSpaceId,
  onSpaceSelect,
  onClose,
}: {
  spaces: Space[];
  activeSpaceId: string | null;
  onSpaceSelect: (id: string | null) => void;
  onClose?: () => void;
}) {
  const all = [LUCID_SPACE, ...spaces.filter((s) => s.id !== "lucid")];

  return (
    <div className="flex h-full flex-col">
      <PanelHeader icon={<Layers className="size-3.5" />} title="Spaces" onClose={onClose} />
      <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-3">
        {all.map((s) => {
          const isActive = s.id === "lucid" ? (activeSpaceId == null || activeSpaceId === "lucid") : activeSpaceId === s.id;
          return (
            <button
              key={s.id}
              onClick={() => onSpaceSelect(s.id === "lucid" ? null : s.id)}
              className={cn(
                "flex items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors",
                isActive
                  ? "border-[var(--color-accent)]/40 bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                  : "border-[var(--color-border)] bg-[var(--color-bg)] text-[var(--color-text)] hover:bg-[var(--color-surface-2)]",
              )}
            >
              <span className="flex-1 truncate text-xs font-medium">{s.name}</span>
              <span className="shrink-0 font-mono text-[9px] uppercase tracking-wider opacity-60">
                {s.node_ids ? `${s.node_ids.length}` : "tout"}
              </span>
            </button>
          );
        })}
        <p className="px-1 pt-2 text-[11px] leading-relaxed text-[var(--color-muted)]">
          Créer, renommer ou supprimer un espace : Paramètres → Spaces.
        </p>
      </div>
    </div>
  );
}

// ── AssistantPanel : chat IA locale sur le brain.md ──────────────────────────

interface ChatMsg { role: "user" | "assistant"; text: string; }

export function AssistantPanel({ onClose }: { onClose?: () => void }) {
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
    <div className="flex h-full flex-col">
      <PanelHeader icon={<MessageCircle className="size-3.5" />} title="Assistant" onClose={onClose} />
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
