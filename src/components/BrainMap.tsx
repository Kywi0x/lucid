import {
  useMemo, useState, useEffect, useRef,
} from "react";
import type { BrainGraph, BrainNode, Space } from "@/lib/types";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { NodePicker } from "@/components/NodePicker";
import { ThinkingOrb, type OrbState } from "thinking-orbs";

// ─── Types ────────────────────────────────────────────────────────────────────

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
  revealKey?: number;
  spaces?: Space[];
  onAddNodeToSpace?: (nodeId: string, spaceId: string) => void;
  onMoveNode?: (nodeId: string, parentId: string) => void;
  onDeleteNode?: (nodeId: string) => void;
  /** Fichiers déposés sur le canvas (drag & drop OS) → import sous `parentId`. */
  onImportFiles?: (paths: string[], parentId: string) => void;
  /** Clic net sur le vide du canvas (ferme le panneau détail). */
  onBackgroundClick?: () => void;
  /** Largeur (px) occupée à droite par le panneau détail : le centre de la
   *  caméra se décale pour garder le graphe centré dans l'espace restant. */
  panelOffset?: number;
  /** Centre la caméra sur ce nœud (k force le re-déclenchement). */
  focus?: { id: string; k: number } | null;
  /** "scanning"/"generating" pilotent l'état de l'orbe-root (sinon elle
   *  respire toute seule, cycle d'états au repos). */
  phase?: "idle" | "scanning" | "generating";
  /** Message affiché sous l'orbe (root) — remplace le texte dessiné dans le canvas. */
  caption?: string | null;
  /** Clic sur l'orbe quand aucun root n'existe encore (canvas vide, pré-scan). */
  onOrbClick?: () => void;
}

interface NodeInfo {
  id: string;
  node: BrainNode;
  r: number;
  finalX: number;
  finalY: number;
  parentId: string | null;
}

// ─── Palette clusters : désaturée en sombre, assombrie en clair ─────────────────

const PALETTE_DARK = [
  "#8f86e8", "#5fb39a", "#c2906b", "#c77e9e",
  "#6e9fcc", "#b8a05c", "#9a7cc4", "#62aba6",
  "#c07a72", "#7580c8",
];
const PALETTE_LIGHT = [
  "#6a5fd1", "#3c8f76", "#a96f44", "#b25e85",
  "#4b7fb0", "#8f7a35", "#7d5ba6", "#3f8a85",
  "#a55a50", "#5563af",
];
const LEAF_ZOOM = 0.5; // seuil d'apparition des feuilles…
const LEAF_FADE = 0.2; // …avec fondu sur cette plage de zoom

// Couleurs du canvas lues depuis les tokens CSS (--canvas-*) : le canvas suit le thème.
interface CanvasTheme {
  dark: boolean;
  bg: string;
  dot: string;
  wire: string;
  label: string;
  labelDim: string;
  sel: string;
  accent: string;
  palette: string[];
}

function readTheme(): CanvasTheme {
  const cs = getComputedStyle(document.documentElement);
  const v = (name: string, fb: string) => cs.getPropertyValue(name).trim() || fb;
  const attr = document.documentElement.getAttribute("data-theme");
  const dark = attr ? attr === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
  return {
    dark,
    bg: v("--canvas-bg", "#07090d"),
    dot: v("--canvas-dot", "rgba(205,216,235,0.07)"),
    wire: v("--canvas-wire", "rgba(163,178,210,0.16)"),
    label: v("--canvas-label", "rgba(212,218,230,0.72)"),
    labelDim: v("--canvas-label-dim", "rgba(212,218,230,0.38)"),
    sel: v("--canvas-sel", "rgba(255,255,255,0.85)"),
    accent: v("--color-accent", "#9d8cff"),
    palette: dark ? PALETTE_DARK : PALETTE_LIGHT,
  };
}

function radiusOf(n: BrainNode): number {
  if (n.kind === "root")                             return 20;
  if (n.kind === "group")                            return 10;
  if (n.kind === "note")                             return 6;
  if (n.kind === "pending")                          return 7; // proposition MCP (fantôme)
  if (n.kind === "espace" || n.kind === "container") return 6;
  return 4; // leaf, page, concept, source
}

export function isLeafKind(kind: string): boolean {
  return kind === "leaf" || kind === "page" || kind === "concept" || kind === "source";
}

export function matches(n: BrainNode, q: string): boolean {
  return [n.label, n.summary, ...n.keywords].join(" ").toLowerCase().includes(q);
}

const CLUSTER_FALLBACK = "#7f8aa8";

/** Couleur de cluster par nœud : DFS pré-ordre depuis la racine, chaque `group`
 *  prend la couleur suivante de la palette, ses descendants héritent.
 *  Partagé entre le canvas et la palette de commande (mêmes couleurs partout). */
export function buildClusterColors(graph: BrainGraph, palette: string[]): Map<string, string> {
  const childrenByParent = new Map<string, BrainNode[]>();
  for (const n of graph.nodes) {
    if (!n.parent_id) continue;
    if (!childrenByParent.has(n.parent_id)) childrenByParent.set(n.parent_id, []);
    childrenByParent.get(n.parent_id)!.push(n);
  }
  const colors = new Map<string, string>();
  const root = graph.nodes.find((n) => n.kind === "root");
  if (!root) return colors;
  // Couleur par hash stable de l'id : un cluster garde SA couleur quoi qu'il
  // arrive autour (timeline, watch auto, régén). Collisions possibles — deux
  // clusters voisins peuvent partager une teinte, prix de la stabilité.
  const walk = (n: BrainNode, inherited: string) => {
    const c = n.kind === "group"
      ? palette[Math.floor(stableHash(n.id) * palette.length) % palette.length]
      : inherited;
    colors.set(n.id, c);
    for (const kid of childrenByParent.get(n.id) ?? []) walk(kid, c);
  };
  walk(root, CLUSTER_FALLBACK);
  return colors;
}

/** Couleurs de clusters pour le thème courant. */
export function clusterColors(graph: BrainGraph): Map<string, string> {
  return buildClusterColors(graph, readTheme().palette);
}

function orbitR(N: number, maxR: number, gap: number, minR: number): number {
  if (N <= 1) return minR;
  return Math.max(minR, (maxR + gap) / Math.sin(Math.PI / N));
}


/** Relaxation de collisions sur grille spatiale : compacte sans chevauchement.
 *  Déterministe (ordre d'insertion) ; les nœuds de structure bougent moins que
 *  les pages (pin), le root ne bouge jamais. Remplace l'ancienne passe O(n²). */
function resolveOverlaps(infos: Map<string, NodeInfo>, iterations = 60, minGap = 16): void {
  const nodes = Array.from(infos.values());
  const CELL = 48;
  const pinOf = (n: NodeInfo) =>
    n.node.kind === "root" ? 1 : !isLeafKind(n.node.kind) ? 0.6 : 0;
  const key = (x: number, y: number) => x * 73856093 ^ y * 19349663;
  for (let iter = 0; iter < iterations; iter++) {
    const grid = new Map<number, NodeInfo[]>();
    for (const n of nodes) {
      const k = key(Math.floor(n.finalX / CELL), Math.floor(n.finalY / CELL));
      let cell = grid.get(k);
      if (!cell) grid.set(k, cell = []);
      cell.push(n);
    }
    for (const a of nodes) {
      const gx = Math.floor(a.finalX / CELL), gy = Math.floor(a.finalY / CELL);
      for (let ox = -1; ox <= 1; ox++) for (let oy = -1; oy <= 1; oy++) {
        const cell = grid.get(key(gx + ox, gy + oy));
        if (!cell) continue;
        for (const b of cell) {
          if (b.id <= a.id) continue; // chaque paire une seule fois
          const dx = b.finalX - a.finalX, dy = b.finalY - a.finalY;
          const dist = Math.hypot(dx, dy) || 0.01;
          const gap = isLeafKind(a.node.kind) || isLeafKind(b.node.kind) ? Math.min(minGap, 8) : minGap;
          const needed = a.r + b.r + gap;
          if (dist < needed) {
            const push = (needed - dist) / dist * 0.5;
            const wa = 1 - pinOf(a), wb = 1 - pinOf(b);
            a.finalX -= dx * push * wa; a.finalY -= dy * push * wa;
            b.finalX += dx * push * wb; b.finalY += dy * push * wb;
          }
        }
      }
    }
  }
}

/** Gaussienne déterministe en [-1, 1] dérivée de l'id (layout stable entre recalculs). */
function gaussOf(id: string, salt: string): number {
  return (stableHash(id + salt + "α") + stableHash(id + salt + "β") + stableHash(id + salt + "γ") - 1.5) / 1.5;
}

function stableHash(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0) / 0xffffffff;
}

function jitter(id: string, axis: string, scale: number): number {
  return (stableHash(id + axis) * 2 - 1) * scale;
}

// ─── Layout sectoriel proportionnel (réutilisé tel quel de l'ancienne version) ──

/** Accroche une position fraîchement calculée à sa position précédente connue
 *  (si on en a une) : 85 % ancienne position, 15 % nouvelle cible. Sans ça, le
 *  layout radial (parts de tarte proportionnelles au poids des frères/sœurs)
 *  replace TOUT le monde à chaque nouveau nœud — vu en direct pendant une
 *  génération, ça lit comme un reshuffle permanent puis une coupure nette au
 *  moment où le graphe final remplace le graphe provisoire. Avec l'ancrage,
 *  une bulle déjà placée glisse doucement vers sa cible au lieu de s'y
 *  téléporter — la structure finale se dessine progressivement, sans à-coup.
 *  N'affecte pas un premier rendu à froid (aucune ancre connue = position brute). */
function withAnchor(
  id: string, x: number, y: number, anchors?: Map<string, { x: number; y: number }>,
): { x: number; y: number } {
  const a = anchors?.get(id);
  if (!a) return { x, y };
  return { x: a.x + (x - a.x) * 0.15, y: a.y + (y - a.y) * 0.15 };
}

function buildLayout(
  graph: BrainGraph,
  anchors?: Map<string, { x: number; y: number }>,
): { infos: Map<string, NodeInfo>; childrenOf: Map<string, string[]> } {
  const infos = new Map<string, NodeInfo>();

  const childrenByParent = new Map<string, BrainNode[]>();
  for (const node of graph.nodes) {
    if (!node.parent_id) continue;
    if (!childrenByParent.has(node.parent_id)) childrenByParent.set(node.parent_id, []);
    childrenByParent.get(node.parent_id)!.push(node);
  }

  const rootNode = graph.nodes.find((n) => n.kind === "root");
  const rootId = rootNode?.id ?? "root";
  if (!rootNode) return { infos, childrenOf: new Map() };

  infos.set(rootId, {
    id: rootId, node: rootNode, r: 20,
    finalX: 0, finalY: 0, parentId: null,
  });

  const GAP = 16;

  // Poids d'un sous-arbre = nombre de pages qu'il contient (min 1) — c'est lui
  // qui dicte la part angulaire de chaque branche (géométrie de l'artifact).
  const weightMemo = new Map<string, number>();
  function weightOf(id: string): number {
    const memo = weightMemo.get(id);
    if (memo !== undefined) return memo;
    const kids = childrenByParent.get(id) ?? [];
    const w = kids.length === 0 ? 1 : kids.reduce((s, k) => s + weightOf(k.id), 0);
    weightMemo.set(id, w);
    return w;
  }

  // Layout radial en secteurs : chaque branche reçoit un intervalle angulaire
  // proportionnel à son poids et le subdivise pour ses enfants ; les distances
  // sont mesurées depuis le CENTRE (anneaux), les pages se blottissent en amas
  // anisotropes contre leur parent. Tout est déterministe (hash stable par id).
  function place(parentId: string, a0: number, a1: number, dist: number, depth: number) {
    const kids = childrenByParent.get(parentId) ?? [];
    if (!kids.length) return;
    const parent = infos.get(parentId)!;
    const px = parent.finalX, py = parent.finalY;

    const branchKids = kids.filter((k) => !isLeafKind(k.kind));
    const leafKids = kids.filter((k) => isLeafKind(k.kind));

    if (branchKids.length) {
      const kidWeights = branchKids.map((k) => weightOf(k.id));
      const total = kidWeights.reduce((a, b) => a + b, 0);
      const span = a1 - a0;
      const pad = branchKids.length > 1 ? span * 0.05 : 0;
      const usable = span - pad * 2;
      const maxKidR = branchKids.reduce((m, k) => Math.max(m, radiusOf(k)), 9);
      // Anneaux resserrés en profondeur : passé le 1er niveau, un enfant reste
      // proche de son parent (les longues cordes nues tuent la lecture).
      const dR = depth === 1
        ? Math.max(170, orbitR(branchKids.length, maxKidR, GAP * 2, 110))
        : 78 + maxKidR + GAP * 0.5;
      // Angle du parent vu du centre : on tire les enfants vers lui pour
      // raccourcir la corde parent→enfant tout en restant dans le secteur.
      const parentAng = px === 0 && py === 0 ? null : Math.atan2(py, px);
      let cursor = a0 + pad;
      branchKids.forEach((kid, ki) => {
        const w = (kidWeights[ki] / total) * usable;
        const mid = cursor + w / 2;
        let ang = mid;
        if (parentAng !== null) {
          const delta = Math.atan2(Math.sin(parentAng - mid), Math.cos(parentAng - mid));
          ang = mid + Math.max(-w * 0.38, Math.min(w * 0.38, delta * 0.5));
        }
        ang += jitter(kid.id, "ang", Math.min(0.09, w * 0.18));
        const d = dist + dR + jitter(kid.id, "rad", dR * 0.3);
        const raw = { x: Math.cos(ang) * d, y: Math.sin(ang) * d };
        const { x: kx, y: ky } = withAnchor(kid.id, raw.x, raw.y, anchors);
        infos.set(kid.id, {
          id: kid.id, node: kid, r: radiusOf(kid),
          finalX: kx, finalY: ky, parentId,
        });
        place(kid.id, cursor, cursor + w, Math.hypot(kx, ky), depth + 1);
        cursor += w;
      });
    }

    if (leafKids.length) {
      // Amas anisotrope : poussé vers l'extérieur (axe centre→parent), étalé le
      // long de la tangente — la « traînée » organique validée dans l'artifact.
      const outAngle = px === 0 && py === 0 ? (a0 + a1) / 2 : Math.atan2(py, px);
      const maxLeafR = leafKids.reduce((m, k) => Math.max(m, radiusOf(k)), 4);
      const sigmaT = Math.max(26, Math.sqrt(leafKids.length) * 18);
      const baseAway = parent.r + maxLeafR + GAP * 0.4;
      const sigmaA = 22 + Math.sqrt(leafKids.length) * 13;
      for (const kid of leafKids) {
        // Direction propre à chaque page (±0.4 rad) : le panache enveloppe le
        // parent au lieu de pointer raide vers l'extérieur — le « fouillis » voulu.
        const oa = outAngle + gaussOf(kid.id, "o") * 0.4;
        const outX = Math.cos(oa), outY = Math.sin(oa);
        const tanX = -outY, tanY = outX;
        const along = gaussOf(kid.id, "t") * sigmaT;
        const away = baseAway + Math.abs(gaussOf(kid.id, "a")) * sigmaA;
        const rawX = px + outX * away + tanX * along + jitter(kid.id, "jx", 9);
        const rawY = py + outY * away + tanY * along + jitter(kid.id, "jy", 9);
        const { x: kx, y: ky } = withAnchor(kid.id, rawX, rawY, anchors);
        infos.set(kid.id, {
          id: kid.id, node: kid, r: radiusOf(kid),
          finalX: kx, finalY: ky, parentId,
        });
        place(kid.id, a0, a1, Math.hypot(kx, ky), depth + 1);
      }
    }
  }

  place(rootId, -Math.PI / 2, Math.PI * 1.5, 0, 1);
  resolveOverlaps(infos, 60, GAP);

  // Mémorise les positions définitives (post-collision) pour ancrer le prochain
  // recalcul — voir withAnchor(). Ignoré si l'appelant ne fournit pas d'ancres
  // (ex. rendu figé du viewer public : pas de session vivante à stabiliser).
  if (anchors) {
    for (const info of infos.values()) anchors.set(info.id, { x: info.finalX, y: info.finalY });
  }

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

function getDescendants(id: string, childrenOf: Map<string, string[]>): string[] {
  const out: string[] = [];
  const q = [id];
  while (q.length) {
    for (const c of childrenOf.get(q.pop()!) ?? []) { out.push(c); q.push(c); }
  }
  return out;
}

// ─── Helpers canvas ─────────────────────────────────────────────────────────────

function hexA(hex: string, a: number): string {
  const h = hex.replace("#", "");
  const r = parseInt(h.slice(0, 2), 16), g = parseInt(h.slice(2, 4), 16), b = parseInt(h.slice(4, 6), 16);
  return `rgba(${r},${g},${b},${a})`;
}

// ─── BrainMap (rendu canvas constellation) ──────────────────────────────────────

// États traversés au repos par l'orbe-root — juste de quoi rester « vivante ».
const IDLE_ORB_STATES: OrbState[] = ["listening", "shaping", "composing"];

export function BrainMap({
  graph, onSelect, selectedId, query,
  revealKey = 0,
  spaces, onAddNodeToSpace, onMoveNode, onDeleteNode, onImportFiles,
  onBackgroundClick, panelOffset = 0, focus = null,
  phase = "idle", caption = null, onOrbClick,
}: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const orbWrapRef = useRef<HTMLDivElement>(null);

  // Pas de root = tout premier onboarding (canvas vide, avant le 1er scan) :
  // l'orbe s'agite pour inviter au clic. Dès qu'un cerveau existe, elle garde
  // un état par défaut fixe au repos — elle ne s'agite que scan/génération.
  const hasRoot = graph.nodes.some((n) => n.kind === "root");
  const [idleIdx, setIdleIdx] = useState(0);
  useEffect(() => {
    if (phase !== "idle" || hasRoot) return;
    const id = setInterval(() => setIdleIdx((i) => (i + 1) % IDLE_ORB_STATES.length), 3500);
    return () => clearInterval(id);
  }, [phase, hasRoot]);
  const orbState: OrbState = phase === "scanning" ? "searching"
    : phase === "generating" ? "solving"
    : hasRoot ? "listening"
    : IDLE_ORB_STATES[idleIdx];

  const [ctxMenu, setCtxMenu] = useState<{ nodeId: string; x: number; y: number } | null>(null);
  const [movePicker, setMovePicker] = useState<string | null>(null); // nodeId à déplacer
  const [theme, setTheme] = useState<CanvasTheme>(readTheme);

  const q = query.trim().toLowerCase();

  const rootId = useMemo(() => graph.nodes.find((n) => n.kind === "root")?.id ?? "", [graph]);
  const parentCandidates = useMemo(
    () => graph.nodes.filter((n) => !isLeafKind(n.kind) && n.kind !== "pending"),
    [graph],
  );
  // Ponts wikilinks ([[Page]] dans les contenus, calculés par App).
  const linkEdges = useMemo(() => graph.edges.filter((e) => e.kind === "link"), [graph]);

  // Positions passées d'un recalcul à l'autre — voir withAnchor() dans buildLayout :
  // stabilise le layout pendant qu'un graphe grandit en direct (scan, génération,
  // watch auto) au lieu de tout reshuffler à chaque nouveau nœud.
  const layoutAnchors = useRef(new Map<string, { x: number; y: number }>());
  const { infos, childrenOf } = useMemo(() => buildLayout(graph, layoutAnchors.current), [graph]);

  // Couleur par nœud : racine = accent du thème, sinon couleur du cluster.
  const { colorOf, neighbors, maxR } = useMemo(() => {
    const colorOf = buildClusterColors(graph, theme.palette);
    for (const info of infos.values()) {
      if (info.node.kind === "root") colorOf.set(info.id, theme.accent);
      else if (!colorOf.has(info.id)) colorOf.set(info.id, CLUSTER_FALLBACK);
    }
    const neighbors = new Map<string, Set<string>>();
    const add = (a: string, b: string) => {
      if (!neighbors.has(a)) neighbors.set(a, new Set());
      neighbors.get(a)!.add(b);
    };
    for (const info of infos.values()) {
      if (info.parentId) { add(info.id, info.parentId); add(info.parentId, info.id); }
    }
    let maxR = 1;
    for (const info of infos.values()) maxR = Math.max(maxR, Math.hypot(info.finalX, info.finalY) + info.r);
    return { colorOf, neighbors, maxR: maxR + 30 };
  }, [graph, infos, theme]);

  // Halo chaud : intensité de « fraîcheur » par nœud (1 = créé aujourd'hui,
  // 0 = plus d'une semaine). Recalculé à chaque évolution du graphe.
  const recentK = useMemo(() => {
    const m = new Map<string, number>();
    const now = Date.now();
    for (const n of graph.nodes) {
      if (!n.date || n.kind === "root") continue;
      const age = (now - Date.parse(n.date)) / 86_400_000;
      if (Number.isNaN(age) || age > 7) continue;
      m.set(n.id, Math.pow(1 - Math.max(0, age) / 7, 0.8));
    }
    return m;
  }, [graph]);

  // Recharge les couleurs du canvas quand le thème change (toggle in-app ou OS).
  useEffect(() => {
    const update = () => setTheme(readTheme());
    const mo = new MutationObserver(update);
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", update);
    return () => { mo.disconnect(); mq.removeEventListener("change", update); };
  }, []);

  // Refs partagés avec la boucle de rendu / les handlers (évite les closures périmées).
  const cam = useRef({ x: 0, y: 0, zoom: 1 });
  const camTarget = useRef({ x: 0, y: 0, zoom: 1 });
  // Premier cadrage instantané ; les suivants (graphe qui grandit en live,
  // régénération) glissent via le lerp caméra au lieu de sauter.
  const didFit = useRef(false);
  const panelOff = useRef({ cur: 0, target: 0 });
  const needsFit = useRef(true);
  const dragOffsets = useRef(new Map<string, { dx: number; dy: number }>());
  const hovered = useRef<string | null>(null);
  // Naissance des nouvelles bulles : id → timestamp de spawn (cascade).
  const spawnAt = useRef(new Map<string, number>());
  // Positions vivantes : chaque bulle glisse vers sa place cible (relayouts fluides,
  // pas de téléportation quand des sœurs apparaissent — replay, watch auto, régén).
  const livePos = useRef(new Map<string, { x: number; y: number; vx?: number; vy?: number }>());
  const prevIds = useRef<Set<string> | null>(null);
  const drag = useRef<{ mode: "node" | "pan"; id?: string; ids?: string[]; moved: number; sx: number; sy: number } | null>(null);
  // Matière : distance en sauts au nœud attrapé — la grappe suit en ressort avec
  // une raideur décroissante par saut, pendant le drag et jusqu'à se reposer.
  const hopOf = useRef(new Map<string, number>());
  const mouse = useRef<{ x: number; y: number } | null>(null);

  const S = useRef<any>(null);
  S.current = {
    infos, childrenOf, colorOf, neighbors, q, selectedId, theme, linkEdges,
    onSelect, onMoveNode, onImportFiles, onBackgroundClick, rootId,
    maxR, recentK, phase,
  };

  // Refit caméra au chargement / à chaque régénération.
  useEffect(() => {
    dragOffsets.current.clear();
    needsFit.current = true;
  }, [revealKey, maxR]);

  // Détecte les nœuds fraîchement apparus (proposition MCP, import, note…) et
  // programme leur « pop » en cascade depuis leur parent. Ignoré au premier
  // rendu, lors d'une régénération complète (genesis) et si l'OS demande
  // moins d'animations.
  useEffect(() => {
    const ids = new Set(graph.nodes.map((n) => n.id));
    // Purge les positions vivantes des nœuds disparus (scrub timeline arrière…).
    for (const id of [...livePos.current.keys()]) {
      if (!ids.has(id)) livePos.current.delete(id);
    }
    const prev = prevIds.current;
    prevIds.current = ids;
    if (!prev || prev.size === 0) return;
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const fresh = [...ids].filter((id) => !prev.has(id));
    if (fresh.length === 0) return;
    // Cascade bornée : peu de nœuds → 90 ms d'écart ; gros lot (replay, scrub,
    // grosse sync) → tout le monde naît dans une fenêtre de 600 ms max.
    const step = Math.min(90, 600 / fresh.length);
    const now = performance.now();
    fresh.forEach((id, i) => spawnAt.current.set(id, now + i * step));
  }, [graph]);

  // Décalage caméra quand le panneau détail est ouvert (lerpé dans draw()).
  useEffect(() => {
    panelOff.current.target = panelOffset;
  }, [panelOffset]);

  // Centrage animé sur un nœud (sélection depuis la palette ⌘K).
  useEffect(() => {
    if (!focus) return;
    const info = infos.get(focus.id);
    if (!info) return;
    const o = dragOffsets.current.get(focus.id) ?? { dx: 0, dy: 0 };
    camTarget.current = {
      x: info.finalX + o.dx,
      y: info.finalY + o.dy,
      zoom: Math.max(1, cam.current.zoom),
    };
  }, [focus, infos]);

  // ── Boucle de rendu ────────────────────────────────────────────────────────
  useEffect(() => {
    const canvas = canvasRef.current!;
    const ctx = canvas.getContext("2d")!;
    let raf = 0;
    let W = 0, H = 0, dpr = 1;
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;

    const resize = () => {
      const el = wrapRef.current!;
      dpr = Math.min(window.devicePixelRatio || 1, 2);
      W = el.clientWidth; H = el.clientHeight;
      canvas.width = W * dpr; canvas.height = H * dpr;
      canvas.style.width = W + "px"; canvas.style.height = H + "px";
    };
    const ro = new ResizeObserver(resize);
    ro.observe(wrapRef.current!);

    // Progression de naissance d'un nœud : 1 = né, 0 = pas encore, entre = en vol.
    const spawnK = (id: string): number => {
      const sp = spawnAt.current.get(id);
      if (sp === undefined) return 1;
      const el = (performance.now() - sp) / 650;
      if (el >= 1) { spawnAt.current.delete(id); return 1; }
      return Math.max(0, el);
    };

    // Position affichée = position vivante (avancée une fois par frame dans draw()).
    const pos = (info: NodeInfo): { x: number; y: number } =>
      livePos.current.get(info.id) ?? { x: info.finalX, y: info.finalY };

    // Alpha de naissance : 0 avant le spawn, fondu rapide ensuite. Les filaments
    // le suivent aussi — jamais de trait vers une bulle pas encore née.
    const spawnA = (id: string): number => {
      const k = spawnK(id);
      return k <= 0 ? 0 : Math.min(1, k * 2.5);
    };

    // Avance chaque position vivante vers sa cible ; les nouvelles bulles
    // naissent sur leur parent puis glissent vers leur place.
    const advancePositions = () => {
      const s = S.current;
      const firstLayout = livePos.current.size === 0;
      const k = reduce ? 1 : 0.16;
      // Matière : répulsion très douce du curseur (hors drag) — appliquée à la
      // cible, le lerp fait l'aller ET le retour. Un frémissement, pas une bousculade.
      const m = !reduce && !drag.current ? mouse.current : null;
      const wm = m ? toWorld(m.x, m.y) : null;
      const zoom = cam.current.zoom;
      const repulseR = 70 / zoom;
      for (const info of s.infos.values() as Iterable<NodeInfo>) {
        const o = dragOffsets.current.get(info.id) ?? { dx: 0, dy: 0 };
        let tx = info.finalX + o.dx, ty = info.finalY + o.dy;
        let lp = livePos.current.get(info.id);
        if (!lp) {
          const par = info.parentId ? livePos.current.get(info.parentId) : undefined;
          lp = firstLayout || !par ? { x: tx, y: ty } : { x: par.x, y: par.y };
          livePos.current.set(info.id, lp);
        }
        if (wm && info.node.kind !== "root") {
          const dx = lp.x - wm.x, dy = lp.y - wm.y;
          const d = Math.hypot(dx, dy);
          if (d < repulseR && d > 0.01) {
            const f = (1 - d / repulseR) ** 2 * (isLeafKind(info.node.kind) ? 7 : 3.5) / zoom;
            tx += (dx / d) * f; ty += (dy / d) * f;
          }
        }
        const hop = hopOf.current.get(info.id) ?? 0;
        if (!reduce && hop > 0) {
          // Ressort par sauts : enfants raides, petits-enfants plus mous, pages
          // traînardes — la grappe s'étire pendant le drag puis se repose.
          const ks = 0.11 * Math.pow(0.58, hop - 1);
          lp.vx = (lp.vx ?? 0) + (tx - lp.x) * ks;
          lp.vy = (lp.vy ?? 0) + (ty - lp.y) * ks;
          lp.vx *= 0.8; lp.vy *= 0.8;
          lp.x += lp.vx; lp.y += lp.vy;
          if (!drag.current && Math.abs(tx - lp.x) < 0.15 && Math.abs(ty - lp.y) < 0.15
            && Math.abs(lp.vx) < 0.05 && Math.abs(lp.vy) < 0.05) {
            lp.x = tx; lp.y = ty; lp.vx = 0; lp.vy = 0;
            hopOf.current.delete(info.id);
          }
        } else {
          lp.x += (tx - lp.x) * k;
          lp.y += (ty - lp.y) * k;
          // Snap à l'arrivée : sans lui le lerp n'atteint jamais la cible et le
          // texte, redessiné en sous-pixel à chaque frame, scintille en continu.
          if (Math.abs(tx - lp.x) < 0.1 && Math.abs(ty - lp.y) < 0.1) { lp.x = tx; lp.y = ty; }
        }
      }
    };

    const draw = (ts: number) => {
      // Le DPR change sans resize CSS (fenêtre déplacée entre écrans, création
      // avant affectation à l'écran Retina) → re-dimensionner le backing store.
      if (Math.min(window.devicePixelRatio || 1, 2) !== dpr && wrapRef.current) resize();
      const s = S.current;
      const time = reduce ? 0 : ts / 1000;
      const c = cam.current, ct = camTarget.current;

      // Décalage du centre quand le panneau détail est ouvert (suit le slide)
      const po = panelOff.current;
      po.cur += (po.target - po.cur) * 0.14;
      if (Math.abs(po.target - po.cur) < 0.1) po.cur = po.target;
      const CX = (W - po.cur) / 2;

      if (needsFit.current && W > 0) {
        const fit = Math.min(1, (Math.min(W - po.target, H) / 2 * 0.88) / s.maxR);
        if (!didFit.current) { c.x = 0; c.y = 0; c.zoom = fit; didFit.current = true; }
        ct.x = 0; ct.y = 0; ct.zoom = fit;
        needsFit.current = false;
      }
      // lerp caméra (pour le focus animé) + snap à l'arrivée (stabilité sous-pixel)
      c.x += (ct.x - c.x) * 0.14; c.y += (ct.y - c.y) * 0.14; c.zoom += (ct.zoom - c.zoom) * 0.14;
      if (Math.abs(ct.x - c.x) < 0.1) c.x = ct.x;
      if (Math.abs(ct.y - c.y) < 0.1) c.y = ct.y;
      if (Math.abs(ct.zoom - c.zoom) < 0.0005) c.zoom = ct.zoom;

      const sx = (wx: number) => CX + (wx - c.x) * c.zoom;
      const sy = (wy: number) => H / 2 + (wy - c.y) * c.zoom;
      const t = s.theme as CanvasTheme;
      // Fondu des feuilles autour du seuil de zoom (plus de pop) — désactivé
      // pendant scan/génération : la carte doit se construire visiblement
      // feuille par feuille, pas rester invisible sous le seuil de zoom tant
      // que la caméra dézoome pour tout faire tenir (demandé par Liam le 2026-07-22).
      const leafA = s.phase && s.phase !== "idle"
        ? 1
        : Math.max(0, Math.min(1, (c.zoom - (LEAF_ZOOM - LEAF_FADE / 2)) / LEAF_FADE));

      advancePositions();

      // Fond
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.globalCompositeOperation = "source-over";
      ctx.fillStyle = t.bg; ctx.fillRect(0, 0, W, H);
      const ox = sx(0), oy = sy(0);
      let g = ctx.createRadialGradient(ox, oy, 0, ox, oy, Math.max(W, H) * 0.72);
      if (t.dark) { g.addColorStop(0, "rgba(34,40,58,0.5)"); g.addColorStop(1, "rgba(34,40,58,0)"); }
      else        { g.addColorStop(0, "rgba(255,255,255,0.85)"); g.addColorStop(1, "rgba(255,255,255,0)"); }
      ctx.fillStyle = g; ctx.fillRect(0, 0, W, H);
      // grille de points
      const gap = 30, gox = ((ox % gap) + gap) % gap, goy = ((oy % gap) + gap) % gap;
      ctx.fillStyle = t.dot;
      for (let x = gox; x < W; x += gap) for (let y = goy; y < H; y += gap) { ctx.beginPath(); ctx.arc(x, y, 0.9, 0, Math.PI * 2); ctx.fill(); }

      const hv = hovered.current;
      const near = hv ? (id: string) => id === hv || (s.neighbors.get(hv)?.has(id) ?? false) : null;
      const visible = (info: NodeInfo) => (isLeafKind(info.node.kind) ? leafA > 0.02 : true);

      ctx.save(); ctx.translate(CX, H / 2); ctx.scale(c.zoom, c.zoom); ctx.translate(-c.x, -c.y);

      // ── Filaments (hairlines droites, 1px écran) ──
      ctx.lineCap = "round";
      for (const info of s.infos.values()) {
        const parent = info.parentId ? s.infos.get(info.parentId) : null;
        if (!parent) continue;
        if (!visible(info) || !visible(parent)) continue;
        const birth = Math.min(spawnA(info.id), spawnA(parent.id));
        if (birth <= 0) continue;
        const a = pos(parent), b = pos(info);
        const conn = !!hv && (info.id === hv || info.parentId === hv);
        const fade = isLeafKind(info.node.kind) ? leafA : 1;
        ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y);
        ctx.globalAlpha = (hv ? (conn ? 1 : 0.15) : 1) * fade * birth;
        ctx.strokeStyle = conn ? hexA(s.colorOf.get(info.id)!, 0.7) : t.wire;
        ctx.lineWidth = (conn ? 1.6 : 1) / c.zoom;
        ctx.stroke();
      }
      ctx.globalAlpha = 1;

      // ── Ponts wikilinks : arcs pointillés entre pages liées par [[…]] ──
      for (const e of s.linkEdges as { source: string; target: string }[]) {
        const ia = s.infos.get(e.source), ib = s.infos.get(e.target);
        if (!ia || !ib || !visible(ia) || !visible(ib)) continue;
        const birth = Math.min(spawnA(e.source), spawnA(e.target));
        if (birth <= 0) continue;
        const a = pos(ia), b = pos(ib);
        const conn = !!hv && (e.source === hv || e.target === hv);
        // Arc perpendiculaire léger pour se distinguer des filaments de l'arbre.
        const mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
        const dx = b.x - a.x, dy = b.y - a.y;
        const d = Math.hypot(dx, dy) || 1;
        const bow = Math.min(d * 0.18, 60);
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.quadraticCurveTo(mx - (dy / d) * bow, my + (dx / d) * bow, b.x, b.y);
        ctx.setLineDash([5 / c.zoom, 4 / c.zoom]);
        ctx.globalAlpha = (hv ? (conn ? 0.95 : 0.1) : 0.4) * birth;
        ctx.strokeStyle = hexA(s.colorOf.get(e.source) ?? "#7f8aa8", conn ? 0.9 : 0.5);
        ctx.lineWidth = (conn ? 1.6 : 1) / c.zoom;
        ctx.stroke();
        ctx.setLineDash([]);
      }
      ctx.globalAlpha = 1;

      // ── Nœuds (constellation : point net + anneau fin) ──
      for (const info of s.infos.values()) {
        if (!visible(info)) continue;
        const p = pos(info);
        const isRoot = info.node.kind === "root";
        if (isRoot) continue; // remplacé par l'overlay HTML de l'orbe (root vivant)
        const isPending = info.node.kind === "pending";
        // Ambre fixe pour les propositions MCP : lisible dans les deux thèmes.
        const color = isPending ? "#e0a33c" : s.colorOf.get(info.id)!;
        const isSel = info.id === s.selectedId;
        const dim = s.q && !matches(info.node, s.q) ? 0.12 : 1;
        const fade = isLeafKind(info.node.kind) ? leafA : 1;
        const vis = (hv && near ? (near(info.id) ? 1 : 0.24) : 1) * dim * fade;
        const boost = (info.id === hv ? 0.5 : 0) + (isSel ? 0.3 : 0);
        const phase = stableHash(info.id) * 6.28;
        const pulse = 0.5 + 0.5 * Math.sin(time * (isRoot ? 1.2 : 0.8) + phase);
        // Pop de naissance : overshoot élastique + fondu.
        const sk = spawnK(info.id);
        if (sk <= 0) continue; // pas encore né
        let rScale = 1, spawnAlpha = 1;
        if (sk < 1) {
          const c1 = 1.70158, c3 = c1 + 1;
          rScale = 1 + c3 * Math.pow(sk - 1, 3) + c1 * Math.pow(sk - 1, 2);
          spawnAlpha = Math.min(1, sk * 2.5);
        }
        const r = info.r * rScale;
        ctx.globalAlpha = vis * spawnAlpha;

        // Halo chaud : les pages créées cette semaine « couvent » — teinte chaude,
        // intensité qui décroît avec l'âge, respiration lente (fixe si reduce).
        const rk = s.recentK.get(info.id) ?? 0;
        if (rk > 0.02 && !isPending) {
          const warm = t.dark ? "#e8a04b" : "#c9873a";
          const breathe = reduce ? 0.75 : 0.65 + 0.35 * Math.sin(time * 1.1 + phase);
          const hr = r * (2.6 + rk * 1.2);
          const wg = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, hr);
          wg.addColorStop(0, hexA(warm, (0.09 + 0.15 * rk) * breathe));
          wg.addColorStop(1, hexA(warm, 0));
          ctx.fillStyle = wg; ctx.beginPath(); ctx.arc(p.x, p.y, hr, 0, Math.PI * 2); ctx.fill();
        }

        // halo très léger (survol/sélection seulement — le root a son propre halo HTML)
        if (boost > 0) {
          const haloR = r * 2.6;
          const hg = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, haloR);
          hg.addColorStop(0, hexA(color, 0.10 + boost * 0.14));
          hg.addColorStop(1, hexA(color, 0));
          ctx.fillStyle = hg; ctx.beginPath(); ctx.arc(p.x, p.y, haloR, 0, Math.PI * 2); ctx.fill();
        }

        // anneau fin (1px écran)
        ctx.beginPath(); ctx.arc(p.x, p.y, r, 0, Math.PI * 2);
        ctx.strokeStyle = hexA(color, 0.35 + boost * 0.4);
        ctx.lineWidth = 1 / c.zoom;
        ctx.stroke();

        // Modification MCP en attente (update/move/merge/link) : anneau orange
        // pointillé distinct de l'ambre des créations — le nœud existe déjà,
        // seul son état va changer si la proposition est acceptée.
        if (info.node.pendingAction) {
          ctx.beginPath();
          ctx.arc(p.x, p.y, r + 3 / c.zoom, 0, Math.PI * 2);
          ctx.setLineDash([3 / c.zoom, 3 / c.zoom]);
          ctx.strokeStyle = hexA("#f97316", 0.85);
          ctx.lineWidth = 1.6 / c.zoom;
          ctx.stroke();
          ctx.setLineDash([]);
        }
        // Proposition MCP : anneau pointillé pulsant (« fantôme » à valider)
        if (isPending) {
          ctx.setLineDash([4 / c.zoom, 3 / c.zoom]);
          ctx.beginPath(); ctx.arc(p.x, p.y, r + 2 + pulse * 2, 0, Math.PI * 2);
          ctx.strokeStyle = hexA(color, 0.35 + 0.4 * pulse);
          ctx.lineWidth = 1.2 / c.zoom;
          ctx.stroke();
          ctx.setLineDash([]);
        }

        // point
        const dotR = Math.max(1.6, r * 0.5);
        ctx.beginPath(); ctx.arc(p.x, p.y, dotR, 0, Math.PI * 2);
        ctx.fillStyle = color; ctx.fill();
        if (boost > 0) {
          ctx.beginPath(); ctx.arc(p.x, p.y, dotR * 0.6, 0, Math.PI * 2);
          ctx.fillStyle = `rgba(255,255,255,${Math.min(1, 0.3 + boost * 0.5)})`; ctx.fill();
        }

        // sélection : anneau net
        if (isSel) {
          ctx.beginPath(); ctx.arc(p.x, p.y, r * 1.7, 0, Math.PI * 2);
          ctx.strokeStyle = t.sel; ctx.lineWidth = 1.5 / c.zoom; ctx.stroke();
        }
      }
      ctx.globalAlpha = 1;
      ctx.restore();

      // ── Labels (espace écran, monospace, taille constante) ──
      for (const info of s.infos.values()) {
        if (!visible(info)) continue;
        if (spawnK(info.id) < 0.6) continue; // label après le pop de la bulle
        const kind = info.node.kind;
        const isRoot = kind === "root";
        if (isRoot) continue; // label HTML sous l'orbe, pas de texte canvas pour le root
        const isLeaf = isLeafKind(kind);
        const isContainer = !isLeaf && !isRoot;
        const isHover = info.id === hv;
        const always = isRoot || (isContainer && kind === "group") || kind === "pending";
        const show = always || isHover || (!!hv && near!(info.id)) || info.id === s.selectedId;
        if (!show) continue;
        const p = pos(info);
        // Texte calé sur la grille de pixels : net à l'arrêt comme en vol.
        const px = Math.round(sx(p.x)), py = sy(p.y);
        const fs = isRoot ? 12 : isContainer ? 10.5 : 9.5;
        ctx.font = `500 ${fs}px ui-monospace, SFMono-Regular, Menlo, monospace`;
        ctx.textAlign = "center"; ctx.textBaseline = "middle";
        const ly = Math.round(py + info.r * c.zoom + (isRoot ? 18 : 12));
        ctx.globalAlpha = isLeaf ? leafA : 1;
        ctx.shadowColor = t.dark ? "rgba(0,0,0,0.7)" : "rgba(245,247,251,0.9)";
        ctx.shadowBlur = 4;
        ctx.fillStyle = isHover || info.id === s.selectedId ? t.sel : isLeaf ? t.labelDim : t.label;
        (ctx as any).letterSpacing = isRoot ? "1.5px" : "0.4px";
        ctx.fillText(isRoot ? info.node.label.toUpperCase() : info.node.label, px, ly);
        ctx.shadowBlur = 0;
      }
      ctx.globalAlpha = 1;
      (ctx as any).letterSpacing = "0px";

      // ── Position de l'overlay HTML de l'orbe (= le root, vivant) ──
      // Suit le root projeté à l'écran ; s'il n'existe pas encore (canvas vide,
      // pré-scan), l'orbe reste fixe au centre — même logique de clic.
      if (orbWrapRef.current) {
        const rootInfo = s.infos.get(s.rootId);
        if (rootInfo) {
          const rp = pos(rootInfo);
          const scale = Math.min(1.6, Math.max(0.55, c.zoom));
          orbWrapRef.current.style.transform =
            `translate(${sx(rp.x)}px, ${sy(rp.y)}px) translate(-50%, -50%) scale(${scale})`;
        } else {
          orbWrapRef.current.style.transform = `translate(${CX}px, ${H / 2}px) translate(-50%, -50%)`;
        }
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);

    // ── Interaction ──
    const rect = () => canvas.getBoundingClientRect();
    const centerX = () => (canvas.clientWidth - panelOff.current.cur) / 2;
    const toWorld = (cx: number, cy: number) => {
      const c = cam.current, r = rect();
      return { x: c.x + (cx - r.left - centerX()) / c.zoom, y: c.y + (cy - r.top - canvas.clientHeight / 2) / c.zoom };
    };
    const showLeavesNow = () => cam.current.zoom >= LEAF_ZOOM;
    const nodeAt = (cx: number, cy: number): NodeInfo | null => {
      const w = toWorld(cx, cy); let best: NodeInfo | null = null, bd = 1e9;
      for (const info of S.current.infos.values()) {
        if (isLeafKind(info.node.kind) && !showLeavesNow()) continue;
        const p = pos(info); const d = Math.hypot(p.x - w.x, p.y - w.y);
        if (d < info.r + 7 && d < bd) { bd = d; best = info; }
      }
      return best;
    };
    const dropTarget = (draggedId: string, cx: number, cy: number): string | null => {
      const w = toWorld(cx, cy);
      const banned = new Set([draggedId, ...getDescendants(draggedId, S.current.childrenOf)]);
      let best: string | null = null, bd = 1e9;
      for (const info of S.current.infos.values()) {
        if (banned.has(info.id) || isLeafKind(info.node.kind)) continue;
        const p = pos(info); const d = Math.hypot(p.x - w.x, p.y - w.y);
        if (d < info.r + 26 && d < bd) { bd = d; best = info.id; }
      }
      return best;
    };

    const onDown = (e: MouseEvent) => {
      if (e.button !== 0) return;
      setCtxMenu(null);
      const n = nodeAt(e.clientX, e.clientY);
      const canDrag = !!n && n.node.kind !== "root" && n.node.kind !== "pending";
      // On garde toujours l'id du nœud sous le curseur → clic (sans déplacement) = sélection,
      // même pour une feuille/page non déplaçable (qui, elle, laisse le pan agir).
      drag.current = canDrag
        ? { mode: "node", id: n!.id, ids: [n!.id, ...getDescendants(n!.id, S.current.childrenOf)], moved: 0, sx: e.clientX, sy: e.clientY }
        : { mode: "pan", id: n?.id, moved: 0, sx: e.clientX, sy: e.clientY };
      if (canDrag) {
        // Matière : profondeur en sauts de chaque descendant → raideur décroissante.
        const hops = new Map<string, number>([[n!.id, 0]]);
        const q: Array<[string, number]> = [[n!.id, 0]];
        while (q.length) {
          const [id, h] = q.pop()!;
          for (const c of S.current.childrenOf.get(id) ?? []) { hops.set(c, h + 1); q.push([c, h + 1]); }
        }
        hopOf.current = hops;
      }
      canvas.style.cursor = canDrag ? "grabbing" : "move";
    };
    const onMove = (e: MouseEvent) => {
      const d = drag.current;
      // Listener window (le drag doit survivre hors du canvas), mais au repos
      // on ignore tout ce qui n'est pas directement le canvas : une surface
      // par-dessus (modale, panneau, onboarding) ne doit pas piloter le hover.
      if (!d && e.target !== canvas) {
        mouse.current = null;
        if (hovered.current) { hovered.current = null; canvas.style.cursor = "default"; }
        return;
      }
      mouse.current = { x: e.clientX, y: e.clientY };
      if (d) {
        const dx = e.clientX - d.sx, dy = e.clientY - d.sy;
        d.moved += Math.abs(dx) + Math.abs(dy); d.sx = e.clientX; d.sy = e.clientY;
        const c = cam.current, ct = camTarget.current;
        if (d.mode === "pan") { c.x -= dx / c.zoom; c.y -= dy / c.zoom; ct.x = c.x; ct.y = c.y; }
        else { const wx = dx / c.zoom, wy = dy / c.zoom; for (const id of d.ids!) { const o = dragOffsets.current.get(id) ?? { dx: 0, dy: 0 }; dragOffsets.current.set(id, { dx: o.dx + wx, dy: o.dy + wy }); } }
        return;
      }
      const n = nodeAt(e.clientX, e.clientY);
      hovered.current = n ? n.id : null;
      canvas.style.cursor = n ? "grab" : "default";
    };
    const onUp = (e: MouseEvent) => {
      const d = drag.current; drag.current = null; canvas.style.cursor = "default";
      if (!d) return;
      // Clic net sur un nœud (quel qu'il soit) → sélection / ouverture.
      if (d.moved < 4 && d.id) {
        const info = S.current.infos.get(d.id);
        if (info) S.current.onSelect(info.node);
        return;
      }
      // Clic net sur le vide → fermeture du panneau détail.
      if (d.moved < 4 && !d.id) {
        S.current.onBackgroundClick?.();
        return;
      }
      // Déplacement d'un nœud lâché sur un autre → reparentage.
      if (d.mode === "node" && d.moved >= 4 && S.current.onMoveNode) {
        const info = S.current.infos.get(d.id!);
        const target = dropTarget(d.id!, e.clientX, e.clientY);
        if (info && target && target !== info.parentId) S.current.onMoveNode(d.id!, target);
      }
    };
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const c = cam.current, ct = camTarget.current, r = rect();
      const w = toWorld(e.clientX, e.clientY);
      const f = Math.exp(-e.deltaY * 0.0015);
      const nz = Math.max(0.12, Math.min(4, c.zoom * f));
      c.zoom = nz; ct.zoom = nz;
      c.x = w.x - (e.clientX - r.left - centerX()) / nz;
      c.y = w.y - (e.clientY - r.top - canvas.clientHeight / 2) / nz;
      ct.x = c.x; ct.y = c.y;
    };
    const onDbl = (e: MouseEvent) => {
      const n = nodeAt(e.clientX, e.clientY);
      if (!n || isLeafKind(n.node.kind) || n.node.kind === "root") return;
      const p = pos(n);
      camTarget.current = { x: p.x, y: p.y, zoom: Math.max(1.4, cam.current.zoom) };
    };
    const onCtx = (e: MouseEvent) => {
      e.preventDefault();
      const n = nodeAt(e.clientX, e.clientY);
      if (n && n.node.kind !== "pending") setCtxMenu({ nodeId: n.id, x: e.clientX, y: e.clientY });
      else setCtxMenu(null);
    };

    canvas.addEventListener("mousedown", onDown);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("dblclick", onDbl);
    canvas.addEventListener("contextmenu", onCtx);
    const onLeave = () => { mouse.current = null; };
    canvas.addEventListener("mouseleave", onLeave);

    // ── Drag & drop de fichiers OS (Tauri) : cible = bulle survolée (ou son
    // parent si feuille), sinon la racine. Le ring de survol sert de feedback.
    const fileDropTarget = (cx: number, cy: number): string => {
      const n = nodeAt(cx, cy);
      if (!n) return S.current.rootId;
      if (n.node.kind === "pending") return n.parentId ?? S.current.rootId; // fantôme : pas encore réel
      return isLeafKind(n.node.kind) ? (n.parentId ?? S.current.rootId) : n.id;
    };
    let unlistenDrop: (() => void) | undefined;
    let dropDisposed = false;
    // Drag & drop OS : uniquement dans Tauri (le viewer web réutilise ce composant).
    if ("__TAURI_INTERNALS__" in window) {
      getCurrentWebview().onDragDropEvent((ev) => {
        if (!S.current.onImportFiles) return;
        const dpr = window.devicePixelRatio || 1;
        if (ev.payload.type === "over") {
          hovered.current = fileDropTarget(ev.payload.position.x / dpr, ev.payload.position.y / dpr);
        } else if (ev.payload.type === "drop") {
          const target = fileDropTarget(ev.payload.position.x / dpr, ev.payload.position.y / dpr);
          hovered.current = null;
          S.current.onImportFiles(ev.payload.paths, target);
        } else if (ev.payload.type === "leave") {
          hovered.current = null;
        }
      }).then((un) => { if (dropDisposed) un(); else unlistenDrop = un; });
    }

    return () => {
      cancelAnimationFrame(raf);
      ro.disconnect();
      canvas.removeEventListener("mousedown", onDown);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
      canvas.removeEventListener("wheel", onWheel);
      canvas.removeEventListener("dblclick", onDbl);
      canvas.removeEventListener("contextmenu", onCtx);
      canvas.removeEventListener("mouseleave", onLeave);
      dropDisposed = true;
      unlistenDrop?.();
    };
    // Monté UNE fois : tout l'état vivant passe par S.current / refs. Redémarrer
    // cette boucle (canvas, listeners, drag&drop) à chaque évolution du graphe
    // provoquait 1-2 frames vides → clignotement à chaque recalcul (timeline, watch).
  }, []);

  // ── Rendu React (canvas + overlays) ──────────────────────────────────────────
  return (
    <div ref={wrapRef} style={{ width: "100%", height: "100%", position: "relative", overflow: "hidden", background: "var(--canvas-bg)" }}>
      <canvas ref={canvasRef} style={{ display: "block", position: "absolute", inset: 0 }} />

      {/* L'orbe EST le root : overlay HTML (pas un nœud canvas) positionné image
          par image par la boucle de rendu ci-dessus, pour rester net au zoom/pan. */}
      <div
        ref={orbWrapRef}
        onClick={() => {
          const rootInfo = infos.get(rootId);
          if (rootInfo) onSelect(rootInfo.node);
          else onOrbClick?.();
        }}
        style={{ position: "absolute", left: 0, top: 0, zIndex: 5, cursor: "pointer", transform: "translate(-9999px,-9999px)" }}
        className="group flex flex-col items-center gap-2"
      >
        <ThinkingOrb state={orbState} size={64} />
        {caption && (
          <span className="whitespace-nowrap text-xs text-[var(--color-muted)] group-hover:text-[var(--color-text)]">
            {caption}
          </span>
        )}
      </div>

      {ctxMenu && (onMoveNode || onDeleteNode || (spaces && onAddNodeToSpace)) && (
        <div
          style={{
            position: "fixed",
            left: Math.min(ctxMenu.x, window.innerWidth - 220),
            top: Math.min(ctxMenu.y, window.innerHeight - 200),
            zIndex: 100,
          }}
          className="rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)] py-1 min-w-[200px]"
        >
          <p className="max-w-[240px] truncate px-3 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
            {infos.get(ctxMenu.nodeId)?.node.label ?? ""}
          </p>
          <div className="my-1 border-t border-[var(--color-border)]" />
          {onMoveNode && (
            <button
              onClick={() => { setMovePicker(ctxMenu.nodeId); setCtxMenu(null); }}
              className="w-full px-3 py-1.5 text-left text-sm text-[var(--color-text)] hover:bg-[var(--color-surface-2)] transition-colors"
            >
              Déplacer vers…
            </button>
          )}
          {spaces && onAddNodeToSpace && (
            <>
              <div className="my-1 border-t border-[var(--color-border)]" />
              <p className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
                Ajouter à la space
              </p>
              <div className="max-h-40 overflow-auto">
                {spaces.filter(s => s.id !== "lucid").map(s => (
                  <button key={s.id}
                    onClick={() => { onAddNodeToSpace(ctxMenu.nodeId, s.id); setCtxMenu(null); }}
                    className="w-full px-3 py-1.5 text-left text-sm text-[var(--color-text)] hover:bg-[var(--color-surface-2)] transition-colors truncate"
                  >
                    {s.name}
                  </button>
                ))}
                {spaces.filter(s => s.id !== "lucid").length === 0 && (
                  <p className="px-3 py-1.5 text-xs text-[var(--color-muted)]">Crée une space d&apos;abord</p>
                )}
              </div>
            </>
          )}
          {onDeleteNode && infos.get(ctxMenu.nodeId)?.node.kind !== "root" && (
            <>
              <div className="my-1 border-t border-[var(--color-border)]" />
              <button
                onClick={() => { onDeleteNode(ctxMenu.nodeId); setCtxMenu(null); }}
                className="w-full px-3 py-1.5 text-left text-sm text-[var(--color-err)] hover:bg-[var(--color-surface-2)] transition-colors"
              >
                Supprimer…
              </button>
            </>
          )}
        </div>
      )}

      {movePicker && onMoveNode && (() => {
        const banned = new Set([movePicker, ...getDescendants(movePicker, childrenOf)]);
        const curParent = infos.get(movePicker)?.parentId ?? null;
        const targets = parentCandidates.filter(n => !banned.has(n.id) && n.id !== curParent);
        return (
          <NodePicker
            title="Déplacer vers"
            candidates={targets}
            graph={graph}
            onPick={(n) => { onMoveNode(movePicker, n.id); setMovePicker(null); }}
            onClose={() => setMovePicker(null)}
          />
        );
      })()}

    </div>
  );
}
