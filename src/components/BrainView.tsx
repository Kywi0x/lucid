import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Sparkles, Loader2, AlertTriangle, RotateCw, Plug, Check } from "lucide-react";
import type { BrainProgress } from "@/lib/api";
import { NeuralBg, LucidOrb } from "@/components/NeuralBg";

/** Écran d'accueil (1er lancement, pas encore de cerveau) : l'orbe Lucid sur le
 *  même fond décoratif que le canvas, un seul geste ("Commencer à créer mon
 *  cerveau") qui lance le scan puis la génération ; démo et connexion manuelle
 *  restent des choix secondaires en retrait. */
export function GenerateEmpty({
  error,
  onStartScan,
  onExploreDemo,
  onOpenSettings,
}: {
  error: string | null;
  /** Bouton principal : scan des dossiers locaux puis génération automatique. */
  onStartScan: () => void;
  /** Contenu d'exemple explorable sans rien connecter. */
  onExploreDemo: () => void;
  /** Ouvre les Réglages (Google Drive, import manuel…). */
  onOpenSettings: () => void;
}) {
  return (
    <div className="relative flex h-full flex-col items-center justify-center gap-4 px-8 text-center">
      <NeuralBg />
      <div className="lucid-rise relative flex flex-col items-center gap-4">
        <LucidOrb size={56} />
        <div>
          <h2 className="text-lg font-semibold">Bienvenue dans Lucid</h2>
          <p className="mt-1 max-w-md text-sm text-[var(--color-muted)]">
            Lucid scanne ton Bureau, tes Documents et tes Téléchargements (PDF,
            Word, PowerPoint, Excel, CSV…) et en dessine ta mind map, 100 % sur
            ta machine — rien ne part en ligne.
          </p>
        </div>
        {error && (
          <div className="flex items-center gap-2 rounded-md border border-[var(--color-err)]/40 bg-[var(--color-err)]/10 px-3 py-2 text-sm text-[var(--color-err)]">
            <AlertTriangle className="size-4 shrink-0" />
            {error}
          </div>
        )}
        <div className="flex flex-col items-center gap-2">
          <button
            onClick={onStartScan}
            className="flex items-center gap-2 rounded-lg bg-[var(--color-accent)] px-5 py-2.5 text-sm font-medium text-white transition-opacity hover:opacity-90"
          >
            <Sparkles className="size-4" />
            Commencer à créer mon cerveau
          </button>
          <button onClick={onExploreDemo} className="text-xs text-[var(--color-accent)] hover:underline">
            Explorer une démo (sans scanner)
          </button>
        </div>
      </div>
      <button
        onClick={onOpenSettings}
        className="relative flex items-center gap-1.5 text-xs text-[var(--color-muted)] transition-colors hover:text-[var(--color-text)]"
      >
        <Plug className="size-3.5" />
        Ou connecte Google Drive / importe un fichier manuellement
      </button>
    </div>
  );
}

/** Liste des auto-détections confirmées pendant le scan (Obsidian, Notes Apple,
 *  Claude Desktop…) — affichée sous la barre de progression du scan/génération. */
export function ScanSteps({ steps }: { steps: string[] }) {
  if (steps.length === 0) return null;
  return (
    <ul className="mt-3 flex flex-col gap-1 text-left">
      {steps.map((s, i) => (
        <li key={i} className="flex items-center gap-1.5 text-xs text-[var(--color-ok)]">
          <Check className="size-3 shrink-0" />
          <span className="truncate text-[var(--color-muted)]">{s}</span>
        </li>
      ))}
    </ul>
  );
}

/** Barre de progression pendant l'analyse IA (réutilisée pour le scan machine
 *  avant la génération — même composant, `label` différencie le contexte). */
export function GenerateProgress({
  progress,
  label = "Analyse locale en cours…",
}: {
  progress: BrainProgress | null;
  label?: string;
}) {
  const pct = progress
    ? Math.round((progress.current / Math.max(progress.total, 1)) * 100)
    : 0;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-5 px-8 text-center">
      <Loader2 className="size-10 animate-spin text-[var(--color-accent)]" />
      <div className="w-full max-w-md">
        <div className="mb-2 flex justify-between text-sm text-[var(--color-muted)]">
          <span>{label}</span>
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
