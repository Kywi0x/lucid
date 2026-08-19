import { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronRight, Folder, HardDrive, Loader2, Minus, Search, Users, X } from "lucide-react";
import {
  googleDriveRoots,
  googleDriveChildren,
  googleDriveSearchFolders,
  googleDriveFolderLabels,
  googleDriveSelection,
  googleDriveSetSelection,
  type DriveFolder,
} from "@/lib/api";
import { cn } from "@/lib/utils";

type Props = {
  onClose: () => void;
  /** Appelé après enregistrement — le parent affiche le message et relance une synchro. */
  onSaved: (msg: string) => void;
};

/**
 * Écran de choix des dossiers Drive — **chargement paresseux**.
 *
 * Mesuré le 18/08/2026 sur un compte professionnel (16 unités partagées) :
 * lister tous les dossiers d'un coup, c'était 24 694 lignes en 42 s, et compter
 * les documents de chacun demandait d'énumérer plus de 100 000 objets — l'écran
 * restait sur « comptage… » plusieurs minutes. On charge donc les racines
 * (~150 entrées, 2 s) puis une requête par dépliage.
 *
 * ponytail: plus de compteurs de documents. Les afficher imposait l'énumération
 * complète, pour un chiffre indicatif ; le nom du dossier suffit à choisir.
 */
export function DriveFolderPicker({ onClose, onSaved }: Props) {
  const [roots, setRoots] = useState<DriveFolder[] | null>(null);
  const [kids, setKids] = useState<Map<string, DriveFolder[]>>(new Map());
  const [byId, setById] = useState<Map<string, DriveFolder>>(new Map());
  const [loading, setLoading] = useState<Set<string>>(new Set());
  const [open, setOpen] = useState<Set<string>>(new Set());
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [initial, setInitial] = useState<Set<string>>(new Set());
  const [orphans, setOrphans] = useState(false);
  const [initialOrphans, setInitialOrphans] = useState(false);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<DriveFolder[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [saving, setSaving] = useState(false);
  const dirtyRef = useRef(false);

  const remember = useCallback((list: DriveFolder[]) => {
    setById((prev) => {
      const next = new Map(prev);
      for (const f of list) next.set(f.id, f);
      return next;
    });
  }, []);

  useEffect(() => {
    let alive = true;
    Promise.all([googleDriveRoots(), googleDriveSelection()])
      .then(([r, sel]) => {
        if (!alive) return;
        setRoots(r);
        remember(r);
        setSelected(new Set(sel.folders));
        setInitial(new Set(sel.folders));
        setOrphans(sel.include_orphans);
        setInitialOrphans(sel.include_orphans);
        // Les cochés ne sont pas forcément dans l'arbre chargé : sans leurs noms,
        // impossible de dire ce qui est synchronisé.
        if (sel.folders.length) googleDriveFolderLabels(sel.folders).then(remember).catch(() => {});
      })
      .catch((e) => alive && setError(String(e)));
    return () => { alive = false; };
  }, [remember]);

  // Fermer avec des modifications non enregistrées les jetait sans un mot :
  // Liam a coché des dossiers, fermé, rouvert, et tout avait disparu (18/08/2026).
  const closeGuarded = useCallback(() => {
    if (dirtyRef.current && !confirm("Modifications non enregistrées. Fermer quand même ?")) return;
    onClose();
  }, [onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.stopPropagation(); closeGuarded(); }
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [closeGuarded]);

  // Recherche côté Drive : l'arbre n'est pas en mémoire, on ne peut pas filtrer localement.
  useEffect(() => {
    const q = query.trim();
    if (q.length < 2) { setHits(null); setSearching(false); return; }
    setSearching(true);
    const t = setTimeout(() => {
      googleDriveSearchFolders(q)
        .then((r) => { setHits(r); remember(r); })
        .catch((e) => setError(String(e)))
        .finally(() => setSearching(false));
    }, 350);
    return () => clearTimeout(t);
  }, [query, remember]);

  async function toggleExpand(id: string) {
    const wasOpen = open.has(id);
    setOpen((prev) => {
      const next = new Set(prev);
      if (wasOpen) next.delete(id); else next.add(id);
      return next;
    });
    if (wasOpen || kids.has(id)) return;
    setLoading((prev) => new Set(prev).add(id));
    try {
      const list = await googleDriveChildren(id);
      setKids((prev) => new Map(prev).set(id, list));
      remember(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading((prev) => { const next = new Set(prev); next.delete(id); return next; });
    }
  }

  /** L'un des ancêtres **chargés** de `id` est-il coché ? (`id` lui-même exclu) */
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

  /** Un descendant chargé est-il coché ? (case « partielle ») */
  function hasSelectedDescendant(id: string, sel: Set<string>): boolean {
    for (const c of kids.get(id) ?? []) {
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
        // Un descendant déjà chargé et coché devient redondant : le parent le couvre.
        const drop = (fid: string) => {
          for (const c of kids.get(fid) ?? []) { next.delete(c.id); drop(c.id); }
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

  const dirty =
    orphans !== initialOrphans ||
    selected.size !== initial.size ||
    [...selected].some((id) => !initial.has(id));

  dirtyRef.current = dirty;

  const mine = (roots ?? []).filter((f) => !f.shared);
  const shared = (roots ?? []).filter((f) => f.shared);
  const treeProps = { kids, loading, selected, open, inheritedFrom, hasSelectedDescendant, toggleExpand, toggle };

  return (
    <div className="absolute inset-0 z-20 flex flex-col justify-end bg-black/20" onClick={closeGuarded}>
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
            {/* Vérifié le 18/08/2026 : décocher retire le dossier du suivi, mais
                les documents restent dans brain.db. Le taire ferait croire à une
                suppression — ou à une conservation, selon ce que l'utilisateur
                imagine. Même règle que les sources passées en stand-by le 06/08. */}
            <p className="mt-0.5 text-[10px] text-[var(--color-muted)]">
              Décocher arrête le suivi — les documents déjà indexés restent dans ton
              cerveau, mais ne seront plus mis à jour.
            </p>
          </div>
          <button
            onClick={closeGuarded}
            className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] transition-colors"
          >
            <X className="size-4" />
          </button>
        </div>

        {/* Recherche */}
        <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-4 py-2">
          {searching
            ? <Loader2 className="size-3.5 shrink-0 animate-spin text-[var(--color-muted)]" />
            : <Search className="size-3.5 shrink-0 text-[var(--color-muted)]" />}
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Chercher un dossier par nom…"
            className="w-full bg-transparent text-xs text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]"
          />
        </div>

        {/* Ce qui est réellement synchronisé — la question de Liam : « voir les
            dossiers synchronisés et pouvoir les desync ». L'arbre paresseux ne
            peut pas la montrer (un dossier coché en profondeur n'est pas chargé),
            cette liste si. */}
        {selected.size > 0 && (
          <div className="flex flex-wrap gap-1.5 border-b border-[var(--color-border)] px-4 py-2">
            <span className="w-full text-[10px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
              Dossiers indexés ({selected.size})
            </span>
            {[...selected].map((id) => (
              <button
                key={id}
                onClick={() => toggle(id)}
                title="Retirer de la sélection"
                className="flex max-w-full items-center gap-1 rounded-full border border-[var(--color-border)] bg-[var(--color-surface-2)] px-2 py-0.5 text-[11px] text-[var(--color-text)] hover:border-[var(--color-err)] hover:text-[var(--color-err)] transition-colors"
              >
                <Folder className="size-3 shrink-0" />
                <span className="truncate">{byId.get(id)?.name ?? "dossier…"}</span>
                <X className="size-3 shrink-0" />
              </button>
            ))}
          </div>
        )}

        {/* Arbre */}
        <div className="min-h-0 flex-1 overflow-y-auto px-2 py-2">
          {error && <p className="px-2 py-3 text-xs text-[var(--color-err)]">{error}</p>}

          {!roots && !error && (
            <p className="flex items-center gap-2 px-2 py-6 text-xs text-[var(--color-muted)]">
              <Loader2 className="size-3.5 animate-spin" /> Lecture des dossiers…
            </p>
          )}

          {hits !== null ? (
            hits.length === 0
              ? <p className="px-2 py-6 text-xs text-[var(--color-muted)]">Aucun dossier ne correspond.</p>
              // Même composant que l'arbre : un résultat de recherche se déplie
              // comme un dossier ordinaire (il ne le faisait pas — 18/08/2026).
              : <Tree nodes={hits} depth={0} {...treeProps} />
          ) : roots && (
            <>
              <Tree nodes={mine} depth={0} {...treeProps} />

              {shared.length > 0 && (
                <>
                  <p className="mt-3 flex items-center gap-1.5 px-2 pb-1 text-[10px] font-medium uppercase tracking-wide text-[var(--color-muted)]">
                    <Users className="size-3" /> Partagés avec moi
                  </p>
                  <Tree nodes={shared} depth={0} {...treeProps} />
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
                </button>
              </div>
            </>
          )}
        </div>

        {/* Pied */}
        <div className="flex flex-col gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <span className="text-[11px] text-[var(--color-muted)]">
            {selected.size === 0
              ? "Tout le Drive sera indexé"
              : `${selected.size} dossier${selected.size > 1 ? "s" : ""} sélectionné${selected.size > 1 ? "s" : ""}`}
          </span>

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
  kids: Map<string, DriveFolder[]>;
  loading: Set<string>;
  selected: Set<string>;
  open: Set<string>;
  inheritedFrom: (id: string, sel: Set<string>) => boolean;
  hasSelectedDescendant: (id: string, sel: Set<string>) => boolean;
  toggleExpand: (id: string) => void;
  toggle: (id: string) => void;
};

function Tree(p: TreeProps) {
  return (
    <>
      {p.nodes.map((f) => {
        const loaded = p.kids.get(f.id);
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
              state={state}
              // On ne sait pas si un dossier a des enfants sans le demander :
              // la flèche est donc toujours offerte, et un dossier vide le dit.
              expandable
              expanded={expanded}
              busy={p.loading.has(f.id)}
              onToggleExpand={() => p.toggleExpand(f.id)}
              onToggle={() => p.toggle(f.id)}
            />
            {expanded && (
              loaded === undefined
                ? null
                : loaded.length === 0
                  ? (
                    <p
                      className="py-1 text-[10px] italic text-[var(--color-muted)]"
                      style={{ paddingLeft: 8 + (p.depth + 1) * 14 + 22 }}
                    >
                      aucun sous-dossier
                    </p>
                  )
                  : <Tree {...p} nodes={loaded} depth={p.depth + 1} />
            )}
          </div>
        );
      })}
    </>
  );
}

type BoxState = "on" | "off" | "partial" | "inherited";

function Row({
  folder, depth, state, expandable, expanded, busy, onToggleExpand, onToggle,
}: {
  folder: DriveFolder;
  depth: number;
  state: BoxState;
  expandable: boolean;
  expanded: boolean;
  busy: boolean;
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
          expanded && !busy && "rotate-90",
        )}
      >
        {busy ? <Loader2 className="size-3 animate-spin" /> : <ChevronRight className="size-3" />}
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
        {folder.shared && depth === 0
          ? <HardDrive className="size-3.5 shrink-0 text-[var(--color-muted)]" />
          : <Folder className="size-3.5 shrink-0 text-[var(--color-muted)]" />}
        <span className="min-w-0 flex-1 truncate text-xs text-[var(--color-text)]" title={folder.name}>
          {folder.name}
        </span>
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
