import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { AlertTriangle, RotateCw, Plug } from "lucide-react";
import { BrainMap } from "@/components/BrainMap";
import { EMPTY_GRAPH } from "@/lib/types";

/** Écran d'accueil (1er lancement, pas encore de cerveau) : le VRAI canvas
 *  (BrainMap, zoom/pan réels), vide — l'orbe est le root, elle porte le clic
 *  de démarrage. Même composant, même orbe, que pendant le scan/la génération
 *  qui suivent : une seule expérience continue, pas un écran puis un autre. */
export function GenerateEmpty({
  error,
  onStartScan,
  onExploreDemo,
  onOpenSettings,
}: {
  error: string | null;
  /** Un seul geste : scan des dossiers locaux puis génération automatique. */
  onStartScan: () => void;
  /** Contenu d'exemple explorable sans rien connecter — retrait, discret. */
  onExploreDemo: () => void;
  /** Ouvre les Réglages (Google Drive, import manuel…) — retrait, discret. */
  onOpenSettings: () => void;
}) {
  return (
    <div className="relative h-full">
      <BrainMap
        graph={EMPTY_GRAPH}
        onSelect={() => {}}
        selectedId={null}
        query=""
        onOrbClick={onStartScan}
        caption={error ? null : "Clique sur moi pour commencer"}
      />
      {error && (
        <div className="pointer-events-none absolute inset-x-0 top-1/2 z-10 flex translate-y-16 justify-center px-8">
          <div className="flex items-center gap-2 rounded-md border border-[var(--color-err)]/40 bg-[var(--color-err)]/10 px-3 py-2 text-sm text-[var(--color-err)]">
            <AlertTriangle className="size-4 shrink-0" />
            {error}
          </div>
        </div>
      )}
      <div className="pointer-events-none absolute inset-x-0 bottom-6 z-10 flex flex-col items-center gap-2">
        <button onClick={onExploreDemo} className="pointer-events-auto text-xs text-[var(--color-muted)]/70 hover:text-[var(--color-text)] hover:underline">
          Explorer une démo
        </button>
        <button
          onClick={onOpenSettings}
          className="pointer-events-auto flex items-center gap-1.5 text-xs text-[var(--color-muted)]/60 transition-colors hover:text-[var(--color-text)]"
        >
          <Plug className="size-3.5" />
          Ou connecte Google Drive / importe un fichier manuellement
        </button>
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
