import { useEffect, useRef, useState, useCallback } from "react";
import { listen } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";
import { Inbox, FilePlus2, FilePen, FileX2 } from "lucide-react";
import { inboxRecent, type InboxEntry } from "@/lib/api";
import { notify } from "@/lib/notify";
import { cn } from "@/lib/utils";
import { ConnectorLogo } from "@/components/SettingsModal";
import type { BrainGraph, BrainNode } from "@/lib/types";

// ponytail: skeleton fonctionnel (structure + logique). Placement/skin à aligner
// sur la maquette Figma de Liam — flux PASSIF (visibilité), jamais de validation.

// Source Inbox (backend) → id connecteur (pour réutiliser ConnectorLogo).
const CONNECTOR_ID: Record<string, string> = {
  local: "local-folder",
  obsidian: "obsidian",
  "apple-notes": "apple-notes",
  "claude-code": "claude-code",
};

function since(atSecs: number): string {
  const s = Math.max(0, Math.floor(Date.now() / 1000 - atSecs));
  if (s < 60) return "à l'instant";
  if (s < 3600) return `il y a ${Math.floor(s / 60)} min`;
  if (s < 86400) return `il y a ${Math.floor(s / 3600)} h`;
  return `il y a ${Math.floor(s / 86400)} j`;
}

const KIND = {
  added:    { icon: FilePlus2, label: "ajouté",   color: "var(--color-ok)" },
  modified: { icon: FilePen,   label: "modifié",  color: "var(--color-accent)" },
  deleted:  { icon: FileX2,    label: "supprimé", color: "var(--color-err)" },
} as const;

/** Retrouve la page (nœud feuille) correspondant à un fichier de l'Inbox. */
function matchNode(entry: InboxEntry, graph: BrainGraph | null): BrainNode | null {
  if (!graph) return null;
  const leaves = graph.nodes.filter((n) => n.connector);
  const base = entry.name.replace(/\.[^.]+$/, "");
  // 1) chemin exact via source_id "dossier::rel" (local / Obsidian).
  const byPath = leaves.find((n) => n.source_id && n.source_id.split("::").join("/") === entry.path);
  if (byPath) return byPath;
  // 2) source_id == nom sans extension (session Claude Code).
  const bySrcId = leaves.find((n) => n.source_id === base);
  if (bySrcId) return bySrcId;
  // 3) repli : label == nom sans extension.
  return leaves.find((n) => n.label === base) ?? null;
}

export function InboxPanel({ graph, onOpenNode }: { graph: BrainGraph | null; onOpenNode: (n: BrainNode) => void }) {
  const [open, setOpen] = useState(false);
  const [entries, setEntries] = useState<InboxEntry[]>([]);
  // Horodatage du plus récent déjà "vu" — pour ne notifier que le VRAI nouveau
  // (et jamais au 1er chargement, où tout est "nouveau" par construction).
  const lastSeen = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    const list = await inboxRecent().catch(() => [] as InboxEntry[]);
    setEntries(list);
    const maxAt = list.reduce((m, e) => Math.max(m, e.at), 0);
    if (lastSeen.current === null) { lastSeen.current = maxAt; return; } // 1er chargement : pas de notif
    // Notif desktop : résumé du nouveau contenu DOCUMENT (jamais Claude Code —
    // une session active écrit en continu et spammerait).
    const fresh = list.filter((e) => e.at > lastSeen.current! && e.source !== "claude-code");
    lastSeen.current = Math.max(lastSeen.current, maxAt);
    if (fresh.length === 1) {
      const k = KIND[fresh[0].kind]?.label ?? "changé";
      void notify("Lucid", `Fichier ${k} : ${fresh[0].name}`);
    } else if (fresh.length > 1) {
      void notify("Lucid", `${fresh.length} fichiers détectés (ajoutés / modifiés / supprimés)`);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Le watcher backend émet "inbox-updated" ~3 s après un changement fs.
    const un = listen("inbox-updated", () => void refresh());
    return () => { un.then((f) => f()); };
  }, [refresh]);

  // Clic : ouvre la PAGE Lucid du fichier. Si la page n'existe pas encore
  // (fichier tout juste détecté, régé pas encore passée), repli sur l'ouverture
  // du fichier sur le disque. Supprimé → rien (ni page ni fichier).
  async function handleOpen(e: InboxEntry) {
    const node = matchNode(e, graph);
    if (node) { onOpenNode(node); setOpen(false); return; }
    if (e.kind === "deleted" || !e.path) return; // path vide = source sans fichier (Notes Apple)
    try { await openPath(e.path); } catch { /* fichier déplacé/inaccessible : silencieux */ }
  }

  return (
    <div className="fixed bottom-4 left-4 z-40">
      {open && (
        <div className="mb-2 flex max-h-[50vh] w-80 flex-col overflow-hidden rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] shadow-[var(--shadow-float)]">
          <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-2.5">
            <span className="text-sm font-semibold text-[var(--color-text)]">Fichiers récents</span>
            <span className="font-mono text-[10px] uppercase tracking-wider text-[var(--color-muted)]">{entries.length}</span>
          </div>
          <div className="flex-1 overflow-y-auto">
            {entries.length === 0 ? (
              <p className="px-4 py-6 text-center text-xs text-[var(--color-muted)]">
                Aucun changement détecté récemment.
              </p>
            ) : (
              entries.map((e, i) => {
                const k = KIND[e.kind] ?? KIND.modified;
                const Icon = k.icon;
                const clickable = e.kind !== "deleted";
                return (
                  <button
                    key={`${e.path}-${i}`}
                    onClick={() => handleOpen(e)}
                    disabled={!clickable}
                    title={clickable ? `${k.label} — ouvrir ${e.name}` : `${e.name} (supprimé)`}
                    className={cn(
                      "flex w-full items-center gap-2.5 border-b border-[var(--color-border)] px-3 py-2 text-left last:border-b-0",
                      clickable ? "hover:bg-[var(--color-surface-2)] transition-colors" : "cursor-default opacity-60",
                    )}
                  >
                    <ConnectorLogo id={CONNECTOR_ID[e.source] ?? e.source} />
                    <span className="min-w-0 flex-1 truncate text-xs text-[var(--color-text)]">{e.name}</span>
                    {e.count > 1 && (
                      <span
                        className="shrink-0 rounded-full bg-[var(--color-surface-2)] px-1.5 py-0.5 font-mono text-[9px] text-[var(--color-muted)]"
                        title={`${e.count} événements sur ce fichier`}
                      >
                        ×{e.count}
                      </span>
                    )}
                    <Icon className="size-3.5 shrink-0" style={{ color: k.color }} />
                    <span className="shrink-0 font-mono text-[10px] text-[var(--color-muted)]">{since(e.at)}</span>
                  </button>
                );
              })
            )}
          </div>
        </div>
      )}
      <button
        onClick={() => { setOpen((o) => !o); void refresh(); }}
        title="Fichiers récents (ajoutés / modifiés / supprimés)"
        className={cn(
          "flex items-center gap-2 rounded-full border px-3 py-2 text-xs font-medium shadow-[var(--shadow-float)] transition-colors",
          open
            ? "border-[var(--color-accent)] bg-[var(--color-accent-soft)] text-[var(--color-accent)]"
            : "border-[var(--color-border)] bg-[var(--color-surface)] text-[var(--color-muted)] hover:text-[var(--color-text)]",
        )}
      >
        <Inbox className="size-4" />
        Inbox
        {entries.length > 0 && (
          <span className="rounded-full bg-[var(--color-accent)] px-1.5 text-[10px] font-semibold text-white">{entries.length}</span>
        )}
      </button>
    </div>
  );
}
