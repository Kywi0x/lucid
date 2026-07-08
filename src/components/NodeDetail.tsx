import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import {
  X, FolderGit2, Brain, FileText,
  Download, RefreshCw, Sparkles, Loader2, ChevronRight, Maximize2, Minimize2, Check,
  History, RotateCcw, Send, Sparkle,
  PanelLeftClose, PanelLeftOpen, PanelRightClose, PanelRightOpen,
} from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import type { BrainGraph, BrainNode, NodeSnapshotInfo } from "@/lib/types";
import { relativeDate, cn } from "@/lib/utils";
import { exportNodeMd, synthesizeNode, saveNodeContent, loadNodeContent, listNodeSnapshots, getNodeSnapshot, renameNode, askNode, generateContent } from "@/lib/api";
import { MarkdownEditor } from "./MarkdownEditor";
import { AiStatusBar, useAiReady, AI_MISSING_HINT } from "./AiStatusBar";
import { Properties } from "./Properties";
import { parseFrontmatter, serializeFrontmatter, type Prop } from "@/lib/frontmatter";
import claudeLogo from "@/assets/claude-logo.png";
import driveLogo  from "@/assets/google_drive.svg.png";
import notionLogo from "@/assets/Notion_app_logo.png";

interface Props {
  node: BrainNode;
  graph: BrainGraph | null;
  onSelect: (node: BrainNode) => void;
  onClose: () => void;
  expanded?: boolean;
  onExpand?: () => void;
  onContentSaved?: (nodeId: string, content: string) => void;
  onCreateNote?: (parentId: string, title: string) => void;
  onNodeRenamed?: (nodeId: string, label: string) => void;
}

const ICON = {
  root: Brain, group: FolderGit2, espace: FolderGit2, container: FolderGit2,
  page: FileText, leaf: FileText, concept: FileText, source: FileText,
} as const;
const KIND_LABEL = {
  root: "Lucid", group: "Espace", espace: "Espace", container: "Espace",
  page: "Page", leaf: "Page", concept: "Page", source: "Page",
} as const;

const CONNECTOR_LOGO: Record<string, string> = {
  "claude-code":  claudeLogo,
  "google-drive": driveLogo,
  "notion":       notionLogo,
};
const CONNECTOR_LABEL: Record<string, string> = {
  "claude-code":  "Claude Code",
  "google-drive": "Google Drive",
  "notion":       "Notion",
};

function ConnectorLogo({ connector }: { connector: string }) {
  const logo = CONNECTOR_LOGO[connector];
  const label = CONNECTOR_LABEL[connector] ?? connector;
  if (logo) return <img src={logo} alt={label} className="size-3.5 shrink-0 rounded-sm object-contain" />;
  return <FileText className="size-3.5 shrink-0 text-[var(--color-accent)]" />;
}

/** Arborescence du projet (depuis le plus haut parent hors racine) : branches
 *  repliables, chemin vers la page courante déplié, page courante surlignée. */
function ProjectTree({ node, graph, ancestors, onSelect }: {
  node: BrainNode;
  graph: BrainGraph;
  ancestors: BrainNode[];
  onSelect: (n: BrainNode) => void;
}) {
  const childrenOf = useMemo(() => {
    const m = new Map<string, BrainNode[]>();
    for (const n of graph.nodes) {
      if (!n.parent_id || n.kind === "pending") continue;
      if (!m.has(n.parent_id)) m.set(n.parent_id, []);
      m.get(n.parent_id)!.push(n);
    }
    return m;
  }, [graph]);

  const top = ancestors[0] ?? node;
  const pathIds = useMemo(
    () => new Set([...ancestors.map((a) => a.id), node.id]),
    [ancestors, node.id],
  );
  const [open, setOpen] = useState<Set<string>>(() => new Set(pathIds));
  useEffect(() => { setOpen((prev) => new Set([...prev, ...pathIds])); }, [pathIds]);

  function Row({ n, depth }: { n: BrainNode; depth: number }) {
    const kids = childrenOf.get(n.id) ?? [];
    const isOpen = open.has(n.id);
    const isCurrent = n.id === node.id;
    if (depth > 8) return null;
    return (
      <li>
        <div
          className={cn(
            "flex items-center gap-0.5 rounded-md py-0.5 pr-1 transition-colors",
            isCurrent ? "bg-[var(--color-accent-soft)]" : "hover:bg-[var(--color-surface-2)]",
          )}
          style={{ paddingLeft: depth * 12 }}
        >
          {kids.length > 0 ? (
            <button
              onClick={() => setOpen((prev) => {
                const s = new Set(prev);
                if (s.has(n.id)) s.delete(n.id); else s.add(n.id);
                return s;
              })}
              className="shrink-0 rounded p-0.5 text-[var(--color-muted)] hover:text-[var(--color-text)]"
            >
              <ChevronRight className={cn("size-3 transition-transform", isOpen && "rotate-90")} />
            </button>
          ) : (
            <span className="w-4 shrink-0" />
          )}
          <button
            onClick={() => onSelect(n)}
            className={cn(
              "min-w-0 flex-1 truncate text-left text-xs transition-colors",
              isCurrent ? "font-medium text-[var(--color-text)]" : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
            )}
          >
            {n.label}
          </button>
        </div>
        {isOpen && kids.length > 0 && (
          <ul>{kids.map((k) => <Row key={k.id} n={k} depth={depth + 1} />)}</ul>
        )}
      </li>
    );
  }

  return (
    <div>
      <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
        Arborescence
      </p>
      <ul><Row n={top} depth={0} /></ul>
    </div>
  );
}

async function downloadNode(node: BrainNode) {
  const path = await save({
    defaultPath: `${node.label}.md`,
    filters: [{ name: "Markdown", extensions: ["md"] }],
  });
  if (path) await exportNodeMd(node.id, path);
}

function useAncestors(node: BrainNode, graph: BrainGraph | null): BrainNode[] {
  return useMemo(() => {
    if (!graph) return [];
    const byId = new Map(graph.nodes.map((n) => [n.id, n]));
    const path: BrainNode[] = [];
    let cur = node;
    while (cur.parent_id && cur.parent_id !== "root") {
      const parent = byId.get(cur.parent_id);
      if (!parent || parent.id === node.id) break; // sécurité anti-boucle
      path.unshift(parent);
      cur = parent;
    }
    return path;
  }, [node.id, graph]);
}

// ─── Panneau de discussion IA (mode plein écran) ────────────────────────────
function NodeChat({ node, childCount, onCollapse }: { node: BrainNode; childCount: number; onCollapse?: () => void }) {
  const [msgs, setMsgs] = useState<{ role: "user" | "assistant"; text: string }[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [withChildren, setWithChildren] = useState(childCount > 0);
  const aiOk = useAiReady();
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => { endRef.current?.scrollIntoView({ behavior: "smooth" }); }, [msgs, loading]);

  async function send() {
    const q = input.trim();
    if (!q || loading) return;
    setMsgs((m) => [...m, { role: "user", text: q }]);
    setInput("");
    setLoading(true);
    try {
      const a = await askNode(node.id, q, withChildren);
      setMsgs((m) => [...m, { role: "assistant", text: a }]);
    } catch (e) {
      setMsgs((m) => [...m, { role: "assistant", text: "Erreur : " + String(e) }]);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-4 py-2.5">
        <Sparkle className="size-3.5 text-[var(--color-accent)]" />
        <span className="flex-1 text-xs font-semibold text-[var(--color-text)]">Lucid IA</span>
        {onCollapse && (
          <button onClick={onCollapse} title="Replier l'assistant"
            className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
            <PanelRightClose className="size-3.5" />
          </button>
        )}
        <button
          onClick={() => setWithChildren((v) => !v)}
          disabled={childCount === 0}
          title="Inclure le contenu des sous-pages dans le contexte"
          className={cn(
            "flex items-center gap-1.5 rounded-full border px-2 py-1 text-[10px] transition-colors disabled:opacity-40",
            withChildren
              ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
              : "border-[var(--color-border)] text-[var(--color-muted)] hover:text-[var(--color-text)]",
          )}
        >
          <span className={cn("size-1.5 rounded-full", withChildren ? "bg-[var(--color-accent)]" : "bg-[var(--color-muted)]")} />
          Sous-pages{childCount > 0 ? ` (${childCount})` : ""}
        </button>
      </div>

      <div className="flex-1 space-y-3 overflow-y-auto px-4 py-4">
        {msgs.length === 0 && (
          <p className="mt-6 text-center text-xs leading-relaxed text-[var(--color-muted)]">
            Pose une question sur cette page{withChildren && childCount > 0 ? " et ses sous-pages" : ""}.{"\n"}
            Tout reste local.
          </p>
        )}
        {msgs.map((m, i) => (
          <div key={i} className={cn("flex", m.role === "user" ? "justify-end" : "justify-start")}>
            <div className={cn(
              "max-w-[85%] whitespace-pre-wrap rounded-xl px-3 py-2 text-xs leading-relaxed",
              m.role === "user"
                ? "bg-[var(--color-accent)] text-white"
                : "border border-[var(--color-border)] bg-[var(--color-bg)] text-[var(--color-text)]",
            )}>
              {m.text}
            </div>
          </div>
        ))}
        {loading && (
          <div className="flex items-center gap-2 text-xs text-[var(--color-muted)]">
            <Loader2 className="size-3.5 animate-spin" /> L'assistant réfléchit…
          </div>
        )}
        <div ref={endRef} />
      </div>

      <div className="border-t border-[var(--color-border)] p-3">
        <div className="flex items-end gap-2">
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); send(); } }}
            placeholder={aiOk === false ? AI_MISSING_HINT : "Poser une question…"}
            rows={1}
            disabled={aiOk === false}
            className="max-h-28 flex-1 resize-none rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-xs text-[var(--color-text)] outline-none disabled:opacity-50"
          />
          <button
            onClick={send}
            disabled={loading || !input.trim() || aiOk === false}
            title={aiOk === false ? AI_MISSING_HINT : undefined}
            className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-[var(--color-accent)] text-white transition-opacity hover:opacity-90 disabled:opacity-40"
          >
            <Send className="size-3.5" />
          </button>
        </div>
        <AiStatusBar input={input} />
      </div>
    </div>
  );
}

export function NodeDetail({ node, graph, onSelect, onClose, expanded, onExpand, onContentSaved, onCreateNote, onNodeRenamed }: Props) {
  const ancestors = useAncestors(node, graph);
  const aiOk = useAiReady();
  const Icon = ICON[node.kind as keyof typeof ICON] ?? FileText;
  const children = graph?.nodes.filter((n) => n.parent_id === node.id) ?? [];

  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  function commitTitle() {
    setEditingTitle(false);
    const l = titleDraft.trim();
    if (l && l !== node.label) renameNode(node.id, l).then(() => onNodeRenamed?.(node.id, l)).catch(console.error);
  }
  const slashPage = onCreateNote ? () => onCreateNote(node.id, "Nouvelle page") : undefined;
  const generate = (instruction: string, includeChildren: boolean) => generateContent(node.id, instruction, includeChildren);

  // Wikilinks : cibles d'autocomplétion (toutes les pages) + navigation par label.
  const linkTargets = useMemo(
    () => (graph?.nodes ?? []).filter((n) => n.kind !== "root" && n.kind !== "pending").map((n) => n.label),
    [graph],
  );
  const navigateToLabel = useCallback((label: string) => {
    const target = graph?.nodes.find((n) => n.label.toLowerCase() === label.toLowerCase() && n.kind !== "pending");
    if (target) onSelect(target);
  }, [graph, onSelect]);

  // Dérivés du markdown (UI pure, rien de stocké) : progression des tâches + sommaire.
  const contentColRef = useRef<HTMLDivElement>(null);

  // Panneaux repliables du mode étendu (préférence conservée entre sessions).
  const [leftOpen, setLeftOpen] = useState(() => localStorage.getItem("lucid.detail.left") !== "0");
  const [chatOpen, setChatOpen] = useState(() => localStorage.getItem("lucid.detail.chat") !== "0");
  function toggleLeft() { setLeftOpen((v) => { localStorage.setItem("lucid.detail.left", v ? "0" : "1"); return !v; }); }
  function toggleChat() { setChatOpen((v) => { localStorage.setItem("lucid.detail.chat", v ? "0" : "1"); return !v; }); }

  const [synth, setSynth] = useState<Partial<BrainNode> | null>(null);
  const [synthesizing, setSynthesizing] = useState(false);
  const [synthError, setSynthError] = useState<string | null>(null);

  const [localContent, setLocalContent] = useState<string | null>(null);
  const [historyMode, setHistoryMode] = useState(false);
  const [nodeSnapshots, setNodeSnapshots] = useState<NodeSnapshotInfo[]>([]);
  const [loadingHistory, setLoadingHistory] = useState(false);
  const [restoringId, setRestoringId] = useState<string | null>(null);
  const display = synth ? { ...node, ...synth } : node;
  const synthTimestamp = synth?.synthesized_at ?? node.synthesized_at;

  async function handleSynthesize() {
    setSynthesizing(true);
    setSynthError(null);
    try {
      const updated = await synthesizeNode(node.id);
      setSynth(updated);
    } catch (e) {
      setSynthError(String(e));
    } finally {
      setSynthesizing(false);
    }
  }

  // Contenu source — chargé depuis le cache local (0 appel API)
  const [sourceText, setSourceText] = useState<string | null>(null);
  const [sourceLoading, setSourceLoading] = useState(false);

  useEffect(() => {
    setSynth(null);
    setLocalContent(null);
    setHistoryMode(false);
    setNodeSnapshots([]);
    setEditingTitle(false);
    if (!node.connector) { setSourceText(null); return; }
    setSourceLoading(true);
    loadNodeContent(node.id)
      .then(setSourceText)
      .catch(() => setSourceText(null))
      .finally(() => setSourceLoading(false));
  }, [node.id]);

  // Propriétés (frontmatter) + corps markdown, séparés : l'éditeur ne voit que le corps.
  // Une seule source de vérité (le contenu sérialisé) ; reseed au changement de nœud /
  // chargement source / restauration — PAS sur l'écho de save (évite le clobber en cours d'édition).
  const [props, setProps] = useState<Prop[]>(() => parseFrontmatter(node.content ?? "").props);
  const [body, setBody] = useState(() => parseFrontmatter(node.content ?? "").body);
  useEffect(() => {
    // `node.content` vaut "" (jamais null) côté Rust : `||` pour que le texte
    // source (PDF/Notion chargé via loadNodeContent) s'affiche quand rien n'est édité.
    const { props: p, body: b } = parseFrontmatter(localContent ?? (node.content || sourceText || ""));
    setProps(p);
    setBody(b);
  }, [node.id, sourceText, localContent]);

  const persistBody = (b: string) => { setBody(b); handleContentChange(serializeFrontmatter(props, b)); };
  const persistProps = (p: Prop[]) => { setProps(p); handleContentChange(serializeFrontmatter(p, body)); };

  const taskStats = useMemo(() => {
    const all = body.match(/^\s*[-*] \[[ xX]\]/gm) ?? [];
    const done = body.match(/^\s*[-*] \[[xX]\]/gm) ?? [];
    return { total: all.length, done: done.length };
  }, [body]);

  const headings = useMemo(() => {
    return [...body.matchAll(/^(#{1,3}) (.+)$/gm)]
      .map((m) => ({ level: m[1].length, text: m[2].trim() }));
  }, [body]);

  function scrollToHeading(text: string) {
    const root: ParentNode = contentColRef.current ?? document;
    for (const el of root.querySelectorAll(".editor-content h1, .editor-content h2, .editor-content h3")) {
      if (el.textContent?.trim() === text) { el.scrollIntoView({ behavior: "smooth", block: "start" }); return; }
    }
  }

  const [saveStatus, setSaveStatus] = useState<"idle" | "saving" | "saved">("idle");
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const saveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const handleContentChange = useCallback((markdown: string) => {
    setSaveStatus("saving");
    if (saveTimer.current) clearTimeout(saveTimer.current);
    saveTimer.current = setTimeout(() => {
      saveNodeContent(node.id, markdown)
        .then(() => {
          onContentSaved?.(node.id, markdown);
          setSaveStatus("saved");
          if (savedTimer.current) clearTimeout(savedTimer.current);
          savedTimer.current = setTimeout(() => setSaveStatus("idle"), 2000);
        })
        .catch(console.error);
    }, 800);
  }, [node.id, onContentSaved]);

  async function openHistory() {
    setHistoryMode(true);
    setLoadingHistory(true);
    try { setNodeSnapshots(await listNodeSnapshots(node.id)); }
    finally { setLoadingHistory(false); }
  }

  async function handleRestoreNodeSnapshot(id: string) {
    setRestoringId(id);
    try {
      const content = await getNodeSnapshot(node.id, id);
      await saveNodeContent(node.id, content);
      setLocalContent(content);
      onContentSaved?.(node.id, content);
      setHistoryMode(false);
    } finally { setRestoringId(null); }
  }

  function nodeRelTime(ts: number) {
    const d = Math.floor(Date.now() / 1000) - ts;
    if (d < 60)    return "il y a quelques secondes";
    if (d < 3600)  return `il y a ${Math.floor(d / 60)} min`;
    if (d < 86400) return `il y a ${Math.floor(d / 3600)} h`;
    return `il y a ${Math.floor(d / 86400)} j`;
  }

  return (
    <div className="flex h-full w-full flex-col bg-[var(--color-surface)]">

      {/* ── Header compact ── */}
      <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-2.5">
        <Icon className="size-4 shrink-0 text-[var(--color-accent)]" />
        <div className="min-w-0 flex-1">
          {editingTitle ? (
            <input
              autoFocus
              value={titleDraft}
              onChange={(e) => setTitleDraft(e.target.value)}
              onBlur={commitTitle}
              onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); commitTitle(); }
                else if (e.key === "Escape") setEditingTitle(false);
              }}
              className="w-full rounded border border-[var(--color-accent)] bg-[var(--color-surface-2)] px-1 py-0.5 text-sm font-semibold leading-tight text-[var(--color-text)] outline-none"
            />
          ) : (
            <h3
              className={cn("truncate text-sm font-semibold leading-tight", node.kind !== "root" && "cursor-text hover:text-[var(--color-accent)]")}
              title={node.kind !== "root" ? "Cliquer pour renommer" : undefined}
              onClick={() => { if (node.kind !== "root") { setTitleDraft(node.label); setEditingTitle(true); } }}
            >
              {node.label}
            </h3>
          )}
          <p className="text-[10px] text-[var(--color-muted)]">
            {KIND_LABEL[node.kind as keyof typeof KIND_LABEL] ?? "Page"}
            {node.kind !== "root" && ` · poids ${node.weight}`}
            {node.connector && (
              <span className="ml-1.5 inline-flex items-center gap-1">
                ·
                <ConnectorLogo connector={node.connector} />
                {CONNECTOR_LABEL[node.connector] ?? node.connector}
              </span>
            )}
          </p>
        </div>

        {saveStatus !== "idle" && (
          <span className={cn(
            "flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] transition-all",
            saveStatus === "saving"
              ? "text-[var(--color-muted)]"
              : "text-[var(--color-ok)]",
          )}>
            {saveStatus === "saving"
              ? <Loader2 className="size-2.5 animate-spin" />
              : <Check className="size-2.5" />}
            {saveStatus === "saving" ? "Enregistrement…" : "Enregistré"}
          </span>
        )}

        {node.kind !== "root" && (
          <>
            <button
              onClick={handleSynthesize}
              disabled={synthesizing || aiOk === false}
              title={aiOk === false ? AI_MISSING_HINT : "Relancer la synthèse IA"}
              className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-accent)] disabled:opacity-40"
            >
              <RefreshCw className={cn("size-3.5", synthesizing && "animate-spin")} />
            </button>
            <button
              onClick={() => downloadNode(node)}
              title="Exporter en .md"
              className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
            >
              <Download className="size-3.5" />
            </button>
            <button
              onClick={historyMode ? () => setHistoryMode(false) : openHistory}
              title={historyMode ? "Retour au contenu" : "Historique des versions"}
              className={cn(
                "rounded-md p-1 transition-colors hover:bg-[var(--color-surface-2)]",
                historyMode ? "text-[var(--color-accent)]" : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
              )}
            >
              <History className="size-3.5" />
            </button>
          </>
        )}
        {onExpand && (
          <button
            onClick={onExpand}
            title={expanded ? "Réduire" : "Agrandir"}
            className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
          >
            {expanded ? <Minimize2 className="size-3.5" /> : <Maximize2 className="size-3.5" />}
          </button>
        )}
        <button
          onClick={onClose}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
        >
          <X className="size-4" />
        </button>
      </div>

      {/* ── Fil d'Ariane ── */}
      {ancestors.length > 0 && (
        <div className="flex items-center gap-0.5 overflow-x-auto border-b border-[var(--color-border)] px-4 py-1.5 scrollbar-none">
          {ancestors.map((a, i) => (
            <span key={a.id} className="flex shrink-0 items-center gap-0.5">
              {i > 0 && <ChevronRight className="size-3 text-[var(--color-border)]" />}
              <button
                onClick={() => onSelect(a)}
                className="max-w-[120px] truncate text-[11px] text-[var(--color-muted)] hover:text-[var(--color-accent)] transition-colors"
              >
                {a.label}
              </button>
            </span>
          ))}
          <ChevronRight className="size-3 shrink-0 text-[var(--color-border)]" />
          <span className="shrink-0 text-[11px] font-medium text-[var(--color-text)]">{node.label}</span>
        </div>
      )}

      {/* ── Progression des tâches (dérivée des - [ ] du markdown) ── */}
      {taskStats.total > 0 && (
        <div className="flex items-center gap-2.5 border-b border-[var(--color-border)] px-4 py-1.5">
          <div className="h-1 flex-1 overflow-hidden rounded-full bg-[var(--color-surface-2)]">
            <div
              className="h-full rounded-full bg-[var(--color-accent)] transition-all duration-300"
              style={{ width: `${Math.round((taskStats.done / taskStats.total) * 100)}%` }}
            />
          </div>
          <span className="shrink-0 font-mono text-[10px] text-[var(--color-muted)]">
            {taskStats.done}/{taskStats.total} ✓
          </span>
        </div>
      )}

      {/* ── Corps ── */}
      {historyMode ? (
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {loadingHistory ? (
            <div className="flex items-center gap-2 py-6 text-xs text-[var(--color-muted)]">
              <Loader2 className="size-3.5 animate-spin" /> Chargement…
            </div>
          ) : nodeSnapshots.length === 0 ? (
            <p className="py-8 text-center text-xs text-[var(--color-muted)]">
              Aucune version sauvegardée.{"\n"}Les versions sont créées automatiquement à chaque modification.
            </p>
          ) : (
            <ul className="space-y-2">
              {nodeSnapshots.map((s) => (
                <li key={s.id} className="rounded-lg border border-[var(--color-border)] p-3">
                  <div className="mb-1.5 flex items-start justify-between gap-2">
                    <span className="text-xs font-medium text-[var(--color-text)]">{nodeRelTime(s.created_at)}</span>
                    <button
                      onClick={() => handleRestoreNodeSnapshot(s.id)}
                      disabled={restoringId === s.id}
                      className="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium text-[var(--color-accent)] transition-colors hover:bg-[var(--color-accent-soft)] disabled:opacity-40"
                    >
                      {restoringId === s.id
                        ? <Loader2 className="size-3 animate-spin" />
                        : <RotateCcw className="size-3" />}
                      Restaurer
                    </button>
                  </div>
                  {s.preview && (
                    <p className="line-clamp-3 text-[11px] leading-relaxed text-[var(--color-muted)]">
                      {s.preview}{s.preview.length >= 150 ? "…" : ""}
                    </p>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : expanded ? (
        // Mode étendu : deux colonnes
        <div className="flex flex-1 overflow-hidden">
          {/* Colonne gauche — arborescence + métadonnées (repliable) */}
          {!leftOpen ? (
            <div className="flex w-9 shrink-0 justify-center border-r border-[var(--color-border)] pt-3">
              <button onClick={toggleLeft} title="Afficher l'arborescence"
                className="h-fit rounded-md p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
                <PanelLeftOpen className="size-4" />
              </button>
            </div>
          ) : (
          <div className="relative w-80 shrink-0 overflow-y-auto border-r border-[var(--color-border)] px-5 py-5 space-y-5">
            <button onClick={toggleLeft} title="Replier l'arborescence"
              className="absolute right-2 top-2 rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
              <PanelLeftClose className="size-3.5" />
            </button>
            {graph && node.kind !== "root" && (
              <ProjectTree node={node} graph={graph} ancestors={ancestors} onSelect={onSelect} />
            )}
            {headings.length >= 3 && (
              <div>
                <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Sommaire</p>
                <ul className="space-y-0.5">
                  {headings.map((h, i) => (
                    <li key={i}>
                      <button
                        onClick={() => scrollToHeading(h.text)}
                        style={{ paddingLeft: (h.level - 1) * 10 }}
                        className="w-full truncate text-left text-xs text-[var(--color-muted)] transition-colors hover:text-[var(--color-text)]"
                      >
                        {h.text}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
            {synthError && (
              <div className="rounded-lg border border-[var(--color-err)]/30 bg-[var(--color-err)]/10 px-3 py-2 text-xs text-[var(--color-err)]">
                {synthError}
              </div>
            )}
            {(display.decisions ?? []).length > 0 && (
              <div>
                <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Décisions clés</p>
                <ul className="space-y-1.5">
                  {(display.decisions ?? []).map((d, i) => (
                    <li key={i} className="border-l-2 border-[var(--color-border)] pl-2.5 text-xs leading-relaxed text-[var(--color-text)]">{d}</li>
                  ))}
                </ul>
              </div>
            )}
            {(display.patterns ?? []).length > 0 && (
              <div>
                <p className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Patterns</p>
                <ul className="space-y-0.5">
                  {(display.patterns ?? []).map((p, i) => (
                    <li key={i} className="text-xs text-[var(--color-muted)]">🔁 {p}</li>
                  ))}
                </ul>
              </div>
            )}
            {/* (Pages enfants supprimé en mode étendu : l'arborescence couvre la navigation) */}
          </div>
          )}

          {/* Colonne droite — synthèse + contenu éditable */}
          <div ref={contentColRef} className="flex-1 overflow-y-auto px-8 py-6">
            <div className="mx-auto max-w-2xl space-y-4">
              {display.summary && (
                <div className="flex gap-2.5 rounded-lg border-l-2 border-[var(--color-accent)]/60 bg-[var(--color-accent-soft)] px-3 py-2.5">
                  <Sparkles className="mt-0.5 size-3 shrink-0 text-[var(--color-accent)]" />
                  <div className="min-w-0">
                    <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--color-accent)]">
                      Synthèse IA
                      {synthTimestamp && <span className="ml-1.5 font-normal normal-case text-[var(--color-accent)]/60">· {relativeDate(synthTimestamp)}</span>}
                    </span>
                    <p className="mt-0.5 text-sm leading-relaxed text-[var(--color-text)]">{display.summary}</p>
                  </div>
                </div>
              )}
              {(display.keywords ?? []).length > 0 && (
                <div className="flex flex-wrap gap-1.5">
                  {(display.keywords ?? []).map((k) => (
                    <span key={k} className="rounded-full border border-[var(--color-border)] px-2 py-0.5 text-[11px] text-[var(--color-muted)]">{k}</span>
                  ))}
                </div>
              )}
              {node.kind !== "root" && (
                sourceLoading ? (
                  <div className="flex items-center gap-2 py-6 text-xs text-[var(--color-muted)]">
                    <Loader2 className="size-3.5 animate-spin" /> Chargement du contenu…
                  </div>
                ) : (
                  <>
                    <Properties props={props} onChange={persistProps} />
                    <MarkdownEditor content={body} onChange={persistBody} placeholder="Écris… (tape / pour l'IA, [[ pour lier une page, colle une image)" onSlashPage={slashPage} onGenerate={aiOk === false ? undefined : generate} linkTargets={linkTargets} onNavigate={navigateToLabel} />
                  </>
                )
              )}
            </div>
          </div>

          {/* Colonne droite — discussion IA (repliable) */}
          {node.kind !== "root" && (
            !chatOpen ? (
              <div className="flex w-9 shrink-0 justify-center border-l border-[var(--color-border)] pt-3">
                <button onClick={toggleChat} title="Afficher l'assistant"
                  className="h-fit rounded-md p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
                  <PanelRightOpen className="size-4" />
                </button>
              </div>
            ) : (
              <div className="flex w-[360px] shrink-0 flex-col border-l border-[var(--color-border)]">
                <NodeChat key={node.id} node={node} childCount={children.length} onCollapse={toggleChat} />
              </div>
            )
          )}
        </div>
      ) : (
        // Mode panneau : colonne unique
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {synthError && (
            <div className="mb-3 rounded-lg border border-[var(--color-err)]/30 bg-[var(--color-err)]/10 px-3 py-2 text-xs text-[var(--color-err)]">{synthError}</div>
          )}
          {display.summary && (
            <div className="mb-4 flex gap-2.5 rounded-lg border-l-2 border-[var(--color-accent)]/60 bg-[var(--color-accent-soft)] px-3 py-2.5">
              <Sparkles className="mt-0.5 size-3 shrink-0 text-[var(--color-accent)]" />
              <div className="min-w-0">
                <span className="text-[10px] font-semibold uppercase tracking-wide text-[var(--color-accent)]">
                  Synthèse IA
                  {synthTimestamp && <span className="ml-1.5 font-normal normal-case text-[var(--color-accent)]/60">· {relativeDate(synthTimestamp)}</span>}
                </span>
                <p className="mt-0.5 text-sm leading-relaxed text-[var(--color-text)]">{display.summary}</p>
              </div>
            </div>
          )}
          {(display.keywords ?? []).length > 0 && (
            <div className="mb-4 flex flex-wrap gap-1.5">
              {(display.keywords ?? []).map((k) => (
                <span key={k} className="rounded-full border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-0.5 text-[11px] text-[var(--color-muted)]">{k}</span>
              ))}
            </div>
          )}
          {node.kind !== "root" && (
            sourceLoading ? (
              <div className="flex items-center gap-2 py-6 text-xs text-[var(--color-muted)]">
                <Loader2 className="size-3.5 animate-spin" /> Chargement du contenu…
              </div>
            ) : (
              <div className="space-y-3">
                <Properties props={props} onChange={persistProps} />
                <MarkdownEditor content={body} onChange={persistBody} placeholder="Écris… (tape / pour l'IA, [[ pour lier une page, colle une image)" onSlashPage={slashPage} onGenerate={aiOk === false ? undefined : generate} linkTargets={linkTargets} onNavigate={navigateToLabel} />
              </div>
            )
          )}
          {(display.decisions ?? []).length > 0 && (
            <div className="mt-4">
              <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Décisions clés</p>
              <ul className="space-y-1.5">
                {(display.decisions ?? []).map((d, i) => (
                  <li key={i} className="border-l-2 border-[var(--color-border)] pl-3 text-xs leading-relaxed text-[var(--color-text)]">{d}</li>
                ))}
              </ul>
            </div>
          )}
          {(display.patterns ?? []).length > 0 && (
            <div className="mt-4">
              <p className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Patterns</p>
              <ul className="space-y-0.5">
                {(display.patterns ?? []).map((p, i) => (
                  <li key={i} className="text-xs text-[var(--color-muted)]">🔁 {p}</li>
                ))}
              </ul>
            </div>
          )}
          {children.length > 0 && (
            <div className="mt-4">
              <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Pages enfants ({children.length})</p>
              <ul className="space-y-0.5">
                {children.map((child) => (
                  <li key={child.id} className="flex items-center gap-1 rounded-md px-2 py-1.5 hover:bg-[var(--color-surface-2)] transition-colors">
                    <button onClick={() => onSelect(child)} className="min-w-0 flex-1 truncate text-left text-xs text-[var(--color-text)] hover:text-[var(--color-accent)] transition-colors">{child.label}</button>
                    <button onClick={() => downloadNode(child)} title={`Exporter ${child.label}`} className="shrink-0 rounded p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"><Download className="size-3" /></button>
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
