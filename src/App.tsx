import { useEffect, useMemo, useState } from "react";
import {
  Network,
  FolderTree,
  FileText,
  RefreshCw,
  Search,
  Settings,
  Layers,
  MessageCircle,
  History,
  RotateCcw,
  Plus,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { BrainMap, isLeafKind } from "@/components/BrainMap";
import { CommandPalette } from "@/components/CommandPalette";
import { FolderView } from "@/components/FolderView";
import { SpacesPanel, AssistantPanel } from "@/components/LeftSidebar";
import { SettingsModal } from "@/components/SettingsModal";
import { ThemeToggle } from "@/components/ThemeToggle";
import {
  GenerateEmpty,
  GenerateProgress,
  MarkdownView,
} from "@/components/BrainView";
import { NodeDetail } from "@/components/NodeDetail";
import {
  generateBrain,
  readBrainGraph,
  connectorsStatus,
  notionSync,
  googleDriveSync,
  aiSetupNeeded,
  listSnapshots,
  restoreSnapshot,
  listSpaces,
  createSpace,
  addNodeToSpace,
  deleteSpace,
  renameSpace,
  createNoteNode,
  setNodeParent,
  type BrainProgress,
} from "@/lib/api";
import type { SnapshotInfo, Space } from "@/lib/types";
import { SetupScreen } from "@/components/SetupScreen";
import type {
  BrainGraph,
  BrainNode,
  ConnectorStatus,
} from "@/lib/types";
import { cn, relativeDate } from "@/lib/utils";

type View = "map" | "folder" | "brain";

function filterGraphBySpace(graph: BrainGraph, nodeIds: string[]): BrainGraph {
  const idSet = new Set(nodeIds);
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));
  function addAncestors(id: string) {
    const n = byId.get(id);
    if (!n?.parent_id) return;
    idSet.add(n.parent_id);
    addAncestors(n.parent_id);
  }
  nodeIds.forEach((id) => addAncestors(id));
  const filtered = new Set([...idSet]);
  const nodes = graph.nodes.filter((n) => filtered.has(n.id));
  const nodeSet = new Set(nodes.map((n) => n.id));
  const edges = graph.edges.filter((e) => nodeSet.has(e.source) && nodeSet.has(e.target));
  return { ...graph, nodes, edges };
}

function App() {
  const [view, setView]       = useState<View>("map");
  const [query, setQuery]     = useState("");
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [focus, setFocus]     = useState<{ id: string; k: number } | null>(null);
  const [leftPanel, setLeftPanel]   = useState<"spaces" | "assistant" | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);

  const [needsSetup, setNeedsSetup] = useState<boolean | null>(null);
  const [graph, setGraph]           = useState<BrainGraph | null>(null);
  const [revealKey, setRevealKey]   = useState(0);
  const [streamLabels, setStreamLabels] = useState<string[]>([]);
  const [streamTotal, setStreamTotal]   = useState(0);
  const [connectors, setConnectors] = useState<ConnectorStatus[]>([]);
  const [generating, setGenerating] = useState(false);
  const [progress, setProgress]   = useState<BrainProgress | null>(null);
  const [error, setError]         = useState<string | null>(null);

  const [selectedNode, setSelectedNode] = useState<BrainNode | null>(null);
  const [nodeExpanded, setNodeExpanded] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [snapshots, setSnapshots] = useState<SnapshotInfo[]>([]);
  const [restoring, setRestoring] = useState(false);

  const [spaces, setSpaces] = useState<Space[]>([]);
  const [activeSpaceId, setActiveSpaceId] = useState<string | null>(null);

  const [noteOpen, setNoteOpen] = useState(false);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteParent, setNoteParent] = useState("");

  const rootId = useMemo(() => graph?.nodes.find((n) => n.kind === "root")?.id ?? "", [graph]);
  const noteParents = useMemo(
    () => graph?.nodes.filter((n) => !isLeafKind(n.kind)) ?? [],
    [graph],
  );
  const lastSync = useMemo(() => {
    const syncs = connectors.filter((c) => c.last_sync).map((c) => c.last_sync!).sort();
    return syncs.length ? syncs[syncs.length - 1] : null;
  }, [connectors]);

  useEffect(() => {
    aiSetupNeeded().then(setNeedsSetup);
    readBrainGraph().then((g) => { if (g) { setGraph(g); setRevealKey((k) => k + 1); } });
    connectorsStatus().then(setConnectors);
    listSpaces().then(setSpaces);
  }, []);

  useEffect(() => {
    const unlisten = listen<BrainProgress>("brain-progress", (e) =>
      setProgress(e.payload),
    );
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const unlisten = listen<{ label: string; total: number }>("brain-node", (e) => {
      setStreamLabels((prev) => [...prev, e.payload.label]);
      setStreamTotal(e.payload.total);
    });
    return () => { unlisten.then((fn) => fn()); };
  }, []);

  async function handleGenerate() {
    setGenerating(true);
    setError(null);
    setProgress(null);
    setStreamLabels([]);
    try {
      // Sync tous les connecteurs connectés avant de régénérer.
      const syncs: Promise<unknown>[] = [];
      for (const c of connectors) {
        if (!c.connected) continue;
        if (c.id === "notion") syncs.push(notionSync().catch(() => {}));
        if (c.id === "google-drive") syncs.push(googleDriveSync().catch(() => {}));
      }
      if (syncs.length) await Promise.all(syncs);

      setGraph(await generateBrain());
      setStreamLabels([]);
      setRevealKey((k) => k + 1);
      connectorsStatus().then(setConnectors);
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
      setProgress(null);
    }
  }

  function selectNode(n: BrainNode) {
    setSelectedNode(n);
  }
  function closeDetail() {
    setSelectedNode(null);
    setNodeExpanded(false);
  }

  function closePalette() {
    setPaletteOpen(false);
    setQuery("");
  }

  function handlePaletteSelect(n: BrainNode) {
    selectNode(n);
    setFocus({ id: n.id, k: Date.now() });
    closePalette();
  }

  // Listener clavier global unique : ⌘K (palette) + Esc (fermetures en cascade).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (paletteOpen) closePalette();
        else { setQuery(""); setPaletteOpen(true); }
        return;
      }
      if (e.key !== "Escape") return;
      const t = e.target as HTMLElement | null;
      if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
      if (settingsOpen) setSettingsOpen(false);
      else if (paletteOpen) closePalette();
      else if (nodeExpanded) setNodeExpanded(false);
      else if (selectedNode) closeDetail();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [settingsOpen, paletteOpen, nodeExpanded, selectedNode]);

  async function handleOpenHistory() {
    setHistoryOpen((o) => !o);
    const list = await listSnapshots();
    setSnapshots(list);
  }

  async function handleRestore(snapshotId: string) {
    setRestoring(true);
    try {
      const g = await restoreSnapshot(snapshotId);
      setGraph(g);
      setRevealKey((k) => k + 1);
      setHistoryOpen(false);
    } finally {
      setRestoring(false);
    }
  }

  function handleContentSaved(nodeId: string, content: string) {
    setGraph((prev) => {
      if (!prev) return prev;
      return { ...prev, nodes: prev.nodes.map((n) => n.id === nodeId ? { ...n, content } : n) };
    });
  }

  function handleNodeRenamed(nodeId: string, label: string) {
    setGraph((prev) => prev ? { ...prev, nodes: prev.nodes.map((n) => n.id === nodeId ? { ...n, label } : n) } : prev);
    setSelectedNode((prev) => prev && prev.id === nodeId ? { ...prev, label } : prev);
  }

  async function handleSpaceCreate(name: string) {
    const s = await createSpace(name);
    setSpaces((prev) => [...prev, s]);
  }

  async function handleSpaceRename(id: string, name: string) {
    await renameSpace(id, name);
    setSpaces((prev) => prev.map((s) => (s.id === id ? { ...s, name } : s)));
  }

  async function handleAddNodeToSpace(nodeId: string, spaceId: string) {
    await addNodeToSpace(spaceId, nodeId);
    // Optimistic update — pas d'aller-retour serveur
    setSpaces((prev) =>
      prev.map((s) =>
        s.id === spaceId && s.node_ids !== null
          ? { ...s, node_ids: [...new Set([...(s.node_ids ?? []), nodeId])] }
          : s,
      ),
    );
  }

  async function refreshGraph() {
    const g = await readBrainGraph();
    if (g) { setGraph(g); setRevealKey((k) => k + 1); }
  }

  async function handleCreateNote(parentId: string, title: string) {
    const n = await createNoteNode(parentId, title);
    await refreshGraph();
    setSelectedNode(n);
  }

  async function handleMoveNode(nodeId: string, parentId: string) {
    await setNodeParent(nodeId, parentId);
    await refreshGraph();
  }

  async function handleSpaceDelete(id: string) {
    await deleteSpace(id);
    setSpaces((prev) => prev.filter((s) => s.id !== id));
    if (activeSpaceId === id) setActiveSpaceId(null);
  }

  const activeSpace = spaces.find((s) => s.id === activeSpaceId);
  const displayGraph = graph && activeSpace?.node_ids
    ? filterGraphBySpace(graph, activeSpace.node_ids)
    : graph;

  if (needsSetup) {
    return <SetupScreen onDone={() => setNeedsSetup(false)} />;
  }

  return (
    <div className="relative h-screen overflow-hidden bg-[var(--color-bg)] text-[var(--color-text)]">

      {!graph && !generating ? (
        <GenerateEmpty error={error} onGenerate={handleGenerate} />
      ) : (
        <>
          {/* ── Canvas principal ── */}
          {view === "map" && (graph || generating) && (
            <BrainMap
              graph={displayGraph ?? { nodes: [], edges: [], markdown: "", report: "", generated_at: "" }}
              onSelect={selectNode}
              selectedId={selectedNode?.id ?? null}
              query={query}
              revealKey={revealKey}
              streamLabels={generating ? streamLabels : []}
              streamTotal={streamTotal}
              spaces={spaces}
              onAddNodeToSpace={handleAddNodeToSpace}
              onMoveNode={handleMoveNode}
              onBackgroundClick={closeDetail}
              panelOffset={selectedNode && !nodeExpanded ? 480 : 0}
              focus={focus}
            />
          )}
          {view === "folder" && displayGraph && (
            <FolderView
              graph={displayGraph}
              onSelect={selectNode}
              selectedId={selectedNode?.id ?? null}
              query={query}
            />
          )}
          {view === "brain" && graph && (
            <MarkdownView markdown={graph.markdown} onRegenerate={handleGenerate} />
          )}

          {/* ── Overlay progression ── */}
          {generating && (
            <div className="absolute inset-0 z-20 flex items-end justify-center pb-24 pointer-events-none">
              <div className="pointer-events-auto rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)]/90 backdrop-blur-md px-6 py-4 shadow-[var(--shadow-float)] min-w-[320px]">
                <GenerateProgress progress={progress} />
              </div>
            </div>
          )}

          {/* ── Dock de widgets (bord gauche) ── */}
          <div className="absolute left-3 top-1/2 z-30 flex -translate-y-1/2 flex-col gap-2">
            <DockBtn
              active={leftPanel === "spaces"}
              title="Spaces"
              onClick={() => setLeftPanel((p) => (p === "spaces" ? null : "spaces"))}
            >
              <Layers className="size-4" />
            </DockBtn>
            <DockBtn
              active={leftPanel === "assistant"}
              title="Assistant"
              onClick={() => setLeftPanel((p) => (p === "assistant" ? null : "assistant"))}
            >
              <MessageCircle className="size-4" />
            </DockBtn>
          </div>

          {/* ── Panneau outil gauche ── */}
          {leftPanel && (
            <div className="panel absolute bottom-4 left-16 top-4 z-30 flex w-[360px] flex-col overflow-hidden rounded-2xl animate-slideInLeft">
              {leftPanel === "spaces" ? (
                <SpacesPanel
                  spaces={spaces}
                  activeSpaceId={activeSpaceId}
                  onSpaceSelect={setActiveSpaceId}
                  onClose={() => setLeftPanel(null)}
                />
              ) : (
                <AssistantPanel onClose={() => setLeftPanel(null)} />
              )}
            </div>
          )}

          {/* ── HUD (pouls du cerveau) ── */}
          {view === "map" && graph && !generating && (
            <div className="pointer-events-none absolute left-4 top-4 z-10 flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-wider text-[var(--color-muted)]">
              <span className="size-1.5 rounded-full bg-[var(--color-ok)]" />
              {graph.nodes.length} nœuds · {spaces.length} espaces
              {lastSync && <> · sync {relativeDate(lastSync)}</>}
            </div>
          )}

          {/* ── Modale nouvelle note ── */}
          {noteOpen && graph && (
            <div className="absolute inset-0 z-40 flex items-center justify-center bg-black/40" onClick={() => setNoteOpen(false)}>
              <div
                onClick={(e) => e.stopPropagation()}
                className="panel w-[340px] rounded-2xl p-4"
              >
                <p className="mb-3 text-sm font-semibold text-[var(--color-text)]">Nouvelle note</p>
                <input
                  autoFocus value={noteTitle} onChange={(e) => setNoteTitle(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && noteTitle.trim()) { handleCreateNote(noteParent || rootId, noteTitle); setNoteOpen(false); }
                    if (e.key === "Escape") { e.stopPropagation(); setNoteOpen(false); }
                  }}
                  placeholder="Titre de la note"
                  className="mb-3 w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
                />
                <label className="mb-1 block text-[11px] uppercase tracking-wide text-[var(--color-muted)]">Rattacher à</label>
                <select
                  value={noteParent} onChange={(e) => setNoteParent(e.target.value)}
                  className="mb-4 w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none"
                >
                  {noteParents.map((n) => (
                    <option key={n.id} value={n.id}>{n.kind === "root" ? "Lucid (racine)" : n.label}</option>
                  ))}
                </select>
                <div className="flex justify-end gap-2">
                  <button onClick={() => setNoteOpen(false)} className="rounded-lg px-3 py-1.5 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)]">Annuler</button>
                  <button
                    onClick={() => { if (noteTitle.trim()) { handleCreateNote(noteParent || rootId, noteTitle); setNoteOpen(false); } }}
                    className="rounded-lg bg-[var(--color-accent)] px-3 py-1.5 text-sm text-white hover:bg-[var(--color-accent-hover)]"
                  >
                    Créer
                  </button>
                </div>
              </div>
            </div>
          )}

          {/* ── Palette de commande ⌘K ── */}
          {paletteOpen && displayGraph && (
            <CommandPalette
              graph={displayGraph}
              query={query}
              onQueryChange={setQuery}
              onSelect={handlePaletteSelect}
              onClose={closePalette}
            />
          )}

          {/* ── Barre flottante bas-centre ── */}
          <div className="pointer-events-none absolute bottom-4 left-1/2 z-10 -translate-x-1/2">
            <div className="panel pointer-events-auto flex items-center gap-1 rounded-2xl px-3 py-2">
              {/* Marque */}
              <div className="mr-1 flex items-center gap-2 pl-1.5 pr-3 border-r border-[var(--color-border)]">
                <span className="size-2 rounded-full bg-[var(--color-accent)] shadow-[0_0_8px_var(--color-accent)]" />
                <span className="font-mono text-xs font-semibold tracking-[0.2em]">LUCID</span>
              </div>

              {/* Modes */}
              <ViewBtn active={view === "map"}    onClick={() => setView("map")}>
                <Network    className="size-4" /> Mind
              </ViewBtn>
              <ViewBtn active={view === "folder"} onClick={() => setView("folder")}>
                <FolderTree className="size-4" /> Dossiers
              </ViewBtn>
              <ViewBtn active={view === "brain"}  onClick={() => setView("brain")}>
                <FileText   className="size-4" /> brain.md
              </ViewBtn>

              {/* Recherche ⌘K */}
              <div className="ml-1 flex items-center pl-3 border-l border-[var(--color-border)]">
                <button
                  onClick={() => { setQuery(""); setPaletteOpen(true); }}
                  className="flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                >
                  <Search className="size-4" />
                  <kbd className="rounded border border-[var(--color-border)] px-1 py-0.5 font-mono text-[9px]">⌘K</kbd>
                </button>
              </div>

              {/* Actions */}
              <div className="ml-1 flex items-center gap-0.5 pl-3 border-l border-[var(--color-border)]">
                {graph && (
                  <button
                    onClick={handleGenerate}
                    title="Régénérer"
                    className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                  >
                    <RefreshCw className="size-4" />
                  </button>
                )}
                <div className="relative">
                  <button
                    onClick={handleOpenHistory}
                    title="Historique des snapshots"
                    className={cn(
                      "rounded-lg p-1.5 transition-colors",
                      historyOpen
                        ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                        : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
                    )}
                  >
                    <History className="size-4" />
                  </button>
                  {historyOpen && (
                    <div className="absolute bottom-full right-0 mb-2 w-72 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)] overflow-hidden">
                      <div className="px-3 py-2 border-b border-[var(--color-border)] text-xs font-semibold text-[var(--color-muted)] uppercase tracking-wider">
                        Snapshots
                      </div>
                      {snapshots.length === 0 ? (
                        <p className="px-3 py-4 text-xs text-[var(--color-muted)] text-center">
                          Aucun snapshot — régénère le graphe pour en créer un.
                        </p>
                      ) : (
                        <ul className="max-h-64 overflow-y-auto">
                          {snapshots.map((s) => (
                            <li key={s.id} className="flex items-center justify-between gap-2 px-3 py-2 hover:bg-[var(--color-surface-2)] transition-colors">
                              <div className="min-w-0">
                                <p className="text-xs font-medium text-[var(--color-text)] truncate">
                                  {relativeTime(s.created_at)}
                                </p>
                                <p className="text-[10px] text-[var(--color-muted)]">{s.node_count} nœuds</p>
                              </div>
                              <button
                                onClick={() => handleRestore(s.id)}
                                disabled={restoring}
                                title="Restaurer ce snapshot"
                                className="shrink-0 flex items-center gap-1 rounded-lg px-2 py-1 text-[10px] font-medium text-[var(--color-accent)] hover:bg-[var(--color-accent-soft)] transition-colors disabled:opacity-40"
                              >
                                <RotateCcw className="size-3" />
                                Restaurer
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </div>
                  )}
                </div>
                {graph && (
                  <button
                    onClick={() => { setNoteTitle(""); setNoteParent(rootId); setNoteOpen(true); }}
                    title="Nouvelle note"
                    className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                  >
                    <Plus className="size-4" />
                  </button>
                )}
                <button
                  onClick={() => setSettingsOpen(true)}
                  title="Paramètres"
                  className="rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)] transition-colors"
                >
                  <Settings className="size-4" />
                </button>
                <ThemeToggle />
              </div>
            </div>
          </div>

          {/* ── Modale Paramètres ── */}
          {settingsOpen && (
            <SettingsModal
              connectors={connectors}
              spaces={spaces}
              onRefresh={() => connectorsStatus().then(setConnectors)}
              onSyncDone={handleGenerate}
              onClose={() => setSettingsOpen(false)}
              onSpaceCreate={handleSpaceCreate}
              onSpaceRename={handleSpaceRename}
              onSpaceDelete={handleSpaceDelete}
            />
          )}

          {/* ── Panneau détail (montage unique : panneau ⇄ plein écran) ── */}
          {selectedNode && (
            <div
              className={cn(
                "absolute overflow-hidden border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]",
                nodeExpanded
                  ? "inset-3 z-40 rounded-2xl"
                  : "bottom-3 right-3 top-3 z-10 w-[480px] rounded-xl animate-slideInRight",
              )}
            >
              <NodeDetail
                node={selectedNode}
                graph={graph}
                onSelect={setSelectedNode}
                onClose={closeDetail}
                expanded={nodeExpanded}
                onExpand={() => setNodeExpanded((v) => !v)}
                onContentSaved={handleContentSaved}
                onCreateNote={handleCreateNote}
                onNodeRenamed={handleNodeRenamed}
              />
            </div>
          )}
        </>
      )}
    </div>
  );
}

function relativeTime(ts: number): string {
  const diff = Math.floor(Date.now() / 1000) - ts;
  if (diff < 60)    return "il y a quelques secondes";
  if (diff < 3600)  return `il y a ${Math.floor(diff / 60)} min`;
  if (diff < 86400) return `il y a ${Math.floor(diff / 3600)} h`;
  return `il y a ${Math.floor(diff / 86400)} j`;
}

function DockBtn({
  active, title, onClick, children,
}: {
  active: boolean;
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      title={title}
      className={cn(
        "panel flex size-[38px] items-center justify-center rounded-xl transition-colors",
        active
          ? "text-[var(--color-accent)] !border-[var(--color-accent)]/40"
          : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

function ViewBtn({
  active, onClick, children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm transition-colors",
        active
          ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
          : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

export default App;
