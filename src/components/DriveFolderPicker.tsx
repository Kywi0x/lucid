import { useCallback, useEffect, useRef, useState } from "react";
import { Check, ChevronRight, File, FileText, Folder, HardDrive, Loader2, Minus, Search, Sigma, Users, X } from "lucide-react";
import {
  googleDriveRoots,
  googleDriveChildren,
  googleDriveFolderCount,
  googleDriveSearchFolders,
  googleDriveFolderLabels,
  googleDriveSelection,
  googleDriveSetSelection,
  type DriveDoc,
  type DriveFolder,
  type DriveFolderCount,
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
 * Les compteurs sont revenus le 24/08/2026, sans reprendre le défaut qui les
 * avait fait retirer (retour du test tiers du 21/08 : « la sélection d'un dossier
 * n'annonce ni le nombre de sous-dossiers ni le nombre de documents éligibles »,
 * or c'est exactement ce qui décide quoi cocher) :
 * - au **dépliage**, les chiffres du niveau direct sortent de la requête qui
 *   charge déjà les sous-dossiers — zéro requête de plus qu'avant ;
 * - le total récursif — ce que cocher ramènerait vraiment — reste **à la
 *   demande**, un clic par dossier. C'est lui qui figeait l'écran quand il
 *   tournait pour chaque ligne.
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
  // Niveau direct, rempli au dépliage (gratuit) ; total récursif, sur clic.
  const [direct, setDirect] = useState<Map<string, DirectInfo>>(new Map());
  const [deep, setDeep] = useState<Map<string, DriveFolderCount>>(new Map());
  const [counting, setCounting] = useState<Set<string>>(new Set());
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
      const res = await googleDriveChildren(id);
      setKids((prev) => new Map(prev).set(id, res.folders));
      remember(res.folders);
      setDirect((prev) => new Map(prev).set(id, {
        files: res.files, docs: res.docs, ignored: res.ignored, truncated: res.truncated,
      }));
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
    // Cocher, c'est le moment où le chiffre sert : « qu'est-ce que je viens de
    // prendre ? ». On le lance donc tout seul ici — un clic de l'utilisateur, un
    // comptage — au lieu d'attendre qu'il trouve le bouton (retour Liam, 24/08).
    if (!selected.has(id) && !deep.has(id) && !counting.has(id)) void count(id);
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

  /** Total récursif d'un dossier — le seul chiffre qui dise ce que cocher ramène,
   *  la sélection descendant tout le sous-arbre. Déclenché par l'utilisateur. */
  async function count(id: string) {
    setCounting((prev) => new Set(prev).add(id));
    try {
      const total = await googleDriveFolderCount(id);
      setDeep((prev) => new Map(prev).set(id, total));
    } catch (e) {
      setError(String(e));
    } finally {
      setCounting((prev) => { const next = new Set(prev); next.delete(id); return next; });
    }
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
  const treeProps = { kids, loading, selected, open, direct, deep, counting, inheritedFrom, hasSelectedDescendant, toggleExpand, toggle, count };

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

/** Niveau direct d'un dossier : ce qu'il contient, nommé ET compté. */
type DirectInfo = { files: DriveDoc[]; docs: number; ignored: number; truncated: boolean };

type TreeProps = {
  nodes: DriveFolder[];
  depth: number;
  kids: Map<string, DriveFolder[]>;
  loading: Set<string>;
  selected: Set<string>;
  open: Set<string>;
  direct: Map<string, DirectInfo>;
  deep: Map<string, DriveFolderCount>;
  counting: Set<string>;
  inheritedFrom: (id: string, sel: Set<string>) => boolean;
  hasSelectedDescendant: (id: string, sel: Set<string>) => boolean;
  toggleExpand: (id: string) => void;
  toggle: (id: string) => void;
  count: (id: string) => void;
};

const s = (n: number) => (n > 1 ? "s" : "");

/** Ce qu'on annonce à droite d'une ligne. Le total récursif prend la main dès
 *  qu'il existe : il répond à la vraie question (« qu'est-ce que je coche ? »).
 *  `≥` quand un plafond a été atteint — un minorant se dit, il ne s'arrondit pas. */
function meta(
  id: string,
  p: Pick<TreeProps, "direct" | "deep" | "kids">,
): string | null {
  const d = p.deep.get(id);
  if (d) {
    const ge = d.truncated ? "≥ " : "";
    const parts = [`${ge}${d.docs} document${s(d.docs)}`];
    if (d.folders > 0) parts.push(`${ge}${d.folders} sous-dossier${s(d.folders)}`);
    if (d.ignored > 0) parts.push(`${d.ignored} illisible${s(d.ignored)}`);
    return `${parts.join(" · ")} au total`;
  }
  const one = p.direct.get(id);
  if (!one) return null;
  const subs = p.kids.get(id)?.length ?? 0;
  const ge = one.truncated ? "≥ " : "";
  const parts: string[] = [];
  if (subs > 0) parts.push(`${subs} sous-dossier${s(subs)}`);
  parts.push(`${ge}${one.docs} document${s(one.docs)} ici`);
  if (one.ignored > 0) parts.push(`${one.ignored} illisible${s(one.ignored)}`);
  return parts.join(" · ");
}

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
              meta={meta(f.id, p)}
              counting={p.counting.has(f.id)}
              counted={p.deep.has(f.id)}
              onCount={() => p.count(f.id)}
              onToggleExpand={() => p.toggleExpand(f.id)}
              onToggle={() => p.toggle(f.id)}
            />
            {expanded && loaded !== undefined && (
              <>
                {/* Les fichiers d'abord : c'est ce que l'utilisateur vient
                    vérifier (« mon doc est-il bien vu ? »). Un compteur seul ne
                    répond pas à cette question. */}
                <Files info={p.direct.get(f.id)} depth={p.depth + 1} />
                {loaded.length > 0 && <Tree {...p} nodes={loaded} depth={p.depth + 1} />}
                {loaded.length === 0 && (p.direct.get(f.id)?.files.length ?? 0) === 0 && (
                  <p
                    className="py-1 text-[10px] italic text-[var(--color-muted)]"
                    style={{ paddingLeft: 8 + (p.depth + 1) * 14 + 22 }}
                  >
                    dossier vide
                  </p>
                )}
              </>
            )}
          </div>
        );
      })}
    </>
  );
}

/** Fichiers directs d'un dossier déplié — non cochables (on coche des dossiers),
 *  mais visibles. Les illisibles sont grisés et le disent : jamais tus (ADR-0015). */
function Files({ info, depth }: { info: DirectInfo | undefined; depth: number }) {
  if (!info || info.files.length === 0) return null;
  const hidden = info.docs + info.ignored - info.files.length;
  return (
    <>
      {info.files.map((d) => (
        <div
          key={`${d.name}-${d.readable}`}
          className="flex items-center gap-2 py-0.5 pr-2 text-[11px]"
          style={{ paddingLeft: 8 + depth * 14 + 22 }}
          title={d.readable ? undefined : "Format que Lucid ne sait pas lire"}
        >
          {d.readable
            ? <FileText className="size-3 shrink-0 text-[var(--color-muted)]" />
            : <File className="size-3 shrink-0 text-[var(--color-muted)] opacity-50" />}
          <span className={cn("min-w-0 truncate", d.readable ? "text-[var(--color-text)]" : "text-[var(--color-muted)] line-through")}>
            {d.name}
          </span>
        </div>
      ))}
      {hidden > 0 && (
        <p className="py-0.5 text-[10px] italic text-[var(--color-muted)]" style={{ paddingLeft: 8 + depth * 14 + 22 }}>
          … et {hidden} autre{hidden > 1 ? "s" : ""} fichier{hidden > 1 ? "s" : ""}
        </p>
      )}
    </>
  );
}

type BoxState = "on" | "off" | "partial" | "inherited";

function Row({
  folder, depth, state, expandable, expanded, busy, meta, counting, counted, onCount, onToggleExpand, onToggle,
}: {
  folder: DriveFolder;
  depth: number;
  state: BoxState;
  expandable: boolean;
  expanded: boolean;
  busy: boolean;
  /** Ce que le dossier contient — null tant qu'on n'a rien mesuré. */
  meta: string | null;
  counting: boolean;
  counted: boolean;
  onCount: () => void;
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

      {meta && (
        <span className="shrink-0 text-[10px] tabular-nums text-[var(--color-muted)]">{meta}</span>
      )}

      {/* Le total récursif ne s'affiche pas tout seul pour chaque ligne : cette
          énumération-là figeait l'écran. Il part au cochage (cf. `toggle`) ou
          sur ce bouton — **libellé en clair** : l'icône Σ seule n'était pas
          comprise (retour Liam, 24/08). */}
      {!counted && (
        <button
          onClick={onCount}
          disabled={counting}
          title="Compter les documents, sous-dossiers inclus"
          className="flex shrink-0 items-center gap-1 rounded px-1.5 py-0.5 text-[10px] text-[var(--color-muted)] underline decoration-dotted underline-offset-2 hover:bg-[var(--color-surface)] hover:text-[var(--color-accent)] transition-colors"
        >
          {counting
            ? <><Loader2 className="size-3 animate-spin" /> comptage…</>
            : <><Sigma className="size-3" /> compter</>}
        </button>
      )}
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
