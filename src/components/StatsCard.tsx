import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { DAYS, dayKey, toCells, type BrainStats } from "@/lib/stats";
import type { ArchivistStats } from "@/lib/api";

// Relevé d'activité — cf. coffre LucidFlow, « Relevé d'activité (valeur mesurée
// du cerveau) » (2026-08-25) et ADR-0025.
//
// Composition « widget macOS » (arbitrage Liam, 2026-09-05) : un en-tête
// discret, UN héros, UN graphique. Ce qui a été retiré, et pourquoi :
//   • les libellés en majuscules espacées — vocabulaire dashboard, pas Apple :
//     la valeur est en gros, le descripteur DESSOUS en casse normale ;
//   • les filets de séparation — chez Apple ils appartiennent aux listes de
//     Réglages ; dans un widget, c'est l'espacement qui groupe ;
//   • la grille 14 cases — c'est le graphe de contributions GitHub. Un
//     histogramme (Temps d'écran, Batterie) dit la même chose en langue Apple,
//     et donne une VRAIE échelle de hauteur là où l'opacité plafonnait vite ;
//   • la barre de progression — remplie à 78 %, l'œil lit « quota consommé »
//     alors qu'il s'agit d'une économie.
//
// Deux règles de la note tiennent toujours : des chiffres calculés (jamais de
// valeur de remplissage), et des barres qui mesurent les consultations PAR
// L'IA, pas la présence de l'utilisateur.

const BAR_H = 15;   // hauteur du graphique, px (maquette « app vivante »)
const BAR_MIN = 3;  // moignon d'un jour vide : le rythme reste lisible

const fmtDay = new Intl.DateTimeFormat("fr-FR", { weekday: "short", day: "numeric", month: "short" });
const fmtSince = new Intl.DateTimeFormat("fr-FR", { day: "numeric", month: "long", year: "numeric" });
const fmtCompact = new Intl.NumberFormat("fr-FR", { notation: "compact", maximumFractionDigits: 1 });
const fmtFull = new Intl.NumberFormat("fr-FR");

function label(back: number): string {
  if (back === 0) return "aujourd'hui";
  if (back === 1) return "hier";
  // `dayKey` rend une date UTC : on la relit en UTC pour ne pas décaler d'un jour.
  return fmtDay.format(new Date(`${dayKey(back)}T12:00:00Z`));
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n > 1 ? many : one}`;
}

const REDUCED = () =>
  typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Compte de 0 jusqu'à `target`. Le chiffre qui monte est la signature Apple
 * (Fitness, Temps d'écran) : c'est ce qui fait exister la valeur comme un
 * résultat plutôt que comme une étiquette. Respecte `prefers-reduced-motion`.
 */
function useCountUp(target: number | null, ms = 1100): number | null {
  const [v, setV] = useState<number | null>(target === null ? null : 0);
  const raf = useRef(0);
  useEffect(() => {
    if (target === null) { setV(null); return; }
    if (REDUCED()) { setV(target); return; }
    const t0 = performance.now();
    const tick = (t: number) => {
      const k = Math.min(1, (t - t0) / ms);
      // Même sortie que --sb-ease : décélération franche, pas de rebond.
      setV(Math.round(target * (1 - Math.pow(1 - k, 3))));
      if (k < 1) raf.current = requestAnimationFrame(tick);
    };
    raf.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf.current);
  }, [target, ms]);
  return v;
}

export function StatsCard({ stats, archivist, className, style }: {
  stats: BrainStats | null;
  /** Métrique n°3 de la note : ce que Lucid a rangé sans l'utilisateur. */
  archivist?: ArchivistStats | null;
  className?: string;
  style?: React.CSSProperties;
}) {
  // Les barres comptent les ÉCHANGES (rafales ≈ questions), pas les appels :
  // une question produit 1 recherche + N lectures, et compter les appels
  // gonflait l'activité d'un facteur ~4. Le détail au survol montre les deux.
  const cells = toCells(stats?.days ?? []);
  const callCells = toCells(stats?.calls ?? []);
  const [hovered, setHovered] = useState<number | null>(null);
  const [grown, setGrown] = useState(false);

  const total = cells.reduce((a, b) => a + b, 0);
  const max = Math.max(0, ...cells);
  const tokens = useCountUp(stats?.tokensSaved ?? null);
  const tidied = archivist ? archivist.moved + archivist.merged : 0;

  useEffect(() => {
    const id = requestAnimationFrame(() => setGrown(true));
    return () => cancelAnimationFrame(id);
  }, []);

  // L'en-tête porte le contexte au repos, le détail du jour au survol — plutôt
  // qu'une infobulle flottante, qui n'existe pas dans un widget macOS.
  const head =
    hovered != null
      ? `${label(DAYS - 1 - hovered)} · ${plural(cells[hovered], "échange")}`
      : total > 0 ? `${plural(total, "échange")} sur ${DAYS} jours`
      : `Ces ${DAYS} derniers jours`;

  const foot = [
    stats?.savedPct != null ? `${Math.round(stats.savedPct)} % de moins` : null,
    tidied > 0 ? `${plural(tidied, "fichier")} rangé${tidied > 1 ? "s" : ""} sans toi` : null,
  ].filter(Boolean).join(" · ");

  return (
    <div className={cn("stats lg", className)} style={style}>
      {/* ── En-tête discret : contexte au repos, détail du jour au survol ── */}
      <span className="st-head">{head}</span>

      {/* ── Le chiffre de une ── */}
      <div className="st-hero">
        <span
          className="n"
          title={stats?.tokensSaved != null ? `${fmtFull.format(stats.tokensSaved)} tokens` : undefined}
        >
          {tokens == null ? "—" : fmtCompact.format(tokens)}
        </span>
        <span className="u">tokens évités</span>
      </div>

      {/* ── Le graphique : un seul, et il porte l'échelle ── */}
      <div className="bars" onMouseLeave={() => setHovered(null)}>
        {cells.map((count, i) => {
          const h = count === 0 ? 2 : Math.max(BAR_MIN, Math.round((count / Math.max(1, max)) * BAR_H));
          return (
            <div
              key={i}
              onMouseEnter={() => setHovered(i)}
              title={
                `${label(DAYS - 1 - i)} — ${plural(count, "échange")}, ` +
                `${plural(callCells[i], "appel")}\n` +
                "Un échange = des appels espacés de moins de 2 min " +
                "(regroupement par inactivité, pas une frontière exacte)."
              }
            >
              <i
                className={count === 0 ? "z" : undefined}
                style={{
                  height: grown ? h : BAR_MIN,
                  transitionDelay: `${i * 26}ms`,
                  opacity: hovered == null || hovered === i ? 1 : 0.34,
                }}
              />
            </div>
          );
        })}
      </div>

      {/* ── Pied : le reste des chiffres, jamais en concurrence avec le héros ── */}
      {foot && (
        <span
          className="st-foot"
          title={
            archivist && tidied > 0
              ? `${plural(archivist.moved, "page déplacée", "pages déplacées")}, ` +
                `${plural(archivist.merged, "doublon fusionné", "doublons fusionnés")}` +
                (archivist.since ? `\ndepuis le ${fmtSince.format(new Date(archivist.since * 1000))}` : "")
              : undefined
          }
        >
          {foot}
        </span>
      )}
    </div>
  );
}
