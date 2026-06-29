import { useEffect, useState } from "react";
import { X, Loader2 } from "lucide-react";
import type { Conversation, ConversationSummary } from "@/lib/types";
import { loadConversation } from "@/lib/api";
import { cn, projectName, relativeDate } from "@/lib/utils";

interface Props {
  summary: ConversationSummary;
  onClose: () => void;
}

export function ConversationDetail({ summary, onClose }: Props) {
  const [conv, setConv] = useState<Conversation | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    setConv(null);
    loadConversation(summary.project_slug, summary.id)
      .then((c) => alive && setConv(c))
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, [summary.project_slug, summary.id]);

  return (
    <div className="flex h-full w-full flex-col bg-[var(--color-surface)]">
      <div className="flex items-start gap-2 border-b border-[var(--color-border)] p-3">
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold">{summary.title}</h3>
          <p className="truncate text-xs text-[var(--color-muted)]">
            {projectName(conv?.project ?? summary.project)} ·{" "}
            {conv?.message_count ?? summary.message_count} messages ·{" "}
            {relativeDate(conv?.last_timestamp ?? summary.last_timestamp)}
          </p>
        </div>
        <button
          onClick={onClose}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
        >
          <X className="size-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        {loading && (
          <div className="flex justify-center py-8 text-[var(--color-muted)]">
            <Loader2 className="size-5 animate-spin" />
          </div>
        )}
        {conv?.messages.map((m, i) => (
          <div
            key={i}
            className={cn(
              "mb-2 rounded-lg border border-[var(--color-border)] px-3 py-2",
              m.role === "user"
                ? "bg-[var(--color-accent-soft)]"
                : "bg-[var(--color-bg)]",
            )}
          >
            <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
              {m.role === "user" ? "Toi" : "Claude"}
            </div>
            <p className="whitespace-pre-wrap break-words text-xs leading-relaxed text-[var(--color-text)]">
              {m.text.length > 1200 ? m.text.slice(0, 1200) + "…" : m.text}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}
