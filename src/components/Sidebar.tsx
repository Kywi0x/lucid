import { useEffect, useMemo, useState } from "react";
import type { BrainGraph, BrainNode, Space } from "@/lib/types";
import { supabase } from "@/lib/supabase";
import { SOURCE_COLOR } from "@/lib/connector-icons";
import {
  IconChevron, IconSearch, IconPlus, IconSidebarLeft, IconBrain, IconChat,
} from "./icons";

/** Largeur de la sidebar — maquette « l'app vivante » : 234 px, posée à 12 px
 *  du bord. Le reste du chrome flottant se cale derrière (cf. `dockLeft`). */
export const SIDEBAR_WIDTH = 234;

/** Indentation d'un palier de l'arbre (maquette : `4px + depth * 15px`). */
const INDENT = 15;

const LUCID_SPACE: Space = { id: "lucid", name: "Lucid", node_ids: null };

function Section({
  title, open, onToggle, children,
}: {
  title: string;
  open: boolean;
  onToggle: () => void;
  children?: React.ReactNode;
}) {
  return (
    <div className="sec" data-open={open}>
      <button type="button" className="ttl" onClick={onToggle}>{title}</button>
      <button
        type="button"
        className="icon-btn chev"
        onClick={onToggle}
        aria-label={open ? `Replier ${title}` : `Déplier ${title}`}
      >
        <IconChevron />
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
    const color = node.connector ? SOURCE_COLOR[node.connector] : undefined;

    return (
      <li key={node.id}>
        <button
          type="button"
          className="row"
          style={{ paddingLeft: 4 + depth * INDENT }}
          aria-selected={selectedId === node.id || undefined}
          data-exp={hasChildren ? isOpen : undefined}
          onClick={() => { if (hasChildren) toggle(node.id); onSelect(node); }}
        >
          <span className="tw">{hasChildren && <IconChevron />}</span>
          <span className="nm">{node.label}</span>
          {color && <span className="dot-src" style={{ background: color }} />}
        </button>
        {hasChildren && (
          <div className="fold" data-open={isOpen}>
            <ul className="tree">{children.map((c) => renderNode(c, depth + 1))}</ul>
          </div>
        )}
      </li>
    );
  }

  const rows = topLevel.filter(nodeMatches);
  if (!rows.length) {
    return (
      <ul className="tree">
        <li style={{ padding: "8px 12px", fontSize: 12, color: "var(--sb-label)" }}>Aucun résultat</li>
      </ul>
    );
  }
  return <ul className="tree">{rows.map((n) => renderNode(n, 0))}</ul>;
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
  onOpenSettings,
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
  /** Pied de la sidebar (avatar + adresse) → Réglages. */
  onOpenSettings?: () => void;
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
    <div className="sidebar lg" style={{ width: SIDEBAR_WIDTH }}>
      <div className="sb-top">
        <div className="sb-row">
          <div className="sb-count">
            <b>{docCount}</b><em>document{docCount > 1 ? "s" : ""}</em>
          </div>
          <div style={{ display: "flex", gap: 2 }}>
            <button
              type="button"
              className="icon-btn accent"
              onClick={onOpenChat}
              title="Lucid IA"
              aria-label="Lucid IA"
              aria-pressed={chatOpen}
            >
              <IconChat className="size-[17px]" />
            </button>
            <button
              type="button"
              className="icon-btn"
              onClick={onCollapse}
              title="Replier la barre latérale"
              aria-label="Replier la barre latérale"
            >
              <IconSidebarLeft className="size-[17px]" />
            </button>
          </div>
        </div>

        <label className="field">
          <input
            type="text"
            value={query}
            onChange={(e) => onQueryChange(e.target.value)}
            placeholder="Rechercher…"
            aria-label="Rechercher un document"
          />
          <IconSearch className="size-[14px]" />
        </label>
      </div>

      <div className="sb-body">
        <Section title="Structure" open={structureOpen} onToggle={() => setStructureOpen((v) => !v)} />
        <div className="fold" data-open={structureOpen && !!graph}>
          <div>
            {graph && (
              <StructureTree graph={graph} onSelect={onSelect} selectedId={selectedId} query={query} />
            )}
          </div>
        </div>

        <Section title="Spaces" open={spacesOpen} onToggle={() => setSpacesOpen((v) => !v)}>
          <button
            type="button"
            className="icon-btn plus"
            onClick={() => { setSpacesOpen(true); setCreating(true); }}
            title="Nouveau space"
            aria-label="Nouveau space"
          >
            <IconPlus className="size-[14px]" />
          </button>
        </Section>

        <div className="fold" data-open={spacesOpen}>
          <div>
            {allSpaces.map((s) => {
              const isActive = s.id === "lucid"
                ? (activeSpaceId == null || activeSpaceId === "lucid")
                : activeSpaceId === s.id;
              const shareable = s.id !== "lucid" && !!onSpaceShare;
              return (
                <div key={s.id} className="space-wrap">
                  <button
                    type="button"
                    className="space group"
                    aria-selected={isActive || undefined}
                    onClick={() => onSpaceSelect(s.id === "lucid" ? null : s.id)}
                  >
                    <span className="bi"><IconBrain /></span>
                    <span className="nm">{s.name}</span>
                    {shareable && (
                      <span
                        role="button"
                        tabIndex={0}
                        onClick={(e) => { e.stopPropagation(); onSpaceShare!(s); }}
                        onKeyDown={(e) => { if (e.key === "Enter") { e.stopPropagation(); onSpaceShare!(s); } }}
                        title="Partager ce space"
                        className="shrink-0 text-[var(--sb-icon)] opacity-0 transition-opacity hover:text-[var(--color-accent)] group-hover:opacity-100"
                      >
                        <IconPlus className="size-[14px] rotate-45" />
                      </span>
                    )}
                  </button>
                </div>
              );
            })}

            {creating && (
              <div className="space-wrap">
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
                  className="w-full rounded-[7px] border-[0.5px] border-[var(--color-accent)] bg-transparent px-2 py-1 text-[12.5px] text-[var(--sb-text)] outline-none placeholder:text-[var(--sb-placeholder)]"
                />
              </div>
            )}

            {sharedWithMe.length > 0 && (
              <>
                <div className="sec"><span className="ttl">Partagés avec moi</span></div>
                {sharedWithMe.map((s) => (
                  <div key={s.id} className="space-wrap">
                    <button type="button" className="space" onClick={() => onOpenShared?.(s.id)}>
                      <span className="bi" style={{ color: "var(--sb-icon)" }}><IconBrain /></span>
                      <span className="nm">{s.title}</span>
                    </button>
                  </div>
                ))}
              </>
            )}
          </div>
        </div>
      </div>

      {email && (
        <button type="button" className="sb-foot" onClick={onOpenSettings} title="Réglages (⌘,)">
          <span className="avatar">{email.slice(0, 1).toUpperCase()}</span>
          <span>{email}</span>
        </button>
      )}
    </div>
  );
}
