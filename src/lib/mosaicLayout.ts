/** Layout « mosaïque » : la connaissance en pavés carrés, sans aucun trait.
 *
 *  La hiérarchie ne s'exprime que par l'écart entre tuiles — c'est toute la
 *  grammaire, et elle tient en trois valeurs :
 *    2 px  → deux notes d'un même dossier
 *    6 px  → deux dossiers d'un même espace
 *    24 px → deux espaces distincts
 *
 *  Rien d'autre n'est encodé. Pas de profondeur : essayée en dégradé d'opacité
 *  puis en liserés d'ancêtres, retirée les deux fois — l'imbrication ne répond
 *  à aucune question qu'on pose devant cette carte (décision Liam 2026-08-25).
 *
 *  Module pur : aucun DOM, aucun React. Le rendu vit dans `MosaicMap.tsx`.
 */
import type { BrainGraph, BrainNode } from "@/lib/types";

export const TILE = 10;      // côté d'une tuile
export const RADIUS = 4;     // arrondi des coins
export const G_NOTE = 2;
export const G_SUB = 6;
export const G_BIG_DEFAULT = 24;
export const ROOT_R = 56;    // rayon réservé au centre (root)
const PITCH = TILE + G_NOTE; // 12 — pas du réseau interne d'un dossier

export interface MosaicTile {
  x: number;
  y: number;
  /** Nœud représenté par cette tuile — c'est lui qu'on sélectionne au clic. */
  node: BrainNode;
  /** Dossier porteur (une tuile-nœud est son propre dossier). */
  folder: MosaicFolder;
  /** true = tuile-dossier (accent), false = tuile-note (gris). */
  isFolder: boolean;
  /** Grappe = espace de premier niveau, sert au test d'écart. */
  cid: number;
}

export interface MosaicFolder {
  id: string;
  node: BrainNode;
  label: string;
  parent: MosaicFolder | null;
  children: MosaicFolder[];
  /** Nœuds sans enfant rattachés ici → une tuile grise chacun. */
  notes: BrainNode[];
  /** Nombre de dossiers de passage absorbés dans celui-ci (1 = aucun). */
  merged: number;
  depth: number;
  tiles: MosaicTile[];
}

export interface MosaicLayout {
  tiles: MosaicTile[];
  folders: MosaicFolder[];
  bounds: { x0: number; y0: number; x1: number; y1: number };
  /** Comptes rendus honnêtes — rien ne disparaît en silence. */
  absorbed: number;   // dossiers de passage fusionnés
  detached: number;   // dossiers qui n'ont pas trouvé de place
  missing: number;    // notes non posées (enclavement)
}

// ─── Arbre ────────────────────────────────────────────────────────────────────

/** Règle de traduction : un nœud AVEC enfants est un dossier (tuile accent), un
 *  nœud SANS enfant est une note (tuile grise) de son dossier parent. C'est la
 *  seule qui survive aux données réelles, où `kind` n'est pas fiable pour ça
 *  (un `leaf` peut porter des enfants après un rangement Archiviste). */
function buildTree(graph: BrainGraph): { root: MosaicFolder; folders: MosaicFolder[] } {
  const byId = new Map<string, BrainNode>();
  for (const n of graph.nodes) if (n.kind !== "pending") byId.set(n.id, n);

  const kids = new Map<string, BrainNode[]>();
  for (const n of byId.values()) {
    const p = n.parent_id;
    if (!p || !byId.has(p)) continue;
    const a = kids.get(p);
    if (a) a.push(n); else kids.set(p, [n]);
  }

  const rootNode =
    graph.nodes.find((n) => n.kind === "root") ??
    [...byId.values()].find((n) => !n.parent_id || !byId.has(n.parent_id));
  if (!rootNode) throw new Error("mosaïque : aucun nœud racine");

  const folders: MosaicFolder[] = [];
  const mk = (node: BrainNode, parent: MosaicFolder | null): MosaicFolder => {
    const f: MosaicFolder = {
      id: node.id, node, label: node.label, parent,
      children: [], notes: [], merged: 1, depth: parent ? parent.depth + 1 : 0, tiles: [],
    };
    parent?.children.push(f);
    folders.push(f);
    return f;
  };

  const root = mk(rootNode, null);
  const walk = (node: BrainNode, folder: MosaicFolder) => {
    for (const c of kids.get(node.id) ?? []) {
      if ((kids.get(c.id) ?? []).length === 0) folder.notes.push(c);
      else walk(c, mk(c, folder));
    }
  };
  walk(rootNode, root);

  // Les notes posées directement sous la racine existent : elles forment leur
  // propre espace au lieu de disparaître (le centre n'a pas de tuiles, il est
  // occupé par le root).
  if (root.notes.length) {
    const bucket = mk({ ...rootNode, id: rootNode.id + "::orphans", label: "Notes à la racine" }, root);
    bucket.notes = root.notes;
    root.notes = [];
  }

  return { root, folders };
}

/** Un dossier sans note et à enfant unique n'est pas un niveau de connaissance :
 *  c'est un « / » dans un chemin. `Documents/D2/D4/D5/D6/D7` doit se lire comme
 *  UN pavé. On absorbe la chaîne avant le layout — mesuré sur brain.db : 16
 *  containers concernés. */
function flattenChains(root: MosaicFolder): { folders: MosaicFolder[]; absorbed: number } {
  const folders: MosaicFolder[] = [];
  let absorbed = 0;

  const visit = (f: MosaicFolder, isRoot: boolean) => {
    while (!isRoot && f.notes.length === 0 && f.children.length === 1) {
      const kid = f.children[0];
      f.label += "  /  " + kid.label;
      f.node = kid.node;            // le clic ouvre la feuille réelle, pas le tuyau
      f.id = kid.id;
      f.notes = kid.notes;
      f.children = kid.children;
      for (const c of f.children) c.parent = f;
      f.merged += 1;
      absorbed += 1;
    }
    folders.push(f);
    for (const c of f.children) visit(c, false);
  };
  visit(root, true);

  // profondeurs recalculées après fusion
  for (const f of folders) {
    let d = 0;
    for (let p = f.parent; p; p = p.parent) d++;
    f.depth = d;
  }
  return { folders, absorbed };
}

// ─── Index spatial ────────────────────────────────────────────────────────────
//
// Deux tuiles TILE×TILE sont séparées d'au moins g si
//   max(|dx| - TILE, |dy| - TILE) >= g.  Test entier, aucune racine carrée.
// Chaque entrée porte sa grappe (cid) et son dossier (fid) : l'écart requis se
// déduit de la parenté, donc un seul test couvre toute la grammaire.

const CELL = 64;
type Grid = Map<string, number[]>;

function gridAdd(g: Grid, x: number, y: number, cid: number, fid: number) {
  const k = Math.floor(x / CELL) + ":" + Math.floor(y / CELL);
  let b = g.get(k);
  if (!b) g.set(k, (b = []));
  b.push(x, y, cid, fid);
}

function makeLegal(g: Grid, gBig: number) {
  return (x: number, y: number, cid: number, fid: number): boolean => {
    const gx = Math.floor(x / CELL), gy = Math.floor(y / CELL);
    for (let i = -1; i <= 1; i++) {
      for (let j = -1; j <= 1; j++) {
        const b = g.get(gx + i + ":" + (gy + j));
        if (!b) continue;
        for (let t = 0; t < b.length; t += 4) {
          const need = b[t + 2] !== cid ? gBig      // autre espace (ou le root, cid -1)
                     : b[t + 3] !== fid ? G_SUB     // autre dossier du même espace
                     : G_NOTE;                      // même dossier
          if (Math.abs(x - b[t]) - TILE < need && Math.abs(y - b[t + 1]) - TILE < need) return false;
        }
      }
    }
    return true;
  };
}

/** Témoins du disque central : cid -1, donc tout le monde lui doit `gBig`. */
function addRootWitnesses(grid: Grid) {
  gridAdd(grid, -TILE / 2, -TILE / 2, -1, -1);
  for (let r = 8; r <= ROOT_R; r += 8) {
    const n = Math.max(6, Math.round((2 * Math.PI * r) / 8));
    for (let a = 0; a < n; a++) {
      const ang = (a / n) * Math.PI * 2;
      gridAdd(grid, Math.cos(ang) * r - TILE / 2, Math.sin(ang) * r - TILE / 2, -1, -1);
    }
  }
}

// ─── Aléa déterministe ────────────────────────────────────────────────────────
// ponytail: même graphe = même carte, mais AJOUTER un nœud repave l'espace et
// peut décaler ses voisins. Mémoire spatiale non garantie entre deux
// générations — c'est la limite connue du pavage. Piste si ça gêne : mémoriser
// la position des espaces d'une passe à l'autre et ne repaver que l'intérieur.
function mulberry32(a: number) {
  return () => {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function hashSeed(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

// ─── Croissance ───────────────────────────────────────────────────────────────

const N4: Array<[number, number]> = [[1, 0], [-1, 0], [0, 1], [0, -1]];
// pondération : compacité du bloc / attraction vers le centre / part d'aléa
// (c'est l'aléa qui casse les formes carrées).
const COMPACT = 3, PULL = 90, JITTER = 1.25, CANDS = 40;

const subtreeNotes = (f: MosaicFolder): number =>
  f.notes.length + f.children.reduce((s, c) => s + subtreeNotes(c), 0);
/** tuiles d'un sous-arbre : ses notes + une tuile-dossier par dossier */
const subtreeTiles = (f: MosaicFolder): number =>
  f.notes.length + 1 + f.children.reduce((s, c) => s + subtreeTiles(c), 0);

export function buildMosaic(graph: BrainGraph, gBig = G_BIG_DEFAULT): MosaicLayout {
  const { root } = buildTree(graph);
  const { folders, absorbed } = flattenChains(root);

  const rnd = mulberry32(hashSeed(root.id + ":" + graph.nodes.length));
  const grid: Grid = new Map();
  addRootWitnesses(grid);
  const legal = makeLegal(grid, gBig);
  const tiles: MosaicTile[] = [];

  const addTile = (x: number, y: number, folder: MosaicFolder, node: BrainNode, isFolder: boolean, cid: number) => {
    const t: MosaicTile = { x, y, node, folder, isFolder, cid };
    tiles.push(t);
    folder.tiles.push(t);
    gridAdd(grid, x, y, cid, folderKey(folder));
    return t;
  };

  // clé numérique stable par dossier (le test d'écart compare des entiers)
  const keys = new Map<MosaicFolder, number>();
  const folderKey = (f: MosaicFolder) => {
    let k = keys.get(f);
    if (k === undefined) keys.set(f, (k = keys.size + 1));
    return k;
  };

  /** Croissance d'un dossier, une tuile à la fois. Chaque candidate est une case
   *  du réseau à PITCH d'une tuile déjà posée du MÊME dossier : les écarts
   *  internes valent donc toujours exactement G_NOTE. */
  function growFolder(folder: MosaicFolder, seed: { x: number; y: number }, cid: number) {
    const fid = folderKey(folder);
    const own = [addTile(seed.x, seed.y, folder, folder.node, true, cid)];
    const mine = new Set([seed.x + "," + seed.y]);

    for (const note of folder.notes) {
      let best: { x: number; y: number } | null = null, bestScore = -Infinity;
      for (const t of own) {
        for (const [dx, dy] of N4) {
          const x = t.x + dx * PITCH, y = t.y + dy * PITCH;
          const k = x + "," + y;
          if (mine.has(k) || !legal(x, y, cid, fid)) continue;
          let nb = 0;
          for (const [ex, ey] of N4) if (mine.has(x + ex * PITCH + "," + (y + ey * PITCH))) nb++;
          const score = nb * COMPACT - Math.hypot(x, y) / PULL + rnd() * JITTER;
          if (score > bestScore) { bestScore = score; best = { x, y }; }
        }
      }
      if (!best) break;   // enclavé : compté dans `missing`, jamais tu
      mine.add(best.x + "," + best.y);
      own.push(addTile(best.x, best.y, folder, note, false, cid));
    }
    return own;
  }

  /** Place libre atteignable depuis une graine. Garde-fou indispensable : sans
   *  lui un dossier démarre dans une alcôve, s'enclave, et ses notes
   *  disparaissent en silence. */
  function roomFrom(x0: number, y0: number, cid: number, fid: number, need: number) {
    const seen = new Set([x0 + "," + y0]);
    const queue: Array<[number, number]> = [[x0, y0]];
    let n = 1;
    while (queue.length && n < need) {
      const [x, y] = queue.pop()!;
      for (const [dx, dy] of N4) {
        const nx = x + dx * PITCH, ny = y + dy * PITCH;
        const k = nx + "," + ny;
        if (seen.has(k)) continue;
        seen.add(k);
        if (!legal(nx, ny, cid, fid)) continue;
        if (++n >= need) return n;
        queue.push([nx, ny]);
      }
    }
    return n;
  }

  /** Graine d'un sous-dossier : contact EXACT à G_SUB avec une tuile de sa
   *  grappe. Le contact se fait avec la FAMILLE, pas avec le parent nommément :
   *  sans étiquettes l'œil ne lit que l'appartenance, et l'exiger du seul parent
   *  asphyxie la croissance (mesuré : un tiers des notes perdues). */
  function seedNear(clusterTiles: MosaicTile[], cid: number, fid: number, need: number) {
    const cands: Array<{ x: number; y: number; s: number }> = [];
    const STEP = TILE + G_SUB;
    for (const t of clusterTiles) {
      for (const [dx, dy] of N4) {
        const x = t.x + dx * STEP, y = t.y + dy * STEP;
        if (legal(x, y, cid, fid)) cands.push({ x, y, s: x * x + y * y });
      }
    }
    cands.sort((a, b) => a.s - b.s);
    let fallback: { x: number; y: number; s: number } | null = null, fallbackRoom = -1;
    for (const c of cands.slice(0, CANDS)) {
      const room = roomFrom(c.x, c.y, cid, fid, need + 1);
      if (room > need) return c;
      if (room > fallbackRoom) { fallbackRoom = room; fallback = c; }
    }
    return fallback ?? cands[0] ?? null;
  }

  /** Graine d'un espace : tir de rayons depuis des ancres déjà posées, on garde
   *  le point valide le plus proche du centre qui a la place. */
  function seedSpace(cid: number, fid: number, anchors: Array<{ x: number; y: number }>, need: number) {
    let best: { x: number; y: number } | null = null, bestScore = Infinity;
    let fallback: { x: number; y: number; s: number } | null = null;
    for (let i = 0; i < 60; i++) {
      const a = anchors[Math.floor(rnd() * anchors.length)];
      const ang = rnd() * Math.PI * 2, ux = Math.cos(ang), uy = Math.sin(ang);
      for (let r = 0; r < 1400; r += 2) {
        const x = Math.round((a.x + ux * r) / 2) * 2, y = Math.round((a.y + uy * r) / 2) * 2;
        if (!legal(x, y, cid, fid)) continue;
        const s = x * x + y * y;
        if (!fallback || s < fallback.s) fallback = { x, y, s };
        if (s < bestScore && roomFrom(x, y, cid, fid, need + 1) > need) { bestScore = s; best = { x, y }; }
        break;
      }
    }
    return best ?? fallback;
  }

  // les plus gros espaces d'abord : ils prennent le pourtour du centre
  const spaces = root.children.slice().sort((a, b) => subtreeNotes(b) - subtreeNotes(a));
  const anchors: Array<{ x: number; y: number }> = [{ x: 0, y: 0 }];
  let detached = 0;
  let cidSeq = 0;

  for (const space of spaces) {
    const cid = cidSeq++;
    const s0 = seedSpace(cid, folderKey(space), anchors, subtreeTiles(space));
    if (!s0) { detached++; continue; }

    const clusterTiles: MosaicTile[] = [];
    // profondeur d'abord, grosses branches d'abord ; la graine de chaque enfant
    // est calculée juste avant sa pousse, donc jamais périmée.
    const growSubtree = (folder: MosaicFolder, pos: { x: number; y: number }) => {
      clusterTiles.push(...growFolder(folder, pos, cid));
      const kids = folder.children.slice().sort((a, b) => subtreeNotes(b) - subtreeNotes(a));
      for (const kid of kids) {
        const ks = seedNear(clusterTiles, cid, folderKey(kid), subtreeTiles(kid));
        if (ks) growSubtree(kid, ks);
        else detached++;
      }
    };
    growSubtree(space, s0);

    // une tuile sur six devient une ancre : assez pour cerner le contour
    for (let i = 0; i < clusterTiles.length; i += 6) anchors.push(clusterTiles[i]);
  }

  // aucune note ne doit disparaître en silence
  let missing = 0;
  for (const f of folders) {
    if (f === root) continue;
    missing += f.notes.length - f.tiles.filter((t) => !t.isFolder).length;
  }

  let x0 = -ROOT_R, y0 = -ROOT_R, x1 = ROOT_R, y1 = ROOT_R;
  for (const t of tiles) {
    if (t.x < x0) x0 = t.x;
    if (t.y < y0) y0 = t.y;
    if (t.x + TILE > x1) x1 = t.x + TILE;
    if (t.y + TILE > y1) y1 = t.y + TILE;
  }

  return { tiles, folders, bounds: { x0, y0, x1, y1 }, absorbed, detached, missing };
}
