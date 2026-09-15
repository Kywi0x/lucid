import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { matches, clusterColors } from "@/components/BrainMap";

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
    <div className="pal-scrim" onMouseDown={onClose}>
      <div
        className="palette lg"
        role="dialog"
        aria-modal="true"
        aria-label="Recherche"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="pal-in">
          <Search className="size-4 shrink-0" />
          <input
            autoFocus
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Aller à une page, un dossier, un space…"
            aria-label="Rechercher"
          />
        </div>

        <div className="pal-list">
          {results.length === 0 ? (
            <p className="pal-empty">Aucun résultat</p>
          ) : (
            results.map((n, i) => (
              <button
                key={n.id}
                ref={i === sel ? selRef : undefined}
                className="pal-row"
                aria-selected={i === sel || undefined}
                onClick={() => onSelect(n)}
                onMouseEnter={() => setIdx(i)}
              >
                <span className="sq" style={{ background: colors.get(n.id) ?? "var(--color-muted)" }} />
                <span className="nm">{n.label}</span>
                <span className="kind">{chipOf(n.kind)}</span>
              </button>
            ))
          )}
        </div>

        <div className="pal-foot">
          <span><span className="kbd">↑↓</span> naviguer</span>
          <span><span className="kbd">↵</span> ouvrir</span>
          <span><span className="kbd">esc</span> fermer</span>
        </div>
      </div>
    </div>
  );
}
