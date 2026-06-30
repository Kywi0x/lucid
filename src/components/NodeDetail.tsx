import { useState, useCallback, useRef, useEffect, useMemo } from "react";
import {
  X, FolderGit2, Brain, FileText,
  Download, RefreshCw, Sparkles, Loader2, ChevronRight, Maximize2, Minimize2, Check,
} from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { relativeDate, cn } from "@/lib/utils";
import { exportNodeMd, synthesizeNode, saveNodeContent, loadNodeContent } from "@/lib/api";
import { MarkdownEditor } from "./MarkdownEditor";
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

export function NodeDetail({ node, graph, onSelect, onClose, expanded, onExpand, onContentSaved }: Props) {
  const ancestors = useAncestors(node, graph);
  const Icon = ICON[node.kind as keyof typeof ICON] ?? FileText;
  const children = graph?.nodes.filter((n) => n.parent_id === node.id) ?? [];

  const [synth, setSynth] = useState<Partial<BrainNode> | null>(null);
  const [synthesizing, setSynthesizing] = useState(false);
  const [synthError, setSynthError] = useState<string | null>(null);
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
    if (!node.connector) { setSourceText(null); return; }
    setSourceLoading(true);
    loadNodeContent(node.id)
      .then(setSourceText)
      .catch(() => setSourceText(null))
      .finally(() => setSourceLoading(false));
  }, [node.id]);

  const editorContent = node.content || sourceText || "";

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

  return (
    <div className="flex h-full w-full flex-col bg-[var(--color-surface)]">

      {/* ── Header compact ── */}
      <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-2.5">
        <Icon className="size-4 shrink-0 text-[var(--color-accent)]" />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-sm font-semibold leading-tight">{node.label}</h3>
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
              : "text-emerald-500",
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
              disabled={synthesizing}
              title="Relancer la synthèse IA"
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

      {/* ── Corps ── */}
      {expanded ? (
        // Mode étendu : deux colonnes
        <div className="flex flex-1 overflow-hidden">
          {/* Colonne gauche — métadonnées (non éditables) */}
          <div className="w-80 shrink-0 overflow-y-auto border-r border-[var(--color-border)] px-5 py-5 space-y-5">
            {synthError && (
              <div className="rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-400">
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
            {children.length > 0 && (
              <div>
                <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">Pages enfants ({children.length})</p>
                <ul className="space-y-0.5">
                  {children.map((child) => (
                    <li key={child.id} className="flex items-center gap-1 rounded-md px-1.5 py-1.5 hover:bg-[var(--color-surface-2)] transition-colors">
                      <button onClick={() => onSelect(child)} className="min-w-0 flex-1 truncate text-left text-xs text-[var(--color-text)] hover:text-[var(--color-accent)] transition-colors">
                        {child.label}
                      </button>
                      <button onClick={() => downloadNode(child)} title={`Exporter ${child.label}`} className="shrink-0 rounded p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
                        <Download className="size-3" />
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>

          {/* Colonne droite — synthèse + contenu éditable */}
          <div className="flex-1 overflow-y-auto px-8 py-6">
            <div className="mx-auto max-w-2xl space-y-4">
              {display.summary && (
                <div className="flex gap-2.5 rounded-lg border-l-2 border-purple-400/60 bg-purple-500/5 px-3 py-2.5">
                  <Sparkles className="mt-0.5 size-3 shrink-0 text-purple-400" />
                  <div className="min-w-0">
                    <span className="text-[10px] font-semibold uppercase tracking-wide text-purple-400">
                      Synthèse IA
                      {synthTimestamp && <span className="ml-1.5 font-normal normal-case text-purple-400/60">· {relativeDate(synthTimestamp)}</span>}
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
                  <MarkdownEditor content={editorContent} onChange={handleContentChange} placeholder="Commencer à écrire…" />
                )
              )}
            </div>
          </div>
        </div>
      ) : (
        // Mode panneau : colonne unique
        <div className="flex-1 overflow-y-auto px-5 py-4">
          {synthError && (
            <div className="mb-3 rounded-lg border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-400">{synthError}</div>
          )}
          {display.summary && (
            <div className="mb-4 flex gap-2.5 rounded-lg border-l-2 border-purple-400/60 bg-purple-500/5 px-3 py-2.5">
              <Sparkles className="mt-0.5 size-3 shrink-0 text-purple-400" />
              <div className="min-w-0">
                <span className="text-[10px] font-semibold uppercase tracking-wide text-purple-400">
                  Synthèse IA
                  {synthTimestamp && <span className="ml-1.5 font-normal normal-case text-purple-400/60">· {relativeDate(synthTimestamp)}</span>}
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
              <MarkdownEditor content={editorContent} onChange={handleContentChange} placeholder="Commencer à écrire…" />
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
