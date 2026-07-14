import { useState } from "react";
import { Layers, MessageCircle, Send, Loader2, X, Plus, Share2 } from "lucide-react";
import { askBrain, createStructure } from "@/lib/api";
import type { Space } from "@/lib/types";
import { cn } from "@/lib/utils";
import { AiStatusBar, useAiReady, AI_MISSING_HINT } from "./AiStatusBar";

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
  onSpaceCreate,
  onSpaceShare,
  onClose,
}: {
  spaces: Space[];
  activeSpaceId: string | null;
  onSpaceSelect: (id: string | null) => void;
  onSpaceCreate: (name: string) => Promise<void>;
  /** Ouvre la modale de partage du space (public / privé sur invitation). */
  onSpaceShare?: (space: Space) => void;
  onClose?: () => void;
}) {
  const all = [LUCID_SPACE, ...spaces.filter((s) => s.id !== "lucid")];
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");

  async function submitCreate() {
    const name = newName.trim();
    if (!name) { setCreating(false); return; }
    await onSpaceCreate(name);
    setNewName("");
    setCreating(false);
  }

  return (
    <div className="flex h-full flex-col">
      <PanelHeader icon={<Layers className="size-3.5" />} title="Spaces" onClose={onClose} />
      <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-3">
        {all.map((s) => {
          const isActive = s.id === "lucid" ? (activeSpaceId == null || activeSpaceId === "lucid") : activeSpaceId === s.id;
          const shareable = s.id !== "lucid" && !!onSpaceShare;
          return (
            <div
              key={s.id}
              onClick={() => onSpaceSelect(s.id === "lucid" ? null : s.id)}
              className={cn(
                "group flex cursor-pointer items-center gap-2 rounded-lg border px-2.5 py-2 text-left transition-colors",
                isActive
                  ? "border-[var(--color-accent)]/40 bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                  : "border-[var(--color-border)] bg-[var(--color-bg)] text-[var(--color-text)] hover:bg-[var(--color-surface-2)]",
              )}
            >
              <span className="flex-1 truncate text-xs font-medium">{s.name}</span>
              {shareable && (
                <button
                  onClick={(e) => { e.stopPropagation(); onSpaceShare!(s); }}
                  title="Partager ce space"
                  className="shrink-0 rounded-md p-1 text-[var(--color-muted)] opacity-0 transition-all hover:text-[var(--color-accent)] group-hover:opacity-100"
                >
                  <Share2 className="size-3.5" />
                </button>
              )}
              <span className="shrink-0 font-mono text-[9px] uppercase tracking-wider opacity-60">
                {s.node_ids ? `${s.node_ids.length}` : "tout"}
              </span>
            </div>
          );
        })}
        {creating ? (
          <input
            autoFocus
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitCreate();
              if (e.key === "Escape") { setNewName(""); setCreating(false); }
            }}
            onBlur={submitCreate}
            placeholder="Nom du space…"
            className="rounded-lg border border-[var(--color-accent)]/50 bg-[var(--color-bg)] px-2.5 py-2 text-xs text-[var(--color-text)] outline-none"
          />
        ) : (
          <button
            onClick={() => setCreating(true)}
            className="flex items-center gap-2 rounded-lg border border-dashed border-[var(--color-border)] px-2.5 py-2 text-left text-xs text-[var(--color-muted)] transition-colors hover:border-[var(--color-accent)]/40 hover:text-[var(--color-accent)]"
          >
            <Plus className="size-3.5" />
            Nouveau space
          </button>
        )}
        <p className="px-1 pt-2 text-[11px] leading-relaxed text-[var(--color-muted)]">
          Renommer ou supprimer : Paramètres → Spaces.
        </p>
      </div>
    </div>
  );
}

// ── AssistantPanel : chat IA locale sur le brain.md ──────────────────────────

interface ChatMsg { role: "user" | "assistant"; text: string; }

// ponytail: détection d'intention par regex, un classifieur LLM si trop de faux positifs
const CREATE_INTENT = /\b(cr[ée]{2}r?|g[ée]n[èe]re|ajoute|construis|fais(?:-moi)?)\b.*\b(structure|arborescence|pages?|nœuds?|noeuds?)\b/i;

export function AssistantPanel({ onClose, onGraphChange, activeSpaceId }: { onClose?: () => void; onGraphChange?: () => void; activeSpaceId?: string | null }) {
  const [messages, setMessages] = useState<ChatMsg[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const aiOk = useAiReady();

  async function send() {
    const q = input.trim();
    if (!q || loading) return;
    setInput("");
    setMessages((m) => [...m, { role: "user", text: q }]);
    setLoading(true);
    try {
      if (CREATE_INTENT.test(q)) {
        const [label, count] = await createStructure(q, undefined, activeSpaceId);
        setMessages((m) => [...m, { role: "assistant", text: `✅ Structure « ${label} » créée (${count} page${count > 1 ? "s" : ""}).` }]);
        onGraphChange?.();
      } else {
        const answer = await askBrain(q);
        setMessages((m) => [...m, { role: "assistant", text: answer }]);
      }
    } catch (e) {
      setMessages((m) => [...m, { role: "assistant", text: `⚠️ ${e}` }]);
    } finally { setLoading(false); }
  }

  return (
    <div className="flex h-full flex-col">
      <PanelHeader icon={<MessageCircle className="size-3.5" />} title="Lucid IA" onClose={onClose} />
      <div className="flex-1 space-y-2 overflow-y-auto p-3">
        {messages.length === 0 && (
          <p className="px-1 pt-2 text-xs leading-relaxed text-[var(--color-muted)]">
            Pose une question sur ton second cerveau — l'IA locale répond à partir
            de ton <code>brain.md</code>. Tu peux aussi lui demander de créer des
            pages : « crée une structure pour gérer un projet web ».
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
            disabled={aiOk === false}
            placeholder={aiOk === false ? AI_MISSING_HINT : "Demander…"}
            className="max-h-24 min-h-9 flex-1 resize-none rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-2 text-xs outline-none placeholder:text-[var(--color-muted)] focus:border-[var(--color-accent)] disabled:opacity-50"
          />
          <button
            onClick={send}
            disabled={loading || !input.trim() || aiOk === false}
            title={aiOk === false ? AI_MISSING_HINT : undefined}
            className="flex size-9 items-center justify-center rounded-md bg-[var(--color-accent)] text-white disabled:opacity-50"
          >
            <Send className="size-4" />
          </button>
        </div>
        <AiStatusBar input={input} />
      </div>
    </div>
  );
}
