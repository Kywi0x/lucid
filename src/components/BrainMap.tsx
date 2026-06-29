import { useMemo } from "react";
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  Handle,
  Position,
  useViewport,
  type Node,
  type Edge,
  type NodeProps,
} from "@xyflow/react";
import {
  forceSimulation,
  forceLink,
  forceManyBody,
  forceCenter,
  forceCollide,
  type SimulationNodeDatum,
} from "d3-force";
import type { BrainGraph, BrainNode } from "@/lib/types";

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
}

interface SimNode extends SimulationNodeDatum {
  id: string;
  r: number;
  node: BrainNode;
}

/** Palette de catégories (lisible en clair comme en sombre). */
const PALETTE = [
  "#7c5cff", "#1f9d72", "#e0823d", "#d6457f", "#3a8ef0",
  "#caa23a", "#9b59b6", "#2bb3a3", "#e0594b", "#5d6cf0",
];
const BRIDGE = "#7c5cff";

const CAT_LABEL: Record<string, string> = {
  root: "Cerveau",
  project: "Projet",
  concept: "Concept",
};

function radiusOf(n: BrainNode): number {
  if (n.kind === "root") return 50;
  if (n.kind === "project") return 24 + Math.sqrt(n.weight) * 7;
  return 13 + Math.sqrt(n.weight) * 5;
}

function matches(n: BrainNode, q: string): boolean {
  if (!q) return true;
  const hay = [n.label, n.summary, ...n.keywords].join(" ").toLowerCase();
  return hay.includes(q);
}

function layout(graph: BrainGraph): {
  nodes: Node[];
  edges: Edge[];
  colorOf: Map<string, string>;
} {
  // Couleur par projet, héritée par ses concepts (concept-pont = violet).
  const projColor = new Map<string, string>();
  let pi = 0;
  for (const n of graph.nodes) {
    if (n.kind === "project") projColor.set(n.id, PALETTE[pi++ % PALETTE.length]);
  }
  const conceptProjects = new Map<string, string[]>();
  for (const e of graph.edges) {
    if (e.kind === "concept") {
      const arr = conceptProjects.get(e.target) ?? [];
      arr.push(e.source);
      conceptProjects.set(e.target, arr);
    }
  }
  const colorOf = new Map<string, string>();
  for (const n of graph.nodes) {
    if (n.kind === "root") colorOf.set(n.id, "#7c5cff");
    else if (n.kind === "project") colorOf.set(n.id, projColor.get(n.id) ?? BRIDGE);
    else {
      const ps = conceptProjects.get(n.id) ?? [];
      colorOf.set(n.id, ps.length === 1 ? projColor.get(ps[0]) ?? BRIDGE : BRIDGE);
    }
  }

  const sim: SimNode[] = graph.nodes.map((n) => ({
    id: n.id,
    r: radiusOf(n),
    node: n,
  }));
  const byId = new Map(sim.map((s) => [s.id, s]));
  type SimLink = { source: string | SimNode; target: string | SimNode };
  const links: SimLink[] = graph.edges
    .filter((e) => byId.has(e.source) && byId.has(e.target))
    .map((e) => ({ source: e.source, target: e.target }));

  forceSimulation(sim)
    .force(
      "link",
      forceLink<SimNode, SimLink>(links)
        .id((d) => d.id)
        .distance((l) => ((l.target as SimNode).node.kind === "project" ? 200 : 95))
        .strength(0.5),
    )
    .force("charge", forceManyBody().strength(-340))
    .force("center", forceCenter(0, 0))
    .force(
      "collide",
      forceCollide<SimNode>().radius((d) => d.r + 12),
    )
    .stop()
    .tick(420);

  const nodes: Node[] = sim.map((s, i) => ({
    id: s.id,
    type: "bubble",
    position: { x: (s.x ?? 0) - s.r, y: (s.y ?? 0) - s.r },
    data: { node: s.node, r: s.r, color: colorOf.get(s.id) ?? BRIDGE, index: i },
    draggable: true,
  }));

  const edges: Edge[] = graph.edges.map((e, i) => ({
    id: `e${i}`,
    source: e.source,
    target: e.target,
    type: "straight",
    style: {
      stroke: "var(--color-border)",
      strokeWidth: e.kind === "project" ? 1.6 : 1,
    },
  }));

  return { nodes, edges, colorOf };
}

function hexWithAlpha(hex: string, alpha: number): string {
  const a = Math.round(alpha * 255).toString(16).padStart(2, "0");
  return `${hex}${a}`;
}

// Seuils de zoom : 0.35 → label, 0.65 → catégorie, 1.1 → résumé court.
function BubbleNode({ data, selected }: NodeProps) {
  const { zoom } = useViewport();
  const node = data.node as BrainNode;
  const r = data.r as number;
  const color = data.color as string;
  const dim = data.dim as boolean;
  const index = (data.index as number) ?? 0;
  const size = r * 2;
  const solid = node.kind !== "concept";

  const showLabel = zoom >= 0.35;
  const showCategory = zoom >= 0.65;
  const showSummary = zoom >= 1.1 && !!node.summary;
  const fontSize = node.kind === "root" ? 13 : node.kind === "project" ? 11 : 9;

  return (
    <div
      className="bubble"
      style={{
        width: size,
        height: size,
        position: "relative",
        background: solid ? color : hexWithAlpha(color, 0.18),
        color: solid ? "#fff" : "var(--color-text)",
        borderRadius: "50%",
        border: `2px solid ${selected ? "var(--color-text)" : color}`,
        boxShadow: selected ? "0 0 0 3px var(--color-accent-soft)" : "var(--shadow-float)",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        textAlign: "center",
        padding: 6,
        fontSize,
        fontWeight: node.kind === "concept" ? 500 : 700,
        lineHeight: 1.15,
        cursor: "pointer",
        opacity: dim ? 0.18 : 1,
        overflow: "hidden",
        animation: "bubblePop 0.35s cubic-bezier(0.34,1.56,0.64,1) both",
        animationDelay: `${Math.min(index * 18, 400)}ms`,
      }}
    >
      <Handle type="target" position={Position.Top} style={HANDLE} />
      {showLabel && (
        <span style={{ pointerEvents: "none", wordBreak: "break-word" }}>
          {node.label}
        </span>
      )}
      {showCategory && (
        <span className="bubble__cat">{CAT_LABEL[node.kind] ?? "Nœud"}</span>
      )}
      {showSummary && (
        <span style={{
          pointerEvents: "none",
          fontSize: fontSize - 1,
          opacity: 0.75,
          fontWeight: 400,
          marginTop: 3,
          lineHeight: 1.2,
          display: "-webkit-box",
          WebkitLineClamp: 2,
          WebkitBoxOrient: "vertical",
          overflow: "hidden",
        }}>
          {node.summary}
        </span>
      )}
      <Handle type="source" position={Position.Bottom} style={HANDLE} />
    </div>
  );
}

const HANDLE: React.CSSProperties = {
  opacity: 0,
  top: "50%",
  left: "50%",
  width: 1,
  height: 1,
  border: "none",
  background: "transparent",
};

const nodeTypes = { bubble: BubbleNode };

export function BrainMap({ graph, onSelect, selectedId, query }: Props) {
  const base = useMemo(() => layout(graph), [graph]);
  const q = query.trim().toLowerCase();

  const decorated = useMemo(
    () =>
      base.nodes.map((n) => ({
        ...n,
        selected: n.id === selectedId,
        data: {
          ...n.data,
          dim: !matches((n.data as { node: BrainNode }).node, q),
        },
      })),
    [base.nodes, selectedId, q],
  );

  return (
    <ReactFlow
      nodes={decorated}
      edges={base.edges}
      nodeTypes={nodeTypes}
      fitView
      minZoom={0.1}
      maxZoom={2.5}
      nodesConnectable={false}
      proOptions={{ hideAttribution: true }}
      onNodeClick={(_, n) => onSelect((n.data as unknown as { node: BrainNode }).node)}
    >
      <Background variant={BackgroundVariant.Dots} gap={22} size={1.5} color="var(--color-border)" />
      <Controls showInteractive={false} />
    </ReactFlow>
  );
}
