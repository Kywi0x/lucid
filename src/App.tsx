import { useEffect, useState } from "react";
import {
  Brain,
  Network,
  FolderTree,
  FileText,
  RefreshCw,
  Search,
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
import { ConversationDetail } from "@/components/ConversationDetail";
import { NodeDetail } from "@/components/NodeDetail";
import {
  generateBrain,
  readBrainGraph,
  connectorsStatus,
  type BrainProgress,
} from "@/lib/api";
import type {
  BrainGraph,
  BrainNode,
  ConnectorStatus,
  ConversationSummary,
  SourceRef,
} from "@/lib/types";
import { cn } from "@/lib/utils";

type View = "map" | "folder" | "brain";

function App() {
  const [view, setView] = useState<View>("map");
  const [query, setQuery] = useState("");

  const [graph, setGraph] = useState<BrainGraph | null>(null);
  const [connectors, setConnectors] = useState<ConnectorStatus[]>([]);
  const [generating, setGenerating] = useState(false);
  const [progress, setProgress] = useState<BrainProgress | null>(null);
  const [error, setError] = useState<string | null>(null);

  const [selectedNode, setSelectedNode] = useState<BrainNode | null>(null);
  const [selectedConv, setSelectedConv] = useState<ConversationSummary | null>(
    null,
  );

  useEffect(() => {
    readBrainGraph().then((g) => g && setGraph(g));
    connectorsStatus().then(setConnectors);
  }, []);

  useEffect(() => {
    const unlisten = listen<BrainProgress>("brain-progress", (e) =>
      setProgress(e.payload),
    );
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  async function handleGenerate() {
    setGenerating(true);
    setError(null);
    setProgress(null);
    try {
      setGraph(await generateBrain());
      connectorsStatus().then(setConnectors);
    } catch (e) {
      setError(String(e));
    } finally {
      setGenerating(false);
      setProgress(null);
    }
  }

  function selectNode(n: BrainNode) {
    setSelectedConv(null);
    setSelectedNode(n);
  }
  function openSource(s: SourceRef) {
    setSelectedNode(null);
    setSelectedConv({
      id: s.id,
      title: s.title,
      project: s.link ?? "",
      project_slug: s.project_slug,
      source: s.connector,
      container_path: [],
      message_count: 0,
      first_timestamp: null,
      last_timestamp: s.timestamp,
    });
  }
  function closeDetail() {
    setSelectedNode(null);
    setSelectedConv(null);
  }

  const showDetail = selectedNode || selectedConv;

  return (
    <div className="flex h-screen flex-col bg-[var(--color-bg)] text-[var(--color-text)]">
      {/* Barre supérieure */}
      <header className="flex h-12 shrink-0 items-center gap-3 border-b border-[var(--color-border)] px-4">
        <Brain className="size-5 text-[var(--color-accent)]" />
        <span className="font-semibold">Lucid</span>

        <div className="ml-4 flex items-center gap-1 rounded-lg bg-[var(--color-surface)] p-0.5">
          <ViewBtn active={view === "map"} onClick={() => setView("map")}>
            <Network className="size-4" /> Mind
          </ViewBtn>
          <ViewBtn active={view === "folder"} onClick={() => setView("folder")}>
            <FolderTree className="size-4" /> Dossiers
          </ViewBtn>
          <ViewBtn active={view === "brain"} onClick={() => setView("brain")}>
            <FileText className="size-4" /> brain.md
          </ViewBtn>
        </div>

        <div className="ml-auto flex items-center gap-1">
          {graph && (
            <button
              onClick={handleGenerate}
              className="flex items-center gap-1.5 rounded-md px-2 py-1 text-sm text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
            >
              <RefreshCw className="size-4" />
              Régénérer
            </button>
          )}
          <ThemeToggle />
        </div>
      </header>

      {/* Corps */}
      <main className="relative min-h-0 flex-1 overflow-hidden">
        {!graph && !generating ? (
          <GenerateEmpty error={error} onGenerate={handleGenerate} />
        ) : (
          <>
            {/* Canvas — reste visible pendant la régénération */}
            {view === "map" && graph && (
              <BrainMap
                graph={graph}
                onSelect={selectNode}
                selectedId={selectedNode?.id ?? null}
                query={query}
              />
            )}
            {view === "folder" && graph && (
              <FolderView
                graph={graph}
                onSelect={selectNode}
                onOpenSource={openSource}
                selectedId={selectedNode?.id ?? null}
                query={query}
              />
            )}
            {view === "brain" && graph && (
              <MarkdownView markdown={graph.markdown} onRegenerate={handleGenerate} />
            )}

            {/* Overlay de progression — graphe existant reste visible en fond */}
            {generating && (
              <div className="absolute inset-0 z-20 flex items-end justify-center pb-8 pointer-events-none">
                <div className="pointer-events-auto rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)]/90 backdrop-blur-md px-6 py-4 shadow-[var(--shadow-float)] min-w-[320px]">
                  <GenerateProgress progress={progress} />
                </div>
              </div>
            )}

            {/* Sidebar flottante gauche */}
            <div className="pointer-events-none absolute bottom-3 left-3 top-3 flex">
              <LeftSidebar
                connectors={connectors}
                onRefresh={() => connectorsStatus().then(setConnectors)}
                onSyncDone={handleGenerate}
              />
            </div>

            {/* Barre de recherche globale (Mind + Dossiers) */}
            {view !== "brain" && (
              <div className="pointer-events-none absolute left-1/2 top-3 -translate-x-1/2">
                <div className="pointer-events-auto flex w-80 items-center gap-2 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface)] px-3 py-2 shadow-[var(--shadow-float)]">
                  <Search className="size-4 text-[var(--color-muted)]" />
                  <input
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Rechercher dans tous les projets…"
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

            {/* Panneau détail flottant à droite */}
            {showDetail && (
              <div className="absolute bottom-3 right-3 top-3 w-[380px] overflow-hidden rounded-xl border border-[var(--color-border)] shadow-[var(--shadow-float)]">
                {selectedNode && (
                  <NodeDetail
                    node={selectedNode}
                    onOpenSource={openSource}
                    onClose={closeDetail}
                  />
                )}
                {selectedConv && (
                  <ConversationDetail summary={selectedConv} onClose={closeDetail} />
                )}
              </div>
            )}
          </>
        )}
      </main>
    </div>
  );
}

function ViewBtn({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded-md px-3 py-1 text-sm transition-colors",
        active
          ? "bg-[var(--color-accent)] text-white"
          : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
      )}
    >
      {children}
    </button>
  );
}

export default App;
