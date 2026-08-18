import { useEffect, useMemo, useState } from "react";
import { Check, ChevronRight, Folder, Loader2, Minus, Search, Users, X } from "lucide-react";
import {
  googleDriveFolders,
  googleDriveFolderCounts,
  googleDriveSelection,
  googleDriveSetSelection,
  type DriveFolder,
} from "@/lib/api";
import { cn } from "@/lib/utils";

/** Clé du bac « fichiers sans dossier » côté Rust (racine du Drive, partage non indexé). */
const ORPHANS = "";

type Props = {
  onClose: () => void;
  /** Appelé après enregistrement — le parent affiche le message et relance une synchro. */
  onSaved: (msg: string) => void;
};

/**
 * Écran de choix des dossiers Drive.
 *
 * Ne liste que les dossiers (rapide même sur un Drive de 1 To) ; les compteurs de
 * documents arrivent en arrière-plan et ne bloquent jamais l'ouverture — sur un très
 * gros compte l'énumération complète est justement ce qui peut traîner.
 */
export function DriveFolderPicker({ onClose, onSaved }: Props) {
  const [folders, setFolders] = useState<DriveFolder[] | null>(null);
  const [error, setError] = useState("");
  const [counts, setCounts] = useState<Record<string, number> | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [initial, setInitial] = useState<Set<string>>(new Set());
  const [orphans, setOrphans] = useState(false);
  const [initialOrphans, setInitialOrphans] = useState(false);
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let alive = true;
    Promise.all([googleDriveFolders(), googleDriveSelection()])
      .then(([f, sel]) => {
        if (!alive) return;
        setFolders(f);
        setSelected(new Set(sel.folders));
        setInitial(new Set(sel.folders));
        setOrphans(sel.include_orphans);
        setInitialOrphans(sel.include_orphans);
      })
      .catch((e) => alive && setError(String(e)));
    // Compteurs en fond : l'écran est utilisable sans eux.
    googleDriveFolderCounts().then((c) => alive && setCounts(c)).catch(() => {});
    return () => { alive = false; };
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); onClose(); }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [onClose]);

  // `kidsOf` et pas `children` : passé en prop à <Tree>, le nom `children` serait
  // capté par React comme contenu de l'élément.
  const { kidsOf, byId, roots, shared } = useMemo(() => {
    const kidsOf = new Map<string, DriveFolder[]>();
    const byId = new Map<string, DriveFolder>();
    for (const f of folders ?? []) byId.set(f.id, f);
    for (const f of folders ?? []) {
      const key = f.parent ?? "__root__";
      const list = kidsOf.get(key) ?? [];
      list.push(f);
      kidsOf.set(key, list);
    }
    for (const list of kidsOf.values()) list.sort((a, b) => a.name.localeCompare(b.name, "fr"));
    const top = kidsOf.get("__root__") ?? [];
    return {
      kidsOf,
      byId,
      roots: top.filter((f) => !f.shared),
      shared: top.filter((f) => f.shared),
    };
  }, [folders]);

  /** Documents d'un dossier, ses descendants compris. */
  const totalOf = useMemo(() => {
    const cache = new Map<string, number>();
    const walk = (id: string): number => {
      const hit = cache.get(id);
      if (hit !== undefined) return hit;
      cache.set(id, 0); // garde-fou cycle
      let n = counts?.[id] ?? 0;
      for (const c of kidsOf.get(id) ?? []) n += walk(c.id);
      cache.set(id, n);
      return n;
    };
    return walk;
  }, [counts, kidsOf]);

  const everything = useMemo(
    () => Object.values(counts ?? {}).reduce((a, b) => a + b, 0),
    [counts],
  );

  /** Total ingéré pour une sélection donnée — sert aussi à chiffrer ce qu'on retire. */
  function totalFor(sel: Set<string>, withOrphans: boolean): number {
    if (sel.size === 0) return everything;
    let n = withOrphans ? counts?.[ORPHANS] ?? 0 : 0;
    for (const id of sel) {
      // Un dossier dont un ancêtre est déjà coché serait compté deux fois.
      if (!inheritedFrom(id, sel)) n += totalOf(id);
    }
    return n;
  }

  /** L'un des ancêtres de `id` est-il coché ? (`id` lui-même exclu) */
  function inheritedFrom(id: string, sel: Set<string>): boolean {
    let cur = byId.get(id)?.parent ?? null;
    const seen = new Set<string>();
    while (cur && !seen.has(cur)) {
      seen.add(cur);
      if (sel.has(cur)) return true;
      cur = byId.get(cur)?.parent ?? null;
    }
    return false;
  }

  function hasSelectedDescendant(id: string, sel: Set<string>): boolean {
    for (const c of kidsOf.get(id) ?? []) {
      if (sel.has(c.id) || hasSelectedDescendant(c.id, sel)) return true;
    }
    return false;
  }

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
        // Un descendant coché devient redondant — on le retire pour garder une
        // sélection lisible (et un compteur juste).
        const drop = (fid: string) => {
          for (const c of kidsOf.get(fid) ?? []) { next.delete(c.id); drop(c.id); }
        };
        drop(id);
      }
      return next;
    });
  }

  async function save() {
    setSaving(true);
    try {
      await googleDriveSetSelection([...selected], orphans);
      onSaved(
        selected.size === 0
          ? "Tout le Drive sera indexé."
          : `${selected.size} dossier${selected.size > 1 ? "s" : ""} sélectionné${selected.size > 1 ? "s" : ""} — lance une synchronisation.`,
      );
      onClose();
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  }

  const before = totalFor(initial, initialOrphans);
  const after = totalFor(selected, orphans);
  const removed = counts ? Math.max(0, before - after) : 0;
  const dirty =
    orphans !== initialOrphans ||
    selected.size !== initial.size ||
    [...selected].some((id) => !initial.has(id));

  const matches = query.trim().toLowerCase();
  const flat = matches
    ? (folders ?? []).filter((f) => f.name.toLowerCase().includes(matches)).slice(0, 200)
    : [];

  return (
    <div className="absolute inset-0 z-20 flex flex-col justify-end bg-black/20" onClick={onClose}>
      <div
        className="m-3 flex max-h-[85%] flex-col overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center gap-3 border-b border-[var(--color-border)] px-4 py-3">
          <div className="flex-1">
            <div className="text-sm font-semibold">Dossiers à indexer</div>
            <p className="text-[11px] text-[var(--color-muted)]">
              Lucid ne lira que ce que tu coches. Aucune case = tout le Drive.
            </p>
          </div>
          <button
            onClick={onClose}
            className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] transition-colors"
          >
            <X className="size-4" />
          </button>
        </div>

        {/* Recherche */}
        <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-4 py-2">
          <Search className="size-3.5 shrink-0 text-[var(--color-muted)]" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Filtrer les dossiers…"
            className="w-full bg-transparent text-xs text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]"
          />
        </div>

        {/* Arbre */}
        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          {error && <p className="px-2 py-3 text-xs text-[var(--color-err)]">{error}</p>}

          {!folders && !error && (
            <p className="flex items-center gap-2 px-2 py-6 text-xs text-[var(--color-muted)]">
              <Loader2 className="size-3.5 animate-spin" /> Lecture de l'arborescence…
            </p>
          )}

          {folders && folders.length === 0 && (
            <p className="px-2 py-6 text-xs text-[var(--color-muted)]">
              Aucun dossier sur ce Drive — tous les fichiers sont à la racine.
            </p>
          )}

          {folders && matches && (
            flat.length === 0
              ? <p className="px-2 py-6 text-xs text-[var(--color-muted)]">Aucun dossier ne correspond.</p>
              : flat.map((f) => (
                  <Row
                    key={f.id}
                    folder={f}
                    depth={0}
                    count={counts ? totalOf(f.id) : null}
                    state={selected.has(f.id) ? "on" : inheritedFrom(f.id, selected) ? "inherited" : "off"}
                    expandable={false}
                    expanded={false}
                    onToggleExpand={() => {}}
                    onToggle={() => toggle(f.id)}
                  />
                ))
          )}

          {folders && !matches && (
            <>
              <Tree
                nodes={roots}
                depth={0}
                {...{ kidsOf, counts, totalOf, selected, inheritedFrom, hasSelectedDescendant, open, setOpen, toggle }}
              />

              {shared.length > 0 && (
                <>
                  <p className="mt-3 flex items-center gap-1.5 px-2 pb-1 text-[10px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
                    <Users className="size-3" /> Partagés avec moi
                  </p>
                  <Tree
                    nodes={shared}
                    depth={0}
                    {...{ kidsOf, counts, totalOf, selected, inheritedFrom, hasSelectedDescendant, open, setOpen, toggle }}
                  />
                </>
              )}

              {/* Sans cette case, les fichiers de la racine disparaîtraient sans un mot. */}
              <div className="mt-3 border-t border-[var(--color-border)] pt-2">
                <button
                  onClick={() => setOrphans((v) => !v)}
                  className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left hover:bg-[var(--color-surface-2)] transition-colors"
                >
                  <Box state={orphans ? "on" : "off"} />
                  <span className="min-w-0 flex-1">
                    <span className="block text-xs text-[var(--color-text)]">
                      Fichiers sans dossier
                    </span>
                    <span className="block text-[10px] text-[var(--color-muted)]">
                      À la racine du Drive ou dans un partage non listé
                    </span>
                  </span>
                  {counts && (
                    <span className="shrink-0 text-[10px] text-[var(--color-muted)]">
                      {counts[ORPHANS] ?? 0}
                    </span>
                  )}
                </button>
              </div>
            </>
          )}
        </div>

        {/* Pied : ce que ça donne concrètement */}
        <div className="flex flex-col gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <div className="flex items-center justify-between text-[11px]">
            <span className="text-[var(--color-muted)]">
              {selected.size === 0
                ? "Tout le Drive"
                : `${selected.size} dossier${selected.size > 1 ? "s" : ""}`}
            </span>
            <span className="text-[var(--color-muted)]">
              {counts === null ? (
                <span className="flex items-center gap-1.5">
                  <Loader2 className="size-3 animate-spin" /> comptage…
                </span>
              ) : (
                `${after} document${after > 1 ? "s" : ""} indexé${after > 1 ? "s" : ""}`
              )}
            </span>
          </div>

          {/* Un retrait n'est pas silencieux : on chiffre ce qui sort du cerveau. */}
          {removed > 0 && (
            <p className="rounded-lg bg-[var(--color-surface-2)] px-2.5 py-1.5 text-[10px] leading-relaxed text-[var(--color-muted)]">
              {removed} document{removed > 1 ? "s" : ""} sortiront de ton cerveau à la
              prochaine synchronisation. Tes fichiers Drive ne sont jamais touchés.
            </p>
          )}

          <div className="flex gap-1.5">
            <button
              onClick={() => { setSelected(new Set()); setOrphans(false); }}
              className="flex-1 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs font-medium text-[var(--color-muted)] hover:border-[var(--color-accent)] hover:text-[var(--color-accent)] transition-colors"
            >
              Tout indexer
            </button>
            <button
              onClick={save}
              disabled={saving || !dirty}
              className="flex flex-1 items-center justify-center gap-2 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-xs font-medium text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-50 transition-colors"
            >
              {saving && <Loader2 className="size-3.5 animate-spin" />}
              Enregistrer
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Arbre ────────────────────────────────────────────────────────────────────

type TreeProps = {
  nodes: DriveFolder[];
  depth: number;
  kidsOf: Map<string, DriveFolder[]>;
  counts: Record<string, number> | null;
  totalOf: (id: string) => number;
  selected: Set<string>;
  inheritedFrom: (id: string, sel: Set<string>) => boolean;
  hasSelectedDescendant: (id: string, sel: Set<string>) => boolean;
  open: Set<string>;
  setOpen: React.Dispatch<React.SetStateAction<Set<string>>>;
  toggle: (id: string) => void;
};

function Tree(p: TreeProps) {
  return (
    <>
      {p.nodes.map((f) => {
        const kids = p.kidsOf.get(f.id) ?? [];
        const expanded = p.open.has(f.id);
        const state = p.selected.has(f.id)
          ? "on"
          : p.inheritedFrom(f.id, p.selected)
            ? "inherited"
            : p.hasSelectedDescendant(f.id, p.selected)
              ? "partial"
              : "off";
        return (
          <div key={f.id}>
            <Row
              folder={f}
              depth={p.depth}
              count={p.counts ? p.totalOf(f.id) : null}
              state={state}
              expandable={kids.length > 0}
              expanded={expanded}
              onToggleExpand={() =>
                p.setOpen((prev) => {
                  const next = new Set(prev);
                  if (next.has(f.id)) next.delete(f.id);
                  else next.add(f.id);
                  return next;
                })
              }
              onToggle={() => p.toggle(f.id)}
            />
            {expanded && kids.length > 0 && <Tree {...p} nodes={kids} depth={p.depth + 1} />}
          </div>
        );
      })}
    </>
  );
}

type BoxState = "on" | "off" | "partial" | "inherited";

function Row({
  folder, depth, count, state, expandable, expanded, onToggleExpand, onToggle,
}: {
  folder: DriveFolder;
  depth: number;
  count: number | null;
  state: BoxState;
  expandable: boolean;
  expanded: boolean;
  onToggleExpand: () => void;
  onToggle: () => void;
}) {
  return (
    <div
      className="flex items-center gap-1 rounded-lg py-1 pr-2 hover:bg-[var(--color-surface-2)] transition-colors"
      style={{ paddingLeft: 8 + depth * 14 }}
    >
      <button
        onClick={onToggleExpand}
        disabled={!expandable}
        className={cn(
          "shrink-0 rounded p-0.5 text-[var(--color-muted)] transition-transform",
          !expandable && "invisible",
          expanded && "rotate-90",
        )}
      >
        <ChevronRight className="size-3" />
      </button>

      {/* Un dossier hérité est coché mais non cliquable : on décoche le parent.
          C'est le seul modèle qui reste lisible sans inventer d'exclusions. */}
      <button
        onClick={onToggle}
        disabled={state === "inherited"}
        title={state === "inherited" ? "Hérité d'un dossier parent" : undefined}
        className={cn(
          "flex min-w-0 flex-1 items-center gap-2 py-0.5 text-left",
          state === "inherited" && "cursor-default opacity-60",
        )}
      >
        <Box state={state} />
        <Folder className="size-3.5 shrink-0 text-[var(--color-muted)]" />
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--color-text)]" title={folder.name}>
          {folder.name}
        </span>
        {count !== null && count > 0 && (
          <span className="shrink-0 text-[10px] text-[var(--color-muted)]">{count}</span>
        )}
      </button>
    </div>
  );
}

function Box({ state }: { state: BoxState }) {
  const filled = state === "on" || state === "inherited";
  return (
    <span
      className={cn(
        "flex size-3.5 shrink-0 items-center justify-center rounded border transition-colors",
        filled
          ? "border-[var(--color-accent)] bg-[var(--color-accent)] text-white"
          : "border-[var(--color-border)] text-[var(--color-accent)]",
      )}
    >
      {filled && <Check className="size-2.5" strokeWidth={3} />}
      {state === "partial" && <Minus className="size-2.5" strokeWidth={3} />}
    </span>
  );
}
