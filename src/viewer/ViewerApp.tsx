import { useEffect, useState } from "react";
import { Loader2 } from "lucide-react";
import { BrainMap } from "@/components/BrainMap";
import { ThemeToggle } from "@/components/ThemeToggle";
import { ReadOnlyDetail } from "@/components/ReadOnlyDetail";
import type { BrainGraph, BrainNode } from "@/lib/types";
import { cn } from "@/lib/utils";
import { payloadToGraph } from "@/lib/shared-space";
import { SUPABASE_URL, SUPABASE_ANON_KEY } from "./config";
import { supa } from "./supa";

/** Viewer public d'un space partagé : les MÊMES composants que l'app
 *  (BrainMap, ProjectTree, thème, styles) — une page ouverte se lit
 *  exactement comme dans NodeDetail, en lecture seule. */

interface SharedSpace {
  title: string;
  updated_at: string;
  graph: BrainGraph;
}

/** Levée quand la ligne est invisible : space privé (pas invité) ou inexistant. */
class DeniedError extends Error {}

async function fetchSpace(id: string): Promise<SharedSpace> {
  // Avec session : le RLS voit l'email du visiteur (spaces privés sur invitation).
  const { data: sess } = await supa.auth.getSession();
  const token = sess.session?.access_token ?? SUPABASE_ANON_KEY;
  const r = await fetch(
    `${SUPABASE_URL}/rest/v1/shared_spaces?id=eq.${encodeURIComponent(id)}&select=title,data,updated_at`,
    { headers: { apikey: SUPABASE_ANON_KEY, authorization: `Bearer ${token}` } },
  );
  if (!r.ok) throw new Error(`Supabase HTTP ${r.status}`);
  const rows = await r.json();
  if (!rows.length) throw new DeniedError();
  const { title, data, updated_at } = rows[0];
  return { title: data.title || title, updated_at, graph: payloadToGraph(data) };
}

export function ViewerApp() {
  const [space, setSpace] = useState<SharedSpace | null>(null);
  const [denied, setDenied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<BrainNode | null>(null);
  const [expanded, setExpanded] = useState(false);

  useEffect(() => {
    const id = new URLSearchParams(location.search).get("id");
    if (!id) { setError("Lien invalide (id manquant)."); return; }
    let cancelled = false;
    const load = () => fetchSpace(id)
      .then((s) => { if (!cancelled) { setDenied(false); setSpace(s); document.title = `${s.title} — Lucid`; } })
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof DeniedError) setDenied(true);
        else setError(String(e instanceof Error ? e.message : e));
      });
    load();
    // Retour de magic link : la session arrive après coup → on retente.
    const { data: sub } = supa.auth.onAuthStateChange((ev) => {
      if (ev === "SIGNED_IN" || ev === "SIGNED_OUT") load();
    });
    // Live : republication → mise à jour sous les yeux du lecteur ;
    // dé-publication / retrait d'accès → gate.
    const ch = supa
      .channel(`shared-${id}`)
      .on(
        "postgres_changes",
        { event: "*", schema: "public", table: "shared_spaces", filter: `id=eq.${id}` },
        (payload) => {
          if (payload.eventType === "DELETE") { setSpace(null); setDenied(true); }
          else load();
        },
      )
      .subscribe();
    return () => { cancelled = true; sub.subscription.unsubscribe(); supa.removeChannel(ch); };
  }, []);

  if (error) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)] px-6">
        <p className="text-sm text-[var(--color-muted)]">{error}</p>
      </div>
    );
  }
  if (denied) return <PrivateGate />;
  if (!space) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)]">
        <Loader2 className="size-6 animate-spin text-[var(--color-muted)]" />
      </div>
    );
  }

  const pageCount = space.graph.nodes.filter((n) => n.kind !== "root").length;

  return (
    <div className="relative h-screen overflow-hidden bg-[var(--color-bg)] text-[var(--color-text)]">
      <BrainMap
        graph={space.graph}
        onSelect={setSelected}
        selectedId={selected?.id ?? null}
        query=""
        onBackgroundClick={() => setSelected(null)}
        panelOffset={selected && !expanded ? 480 : 0}
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

      {/* Panneau page ouverte — même structure que NodeDetail, en lecture seule */}
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
            graph={space.graph}
            onSelect={setSelected}
            onClose={() => { setSelected(null); setExpanded(false); }}
            expanded={expanded}
            onExpand={() => setExpanded((v) => !v)}
          />
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

/** Space privé (ou inexistant) : la connexion sert UNIQUEMENT à prouver son
 *  identité — l'accès est décidé par le propriétaire (liste d'invités, RLS).
 *  Personne ne peut s'auto-inviter ni demander l'accès d'ici. */
function PrivateGate() {
  const [email, setEmail] = useState<string | null | undefined>(undefined); // email de session
  const [draft, setDraft] = useState("");
  const [sent, setSent] = useState(false);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    supa.auth.getSession().then(({ data }) => setEmail(data.session?.user.email ?? null));
  }, []);

  async function sendLink() {
    const e = draft.trim().toLowerCase();
    if (!/^\S+@\S+\.\S+$/.test(e)) return;
    setBusy(true); setErr(null);
    try {
      const { error } = await supa.auth.signInWithOtp({
        email: e,
        options: { emailRedirectTo: location.href },
      });
      if (error) throw error;
      setSent(true);
    } catch (ex) {
      setErr(String(ex instanceof Error ? ex.message : ex));
    } finally { setBusy(false); }
  }

  return (
    <div className="flex h-screen items-center justify-center bg-[var(--color-bg)] px-6">
      <div className="w-full max-w-sm rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] p-6 shadow-[var(--shadow-float)]">
        <p className="mb-1 text-sm font-semibold text-[var(--color-text)]">Space privé</p>
        {email === undefined ? (
          <Loader2 className="mt-3 size-4 animate-spin text-[var(--color-muted)]" />
        ) : email ? (
          <>
            <p className="text-xs leading-relaxed text-[var(--color-muted)]">
              Connecté en tant que <strong className="text-[var(--color-text)]">{email}</strong> — cet
              email n'a pas accès à ce space (ou le space n'existe plus). Seul le propriétaire
              peut t'inviter.
            </p>
            <button
              onClick={() => supa.auth.signOut()}
              className="mt-4 w-full rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-text)] hover:bg-[var(--color-surface-2)]"
            >
              Se connecter avec un autre email
            </button>
          </>
        ) : sent ? (
          <p className="text-xs leading-relaxed text-[var(--color-muted)]">
            Lien de connexion envoyé à <strong className="text-[var(--color-text)]">{draft.trim()}</strong> —
            clique dessus depuis ta boîte mail pour revenir ici identifié.
          </p>
        ) : (
          <>
            <p className="text-xs leading-relaxed text-[var(--color-muted)]">
              Ce space est privé (ou n'existe plus). Si son propriétaire t'a invité,
              connecte-toi avec <strong>l'email invité</strong> pour y accéder.
            </p>
            <input
              type="email"
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") sendLink(); }}
              placeholder="email@exemple.fr"
              className="mt-3 w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
            />
            <button
              onClick={sendLink}
              disabled={busy || !/^\S+@\S+\.\S+$/.test(draft.trim())}
              className="mt-2 w-full rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40"
            >
              {busy ? <Loader2 className="mx-auto size-4 animate-spin" /> : "Me connecter"}
            </button>
            {err && <p className="mt-2 text-xs text-[var(--color-err)]">{err}</p>}
          </>
        )}
      </div>
    </div>
  );
}
