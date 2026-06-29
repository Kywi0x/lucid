import {
  X,
  FolderGit2,
  Lightbulb,
  Brain,
  Terminal,
  FileText,
  ExternalLink,
  MessageSquare,
} from "lucide-react";
import { openPath } from "@tauri-apps/plugin-opener";
import type { BrainNode, SourceRef } from "@/lib/types";
import { relativeDate } from "@/lib/utils";

interface Props {
  node: BrainNode;
  onOpenSource: (source: SourceRef) => void;
  onClose: () => void;
}

const ICON = { root: Brain, project: FolderGit2, concept: Lightbulb } as const;
const KIND_LABEL = { root: "Cerveau", project: "Projet", concept: "Concept" } as const;

/** Métadonnées d'affichage par connecteur (extensible : cowork, drive…). */
const CONNECTOR = {
  "claude-code": { label: "Claude Code", icon: Terminal },
} as const;

function connectorMeta(id: string) {
  return CONNECTOR[id as keyof typeof CONNECTOR] ?? { label: id, icon: FileText };
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mb-4">
      <h4 className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
        {title}
      </h4>
      {children}
    </div>
  );
}

export function NodeDetail({ node, onOpenSource, onClose }: Props) {
  const Icon = ICON[node.kind] ?? Lightbulb;

  return (
    <div className="flex h-full w-full flex-col bg-[var(--color-surface)]">
      <div className="flex items-start gap-2 border-b border-[var(--color-border)] p-3">
        <Icon className="mt-0.5 size-5 shrink-0 text-[var(--color-accent)]" />
        <div className="min-w-0 flex-1">
          <p className="text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
            {KIND_LABEL[node.kind] ?? "Nœud"}
            {node.kind !== "root" && ` · poids ${node.weight}`}
          </p>
          <h3 className="break-words text-sm font-semibold">{node.label}</h3>
        </div>
        <button
          onClick={onClose}
          className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
        >
          <X className="size-4" />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-3">
        {node.summary && (
          <Section title="Résumé">
            <p className="text-sm leading-relaxed text-[var(--color-text)]">
              {node.summary}
            </p>
          </Section>
        )}

        {node.keywords.length > 0 && (
          <Section title="Mots-clés">
            <div className="flex flex-wrap gap-1.5">
              {node.keywords.map((k) => (
                <span
                  key={k}
                  className="rounded-full border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-0.5 text-xs text-[var(--color-muted)]"
                >
                  {k}
                </span>
              ))}
            </div>
          </Section>
        )}

        {node.decisions.length > 0 && (
          <Section title="Décisions clés">
            <ul className="space-y-1.5">
              {node.decisions.map((d, i) => (
                <li
                  key={i}
                  className="rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-xs leading-relaxed"
                >
                  {d}
                </li>
              ))}
            </ul>
          </Section>
        )}

        {node.patterns.length > 0 && (
          <Section title="Patterns récurrents">
            <ul className="space-y-1">
              {node.patterns.map((p, i) => (
                <li key={i} className="text-xs text-[var(--color-muted)]">
                  🔁 {p}
                </li>
              ))}
            </ul>
          </Section>
        )}

        {node.sources.length > 0 && (
          <Section title={`Sources (${node.sources.length})`}>
            <ul className="space-y-1.5">
              {node.sources.map((s, i) => {
                const meta = connectorMeta(s.connector);
                const CIcon = meta.icon;
                return (
                  <li
                    key={`${s.connector}:${s.id}:${i}`}
                    className="group flex items-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-2.5 py-2"
                  >
                    <button
                      onClick={() => onOpenSource(s)}
                      className="flex min-w-0 flex-1 items-center gap-2 text-left"
                      title="Ouvrir la conversation"
                    >
                      <CIcon className="size-3.5 shrink-0 text-[var(--color-accent)]" />
                      <span className="min-w-0 flex-1">
                        <span className="line-clamp-1 text-xs text-[var(--color-text)]">
                          {s.title}
                        </span>
                        <span className="flex items-center gap-1.5 text-[10px] text-[var(--color-muted)]">
                          {meta.label}
                          {s.timestamp && (
                            <>
                              <span>·</span>
                              {relativeDate(s.timestamp)}
                            </>
                          )}
                        </span>
                      </span>
                      <MessageSquare className="size-3 shrink-0 text-[var(--color-muted)] opacity-0 group-hover:opacity-100" />
                    </button>
                    {s.link && (
                      <button
                        onClick={() => openPath(s.link!).catch(() => {})}
                        title={`Ouvrir : ${s.link}`}
                        className="rounded p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
                      >
                        <ExternalLink className="size-3.5" />
                      </button>
                    )}
                  </li>
                );
              })}
            </ul>
          </Section>
        )}

        {!node.summary &&
          node.keywords.length === 0 &&
          node.sources.length === 0 && (
            <p className="text-sm text-[var(--color-muted)]">Aucun détail.</p>
          )}
      </div>
    </div>
  );
}
