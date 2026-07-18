import { useMemo, useState } from "react";
import {
  ChevronRight, ChevronDown, FolderGit2, FileText, Brain,
} from "lucide-react";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { cn } from "@/lib/utils";

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
}

export function FolderView({ graph, onSelect, selectedId, query }: Props) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const q = query.trim().toLowerCase();

  // Build children map from parent_id
  const childrenOf = useMemo(() => {
    const map = new Map<string, BrainNode[]>();
    for (const node of graph.nodes) {
      if (!node.parent_id) continue;
      if (!map.has(node.parent_id)) map.set(node.parent_id, []);
      map.get(node.parent_id)!.push(node);
    }
    return map;
  }, [graph]);

  const rootNode = graph.nodes.find((n) => n.kind === "root");
  const topLevel = childrenOf.get(rootNode?.id ?? "root") ?? [];

  function toggle(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  function nodeMatches(node: BrainNode): boolean {
    if (!q) return true;
    if ([node.label, node.summary, ...node.keywords].join(" ").toLowerCase().includes(q)) return true;
    // Check descendants recursively
    for (const child of childrenOf.get(node.id) ?? []) {
      if (nodeMatches(child)) return true;
    }
    return false;
  }

  function renderNode(node: BrainNode, depth: number): React.ReactNode {
    if (!nodeMatches(node)) return null;
    const children = childrenOf.get(node.id) ?? [];
    const hasChildren = children.length > 0;
    const isOpen = expanded.has(node.id) || !!q;
    const isLeaf = !hasChildren;

    return (
      <li key={node.id}>
        <div
          className={cn(
            "flex items-center gap-1.5 rounded-md px-2 py-1.5",
            selectedId === node.id
              ? "bg-[var(--color-accent-soft)]"
              : "hover:bg-[var(--color-surface-2)]",
          )}
          style={{ paddingLeft: `${0.5 + depth * 1.25}rem` }}
        >
          {hasChildren ? (
            <button onClick={() => toggle(node.id)} className="text-[var(--color-muted)]">
              {isOpen
                ? <ChevronDown className="size-4" />
                : <ChevronRight className="size-4" />}
            </button>
          ) : (
            <span className="size-4 shrink-0" />
          )}
          <button
            onClick={() => onSelect(node)}
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
          >
            {isLeaf
              ? <FileText className="size-4 shrink-0 text-[var(--color-muted)]" />
              : <FolderGit2 className="size-4 shrink-0 text-[var(--color-accent)]" />}
            <span className="truncate text-sm font-medium">{node.label}</span>
            {!isLeaf && (
              <span className="ml-auto text-xs text-[var(--color-muted)]">
                {children.length}
              </span>
            )}
          </button>
        </div>

        {isOpen && hasChildren && (
          <ul>
            {children.map((child) => renderNode(child, depth + 1))}
          </ul>
        )}
      </li>
    );
  }

  const totalLeaves = graph.nodes.filter((n) => !childrenOf.has(n.id) && n.kind !== "root").length;

  return (
    <div className="h-full overflow-y-auto px-6 py-5">
      <div className="mx-auto max-w-2xl">
        <div className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <Brain className="size-4 text-[var(--color-accent)]" />
          Lucid
          <span className="text-xs font-normal text-[var(--color-muted)]">
            {topLevel.length} espaces · {totalLeaves} documents
          </span>
        </div>
        <ul className="space-y-0.5">
          {topLevel.filter(nodeMatches).map((node) => renderNode(node, 0))}
        </ul>
      </div>
    </div>
  );
}
