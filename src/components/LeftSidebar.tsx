import { useState } from "react";
import { MessageCircle, Send, Loader2, X } from "lucide-react";
import { askBrain, createStructure } from "@/lib/api";
import { cn } from "@/lib/utils";
import { AiStatusBar, useAiReady, AI_MISSING_HINT } from "./AiStatusBar";

// Panneaux ouverts par le dock de widgets (bord gauche). La gestion des
// connecteurs / spaces / modèle vit dans SettingsModal — ici : usage quotidien.

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
    <>
      <div className="chat-head">
        <span className="bi"><MessageCircle className="size-[15px]" /></span>
        <span className="t">Lucid IA</span>
        {onClose && (
          <button className="icon-btn" onClick={onClose} aria-label="Fermer">
            <X className="size-4" />
          </button>
        )}
      </div>

      <div className="chat-log">
        {messages.length === 0 && (
          <p className="chat-hint">
            Pose une question sur ton second cerveau — l'IA locale répond à partir
            de ton <code>brain.md</code>. Tu peux aussi lui demander de créer des
            pages : « crée une structure pour gérer un projet web ».
          </p>
        )}
        {messages.map((m, i) => (
          <div key={i} className={cn("bub", m.role === "user" ? "me" : "ai")}>{m.text}</div>
        ))}
        {loading && (
          <div className="chat-wait">
            <span className="dots"><i /><i /><i /></span> réflexion…
          </div>
        )}
      </div>

      <div className="chat-in">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
          rows={1}
          disabled={aiOk === false}
          placeholder={aiOk === false ? AI_MISSING_HINT : "Demander…"}
        />
        <button
          className="send"
          onClick={send}
          disabled={loading || !input.trim() || aiOk === false}
          title={aiOk === false ? AI_MISSING_HINT : undefined}
        >
          {loading ? <Loader2 className="size-4 animate-spin" /> : <Send className="size-4" />}
        </button>
      </div>
      <AiStatusBar input={input} />
    </>
  );
}
