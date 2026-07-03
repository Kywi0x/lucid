import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { matches, clusterColors } from "@/components/BrainMap";
import { cn } from "@/lib/utils";

interface Props {
  graph: BrainGraph;
  query: string;
  onQueryChange: (q: string) => void;
  onSelect: (n: BrainNode) => void;
  onClose: () => void;
}

const KIND_CHIP: Record<string, string> = {
  root: "Lucid", group: "Espace", espace: "Espace", container: "Espace", note: "Note",
};
const chipOf = (k: string) => KIND_CHIP[k] ?? "Page";

/** Palette de commande ⌘K : recherche + navigation clavier dans le graphe. */
export function CommandPalette({ graph, query, onQueryChange, onSelect, onClose }: Props) {
  const [idx, setIdx] = useState(0);
  const selRef = useRef<HTMLButtonElement>(null);

  const colors = useMemo(() => clusterColors(graph), [graph]);
  const q = query.trim().toLowerCase();
  const results = useMemo(() => {
    const nodes = q
      ? graph.nodes.filter((n) => matches(n, q))
      : graph.nodes.filter((n) => n.kind === "root" || n.kind === "group");
    return nodes.slice(0, 30);
  }, [graph, q]);

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
      onSelect(results[sel]);
    }
  }

  return (
    <div className="absolute inset-0 z-50 bg-black/30" onMouseDown={onClose}>
      <div
        className="panel absolute left-1/2 top-20 w-[520px] max-w-[calc(100vw-2rem)] -translate-x-1/2 overflow-hidden rounded-2xl"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2.5 border-b border-[var(--color-border)] px-4 py-3">
          <Search className="size-4 shrink-0 text-[var(--color-muted)]" />
          <input
            autoFocus
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Rechercher un nœud…"
            className="w-full bg-transparent text-sm outline-none placeholder:text-[var(--color-muted)]"
          />
          <kbd className="shrink-0 rounded border border-[var(--color-border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--color-muted)]">
            ESC
          </kbd>
        </div>

        <div className="max-h-80 overflow-y-auto py-1">
          {results.length === 0 ? (
            <p className="px-4 py-6 text-center text-xs text-[var(--color-muted)]">Aucun résultat</p>
          ) : (
            results.map((n, i) => (
              <button
                key={n.id}
                ref={i === sel ? selRef : undefined}
                onClick={() => onSelect(n)}
                onMouseEnter={() => setIdx(i)}
                className={cn(
                  "flex w-full items-center gap-2.5 px-4 py-2 text-left text-sm transition-colors",
                  i === sel ? "bg-[var(--color-accent-soft)]" : "",
                )}
              >
                <span
                  className="size-1.5 shrink-0 rounded-full"
                  style={{ background: colors.get(n.id) ?? "var(--color-muted)" }}
                />
                <span className="flex-1 truncate text-[var(--color-text)]">{n.label}</span>
                <span className="shrink-0 rounded border border-[var(--color-border)] px-1.5 py-0.5 font-mono text-[9px] uppercase tracking-wider text-[var(--color-muted)]">
                  {chipOf(n.kind)}
                </span>
              </button>
            ))
          )}
        </div>

        <div className="border-t border-[var(--color-border)] px-4 py-2 font-mono text-[10px] tracking-wide text-[var(--color-muted)]">
          ↑↓ naviguer · ↵ ouvrir et centrer
        </div>
      </div>
    </div>
  );
}
