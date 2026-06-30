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
  type BrainProgress,
} from "@/lib/api";
import { SetupScreen } from "@/components/SetupScreen";
import type {
  BrainGraph,
  BrainNode,
  ConnectorStatus,
} from "@/lib/types";
import { cn } from "@/lib/utils";

type View = "map" | "folder" | "brain";

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

  useEffect(() => {
    aiSetupNeeded().then(setNeedsSetup);
    readBrainGraph().then((g) => { if (g) { setGraph(g); setRevealKey((k) => k + 1); } });
    connectorsStatus().then(setConnectors);
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

  function handleContentSaved(nodeId: string, content: string) {
    setGraph((prev) => {
      if (!prev) return prev;
      return { ...prev, nodes: prev.nodes.map((n) => n.id === nodeId ? { ...n, content } : n) };
    });
  }

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
              graph={graph ?? { nodes: [], edges: [], markdown: "", report: "", generated_at: "" }}
              onSelect={selectNode}
              selectedId={selectedNode?.id ?? null}
              query={query}
              revealKey={revealKey}
              streamLabels={generating ? streamLabels : []}
              streamTotal={streamTotal}
            />
          )}
          {view === "folder" && graph && (
            <FolderView
              graph={graph}
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
              />
            </div>
          )}
        </>
      )}
    </div>
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
          ? "bg-[var(--color-accent)] text-white"
          : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

export default App;
