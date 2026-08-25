/** Vue « Mosaïque » — 3ᵉ mode de lecture du cerveau, à côté de Mind et Dossiers.
 *
 *  Zéro trait : la relation est l'adjacence. Trois écarts pour toute grammaire
 *  (2 / 6 / 24 px, cf. `mosaicLayout.ts`), et de la densité — quelques milliers
 *  de documents sur un écran, sans pelote ni défilement.
 *
 *  ponytail: pas de reprise du menu contextuel, du drop de fichiers OS, de la
 *  timeline ni de l'orbe de pensée. La vue Mind garde tout ça ; ici on lit et on
 *  sélectionne, point. À enrichir seulement si cette vue est retenue après la
 *  phase design.
 */
import { useEffect, useMemo, useRef } from "react";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { matches } from "@/components/BrainMap";
import {
  buildMosaic, TILE, RADIUS, ROOT_R,
  type MosaicFolder, type MosaicLayout, type MosaicTile,
} from "@/lib/mosaicLayout";

interface Props {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
  onBackgroundClick?: () => void;
  /** Largeur (px) du panneau détail à droite : le recadrage se décale d'autant. */
  panelOffset?: number;
}

/** Barème d'opacité. Au repos les tuiles respirent (elles ne sont jamais
 *  pleines) ; le survol rend sa présence entière au dossier pointé et efface le
 *  reste. Un seul endroit à toucher pour régler l'ensemble. */
const A_REST  = 0.6;    // repos
const A_FOCUS = 1;      // dossier survolé
const A_SPACE = 0.45;   // son espace de premier niveau
const A_FAR   = 0.1;    // tout le reste pendant un survol
const A_MISS  = 0.08;   // hors résultats de recherche

/** Espace de premier niveau d'un dossier — sert aux trois plans du survol. */
function spaceOf(f: MosaicFolder): MosaicFolder {
  let cur = f;
  while (cur.parent && cur.parent.parent) cur = cur.parent;
  return cur;
}

export function MosaicMap({
  graph, onSelect, selectedId, query, onBackgroundClick, panelOffset = 0,
}: Props) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const tipRef = useRef<HTMLDivElement>(null);
  const caretRef = useRef<HTMLDivElement>(null);
  const api = useRef<{ shift: (dx: number) => void; refit: () => void } | null>(null);
  const lastOffset = useRef(0);

  const layout = useMemo<MosaicLayout | null>(() => {
    if (!graph.nodes.length) return null;
    try { return buildMosaic(graph); } catch { return null; }
  }, [graph]);

  // Ce que la boucle d'évènements doit lire sans se re-souscrire à chaque render.
  const S = useRef({ layout, onSelect, onBackgroundClick, selectedId, query, panelOffset });
  S.current = { layout, onSelect, onBackgroundClick, selectedId, query, panelOffset };

  useEffect(() => {
    const prev = lastOffset.current;
    lastOffset.current = panelOffset;
    if (prev !== panelOffset) api.current?.shift((prev - panelOffset) / 2);
  }, [panelOffset]);

  useEffect(() => {
    const canvasEl = canvasRef.current, wrapEl = wrapRef.current;
    if (!canvasEl || !wrapEl) return;
    const ctx2d = canvasEl.getContext("2d");
    if (!ctx2d) return;
    // Types non-nullables explicites : TypeScript ne propage pas un
    // rétrécissement de type dans les `function` hoistées ci-dessous.
    const cv: HTMLCanvasElement = canvasEl;
    const wrap: HTMLDivElement = wrapEl;
    const ctx: CanvasRenderingContext2D = ctx2d;

    const view = { x: 0, y: 0, k: 1 };
    /** Taille CSS du canvas, figée au `resize`. Comme dans `BrainMap` : la
     *  taille est IMPOSÉE (`style.width/height`), pas déduite d'`inset: 0`.
     *  Sans ça, le backing store et la boîte CSS peuvent divorcer et toute la
     *  conversion écran→monde se décale — zoom qui rate le curseur, survol à
     *  côté (retour de Liam le 2026-08-25). */
    let W = 0, H = 0;
    let hover: MosaicTile | null = null;
    let dpr = 1;
    /** Dernière position connue du curseur, en px CSS relatifs au canvas.
     *  Indispensable : un zoom ou un glisser déplace les tuiles SOUS un curseur
     *  immobile, il faut donc re-désigner sans attendre un `pointermove`. */
    let ptr: { x: number; y: number } | null = null;

    const tokens = () => {
      const cs = getComputedStyle(document.documentElement);
      const v = (n: string, fb: string) => cs.getPropertyValue(n).trim() || fb;
      return {
        bg: v("--canvas-bg", "#07090d"),
        // token OPAQUE volontairement : l'opacité est pilotée par `globalAlpha`,
        // sinon « 100 % au survol » resterait plafonné par l'alpha du token.
        note: v("--color-muted", "#8a93a6"),
        node: v("--color-accent", "#9d8cff"),
        sel: v("--canvas-sel", "rgba(255,255,255,0.85)"),
      };
    };

    // On dessine en espace ÉCRAN, pas dans un `ctx.scale(k)`.
    //
    // Sous transformation, une tuile part de coordonnées monde entières mais
    // atterrit à x = 337.48 : ses quatre bords tombent entre deux pixels
    // physiques et l'antialiasing les rend laiteux — sur 470 petits carrés, tout
    // paraît sale. Ici chaque tuile est arrondie à la grille de pixels physiques
    // et reçoit une taille UNIQUE, calculée une fois. Le prix : les écarts
    // peuvent varier d'un demi-pixel physique — invisible, là où le flou ne
    // l'était pas.
    const paint = (list: MosaicTile[], color: string, alpha: number) => {
      if (!list.length) return;
      const size = Math.max(1, Math.round(TILE * view.k * dpr)) / dpr;
      ctx.globalAlpha = alpha;
      ctx.fillStyle = color;
      ctx.beginPath();
      if (size < 4) {
        // un coin arrondi sur un carré de 3 px n'est plus un coin, c'est du flou
        for (const t of list) {
          ctx.rect(Math.round((view.x + t.x * view.k) * dpr) / dpr,
                   Math.round((view.y + t.y * view.k) * dpr) / dpr, size, size);
        }
      } else {
        const rad = Math.min(RADIUS * view.k, size / 2);
        for (const t of list) {
          ctx.roundRect(Math.round((view.x + t.x * view.k) * dpr) / dpr,
                        Math.round((view.y + t.y * view.k) * dpr) / dpr, size, size, rad);
        }
      }
      ctx.fill();
    };

    function draw() {
      const L = S.current.layout;
      const c = tokens();
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.fillStyle = c.bg;
      ctx.fillRect(0, 0, W, H);
      if (!L) return;

      const hf = hover ? hover.folder : null;
      const hs = hf ? spaceOf(hf) : null;

      // le centre, nu — c'est le root
      ctx.beginPath();
      ctx.arc(view.x, view.y, ROOT_R * view.k, 0, Math.PI * 2);
      ctx.fillStyle = c.node;
      ctx.globalAlpha = hf ? A_FAR * 2 : A_REST;
      ctx.fill();

      const q = S.current.query.trim().toLowerCase();
      if (q) {
        // recherche : ce qui répond garde sa présence, le reste s'efface
        const hit: MosaicTile[] = [], hitN: MosaicTile[] = [];
        const miss: MosaicTile[] = [], missN: MosaicTile[] = [];
        for (const t of L.tiles) {
          const ok = matches(t.node, q);
          (t.isFolder ? (ok ? hitN : missN) : (ok ? hit : miss)).push(t);
        }
        paint(miss, c.note, A_MISS);
        paint(missN, c.node, A_MISS);
        paint(hit, c.note, A_FOCUS);
        paint(hitN, c.node, A_FOCUS);
      } else if (!hf) {
        paint(L.tiles.filter((t) => !t.isFolder), c.note, A_REST);
        paint(L.tiles.filter((t) => t.isFolder), c.node, A_REST);
      } else {
        // trois plans de lecture : le dossier survolé, son espace, le reste
        const b: MosaicTile[][] = [[], [], [], [], [], []];
        for (const t of L.tiles) {
          const lvl = t.folder === hf ? 0 : spaceOf(t.folder) === hs ? 1 : 2;
          b[lvl * 2 + (t.isFolder ? 1 : 0)].push(t);
        }
        const A = [A_FOCUS, A_SPACE, A_FAR];
        for (let l = 2; l >= 0; l--) {
          paint(b[l * 2], c.note, A[l]);
          paint(b[l * 2 + 1], c.node, A[l]);
        }
      }

      // la sélection : un liseré, jamais un remplissage (la couleur dit le type)
      const sel = S.current.selectedId;
      if (sel) {
        const marked = L.tiles.filter((t) => t.node.id === sel);
        if (marked.length) {
          const size = Math.max(1, Math.round(TILE * view.k * dpr)) / dpr;
          // trait d'épaisseur impaire → centré sur un demi-pixel, sinon il bave
          const lw = Math.max(1, Math.round(1.5 * dpr)) / dpr;
          ctx.globalAlpha = 1;
          ctx.strokeStyle = c.sel;
          ctx.lineWidth = lw;
          ctx.beginPath();
          for (const t of marked) {
            const x = Math.round((view.x + t.x * view.k) * dpr) / dpr;
            const y = Math.round((view.y + t.y * view.k) * dpr) / dpr;
            ctx.roundRect(x - 2 + lw / 2, y - 2 + lw / 2,
                          size + 4 - lw, size + 4 - lw,
                          Math.min(RADIUS * view.k + 2, (size + 4) / 2));
          }
          ctx.stroke();
        }
      }

      ctx.globalAlpha = 1;
      if (hover) placeTip(hover);
    }

    // ── infobulle : ancrée sur la tuile, chevron qui la désigne ────────────
    // Manipulée en impératif (pas d'état React) : un survol ne doit pas
    // provoquer de rendu React, et il faut mesurer la bulle après avoir écrit
    // son texte pour la recadrer juste.
    function setTip(t: MosaicTile | null) {
      const tip = tipRef.current;
      if (!tip) return;
      if (!t) { tip.style.opacity = "0"; return; }
      tip.firstChild!.textContent = t.isFolder ? t.folder.label : t.node.label;
      tip.style.opacity = "1";
    }

    function placeTip(t: MosaicTile) {
      const tip = tipRef.current, caret = caretRef.current;
      if (!tip || !caret) return;
      const w = tip.offsetWidth, h = tip.offsetHeight;
      const PAD = 8, GAP = 7;
      const cx = view.x + (t.x + TILE / 2) * view.k;
      const top = view.y + t.y * view.k;

      let y = top - GAP - h, below = false;
      if (y < PAD) { y = top + TILE * view.k + GAP; below = true; }
      const x = Math.max(PAD, Math.min(cx - w / 2, W - PAD - w));

      tip.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
      // deux recadrages indépendants : la bulle rentre dans la vue, la pointe
      // reste sur la tuile.
      caret.style.left = `${Math.max(6, Math.min(w - 6, cx - x)) - 3.5}px`;
      caret.style.top = below ? "-4px" : "";
      caret.style.bottom = below ? "" : "-4px";
    }

    // ── caméra ─────────────────────────────────────────────────────────────
    function fit() {
      const L = S.current.layout;
      if (!L) return;
      const b = L.bounds, pad = 44;
      const off = S.current.panelOffset;
      const w = Math.max(1, W - off), h = H;
      view.k = Math.max(0.06, Math.min((w - pad * 2) / (b.x1 - b.x0), (h - pad * 2) / (b.y1 - b.y0)));
      view.x = w / 2 - ((b.x0 + b.x1) / 2) * view.k;
      view.y = h / 2 - ((b.y0 + b.y1) / 2) * view.k;
      draw();
    }

    function pick(px: number, py: number): MosaicTile | null {
      const L = S.current.layout;
      if (!L) return null;
      const wx = (px - view.x) / view.k, wy = (py - view.y) / view.k;
      // Quand la tuile est grosse à l'écran on exige le survol franc ; quand
      // elle tombe à 2 px on aide le curseur. Une tolérance fixe faisait
      // sauter la désignation entre voisines au zoom avant.
      const tol = Math.max(0, 5 - (TILE * view.k) / 2) / view.k;
      let best: MosaicTile | null = null, bestD = Infinity;
      for (const t of L.tiles) {
        const dx = Math.max(t.x - wx, 0, wx - (t.x + TILE));
        const dy = Math.max(t.y - wy, 0, wy - (t.y + TILE));
        const d = dx * dx + dy * dy;
        if (d <= tol * tol && d < bestD) { bestD = d; best = t; }
      }
      return best;
    }

    /** Re-désigne sous le curseur immobile. Appelé après tout changement de
     *  caméra. Redessine seulement si la cible a changé. */
    function refreshHover() {
      if (!ptr) return;
      const t = pick(ptr.x, ptr.y);
      if (t === hover) return;
      hover = t;
      setTip(t);
      draw();
    }

    function resize() {
      dpr = Math.min(2, window.devicePixelRatio || 1);
      W = wrap.clientWidth;
      H = wrap.clientHeight;
      cv.width = Math.max(1, Math.round(W * dpr));
      cv.height = Math.max(1, Math.round(H * dpr));
      cv.style.width = W + "px";
      cv.style.height = H + "px";
      draw();
    }

    /** Origine du canvas en coordonnées viewport. Isolée pour n'avoir qu'UN
     *  endroit qui convertisse un évènement souris en pixels canvas — et pour
     *  crier si la boîte mesurée ne fait pas la taille imposée (le symptôme
     *  exact d'un décalage de zoom/survol). */
    let warned = false;
    function origin() {
      const r = cv.getBoundingClientRect();
      if (!warned && (Math.abs(r.width - W) > 1 || Math.abs(r.height - H) > 1)) {
        warned = true;
        console.warn("[MosaicMap] boîte canvas %o×%o ≠ taille imposée %o×%o — la conversion écran→monde va se décaler",
          Math.round(r.width), Math.round(r.height), W, H);
      }
      return r;
    }

    // ── interactions ───────────────────────────────────────────────────────
    let drag: { x: number; y: number; vx: number; vy: number } | null = null;
    let downAt: { x: number; y: number } | null = null;

    const onDown = (e: PointerEvent) => {
      downAt = { x: e.clientX, y: e.clientY };
      drag = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y };
      cv.style.cursor = "grabbing";
      cv.setPointerCapture(e.pointerId);
    };

    const onMove = (e: PointerEvent) => {
      if (drag) {
        view.x = drag.vx + (e.clientX - drag.x);
        view.y = drag.vy + (e.clientY - drag.y);
        const r0 = origin();
        ptr = { x: e.clientX - r0.left, y: e.clientY - r0.top };
        draw();
        return;
      }
      const r = origin();
      ptr = { x: e.clientX - r.left, y: e.clientY - r.top };
      const t = pick(ptr.x, ptr.y);
      if (t !== hover) { hover = t; setTip(t); draw(); }
    };

    const onUp = (e: PointerEvent) => {
      drag = null;
      cv.style.cursor = "grab";
      cv.releasePointerCapture(e.pointerId);
      refreshHover();   // la carte a glissé sous le curseur
    };

    const onLeave = () => {
      ptr = null;
      if (hover) { hover = null; setTip(null); draw(); }
    };

    const onClick = (e: MouseEvent) => {
      // un glisser n'est pas un clic
      if (downAt && Math.hypot(e.clientX - downAt.x, e.clientY - downAt.y) > 4) return;
      const L = S.current.layout;
      const r = origin();
      const px = e.clientX - r.left, py = e.clientY - r.top;
      const t = pick(px, py);
      if (t) { S.current.onSelect(t.isFolder ? t.folder.node : t.node); return; }
      const wx = (px - view.x) / view.k, wy = (py - view.y) / view.k;
      if (L && wx * wx + wy * wy <= ROOT_R * ROOT_R) {
        const root = L.folders.find((f) => !f.parent);
        if (root) { S.current.onSelect(root.node); return; }
      }
      S.current.onBackgroundClick?.();
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const r = origin();
      const px = e.clientX - r.left, py = e.clientY - r.top;
      const wx = (px - view.x) / view.k, wy = (py - view.y) / view.k;
      const k = Math.max(0.15, Math.min(10, view.k * Math.exp(-e.deltaY * 0.0016)));
      view.k = k;
      view.x = px - wx * k;
      view.y = py - wy * k;
      ptr = { x: px, y: py };
      draw();
      refreshHover();
    };

    cv.style.cursor = "grab";
    cv.addEventListener("pointerdown", onDown);
    cv.addEventListener("pointermove", onMove);
    cv.addEventListener("pointerup", onUp);
    cv.addEventListener("pointerleave", onLeave);
    cv.addEventListener("click", onClick);
    cv.addEventListener("wheel", onWheel, { passive: false });
    cv.addEventListener("dblclick", fit);

    const mq = matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", draw);
    const themeObs = new MutationObserver(() => draw());
    themeObs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });

    // ouverture/fermeture du panneau détail : on décale la vue de la moitié de
    // la largeur libérée, sans toucher au zoom courant.
    api.current = {
      shift(dx: number) { view.x += dx; draw(); refreshHover(); },
      refit: fit,
    };

    const ro = new ResizeObserver(resize);
    ro.observe(wrap);
    // E. un changement de densité de pixels (écran externe) ne déclenche pas le
    // ResizeObserver : on écoute aussi la fenêtre.
    const onWinResize = () => {
      if (Math.min(2, window.devicePixelRatio || 1) !== dpr) resize();
    };
    window.addEventListener("resize", onWinResize);
    resize();
    fit();

    return () => {
      api.current = null;
      window.removeEventListener("resize", onWinResize);
      cv.removeEventListener("pointerdown", onDown);
      cv.removeEventListener("pointermove", onMove);
      cv.removeEventListener("pointerup", onUp);
      cv.removeEventListener("pointerleave", onLeave);
      cv.removeEventListener("click", onClick);
      cv.removeEventListener("wheel", onWheel);
      cv.removeEventListener("dblclick", fit);
      mq.removeEventListener("change", draw);
      themeObs.disconnect();
      ro.disconnect();
    };
    // `layout` est lu via S.current, mais un nouveau graphe doit recadrer :
    // c'est le seul motif de re-souscription.
  }, [layout]);

  return (
    <div
      ref={wrapRef}
      style={{ width: "100%", height: "100%", position: "relative", overflow: "hidden", background: "var(--canvas-bg)" }}
    >
      <canvas ref={canvasRef} style={{ display: "block", position: "absolute", inset: 0 }} />

      {/* Infobulle : le nom, rien d'autre. Toujours montée, pilotée en impératif. */}
      <div
        ref={tipRef}
        className="panel pointer-events-none absolute left-0 top-0 z-10 max-w-[260px] rounded px-1.5 py-0.5 text-[11px] leading-snug text-[var(--color-text)]"
        style={{ opacity: 0, transition: "opacity .1s" }}
      >
        <span className="block overflow-hidden text-ellipsis whitespace-nowrap" />
        {/* carré tourné à 45° : la bulle masque la couture */}
        <div
          ref={caretRef}
          className="absolute size-[7px] rotate-45 border-b border-r border-[var(--color-border)] bg-[var(--color-surface)]"
          style={{ bottom: "-4px" }}
        />
      </div>

      {layout && (layout.missing > 0 || layout.detached > 0) && (
        // jamais d'échec silencieux : si le pavage a perdu des tuiles, il le dit
        <div className="pointer-events-none absolute bottom-3 left-3 rounded px-2 py-1 text-[10px] text-[var(--color-muted)]">
          {layout.missing > 0 && `${layout.missing} notes non placées`}
          {layout.missing > 0 && layout.detached > 0 && " · "}
          {layout.detached > 0 && `${layout.detached} dossiers détachés`}
        </div>
      )}
    </div>
  );
}
