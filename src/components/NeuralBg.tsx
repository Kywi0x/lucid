import { cn } from "@/lib/utils";

/** Nœuds décoratifs (positions en %, tailles en px, couleurs clusters du graphe). */
const NODES = [
  { x: 20, y: 26, s: 22, c: "#8F86E8", label: "projets" },
  { x: 76, y: 22, s: 18, c: "#5FB39A", label: "notes" },
  { x: 84, y: 62, s: 14, c: "#6E9FCC", label: null },
  { x: 66, y: 80, s: 20, c: "#C2906B", label: "sources" },
  { x: 30, y: 74, s: 12, c: "#C77E9E", label: null },
  { x: 10, y: 52, s: 10, c: "#5FB39A", label: null },
  { x: 46, y: 12, s: 10, c: "#6E9FCC", label: null },
];

const WIRES: [number, number][] = [[0, 1], [0, 5], [0, 4], [1, 2], [2, 3], [3, 4], [1, 6]];

/** Fond « constellation » décoratif, theme-aware (grille de points + orbes + fils).
 *  Utilisé par la page de connexion et l'onboarding — pur CSS/SVG, zéro dépendance. */
export function NeuralBg({ className }: { className?: string }) {
  return (
    <div
      aria-hidden
      className={cn("pointer-events-none absolute inset-0 overflow-hidden", className)}
      style={{
        background:
          "radial-gradient(ellipse 70% 55% at 50% 42%, var(--neural-halo), transparent 72%)," +
          "radial-gradient(var(--canvas-dot) 1px, transparent 1.2px)," +
          "var(--canvas-bg)",
        backgroundSize: "auto, 28px 28px, auto",
      }}
    >
      <svg className="absolute inset-0 h-full w-full" viewBox="0 0 100 100" preserveAspectRatio="none">
        {WIRES.map(([a, b], i) => (
          <line
            key={i}
            x1={NODES[a].x} y1={NODES[a].y} x2={NODES[b].x} y2={NODES[b].y}
            stroke="var(--canvas-wire)" strokeWidth="0.12"
            strokeDasharray={i === WIRES.length - 1 ? "0.5 1.5" : undefined}
            vectorEffect="non-scaling-stroke"
          />
        ))}
      </svg>
      {NODES.map((n, i) => (
        <span
          key={i}
          className="lucid-orb absolute rounded-full"
          style={{
            left: `${n.x}%`,
            top: `${n.y}%`,
            width: n.s,
            height: n.s,
            transform: "translate(-50%, -50%)",
            background: `radial-gradient(circle at 35% 30%, rgba(255,255,255,0.65), color-mix(in srgb, ${n.c} 80%, var(--color-surface)) 55%, color-mix(in srgb, ${n.c} 20%, transparent) 100%)`,
            boxShadow: `0 0 ${n.s}px ${n.s / 6}px color-mix(in srgb, ${n.c} 28%, transparent)`,
            animationDelay: `${i * 0.9}s`,
          }}
        >
          {n.label && (
            <span
              className="absolute left-1/2 top-full mt-1.5 -translate-x-1/2 whitespace-nowrap font-mono text-[9px] tracking-[0.08em]"
              style={{ color: "var(--canvas-label-dim)" }}
            >
              {n.label}
            </span>
          )}
        </span>
      ))}
    </div>
  );
}

/** L'orbe-logo Lucid (le nœud central du graphe, en médaillon). */
export function LucidOrb({ size = 40 }: { size?: number }) {
  return (
    <span
      aria-hidden
      className="lucid-orb inline-block rounded-full"
      style={{
        width: size,
        height: size,
        background:
          "radial-gradient(circle at 35% 30%, #ffffffb0, color-mix(in srgb, var(--color-accent) 85%, var(--color-surface)) 55%, color-mix(in srgb, var(--color-accent) 25%, transparent) 100%)",
        boxShadow: "0 0 24px 4px color-mix(in srgb, var(--color-accent) 35%, transparent)",
      }}
    />
  );
}
