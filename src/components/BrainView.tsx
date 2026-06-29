import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Brain, Sparkles, Loader2, AlertTriangle, RotateCw } from "lucide-react";
import type { BrainProgress } from "@/lib/api";

/** État vide : invite à générer le second cerveau. */
export function GenerateEmpty({
  error,
  onGenerate,
}: {
  error: string | null;
  onGenerate: () => void;
}) {
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 px-8 text-center">
      <Brain className="size-12 text-[var(--color-accent)]" />
      <div>
        <h2 className="text-lg font-semibold">
          Génère ta mind map intelligente
        </h2>
        <p className="mt-1 max-w-md text-sm text-[var(--color-muted)]">
          L'IA locale (Gemma 3) analyse tes conversations Claude Code et en
          extrait une carte de bulles : projets, concepts et leurs connexions.
          100% sur ta machine.
        </p>
      </div>
      {error && (
        <div className="flex items-center gap-2 rounded-md border border-red-500/40 bg-red-500/10 px-3 py-2 text-sm text-red-300">
          <AlertTriangle className="size-4 shrink-0" />
          {error}
        </div>
      )}
      <button
        onClick={onGenerate}
        className="flex items-center gap-2 rounded-lg bg-[var(--color-accent)] px-4 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90"
      >
        <Sparkles className="size-4" />
        Générer ma mind map
      </button>
    </div>
  );
}

/** Barre de progression pendant l'analyse IA. */
export function GenerateProgress({
  progress,
}: {
  progress: BrainProgress | null;
}) {
  const pct = progress
    ? Math.round((progress.current / progress.total) * 100)
    : 0;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-5 px-8 text-center">
      <Loader2 className="size-10 animate-spin text-[var(--color-accent)]" />
      <div className="w-full max-w-md">
        <div className="mb-2 flex justify-between text-sm text-[var(--color-muted)]">
          <span>Analyse locale en cours…</span>
          {progress && (
            <span>
              {progress.current}/{progress.total}
            </span>
          )}
        </div>
        <div className="h-2 overflow-hidden rounded-full bg-[var(--color-surface-2)]">
          <div
            className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
            style={{ width: `${pct}%` }}
          />
        </div>
        {progress && (
          <p className="mt-3 truncate text-xs text-[var(--color-muted)]">
            {progress.label}
          </p>
        )}
      </div>
    </div>
  );
}

/** Rendu du brain.md. */
export function MarkdownView({
  markdown,
  onRegenerate,
}: {
  markdown: string;
  onRegenerate: () => void;
}) {
  return (
    <div className="relative h-full overflow-y-auto">
      <button
        onClick={onRegenerate}
        title="Régénérer"
        className="absolute right-4 top-4 z-10 flex items-center gap-1.5 rounded-md border border-[var(--color-border)] bg-[var(--color-surface)] px-2.5 py-1.5 text-xs text-[var(--color-muted)] hover:text-[var(--color-text)]"
      >
        <RotateCw className="size-3.5" />
        Régénérer
      </button>
      <div className="prose-brain mx-auto max-w-3xl px-8 py-6">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>{markdown}</ReactMarkdown>
      </div>
    </div>
  );
}
