import { useMemo, useRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ChevronRight, FileText, Maximize2, Minimize2, Sparkles, X } from "lucide-react";
import { ICON, KIND_LABEL, ProjectTree, useAncestors } from "@/components/NodeDetail";
import type { BrainGraph, BrainNode } from "@/lib/types";

/** Réplique lecture-seule de NodeDetail : mêmes zones, mêmes classes —
 *  sans édition, IA, historique ni export. */
export function ReadOnlyDetail({ node, graph, onSelect, onClose, expanded, onExpand }: {
  node: BrainNode;
  graph: BrainGraph;
  onSelect: (n: BrainNode) => void;
  onClose: () => void;
  expanded: boolean;
  onExpand: () => void;
}) {
  const ancestors = useAncestors(node, graph);
  const Icon = ICON[node.kind as keyof typeof ICON] ?? FileText;
  const children = graph.nodes.filter((n) => n.parent_id === node.id);
  const body = node.content || node.summary || "";
  const contentColRef = useRef<HTMLDivElement>(null);

  const taskStats = useMemo(() => {
    const all = body.match(/^\s*[-*] \[[ xX]\]/gm) ?? [];
    const done = body.match(/^\s*[-*] \[[xX]\]/gm) ?? [];
    return { total: all.length, done: done.length };
  }, [body]);

  const headings = useMemo(
    () => [...body.matchAll(/^(#{1,3}) (.+)$/gm)].map((m) => ({ level: m[1].length, text: m[2].trim() })),
    [body],
  );

  function scrollToHeading(text: string) {
    const root: ParentNode = contentColRef.current ?? document;
    for (const el of root.querySelectorAll(".editor-content h1, .editor-content h2, .editor-content h3")) {
      if (el.textContent?.trim() === text) { el.scrollIntoView({ behavior: "smooth", block: "start" }); return; }
    }
  }

  const contentBlock = (
    <>
      {node.summary && node.content && (
        <div className="mb-4 flex gap-2.5 rounded-lg border-l-2 border-[var(--color-accent)]/60 bg-[var(--color-accent-soft)] px-3 py-2.5">
          <Sparkles className="mt-0.5 size-3 shrink-0 text-[var(--color-accent)]" />
          <div className="min-w-0">
            <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--color-accent)]">Synthèse IA</span>
            <p className="mt-0.5 text-sm leading-relaxed text-[var(--color-text)]">{node.summary}</p>
          </div>
        </div>
      )}
      {node.keywords.length > 0 && (
        <div className="mb-4 flex flex-wrap gap-1.5">
          {node.keywords.map((k) => (
            <span key={k} className="rounded-full border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-0.5 text-[11px] text-[var(--color-muted)]">{k}</span>
          ))}
        </div>
      )}
      <div className="editor-content prose-brain text-sm">
        <ReactMarkdown remarkPlugins={[remarkGfm]}>
          {body || "*(page sans contenu)*"}
        </ReactMarkdown>
      </div>
    </>
  );

  const childrenBlock = children.length > 0 && (
    <div className="mt-4">
      <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Pages enfants ({children.length})</p>
      <ul className="space-y-0.5">
        {children.map((child) => (
          <li key={child.id}>
            <button
              onClick={() => onSelect(child)}
              className="w-full truncate rounded-md px-2 py-1.5 text-left text-xs text-[var(--color-text)] transition-colors hover:bg-[var(--color-surface-2)] hover:text-[var(--color-accent)]"
            >
              {child.label}
            </button>
          </li>
        ))}
      </ul>
    </div>
  );

  return (
    <div className="flex h-full w-full flex-col bg-[var(--color-surface)]">
      {/* ── Header compact ── */}
      <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-2.5">
        <Icon className="size-4 shrink-0 text-[var(--color-accent)]" />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold leading-tight">{node.label}</h3>
          <p className="text-[10px] text-[var(--color-muted)]">
            {KIND_LABEL[node.kind as keyof typeof KIND_LABEL] ?? "Page"}
            {node.kind !== "root" && ` · poids ${node.weight}`}
          </p>
        </div>
        <button
          onClick={onExpand}
          title={expanded ? "Réduire" : "Agrandir"}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
        >
          {expanded ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
        </button>
        <button
          onClick={onClose}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
        >
          <X className="size-4" />
        </button>
      </div>

      {/* ── Fil d'Ariane ── */}
      {ancestors.length > 0 && (
        <div className="flex items-center gap-0.5 overflow-x-auto border-b border-[var(--color-border)] px-4 py-1.5 scrollbar-none">
          {ancestors.map((a, i) => (
            <span key={a.id} className="flex shrink-0 items-center gap-0.5">
              {i > 0 && <ChevronRight className="size-3 text-[var(--color-border)]" />}
              <button
                onClick={() => onSelect(a)}
                className="max-w-[120px] truncate text-[11px] text-[var(--color-muted)] hover:text-[var(--color-accent)] transition-colors"
              >
                {a.label}
              </button>
            </span>
          ))}
          <ChevronRight className="size-3 shrink-0 text-[var(--color-border)]" />
          <span className="shrink-0 text-[11px] font-medium text-[var(--color-text)]">{node.label}</span>
        </div>
      )}

      {/* ── Progression des tâches ── */}
      {taskStats.total > 0 && (
        <div className="flex items-center gap-2.5 border-b border-[var(--color-border)] px-4 py-1.5">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
              style={{ width: `${Math.round((taskStats.done / taskStats.total) * 100)}%` }}
            />
          </div>
          <span className="shrink-0 font-mono text-[10px] text-[var(--color-muted)]">
            {taskStats.done}/{taskStats.total} ✓
          </span>
        </div>
      )}

      {/* ── Corps ── */}
      {expanded ? (
        // Mode étendu : arborescence + sommaire à gauche, contenu au centre
        <div className="flex flex-1 overflow-hidden">
          <div className="w-80 shrink-0 space-y-5 overflow-y-auto border-r border-[var(--color-border)] px-5 py-5">
            {node.kind !== "root" && (
              <ProjectTree node={node} graph={graph} ancestors={ancestors} onSelect={onSelect} />
            )}
            {headings.length >= 3 && (
              <div>
                <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Sommaire</p>
                <ul className="space-y-0.5">
                  {headings.map((h, i) => (
                    <li key={i}>
                      <button
                        onClick={() => scrollToHeading(h.text)}
                        style={{ paddingLeft: (h.level - 1) * 10 }}
                        className="w-full truncate text-left text-xs text-[var(--color-muted)] transition-colors hover:text-[var(--color-text)]"
                      >
                        {h.text}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
          <div ref={contentColRef} className="flex-1 overflow-y-auto px-8 py-6">
            <div className="mx-auto max-w-2xl space-y-4">{contentBlock}</div>
          </div>
        </div>
      ) : (
        // Mode panneau : colonne unique, pages enfants en bas
        <div ref={contentColRef} className="flex-1 overflow-y-auto px-5 py-4">
          {contentBlock}
          {childrenBlock}
        </div>
      )}
    </div>
  );
}
