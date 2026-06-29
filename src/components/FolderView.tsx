import { useMemo, useState } from "react";
import {
  ChevronRight,
  ChevronDown,
  FolderGit2,
  Lightbulb,
  MessageSquare,
  Brain,
} from "lucide-react";
import type { BrainGraph, BrainNode, SourceRef } from "@/lib/types";
import { cn } from "@/lib/utils";

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  onOpenSource: (s: SourceRef) => void;
  selectedId: string | null;
  query: string;
}

export function FolderView({
  graph,
  onSelect,
  onOpenSource,
  selectedId,
  query,
}: Props) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const q = query.trim().toLowerCase();

  const { projects, conceptsByProject } = useMemo(() => {
    const projects = graph.nodes.filter((n) => n.kind === "project");
    const conceptNode = new Map(
      graph.nodes.filter((n) => n.kind === "concept").map((n) => [n.id, n]),
    );
    const conceptsByProject = new Map<string, BrainNode[]>();
    for (const e of graph.edges) {
      if (e.kind !== "concept") continue;
      const c = conceptNode.get(e.target);
      if (!c) continue;
      const arr = conceptsByProject.get(e.source) ?? [];
      arr.push(c);
      conceptsByProject.set(e.source, arr);
    }
    return { projects, conceptsByProject };
  }, [graph]);

  function toggle(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  const projMatch = (p: BrainNode) =>
    !q ||
    [p.label, p.summary, ...p.keywords].join(" ").toLowerCase().includes(q) ||
    (conceptsByProject.get(p.id) ?? []).some((c) =>
      c.label.toLowerCase().includes(q),
    ) ||
    p.sources.some((s) => s.title.toLowerCase().includes(q));

  return (
    <div className="h-full overflow-y-auto px-6 py-5">
      <div className="mx-auto max-w-2xl">
        <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Brain className="size-4 text-[var(--color-accent)]" />
          Second Brain
          <span className="text-xs font-normal text-[var(--color-muted)]">
            {projects.length} projets
          </span>
        </div>

        <ul className="space-y-0.5">
          {projects.filter(projMatch).map((p) => {
            const open = expanded.has(p.id) || !!q;
            const concepts = conceptsByProject.get(p.id) ?? [];
            return (
              <li key={p.id}>
                <div
                  className={cn(
                    "flex items-center gap-1.5 rounded-md px-2 py-1.5",
                    selectedId === p.id
                      ? "bg-[var(--color-accent-soft)]"
                      : "hover:bg-[var(--color-surface-2)]",
                  )}
                >
                  <button onClick={() => toggle(p.id)} className="text-[var(--color-muted)]">
                    {open ? (
                      <ChevronDown className="size-4" />
                    ) : (
                      <ChevronRight className="size-4" />
                    )}
                  </button>
                  <button
                    onClick={() => onSelect(p)}
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                  >
                    <FolderGit2 className="size-4 shrink-0 text-[var(--color-accent)]" />
                    <span className="truncate text-sm font-medium">{p.label}</span>
                    <span className="ml-auto text-xs text-[var(--color-muted)]">
                      {p.sources.length} src · {concepts.length} concepts
                    </span>
                  </button>
                </div>

                {open && (
                  <ul className="ml-7 border-l border-[var(--color-border)] pl-3">
                    {p.sources.map((s, i) => (
                      <li key={`s${i}`}>
                        <button
                          onClick={() => onOpenSource(s)}
                          className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
                        >
                          <MessageSquare className="size-3.5 shrink-0" />
                          <span className="truncate">{s.title}</span>
                        </button>
                      </li>
                    ))}
                    {concepts.map((c) => (
                      <li key={c.id}>
                        <button
                          onClick={() => onSelect(c)}
                          className={cn(
                            "flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-xs hover:bg-[var(--color-surface-2)]",
                            selectedId === c.id && "bg-[var(--color-accent-soft)]",
                          )}
                        >
                          <Lightbulb className="size-3.5 shrink-0 text-[var(--color-accent)]" />
                          <span className="truncate">{c.label}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}
