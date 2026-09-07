import { useEffect, useMemo, useState } from "react";
import type { BrainGraph, BrainNode, Space } from "@/lib/types";
import { supabase } from "@/lib/supabase";
import { cn } from "@/lib/utils";
import {
  IconChevron, IconSearch, IconPlus, IconSidebarLeft, IconBrain, IconChat,
} from "./icons";

/** Largeur totale de la sidebar (240 de contenu + 12 de padding de chaque côté). */
export const SIDEBAR_WIDTH = 264;

// Indentation de l'arbre : 24 px par palier, l'icône de pliage occupe le
// premier des deux (maquette Figma « Lucid v2 » — une feuille n'a pas d'icône
// et son libellé retombe donc sur celui de son dossier parent).
const INDENT = 24;
const BASE_PAD = 12;

const LUCID_SPACE: Space = { id: "lucid", name: "Lucid", node_ids: null };

function SectionHeader({
  title, open, onToggle, children,
}: {
  title: string;
  open: boolean;
  onToggle: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex w-full items-center gap-1 pb-[5px] pl-[18px] pr-3 pt-[15px]">
      <button
        type="button"
        onClick={onToggle}
        className="flex-1 text-left text-sm font-medium leading-[14px] text-[var(--sb-label)]"
      >
        {title}
      </button>
      <button
        type="button"
        onClick={onToggle}
        aria-label={open ? `Replier ${title}` : `Déplier ${title}`}
        className="text-[var(--sb-icon)] transition-colors duration-150 hover:text-[var(--sb-label)]"
      >
        <IconChevron
          className={cn(
            "size-4 transition-transform duration-[240ms] [transition-timing-function:var(--sb-ease)]",
            !open && "-rotate-90",
          )}
        />
      </button>
      {children}
    </div>
  );
}

// ── Arbre « Structure » ──────────────────────────────────────────────────────

function StructureTree({
  graph, onSelect, selectedId, query,
}: {
  graph: BrainGraph;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
}) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const q = query.trim().toLowerCase();

  const childrenOf = useMemo(() => {
    const map = new Map<string, BrainNode[]>();
    for (const node of graph.nodes) {
      if (!node.parent_id) continue;
      if (!map.has(node.parent_id)) map.set(node.parent_id, []);
      map.get(node.parent_id)!.push(node);
    }
    return map;
  }, [graph]);

  const rootNode = graph.nodes.find((n) => n.kind === "root");
  const topLevel = childrenOf.get(rootNode?.id ?? "root") ?? [];

  function toggle(id: string) {
    setExpanded((prev) => {
      const next = new Set(prev);
      next.has(id) ? next.delete(id) : next.add(id);
      return next;
    });
  }

  function nodeMatches(node: BrainNode): boolean {
    if (!q) return true;
    if ([node.label, node.summary, ...node.keywords].join(" ").toLowerCase().includes(q)) return true;
    for (const child of childrenOf.get(node.id) ?? []) {
      if (nodeMatches(child)) return true;
    }
    return false;
  }

  function renderNode(node: BrainNode, depth: number): React.ReactNode {
    if (!nodeMatches(node)) return null;
    const children = childrenOf.get(node.id) ?? [];
    const hasChildren = children.length > 0;
    const isOpen = expanded.has(node.id) || !!q;
    const selected = selectedId === node.id;

    return (
      <li key={node.id}>
        <div
          onClick={() => { if (hasChildren) toggle(node.id); onSelect(node); }}
          style={{ paddingLeft: BASE_PAD + depth * INDENT }}
          className={cn(
            "sb-press flex cursor-pointer items-center gap-2 rounded-[6px] py-1 pr-3",
            "transition-colors duration-150 [transition-timing-function:var(--sb-ease)]",
            selected
              ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
              : "text-[var(--sb-text)] hover:bg-[var(--sb-hover)]",
          )}
        >
          {hasChildren && (
            <button
              type="button"
              aria-expanded={isOpen}
              aria-label={isOpen ? `Replier ${node.label}` : `Déplier ${node.label}`}
              onClick={(e) => { e.stopPropagation(); toggle(node.id); }}
              // Cible de 24 px sans grossir la ligne : le glyphe reste à 16 px,
              // la marge négative absorbe le padding.
              className={cn(
                "-m-1 shrink-0 rounded p-1 transition-colors duration-150",
                selected ? "text-[var(--color-accent)]" : "text-[var(--sb-icon)]",
              )}
            >
                <IconChevron
                  className={cn(
                    "size-4 transition-transform duration-[240ms] [transition-timing-function:var(--sb-ease)]",
                    !isOpen && "-rotate-90",
                  )}
                />
            </button>
          )}
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px]",
              hasChildren ? "font-medium" : "font-normal",
            )}
          >
            {node.label}
          </span>
        </div>
        {hasChildren && (
          <div className="sb-collapse" data-open={isOpen}>
            <ul>{children.map((c) => renderNode(c, depth + 1))}</ul>
          </div>
        )}
      </li>
    );
  }

  return (
    <ul className="w-full px-3 py-1">
      {topLevel.filter(nodeMatches).map((n) => renderNode(n, 0))}
    </ul>
  );
}

// ── Sidebar ──────────────────────────────────────────────────────────────────

export function Sidebar({
  graph,
  onSelect,
  selectedId,
  query,
  onQueryChange,
  onCollapse,
  onOpenChat,
  chatOpen = false,
  spaces,
  activeSpaceId,
  onSpaceSelect,
  onSpaceCreate,
  onSpaceShare,
  sharedWithMe = [],
  onOpenShared,
}: {
  graph: BrainGraph | null;
  onSelect: (node: BrainNode) => void;
  selectedId: string | null;
  query: string;
  onQueryChange: (q: string) => void;
  onCollapse: () => void;
  /** Bulle « open-chat » de la top-bar → panneau Lucid IA. */
  onOpenChat?: () => void;
  chatOpen?: boolean;
  spaces: Space[];
  activeSpaceId: string | null;
  onSpaceSelect: (id: string | null) => void;
  onSpaceCreate: (name: string) => Promise<void>;
  onSpaceShare?: (space: Space) => void;
  sharedWithMe?: { id: string; title: string }[];
  onOpenShared?: (id: string) => void;
}) {
  const [structureOpen, setStructureOpen] = useState(true);
  const [spacesOpen, setSpacesOpen] = useState(true);
  const [creating, setCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [email, setEmail] = useState("");

  // L'adresse affichée vient de la session — jamais une valeur en dur.
  useEffect(() => {
    supabase?.auth.getSession().then(({ data }) => setEmail(data.session?.user.email ?? ""));
  }, []);

  const docCount = useMemo(() => {
    if (!graph) return 0;
    const parents = new Set(graph.nodes.map((n) => n.parent_id).filter(Boolean));
    return graph.nodes.filter((n) => n.kind !== "root" && !parents.has(n.id)).length;
  }, [graph]);

  const allSpaces = [LUCID_SPACE, ...spaces.filter((s) => s.id !== "lucid")];

  async function submitCreate() {
    const name = newName.trim();
    if (!name) { setCreating(false); return; }
    await onSpaceCreate(name);
    setNewName("");
    setCreating(false);
  }

  return (
    <div
      className="sidebar glass flex h-full flex-col rounded-[22px] p-3 text-[var(--sb-text)]"
      style={{ width: SIDEBAR_WIDTH }}
    >
      {/* ── top-bar ── */}
      <div className="flex w-full shrink-0 flex-col gap-3 px-3 py-2">
        <div className="flex w-full items-center justify-between">
          <div className="flex items-center gap-0.5">
            <span className="whitespace-nowrap text-sm font-semibold leading-[14px]">
              {docCount} Documents
            </span>
            <button
              type="button"
              onClick={onOpenChat}
              title="Lucid IA"
              className={cn(
                "sb-press p-0.5 transition-[opacity,transform] duration-200",
                "[transition-timing-function:var(--sb-ease)] hover:scale-110 hover:opacity-100",
                chatOpen ? "opacity-100" : "opacity-90",
              )}
              style={{ color: "var(--color-accent)" }}
            >
              <IconChat className="size-5" />
            </button>
          </div>
          <button
            type="button"
            onClick={onCollapse}
            title="Replier la barre latérale"
            className="sb-press text-[var(--sb-text)] opacity-70 transition-opacity duration-200 hover:opacity-100"
          >
            <IconSidebarLeft className="size-5" />
          </button>
        </div>

        <div className="flex h-8 w-full items-center gap-2 rounded-lg border-[0.5px] border-[var(--sb-field)] px-2 transition-colors duration-200 [transition-timing-function:var(--sb-ease)] focus-within:border-[var(--color-accent)]">
          <input
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            placeholder="Rechercher un fichier…"
            className="min-w-0 flex-1 bg-transparent text-[13px] outline-none placeholder:text-[var(--sb-placeholder)]"
          />
          <IconSearch className="size-4 shrink-0 text-[var(--sb-label)]" />
        </div>
      </div>

      {/* ── content-bar ── */}
      <div className="flex min-h-0 w-full flex-1 flex-col overflow-y-auto">
        <div className="w-full shrink-0">
          <SectionHeader title="Structure" open={structureOpen} onToggle={() => setStructureOpen((v) => !v)} />
        </div>
        <div className="sb-collapse shrink-0" data-open={structureOpen && !!graph}>
          <div className="max-h-[45vh] overflow-y-auto">
            {graph && (
              <StructureTree graph={graph} onSelect={onSelect} selectedId={selectedId} query={query} />
            )}
          </div>
        </div>

        <div className="w-full shrink-0">
          <SectionHeader title="Spaces" open={spacesOpen} onToggle={() => setSpacesOpen((v) => !v)}>
            <button
              type="button"
              onClick={() => { setSpacesOpen(true); setCreating(true); }}
              title="Nouveau space"
              className="sb-press text-[var(--sb-icon)] transition-[color,transform] duration-200 [transition-timing-function:var(--sb-ease)] hover:rotate-90 hover:text-[var(--color-accent)]"
            >
              <IconPlus className="size-4" />
            </button>
          </SectionHeader>
        </div>

        <div className="sb-collapse w-full shrink-0" data-open={spacesOpen}>
          <div>
            {allSpaces.map((s) => {
              const isActive = s.id === "lucid"
                ? (activeSpaceId == null || activeSpaceId === "lucid")
                : activeSpaceId === s.id;
              const shareable = s.id !== "lucid" && !!onSpaceShare;
              return (
                // Chaque space a son propre conteneur px-3/py-1 : c'est ce qui
                // donne les 8 px d'air entre les lignes de la maquette.
                <div key={s.id} className="w-full px-3 py-1">
                  <div
                    onClick={() => onSpaceSelect(s.id === "lucid" ? null : s.id)}
                    className={cn(
                      "sb-press group flex cursor-pointer items-center gap-2 rounded-[6px] px-3 py-1",
                      "transition-colors duration-150 [transition-timing-function:var(--sb-ease)]",
                      isActive
                        ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                        : "text-[var(--sb-text)] hover:bg-[var(--sb-hover)]",
                    )}
                  >
                    <span style={{ color: "var(--color-accent)" }}>
                      <IconBrain className="size-5 shrink-0" />
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{s.name}</span>
                    {shareable && (
                      <button
                        type="button"
                        onClick={(e) => { e.stopPropagation(); onSpaceShare!(s); }}
                        title="Partager ce space"
                        className="shrink-0 text-[var(--sb-icon)] opacity-0 transition-opacity hover:text-[var(--color-accent)] group-hover:opacity-100"
                      >
                        <IconPlus className="size-4 rotate-45" />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}

            {creating && (
              <div className="w-full px-3 py-1">
                <input
                  autoFocus
                  value={newName}
                  onChange={(e) => setNewName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") submitCreate();
                    if (e.key === "Escape") { setNewName(""); setCreating(false); }
                  }}
                  onBlur={submitCreate}
                  placeholder="Nom du space…"
                  className="w-full rounded-[6px] border-[0.5px] border-[var(--color-accent)] bg-transparent px-3 py-1 text-[13px] outline-none placeholder:text-[var(--sb-placeholder)]"
                />
              </div>
            )}

            {sharedWithMe.length > 0 && (
              <>
                <div className="px-[18px] pb-[5px] pt-[15px] text-sm font-medium leading-[14px] text-[var(--sb-label)]">
                  Partagés avec moi
                </div>
                {sharedWithMe.map((s) => (
                  <div key={s.id} className="w-full px-3 py-1">
                    <button
                      type="button"
                      onClick={() => onOpenShared?.(s.id)}
                      className="flex w-full cursor-pointer items-center gap-2 rounded-[6px] px-3 py-1 text-left text-[var(--sb-text)] hover:bg-[var(--sb-hover)]"
                    >
                      <span className="text-[var(--sb-icon)]">
                        <IconBrain className="size-5 shrink-0" />
                      </span>
                      <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{s.title}</span>
                    </button>
                  </div>
                ))}
              </>
            )}
          </div>
        </div>
      </div>

      {/* ── bottom-bar ── */}
      {email && (
        <div className="flex w-full shrink-0 items-center py-2">
          <span className="truncate text-sm font-semibold leading-[14px]">{email}</span>
        </div>
      )}
    </div>
  );
}
