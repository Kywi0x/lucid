import { useEffect, useRef, useState } from "react";
import { Pause, Play } from "lucide-react";
import { cn } from "@/lib/utils";

const DAY = 86_400_000;
const REPLAY_MS = 10_000; // durée d'un replay complet

interface Props {
  /** Bornes (epoch ms) des dates du graphe. */
  min: number;
  max: number;
  /** Curseur temporel courant (epoch ms). */
  value: number;
  onChange: (v: number) => void;
}

/** Slider temporel permanent : quasi invisible au repos (mini ▶ + trait fin),
 *  s'ouvre en pilule au survol ou dès qu'on scrub/rejoue. */
export function TimelineBar({ min, max, value, onChange }: Props) {
  const [playing, setPlaying] = useState(false);
  const [hover, setHover] = useState(false);
  const raf = useRef(0);

  // Replay : anime le curseur de sa position courante jusqu'à max, easing doux.
  useEffect(() => {
    if (!playing) return;
    const from = value >= max ? min : value;
    const span = max - from;
    const dur = REPLAY_MS * (span / (max - min || 1));
    let start: number | null = null;
    const tick = (ts: number) => {
      if (start === null) start = ts;
      const t = Math.min(1, (ts - start) / (dur || 1));
      const e = 1 - Math.pow(1 - t, 2); // easeOutQuad : rapide au début, se pose à la fin
      onChange(from + span * e);
      if (t < 1) raf.current = requestAnimationFrame(tick);
      else setPlaying(false);
    };
    raf.current = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing]);

  const atEnd = value >= max;
  const expanded = playing || !atEnd || hover;
  const label = atEnd
    ? "Aujourd'hui"
    : new Date(value).toLocaleDateString("fr-FR", { day: "numeric", month: "short", year: "numeric" });

  return (
    <div
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
      className={cn(
        "pointer-events-auto flex items-center rounded-full transition-all duration-300",
        expanded
          ? "gap-3 border border-[var(--color-border)] bg-[var(--color-surface)]/75 py-1.5 pl-1.5 pr-3 shadow-[var(--shadow-float)] backdrop-blur-md"
          : "gap-2 border border-transparent px-1.5 py-1.5 opacity-30 hover:opacity-100",
      )}
    >
      <button
        onClick={() => setPlaying((p) => !p)}
        title={playing ? "Pause" : "Rejouer la naissance du cerveau"}
        className={cn(
          "flex shrink-0 items-center justify-center rounded-full transition-all duration-300",
          expanded
            ? "size-7 bg-[var(--color-accent)] text-white hover:bg-[var(--color-accent-hover)]"
            : "size-5 text-[var(--color-muted)]",
        )}
      >
        {playing ? <Pause className="size-3" /> : <Play className="ml-0.5 size-3" />}
      </button>
      <input
        type="range"
        min={min}
        max={max}
        step={DAY}
        value={value}
        onChange={(e) => { setPlaying(false); onChange(Number(e.target.value)); }}
        className={cn(
          "cursor-pointer transition-all duration-300",
          expanded ? "h-auto w-64" : "h-1 w-36 opacity-60",
        )}
        style={{ accentColor: "var(--color-accent)" }}
      />
      <span
        className={cn(
          "shrink-0 overflow-hidden whitespace-nowrap text-center text-[11px] tabular-nums text-[var(--color-muted)] transition-all duration-300",
          expanded ? "w-20" : "w-0",
        )}
      >
        {label}
      </span>
    </div>
  );
}
