import { useEffect, useState } from "react";
import { ArrowLeft, Check, Copy, Eye, FolderDown, Loader2 } from "lucide-react";
import { BrainMap } from "@/components/BrainMap";
import { ReadOnlyDetail } from "@/components/ReadOnlyDetail";
import { fetchSharedSpace, sharedSpaceUrl } from "@/lib/share";
import { payloadToGraph } from "@/lib/shared-space";
import { importSharedSpace } from "@/lib/api";
import { supabase } from "@/lib/supabase";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { cn, copyText } from "@/lib/utils";

/** Un space partagé avec moi, ouvert DANS l'app — lecture seule, avec le geste
 *  « Copier dans mon cerveau » (fork V1) : il devient un projet à moi. */
export function RemoteSpaceView({ spaceId, onClose, onForked }: {
  spaceId: string;
  onClose: () => void;
  /** Appelé après un fork réussi, avec le nœud projet créé. */
  onForked?: (proj: BrainNode) => void;
}) {
  const [graph, setGraph] = useState<BrainGraph | null>(null);
  const [rawData, setRawData] = useState<unknown>(null);
  const [title, setTitle] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<BrainNode | null>(null);
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [forking, setForking] = useState(false);
  const [forkErr, setForkErr] = useState<string | null>(null);

  async function handleFork() {
    if (!rawData || forking) return;
    setForking(true); setForkErr(null);
    try {
      const proj = await importSharedSpace(rawData, spaceId);
      onForked?.(proj);
    } catch (e) {
      setForkErr(String(e instanceof Error ? e.message : e));
    } finally { setForking(false); }
  }

  useEffect(() => {
    let cancelled = false;
    const load = () =>
      fetchSharedSpace(spaceId)
        .then((row) => { if (!cancelled) { setTitle(row.title); setRawData(row.data); setGraph(payloadToGraph(row.data)); } })
        .catch((e) => { if (!cancelled) setError(String(e instanceof Error ? e.message : e)); });
    load();

    // Live : le propriétaire republie → les nouvelles bulles poussent toutes
    // seules ; il dé-publie (ou me retire) → le space se ferme proprement.
    if (!supabase) return;
    const ch = supabase
      .channel(`shared-${spaceId}`)
      .on(
        "postgres_changes",
        { event: "*", schema: "public", table: "shared_spaces", filter: `id=eq.${spaceId}` },
        (payload) => {
          if (payload.eventType === "DELETE") setError("Ce space n'est plus partagé.");
          else load(); // UPDATE : refetch (le RLS re-vérifie mon accès au passage)
        },
      )
      .subscribe();
    return () => { cancelled = true; supabase!.removeChannel(ch); };
  }, [spaceId]);

  async function copyLink() {
    if (await copyText(sharedSpaceUrl(spaceId))) {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    }
  }

  return (
    <div className="absolute inset-0 z-40 bg-[var(--color-bg)] text-[var(--color-text)]">
      {error ? (
        <div className="flex h-full items-center justify-center px-6">
          <div className="text-center">
            <p className="text-sm text-[var(--color-muted)]">{error}</p>
            <button onClick={onClose} className="mt-4 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs hover:bg-[var(--color-surface-2)]">
              Retour à mon cerveau
            </button>
          </div>
        </div>
      ) : !graph ? (
        <div className="flex h-full items-center justify-center">
          <Loader2 className="size-6 animate-spin text-[var(--color-muted)]" />
        </div>
      ) : (
        <>
          <BrainMap
            graph={graph}
            onSelect={setSelected}
            selectedId={selected?.id ?? null}
            query=""
            onBackgroundClick={() => setSelected(null)}
            panelOffset={selected && !expanded ? 480 : 0}
            focus={selected ? { id: selected.id, k: 1 } : null}
          />

          {/* Bandeau : retour + titre + lecture seule + lien navigateur */}
          <div className="absolute left-3 top-3 z-20 flex items-center gap-2 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)]/85 py-1.5 pl-1.5 pr-3 shadow-[var(--shadow-float)] backdrop-blur-md">
            <button
              onClick={onClose}
              title="Retour à mon cerveau"
              className="flex size-7 items-center justify-center rounded-full text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
            >
              <ArrowLeft className="size-4" />
            </button>
            <p className="max-w-[240px] truncate text-sm font-semibold">{title}</p>
            <span className="flex items-center gap-1 rounded-full bg-[var(--color-accent-soft)] px-2 py-0.5 text-[10px] font-medium text-[var(--color-accent)]">
              <Eye className="size-3" /> Lecture seule
            </span>
            <button
              onClick={copyLink}
              title="Copier le lien navigateur"
              className={cn(
                "flex size-7 items-center justify-center rounded-full transition-colors",
                copied ? "text-[var(--color-ok)]" : "text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]",
              )}
            >
              {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            </button>
            <button
              onClick={handleFork}
              disabled={forking || !rawData}
              title="Crée une copie de ce space comme projet dans ton cerveau — éditable, lisible par tes IA"
              className="ml-1 flex items-center gap-1.5 rounded-full bg-[var(--color-accent)] px-3 py-1 text-[11px] font-medium text-white transition-colors hover:bg-[var(--color-accent-hover)] disabled:opacity-50"
            >
              {forking ? <Loader2 className="size-3 animate-spin" /> : <FolderDown className="size-3" />}
              Copier dans mon cerveau
            </button>
          </div>

          {forkErr && (
            <p className="absolute left-3 top-14 z-20 max-w-sm rounded-lg border border-[var(--color-err)]/30 bg-[var(--color-surface)] px-3 py-2 text-[11px] text-[var(--color-err)] shadow-[var(--shadow-float)]">
              {forkErr}
            </p>
          )}

          {selected && (
            <div
              className={cn(
                "panel absolute z-30 overflow-hidden rounded-xl",
                expanded ? "inset-3" : "bottom-3 right-3 top-3 w-[480px] max-w-[calc(100vw-24px)] animate-slideInRight",
              )}
            >
              <ReadOnlyDetail
                key={selected.id}
                node={selected}
                graph={graph}
                onSelect={setSelected}
                onClose={() => { setSelected(null); setExpanded(false); }}
                expanded={expanded}
                onExpand={() => setExpanded((v) => !v)}
              />
            </div>
          )}
        </>
      )}
    </div>
  );
}
