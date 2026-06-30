import {
  useMemo, useState, useEffect, useRef, useCallback,
} from "react";
import {
  ReactFlow,
  Background, BackgroundVariant,
  Handle, Position,
  useNodesState,
  useViewport,
  useReactFlow,
  type Node, type Edge, type NodeProps,
} from "@xyflow/react";
import type { BrainGraph, BrainNode } from "@/lib/types";

// ─── Types ────────────────────────────────────────────────────────────────────

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
  revealKey?: number;
  streamLabels?: string[];
  streamTotal?: number;
}

interface NodeInfo {
  id: string;
  node: BrainNode;
  r: number;
  finalX: number;
  finalY: number;
  birthX: number;
  birthY: number;
  birthDelay: number;
  parentId: string | null;
}

// ─── Palette (genesis uniquement) ─────────────────────────────────────────────

const PALETTE = [
  "#7b6fe0", "#3ea882", "#c97c4a", "#c95c88",
  "#4a8dcc", "#b08a28", "#8752ba", "#38a09a",
  "#b84c44", "#5060c0",
];

function radiusOf(n: BrainNode): number {
  if (n.kind === "root")                             return 44;
  if (n.kind === "group")                            return 26 + Math.sqrt(n.weight) * 3;
  if (n.kind === "espace" || n.kind === "container") return 14 + Math.sqrt(n.weight) * 3.5;
  return 9; // leaf, page, concept, source
}

function isLeafKind(kind: string): boolean {
  return kind === "leaf" || kind === "page" || kind === "concept" || kind === "source";
}

function matches(n: BrainNode, q: string): boolean {
  return [n.label, n.summary, ...n.keywords].join(" ").toLowerCase().includes(q);
}

function orbitR(N: number, maxR: number, gap: number, minR: number): number {
  if (N <= 1) return minR;
  return Math.max(minR, (maxR + gap) / Math.sin(Math.PI / N));
}

function arcOrbitR(N: number, rs: number, gap: number, spread: number, minR: number): number {
  if (N <= 1) return minR;
  const angularStep = spread / (N - 1);
  const minByArc = (rs + gap / 2) / Math.sin(angularStep / 2);
  return Math.max(minR, minByArc);
}

function stableHash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0) / 0xffffffff;
}

function jitter(id: string, axis: string, scale: number): number {
  return (stableHash(id + axis) * 2 - 1) * scale;
}

function arcSpread(N: number): number {
  if (N <= 1) return 0;
  return Math.min(Math.PI * 0.83, Math.PI / 3 + N * 0.2);
}

// ─── Layout radial récursif (parent_id) ───────────────────────────────────────

function buildLayout(
  graph: BrainGraph,
): { infos: Map<string, NodeInfo>; childrenOf: Map<string, string[]> } {
  const infos = new Map<string, NodeInfo>();

  // Index children par parent_id
  const childrenByParent = new Map<string, BrainNode[]>();
  for (const node of graph.nodes) {
    if (!node.parent_id) continue;
    if (!childrenByParent.has(node.parent_id)) childrenByParent.set(node.parent_id, []);
    childrenByParent.get(node.parent_id)!.push(node);
  }

  const rootNode = graph.nodes.find((n) => n.kind === "root");
  const rootId = rootNode?.id ?? "root";
  if (rootNode) {
    infos.set(rootId, {
      id: rootId, node: rootNode, r: 44,
      finalX: 0, finalY: 0, birthX: 0, birthY: 0, birthDelay: 0, parentId: null,
    });
  }

  function placeLevel(
    parentId: string, px: number, py: number,
    gpx: number, gpy: number,
    parentDelay: number, depth: number,
  ) {
    const kids = childrenByParent.get(parentId) ?? [];
    if (!kids.length) return;
    const N = kids.length;
    const maxR = kids.reduce((m, k) => Math.max(m, radiusOf(k)), 9);
    const isFirst = depth === 1;
    const spread = isFirst ? Math.PI * 2 : arcSpread(N);
    const R = isFirst
      ? orbitR(N, maxR, 50, 160)
      : arcOrbitR(N, maxR, 18, spread, Math.max(28, 68 - depth * 8));
    const outAngle = Math.atan2(py - gpy, px - gpx);

    kids.forEach((kid, ki) => {
      const t = N <= 1 ? 0.5 : ki / (N - 1);
      const baseAngle = isFirst
        ? (ki / N) * Math.PI * 2 - Math.PI / 2
        : outAngle - spread / 2 + t * spread;
      const jAng = jitter(kid.id, "ang", 0.18 / Math.max(N, 3));
      const jRad = jitter(kid.id, "rad", R * 0.12);
      const kx = px + (R + jRad) * Math.cos(baseAngle + jAng);
      const ky = py + (R + jRad) * Math.sin(baseAngle + jAng);
      const kidDelay = parentDelay + (ki + 1) * (isLeafKind(kid.kind) ? 60 : 200);

      infos.set(kid.id, {
        id: kid.id, node: kid, r: radiusOf(kid),
        finalX: kx, finalY: ky, birthX: px, birthY: py,
        birthDelay: kidDelay, parentId: parentId,
      });

      placeLevel(kid.id, kx, ky, px, py, kidDelay, depth + 1);
    });
  }

  placeLevel(rootId, 0, 0, 0, 100, 0, 1);

  const childrenOf = new Map<string, string[]>();
  for (const info of infos.values()) {
    if (!childrenOf.has(info.id)) childrenOf.set(info.id, []);
    if (info.parentId) {
      if (!childrenOf.has(info.parentId)) childrenOf.set(info.parentId, []);
      childrenOf.get(info.parentId)!.push(info.id);
    }
  }

  return { infos, childrenOf };
}

function computePushOffsets(
  infos: Map<string, NodeInfo>,
  focusedId: string,
  dragOffsets: Map<string, { dx: number; dy: number }>,
): Map<string, { dx: number; dy: number }> {
  const focused = infos.get(focusedId);
  if (!focused) return new Map();
  const fx = focused.finalX + (dragOffsets.get(focusedId)?.dx ?? 0);
  const fy = focused.finalY + (dragOffsets.get(focusedId)?.dy ?? 0);
  const PUSH = 230;
  const result = new Map<string, { dx: number; dy: number }>();
  for (const info of infos.values()) {
    if (info.id === focusedId || info.node.kind === "root") continue;
    if (info.parentId === focusedId) continue;
    const isLeaf = isLeafKind(info.node.kind);
    const isContainer = !isLeaf;
    if (!isContainer && !isLeaf) continue;
    if (isLeaf) {
      const par = infos.get(info.parentId ?? "");
      if (par?.parentId === focusedId) continue;
    }
    const refId = isLeaf ? (info.parentId ?? info.id) : info.id;
    const ref = infos.get(refId)!;
    const rx = ref.finalX + (dragOffsets.get(refId)?.dx ?? 0);
    const ry = ref.finalY + (dragOffsets.get(refId)?.dy ?? 0);
    const ddx = rx - fx, ddy = ry - fy;
    const dist = Math.sqrt(ddx * ddx + ddy * ddy) || 1;
    result.set(info.id, { dx: (ddx / dist) * PUSH, dy: (ddy / dist) * PUSH });
  }
  return result;
}

function getDescendants(id: string, childrenOf: Map<string, string[]>): string[] {
  const out: string[] = [];
  const q = [id];
  while (q.length) {
    for (const c of childrenOf.get(q.pop()!) ?? []) { out.push(c); q.push(c); }
  }
  return out;
}

// ─── Construction des nœuds RF ────────────────────────────────────────────────

function makeNodes(
  infos: Map<string, NodeInfo>,
  dragOffsets: Map<string, { dx: number; dy: number }>,
  revealKey: number,
  showLeaves: boolean,
  selectedId: string | null,
  q: string,
  focusedId: string | null,
): Node[] {
  const pushOffsets = focusedId
    ? computePushOffsets(infos, focusedId, dragOffsets)
    : null;

  return Array.from(infos.values())
    .filter((info) => {
      if (isLeafKind(info.node.kind)) return showLeaves;
      return true;
    })
    .map((info, i) => {
      const off   = dragOffsets.get(info.id) ?? { dx: 0, dy: 0 };
      const push  = pushOffsets?.get(info.id) ?? { dx: 0, dy: 0 };
      const moved = off.dx !== 0 || off.dy !== 0;
      const isLeaf = isLeafKind(info.node.kind);
      const isParentFocused = isLeaf && info.parentId === focusedId;
      return {
        id: info.id,
        type: "bubble",
        position: {
          x: info.finalX - info.r + off.dx + push.dx,
          y: info.finalY - info.r + off.dy + push.dy,
        },
        selected: info.id === selectedId,
        draggable: !isLeaf,
        data: {
          node: info.node, r: info.r, idx: i,
          bx: moved ? 0 : info.birthX - info.finalX,
          by: moved ? 0 : info.birthY - info.finalY,
          delay: moved ? 0 : info.birthDelay,
          revealKey, isLeaf, isParentFocused,
          parentId: info.parentId,
          dim: q ? !matches(info.node, q) : false,
        },
      };
    });
}

function makeGenesisNodes(labels: string[], total: number, revealKey: number): Node[] {
  const G = Math.max(total, labels.length, 1);
  const Rg = orbitR(G, 26, 50, 220);

  const nodes: Node[] = [{
    id: "g-root", type: "genesis",
    position: { x: -44, y: -44 },
    data: { isRoot: true, revealKey },
    draggable: false,
  }];

  labels.forEach((label, i) => {
    const baseAngle = (i / G) * Math.PI * 2 - Math.PI / 2;
    const jAng = jitter(label, "ang", 0.18 / Math.max(G, 3));
    const jRad = jitter(label, "rad", Rg * 0.14);
    const gx   = (Rg + jRad) * Math.cos(baseAngle + jAng);
    const gy   = (Rg + jRad) * Math.sin(baseAngle + jAng);
    const r    = 18;
    const color = PALETTE[i % PALETTE.length];

    nodes.push({
      id: `g-${i}`, type: "genesis",
      position: { x: gx - r, y: gy - r },
      data: { label, idx: i, revealKey: i, r, color },
      draggable: false,
    });

    const NB_SAT = 4;
    const Rsat = 48;
    for (let s = 0; s < NB_SAT; s++) {
      const sa = (s / NB_SAT) * Math.PI * 2 + i * 0.7;
      nodes.push({
        id: `g-${i}-s-${s}`, type: "genesisSat",
        position: { x: gx + Rsat * Math.cos(sa) - 5, y: gy + Rsat * Math.sin(sa) - 5 },
        data: { idx: s, parentIdx: i, revealKey: i, color },
        draggable: false,
      });
    }
  });

  return nodes;
}

// ─── Edges ────────────────────────────────────────────────────────────────────

function makeEdges(infos: Map<string, NodeInfo>, showLeaves: boolean): Edge[] {
  return Array.from(infos.values())
    .filter((info) => {
      if (!info.parentId || info.parentId === "root") return false;
      if (isLeafKind(info.node.kind)) return showLeaves;
      return true;
    })
    .map((info) => {
      const isLeaf = isLeafKind(info.node.kind);
      return {
        id: `e-${info.parentId}-${info.id}`,
        source: info.parentId!,
        target: info.id,
        type: "straight",
        animated: false,
        style: {
          stroke: "var(--color-border)",
          strokeWidth: isLeaf ? 0.6 : 1,
          opacity: isLeaf ? 0.5 : 0.4,
          strokeDasharray: isLeaf ? "3 6" : undefined,
        },
      };
    });
}

// ─── Handle invisible ────────────────────────────────────────────────────────

const H: React.CSSProperties = {
  opacity: 0, top: "50%", left: "50%", width: 1, height: 1,
  border: "none", background: "transparent",
};

// ─── BubbleNode ───────────────────────────────────────────────────────────────

function BubbleNode({ data }: NodeProps) {
  const node    = data.node          as BrainNode;
  const r       = data.r             as number;
  const dim     = (data.dim          as boolean) ?? false;
  const bx      = (data.bx           as number)  ?? 0;
  const by      = (data.by           as number)  ?? 0;
  const delay   = (data.delay        as number)  ?? 0;
  const revealKey = (data.revealKey  as number)  ?? 0;
  const isLeaf  = (data.isLeaf       as boolean) ?? false;
  const isParentFocused = (data.isParentFocused as boolean) ?? false;
  const isRoot  = node.kind === "root";
  const isGroup = node.kind === "group" || node.kind === "container";
  const innerRef = useRef<HTMLDivElement>(null);
  const size = r * 2;

  useEffect(() => {
    const el = innerRef.current;
    if (!el) return;
    const anim = el.animate(
      [
        { transform: `translate(${bx}px,${by}px) scale(0.18)`, opacity: 0 },
        { transform: "translate(0,0) scale(1)", opacity: 1 },
      ],
      {
        duration: isLeaf ? 320 : 520,
        delay,
        fill: "both",
        easing: isLeaf ? "ease-out" : "cubic-bezier(0.34, 1.56, 0.64, 1)",
      },
    );
    return () => anim.cancel();
  }, [revealKey]); // eslint-disable-line

  // ── Lucid (root) ──────────────────────────────────────────────────────────
  if (isRoot) {
    return (
      <div style={{
        position: "relative", width: size, height: size + 28,
        cursor: "default", userSelect: "none",
        opacity: dim ? 0.15 : 1, transition: "opacity 0.2s",
      }}>
        <div ref={innerRef} className="lucid-orb" style={{
          width: size, height: size,
          animation: "lucidMorph 5.5s ease-in-out infinite, lucidGlow 4s ease-in-out infinite",
        }}>
          <Handle type="target" position={Position.Top}    style={H} />
          <Handle type="source" position={Position.Bottom} style={H} />
        </div>
        <div style={{
          position: "absolute", top: size + 6, left: "50%",
          transform: "translateX(-50%)", fontSize: 11, fontWeight: 700,
          letterSpacing: "0.12em", textTransform: "uppercase",
          color: "var(--color-text)", pointerEvents: "none", whiteSpace: "nowrap",
        }}>
          Lucid
        </div>
      </div>
    );
  }

  // ── Feuille (leaf/page/concept) ───────────────────────────────────────────
  if (isLeaf) {
    return (
      <div className="node-outer" style={{
        position: "relative", width: size, height: size,
        cursor: "pointer", userSelect: "none",
        opacity: dim ? 0.07 : 1, transition: "opacity 0.2s",
      }}>
        <div ref={innerRef} className="node-concept" style={{
          width: size, height: size, borderRadius: "50%",
        }}>
          <Handle type="target" position={Position.Top}    style={H} />
          <Handle type="source" position={Position.Bottom} style={H} />
        </div>
        <div className={`node-label${isParentFocused ? " node-label--visible" : ""}`}>
          {node.label}
        </div>
      </div>
    );
  }

  // ── Conteneur / Groupe ────────────────────────────────────────────────────
  return (
    <div style={{
      position: "relative", width: size, height: size + 26,
      cursor: "grab", userSelect: "none",
      opacity: dim ? 0.1 : 1, transition: "opacity 0.2s",
    }}>
      <div ref={innerRef} className="bubble node-project" style={{
        width: size, height: size, borderRadius: "50%",
        boxShadow: isGroup
          ? "0 4px 18px rgba(0,0,0,0.14), inset 0 1px 0 rgba(255,255,255,0.18)"
          : "0 2px 8px rgba(0,0,0,0.08), inset 0 1px 0 rgba(255,255,255,0.12)",
      }}>
        <Handle type="target" position={Position.Top}    style={H} />
        <Handle type="source" position={Position.Bottom} style={H} />
      </div>
      <div style={{
        position: "absolute", top: size + 5, left: "50%",
        transform: "translateX(-50%)", width: Math.max(size + 24, 64),
        textAlign: "center", fontSize: isGroup ? 11 : 10,
        fontWeight: isGroup ? 650 : 500,
        color: "var(--color-text)",
        lineHeight: 1.25, pointerEvents: "none",
        overflow: "hidden", display: "-webkit-box",
        WebkitLineClamp: 2, WebkitBoxOrient: "vertical",
      }}>
        {node.label}
      </div>
    </div>
  );
}

// ─── GenesisNode ─────────────────────────────────────────────────────────────

function GenesisNode({ data }: NodeProps) {
  const isRoot = !!(data.isRoot);
  const label  = (data.label     as string) ?? "";
  const idx    = (data.idx       as number) ?? 0;
  const rvk    = (data.revealKey as number) ?? 0;
  const color  = (data.color     as string) ?? PALETTE[0];
  const size   = isRoot ? 88 : 28;
  const ref    = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const anim = el.animate(
      [{ opacity: 0, transform: "scale(0.2)" }, { opacity: 1, transform: "scale(1)" }],
      { duration: 480, delay: isRoot ? 0 : idx * 60,
        fill: "both", easing: "cubic-bezier(0.34, 1.56, 0.64, 1)" },
    );
    return () => anim.cancel();
  }, [rvk]); // eslint-disable-line

  if (isRoot) {
    return (
      <div style={{ width: size, height: size }}>
        <div ref={ref} className="lucid-orb" style={{
          width: size, height: size,
          animation: "lucidMorph 5.5s ease-in-out infinite, lucidGlow 4s ease-in-out infinite",
        }} />
      </div>
    );
  }

  return (
    <div style={{ position: "relative", width: size, height: size + 18 }}>
      <div ref={ref} style={{
        width: size, height: size, borderRadius: "50%",
        background: `radial-gradient(ellipse at 35% 30%, ${color}99, ${color}55)`,
        border: `1.5px solid ${color}88`, boxShadow: `0 0 8px ${color}55`,
      }} />
      <div style={{
        position: "absolute", top: size + 3, left: "50%",
        transform: "translateX(-50%)", fontSize: 8,
        color: "var(--color-muted)", textAlign: "center",
        whiteSpace: "nowrap", maxWidth: 70,
        overflow: "hidden", textOverflow: "ellipsis", pointerEvents: "none",
      }}>
        {label}
      </div>
    </div>
  );
}

// ─── GenesisSat ───────────────────────────────────────────────────────────────

function GenesisSat({ data }: NodeProps) {
  const idx       = (data.idx       as number) ?? 0;
  const parentIdx = (data.parentIdx as number) ?? 0;
  const rvk       = (data.revealKey as number) ?? 0;
  const color     = (data.color     as string) ?? PALETTE[0];
  const ref       = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const anim = el.animate(
      [{ opacity: 0, transform: "scale(0)" }, { opacity: 0.7, transform: "scale(1)" }],
      { duration: 300, delay: parentIdx * 300 + 380 + idx * 60, fill: "both", easing: "ease-out" },
    );
    return () => anim.cancel();
  }, [rvk]); // eslint-disable-line

  return (
    <div ref={ref} style={{
      width: 10, height: 10, borderRadius: "50%",
      background: color, opacity: 0.6, boxShadow: `0 0 6px ${color}88`,
    }} />
  );
}

// ─── ZoomSync ────────────────────────────────────────────────────────────────

function ZoomSync({ onChange }: { onChange: (z: number) => void }) {
  const { zoom } = useViewport();
  const prev = useRef(zoom);
  useEffect(() => {
    if (Math.abs(zoom - prev.current) > 0.04) {
      prev.current = zoom;
      onChange(zoom);
    }
  }, [zoom, onChange]);
  return null;
}

// ─── FocusSync ───────────────────────────────────────────────────────────────

function FocusSync({
  focusedId, infos, dragOffsets,
}: {
  focusedId: string | null;
  infos: Map<string, NodeInfo>;
  dragOffsets: React.RefObject<Map<string, { dx: number; dy: number }>>;
}) {
  const { setCenter } = useReactFlow();
  useEffect(() => {
    if (!focusedId) return;
    const info = infos.get(focusedId);
    if (!info) return;
    const off = dragOffsets.current?.get(focusedId) ?? { dx: 0, dy: 0 };
    setCenter(info.finalX + off.dx, info.finalY + off.dy, { zoom: 1.6, duration: 550 });
  }, [focusedId]); // eslint-disable-line
  return null;
}

const nodeTypes = { bubble: BubbleNode, genesis: GenesisNode, genesisSat: GenesisSat };

// ─── BrainMap ─────────────────────────────────────────────────────────────────

export function BrainMap({
  graph, onSelect, selectedId, query,
  revealKey = 0, streamLabels = [], streamTotal = 0,
}: Props) {
  const [showLeaves, setShowLeaves] = useState(false);
  const [focusedId, setFocusedId]   = useState<string | null>(null);
  const q = query.trim().toLowerCase();

  const { infos, childrenOf } = useMemo(() => buildLayout(graph), [graph]);

  const dragOffsets = useRef(new Map<string, { dx: number; dy: number }>());

  const prevRevealKey = useRef(revealKey);
  if (revealKey !== prevRevealKey.current) {
    prevRevealKey.current = revealKey;
    dragOffsets.current.clear();
  }

  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([]);
  const isGenesis = streamLabels.length > 0;

  useEffect(() => {
    setNodes(
      isGenesis
        ? makeGenesisNodes(streamLabels, streamTotal, revealKey)
        : makeNodes(infos, dragOffsets.current, revealKey, showLeaves, selectedId, q, focusedId),
    );
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isGenesis, streamLabels, streamTotal, infos, revealKey, showLeaves, selectedId, q, focusedId]);

  const prevDrag = useRef(new Map<string, { x: number; y: number }>());

  const handleNodeDrag = useCallback(
    (_: unknown, node: Node) => {
      const info = infos.get(node.id);
      if (!info) return;
      const cx = node.position.x + info.r;
      const cy = node.position.y + info.r;
      const prev = prevDrag.current.get(node.id);
      if (prev) {
        const dx = cx - prev.x, dy = cy - prev.y;
        if (Math.abs(dx) > 0.5 || Math.abs(dy) > 0.5) {
          const kids = getDescendants(node.id, childrenOf);
          if (kids.length) {
            for (const id of kids) {
              const d = dragOffsets.current.get(id) ?? { dx: 0, dy: 0 };
              dragOffsets.current.set(id, { dx: d.dx + dx, dy: d.dy + dy });
            }
            setNodes((ns) => ns.map((n) =>
              kids.includes(n.id)
                ? { ...n, position: { x: n.position.x + dx, y: n.position.y + dy } }
                : n,
            ));
          }
        }
      }
      prevDrag.current.set(node.id, { x: cx, y: cy });
    },
    [infos, childrenOf, setNodes],
  );

  const handleNodeDragStop = useCallback(
    (_: unknown, node: Node) => {
      const info = infos.get(node.id);
      if (info) {
        dragOffsets.current.set(node.id, {
          dx: node.position.x + info.r - info.finalX,
          dy: node.position.y + info.r - info.finalY,
        });
      }
      prevDrag.current.delete(node.id);
    },
    [infos],
  );

  const handleNodeClick = useCallback(
    (_: React.MouseEvent, n: Node) => {
      if (isGenesis) return;
      const nd = n.data as { node?: BrainNode };
      if (nd.node) onSelect(nd.node);
    },
    [isGenesis, onSelect],
  );

  const handleNodeDoubleClick = useCallback(
    (_: React.MouseEvent, n: Node) => {
      if (isGenesis) return;
      const nd = n.data as { node?: BrainNode };
      if (!nd.node) return;
      const k = nd.node.kind;
      if (isLeafKind(k) || k === "root") return;
      setFocusedId((prev) => prev === n.id ? null : n.id);
    },
    [isGenesis],
  );

  const handlePaneClick = useCallback(() => setFocusedId(null), []);

  const handleZoom = useCallback((z: number) => {
    setShowLeaves(z >= 0.4);
  }, []);

  const edges = useMemo(
    () => isGenesis ? [] : makeEdges(infos, showLeaves),
    [isGenesis, infos, showLeaves],
  );

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onNodeDrag={handleNodeDrag}
      onNodeDragStop={handleNodeDragStop}
      onNodeClick={handleNodeClick}
      onNodeDoubleClick={handleNodeDoubleClick}
      onPaneClick={handlePaneClick}
      nodeTypes={nodeTypes}
      fitView
      fitViewOptions={{ padding: 0.15 }}
      minZoom={0.07}
      maxZoom={3}
      nodesConnectable={false}
      proOptions={{ hideAttribution: true }}
    >
      <ZoomSync onChange={handleZoom} />
      <FocusSync focusedId={focusedId} infos={infos} dragOffsets={dragOffsets} />
      <Background
        variant={BackgroundVariant.Dots}
        gap={20} size={1.2}
        color="var(--color-border)"
      />
    </ReactFlow>
  );
}
