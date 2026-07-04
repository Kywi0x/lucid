import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { matches, clusterColors } from "@/components/BrainMap";
import { cn } from "@/lib/utils";

interface Props {
  /** Titre affiché en tête (ex. « Déplacer vers », « Rattacher à »). */
  title: string;
  /** Nœuds proposés comme destinations (déjà filtrés par l'appelant). */
  candidates: BrainNode[];
  /** Graphe complet — sert au fil d'Ariane et aux couleurs de cluster. */
  graph: BrainGraph;
  onPick: (n: BrainNode) => void;
  onClose: () => void;
}

/** Fil d'Ariane d'un nœud : « Lucid › Cours médecine › Anatomie ». */
function pathOf(n: BrainNode, byId: Map<string, BrainNode>): string {
  const parts: string[] = [];
  let cur = n.parent_id ? byId.get(n.parent_id) : undefined;
  let guard = 0;
  while (cur && guard++ < 12) {
    parts.unshift(cur.label);
    cur = cur.parent_id ? byId.get(cur.parent_id) : undefined;
  }
  return parts.join(" › ");
}

/** Sélecteur de nœud avec recherche + navigation clavier.
 *  Même langage visuel que la CommandPalette (⌘K). */
export function NodePicker({ title, candidates, graph, onPick, onClose }: Props) {
  const [query, setQuery] = useState("");
  const [idx, setIdx] = useState(0);
  const selRef = useRef<HTMLButtonElement>(null);

  const byId = useMemo(() => new Map(graph.nodes.map((n) => [n.id, n])), [graph]);
  const colors = useMemo(() => clusterColors(graph), [graph]);

  const q = query.trim().toLowerCase();
  const results = useMemo(() => {
    const list = q ? candidates.filter((n) => matches(n, q)) : candidates;
    return list.slice(0, 30);
  }, [candidates, q]);

  const sel = Math.min(idx, Math.max(0, results.length - 1));
  useEffect(() => setIdx(0), [q]);
  useEffect(() => { selRef.current?.scrollIntoView({ block: "nearest" }); }, [sel]);

  function onKeyDown(e: React.KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      setIdx((i) => Math.min(i + 1, results.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter" && results[sel]) {
      e.preventDefault();
      onPick(results[sel]);
    }
  }

  return (
    <div className="absolute inset-0 z-50 bg-black/30" onMouseDown={onClose}>
      <div
        className="panel absolute left-1/2 top-24 w-[440px] max-w-[calc(100vw-2rem)] -translate-x-1/2 overflow-hidden rounded-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 border-b border-[var(--color-border)] px-4 py-3">
          <Search className="size-4 shrink-0 text-[var(--color-muted)]" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder={`${title}…`}
            className="w-full bg-transparent text-sm outline-none placeholder:text-[var(--color-muted)]"
          />
          <kbd className="shrink-0 rounded border border-[var(--color-border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--color-muted)]">
            ESC
          </kbd>
        </div>

        <div className="max-h-72 overflow-y-auto py-1">
          {results.length === 0 ? (
            <p className="px-4 py-6 text-center text-xs text-[var(--color-muted)]">
              {candidates.length === 0 ? "Aucune destination possible" : "Aucun résultat"}
            </p>
          ) : (
            results.map((n, i) => {
              const path = pathOf(n, byId);
              return (
                <button
                  key={n.id}
                  ref={i === sel ? selRef : undefined}
                  onClick={() => onPick(n)}
                  onMouseEnter={() => setIdx(i)}
                  className={cn(
                    "flex w-full items-center gap-2.5 px-4 py-2 text-left transition-colors",
                    i === sel ? "bg-[var(--color-accent-soft)]" : "",
                  )}
                >
                  <span
                    className="size-1.5 shrink-0 rounded-full"
                    style={{ background: colors.get(n.id) ?? "var(--color-muted)" }}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm text-[var(--color-text)]">{n.label}</span>
                    {path && (
                      <span className="block truncate text-[11px] text-[var(--color-muted)]">{path}</span>
                    )}
                  </span>
                </button>
              );
            })
          )}
        </div>

        <div className="border-t border-[var(--color-border)] px-4 py-2 font-mono text-[10px] tracking-wide text-[var(--color-muted)]">
          {title} · ↑↓ naviguer · ↵ choisir
        </div>
      </div>
    </div>
  );
}
