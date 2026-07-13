import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Loader2, Maximize2, Minimize2, X } from "lucide-react";
import { BrainMap } from "@/components/BrainMap";
import { ThemeToggle } from "@/components/ThemeToggle";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { cn } from "@/lib/utils";
import { SUPABASE_URL, SUPABASE_ANON_KEY } from "./config";

/** Viewer public d'un space partagé : les MÊMES composants que l'app
 *  (BrainMap, thème, styles), en lecture seule dans le navigateur. */

interface SharedSpace {
  title: string;
  updated_at: string;
  graph: BrainGraph;
}

async function fetchSpace(id: string): Promise<SharedSpace> {
  const r = await fetch(
    `${SUPABASE_URL}/rest/v1/shared_spaces?id=eq.${encodeURIComponent(id)}&select=title,data,updated_at`,
    { headers: { apikey: SUPABASE_ANON_KEY, authorization: `Bearer ${SUPABASE_ANON_KEY}` } },
  );
  if (!r.ok) throw new Error(`Supabase HTTP ${r.status}`);
  const rows = await r.json();
  if (!rows.length) throw new Error("Ce space n'existe pas (ou n'est plus partagé).");
  const { title, data, updated_at } = rows[0];
  const nodes: BrainNode[] = data.nodes.map((n: Partial<BrainNode>) => ({
    summary: "", keywords: [], decisions: [], patterns: [], content: "",
    ...n,
  } as BrainNode));
  return {
    title: data.title || title,
    updated_at,
    graph: { nodes, edges: data.edges ?? [], markdown: "", report: "", generated_at: "" },
  };
}

export function ViewerApp() {
  const [space, setSpace] = useState<SharedSpace | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<BrainNode | null>(null);
  const [wide, setWide] = useState(false);

  useEffect(() => {
    const id = new URLSearchParams(location.search).get("id");
    if (!id) { setError("Lien invalide (id manquant)."); return; }
    fetchSpace(id)
      .then((s) => { setSpace(s); document.title = `${s.title} — Lucid`; })
      .catch((e) => setError(String(e instanceof Error ? e.message : e)));
  }, []);

  const byId = useMemo(() => new Map((space?.graph.nodes ?? []).map((n) => [n.id, n])), [space]);
  const childrenOf = useMemo(() => {
    const m = new Map<string, BrainNode[]>();
    for (const n of space?.graph.nodes ?? []) {
      if (!n.parent_id) continue;
      const a = m.get(n.parent_id);
      if (a) a.push(n); else m.set(n.parent_id, [n]);
    }
    return m;
  }, [space]);

  const crumb = useMemo(() => {
    const chain: BrainNode[] = [];
    let cur = selected;
    while (cur?.parent_id && byId.has(cur.parent_id)) {
      cur = byId.get(cur.parent_id)!;
      chain.unshift(cur);
    }
    return chain;
  }, [selected, byId]);

  if (error) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)] px-6">
        <p className="text-sm text-[var(--color-muted)]">{error}</p>
      </div>
    );
  }
  if (!space) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)]">
        <Loader2 className="size-6 animate-spin text-[var(--color-muted)]" />
      </div>
    );
  }

  const pageCount = space.graph.nodes.filter((n) => n.kind !== "root").length;
  const kids = selected ? (childrenOf.get(selected.id) ?? []) : [];

  return (
    <div className="relative h-screen overflow-hidden bg-[var(--color-bg)] text-[var(--color-text)]">
      <BrainMap
        graph={space.graph}
        onSelect={setSelected}
        selectedId={selected?.id ?? null}
        query=""
        onBackgroundClick={() => setSelected(null)}
        panelOffset={selected && !wide ? 480 : 0}
        focus={selected ? { id: selected.id, k: 1 } : null}
      />

      {/* Titre du space (haut gauche) + thème (haut droite) */}
      <div className="pointer-events-none absolute left-5 top-4 z-20">
        <h1 className="text-[15px] font-semibold">{space.title}</h1>
        <p className="text-[11px] text-[var(--color-muted)]">
          {pageCount} page{pageCount > 1 ? "s" : ""} · mis à jour le{" "}
          {new Date(space.updated_at).toLocaleDateString("fr-FR")}
        </p>
      </div>
      <div className="absolute right-3 top-3 z-20 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)]/75 px-1.5 py-1 shadow-[var(--shadow-float)] backdrop-blur-md">
        <ThemeToggle />
      </div>

      {/* Panneau de lecture — même design que NodeDetail, en lecture seule */}
      {selected && (
        <div
          className={cn(
            "panel absolute z-30 overflow-y-auto rounded-xl",
            wide ? "inset-3" : "bottom-3 right-3 top-3 w-[480px] max-w-[calc(100vw-24px)] animate-slideInRight",
          )}
        >
          <div className={cn("px-6 py-5", wide && "mx-auto max-w-3xl")}>
            <div className="mb-2 flex items-start gap-1">
              <div className="min-w-0 flex-1">
                {crumb.length > 0 && (
                  <p className="mb-1 truncate text-[11px] text-[var(--color-muted)]">
                    {crumb.map((a, i) => (
                      <span key={a.id}>
                        {i > 0 && <span className="mx-1 opacity-50">/</span>}
                        <button
                          onClick={() => setSelected(a)}
                          className="hover:text-[var(--color-accent)]"
                        >
                          {a.label}
                        </button>
                      </span>
                    ))}
                  </p>
                )}
                <h2 className="text-base font-semibold">{selected.label}</h2>
                {selected.keywords.length > 0 && (
                  <p className="mt-0.5 text-[11px] text-[var(--color-muted)]">
                    {selected.keywords.join(" · ")}
                  </p>
                )}
              </div>
              <button
                onClick={() => setWide((w) => !w)}
                title={wide ? "Réduire" : "Agrandir"}
                className="shrink-0 rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
              >
                {wide ? <Minimize2 className="size-4" /> : <Maximize2 className="size-4" />}
              </button>
              <button
                onClick={() => { setSelected(null); setWide(false); }}
                title="Fermer"
                className="shrink-0 rounded-lg p-1.5 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]"
              >
                <X className="size-4" />
              </button>
            </div>

            <div className="prose-brain text-sm">
              <ReactMarkdown remarkPlugins={[remarkGfm]}>
                {selected.content || selected.summary || "*(page sans contenu)*"}
              </ReactMarkdown>
            </div>

            {kids.length > 0 && (
              <div className="mt-4">
                <p className="mb-1.5 text-[10px] font-semibold uppercase tracking-wide text-[var(--color-muted)]">
                  Pages enfants ({kids.length})
                </p>
                <ul className="space-y-0.5">
                  {kids.map((child) => (
                    <li key={child.id}>
                      <button
                        onClick={() => setSelected(child)}
                        className="w-full truncate rounded-md px-2 py-1.5 text-left text-xs text-[var(--color-text)] transition-colors hover:bg-[var(--color-surface-2)] hover:text-[var(--color-accent)]"
                      >
                        {child.label}
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </div>
        </div>
      )}

      {/* CTA — la boucle de croissance */}
      {!selected && (
        <div className="pointer-events-none absolute inset-x-0 bottom-4 z-20 flex justify-center">
          <a
            href="../"
            className="pointer-events-auto flex items-center gap-2 rounded-full border border-[var(--color-border)] bg-[var(--color-surface)]/85 px-4 py-2 text-xs text-[var(--color-muted)] shadow-[var(--shadow-float)] backdrop-blur-md transition-colors hover:text-[var(--color-text)]"
          >
            ✦ Créé avec <span className="font-semibold text-[var(--color-accent)]">Lucid</span> — construis ton second cerveau
          </a>
        </div>
      )}
    </div>
  );
}
