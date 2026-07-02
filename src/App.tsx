import { useEffect, useState } from "react";
import {
  Brain,
  Network,
  FolderTree,
  FileText,
  RefreshCw,
  Search,
  ChevronLeft,
  ChevronRight,
  PanelLeft,
  History,
  RotateCcw,
} from "lucide-react";
import { listen } from "@tauri-apps/api/event";
import { BrainMap } from "@/components/BrainMap";
import { FolderView } from "@/components/FolderView";
import { LeftSidebar } from "@/components/LeftSidebar";
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
import { cn } from "@/lib/utils";

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
  const [panelOpen, setPanelOpen]   = useState(false);
  const [leftOpen, setLeftOpen]     = useState(false);

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
    setPanelOpen(true);
  }
  function closeDetail() {
    setSelectedNode(null);
    setPanelOpen(false);
    setNodeExpanded(false);
  }

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

  const hasDetail = !!selectedNode;

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
              onCreateNote={handleCreateNote}
              onMoveNode={handleMoveNode}
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

          {/* ── Sidebar gauche flottante ── */}
          {leftOpen && (
            <div className="glass absolute bottom-4 left-4 top-4 z-30 flex w-[360px] flex-col overflow-hidden rounded-2xl animate-slideInLeft">
              <LeftSidebar
                connectors={connectors}
                onRefresh={() => connectorsStatus().then(setConnectors)}
                onSyncDone={handleGenerate}
                onClose={() => setLeftOpen(false)}
                spaces={spaces}
                activeSpaceId={activeSpaceId}
                onSpaceSelect={setActiveSpaceId}
                onSpaceCreate={handleSpaceCreate}
                onSpaceDelete={handleSpaceDelete}
              />
            </div>
          )}

          {/* ── Barre de recherche ── */}
          {view !== "brain" && (
            <div className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2 z-10">
              <div className="pointer-events-auto flex w-72 items-center gap-2 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)]/90 backdrop-blur-md px-3 py-2 shadow-[var(--shadow-float)]">
                <Search className="size-4 shrink-0 text-[var(--color-muted)]" />
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Rechercher…"
                  className="w-full bg-transparent text-sm outline-none placeholder:text-[var(--color-muted)]"
                />
                {query && (
                  <button
                    onClick={() => setQuery("")}
                    className="text-xs text-[var(--color-muted)] hover:text-[var(--color-text)]"
                  >
                    ✕
                  </button>
                )}
              </div>
            </div>
          )}

          {/* ── Barre flottante bas-centre ── */}
          <div className="pointer-events-none absolute bottom-4 left-1/2 z-10 -translate-x-1/2">
            <div className="glass pointer-events-auto flex items-center gap-1 rounded-2xl px-3 py-2">
              {/* Toggle sidebar gauche */}
              <button
                onClick={() => setLeftOpen((o) => !o)}
                title="Connecteurs & Assistant"
                className={cn(
                  "mr-1 rounded-lg p-1.5 transition-colors",
                  leftOpen
                    ? "bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
                    : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
                )}
              >
                <PanelLeft className="size-4" />
              </button>

              {/* Logo */}
              <div className="mr-1 flex items-center gap-1.5 pr-3 border-r border-[var(--color-border)]">
                <Brain className="size-4 text-[var(--color-accent)]" />
                <span className="text-sm font-semibold">Lucid</span>
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
                <ThemeToggle />
              </div>
            </div>
          </div>

          {/* ── Panneau détail droit ── */}
          {hasDetail && !panelOpen && (
            <button
              onClick={() => setPanelOpen(true)}
              className="absolute right-0 top-1/2 z-10 -translate-y-1/2 flex h-16 w-5 items-center justify-center rounded-l-lg border border-r-0 border-[var(--color-border)] bg-[var(--color-surface)] text-[var(--color-muted)] shadow-[var(--shadow-float)] hover:text-[var(--color-text)] transition-colors"
              title="Ouvrir le détail"
            >
              <ChevronLeft className="size-3.5" />
            </button>
          )}

          {hasDetail && panelOpen && !nodeExpanded && (
            <>
              <div className="absolute bottom-3 right-3 top-3 z-10 w-[480px] overflow-hidden rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]">
                {selectedNode && (
                  <NodeDetail
                    node={selectedNode}
                    graph={graph}
                    onSelect={setSelectedNode}
                    onClose={closeDetail}
                    expanded={false}
                    onExpand={() => setNodeExpanded(true)}
                    onContentSaved={handleContentSaved}
                    onCreateNote={handleCreateNote}
                    onNodeRenamed={handleNodeRenamed}
                  />
                )}
              </div>
              <button
                onClick={() => setPanelOpen(false)}
                className="absolute right-[483px] top-1/2 z-20 -translate-y-1/2 flex h-10 w-5 items-center justify-center rounded-l-lg border border-r-0 border-[var(--color-border)] bg-[var(--color-surface)] text-[var(--color-muted)] hover:text-[var(--color-text)] transition-colors"
                title="Masquer"
              >
                <ChevronRight className="size-3.5" />
              </button>
            </>
          )}

          {/* ── Modal pleine page (mode étendu) ── */}
          {hasDetail && nodeExpanded && selectedNode && (
            <div
              className="absolute inset-3 z-40 overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]"
            >
              <NodeDetail
                node={selectedNode}
                graph={graph}
                onSelect={(n) => { setSelectedNode(n); }}
                onClose={closeDetail}
                expanded
                onExpand={() => setNodeExpanded(false)}
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
          ? "bg-[var(--color-accent)] text-white"
          : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

export default App;
