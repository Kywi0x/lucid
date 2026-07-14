import { useEffect, useState } from "react";
import { Check, Copy, Globe, Loader2, Lock, Trash2, X } from "lucide-react";
import {
  fetchShareState, publishSpace, unpublishSpace,
  type ShareState,
} from "@/lib/share";
import type { BrainGraph, Space } from "@/lib/types";
import { cn, copyText } from "@/lib/utils";

const EMAIL_RE = /^\S+@\S+\.\S+$/;

/** Modale de partage d'un space : lien public, ou privé sur invitation.
 *  Seul le propriétaire invite — la liste d'emails vit ici, nulle part ailleurs. */
export function ShareModal({ space, subgraph, onClose }: {
  space: Space;
  subgraph: BrainGraph;
  onClose: () => void;
}) {
  const [loading, setLoading] = useState(true);
  const [state, setState] = useState<(ShareState & { url: string }) | null>(null);
  const [visibility, setVisibility] = useState<"public" | "private">("public");
  const [emails, setEmails] = useState<string[]>([]);
  const [emailDraft, setEmailDraft] = useState("");
  const [busy, setBusy] = useState<"publish" | "unpublish" | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetchShareState(space)
      .then((s) => {
        if (s) { setState(s); setVisibility(s.visibility); setEmails(s.allowed_emails); }
      })
      .catch((e) => setError(String(e instanceof Error ? e.message : e)))
      .finally(() => setLoading(false));
  }, [space]);

  function addEmail() {
    const e = emailDraft.trim().toLowerCase();
    if (!EMAIL_RE.test(e)) return;
    if (!emails.includes(e)) setEmails((prev) => [...prev, e]);
    setEmailDraft("");
  }

  async function handlePublish() {
    setBusy("publish"); setError(null);
    try {
      const s = await publishSpace(space, subgraph, { visibility, allowedEmails: emails });
      setState(s);
      if (await copyText(s.url)) { setCopied(true); setTimeout(() => setCopied(false), 2000); }
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally { setBusy(null); }
  }

  async function handleUnpublish() {
    if (!confirm(`Retirer « ${space.name} » du web ? Le lien ne fonctionnera plus.`)) return;
    setBusy("unpublish"); setError(null);
    try {
      await unpublishSpace(space);
      setState(null); setEmails([]); setVisibility("public");
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally { setBusy(null); }
  }

  async function handleCopy() {
    if (state && (await copyText(state.url))) { setCopied(true); setTimeout(() => setCopied(false), 2000); }
  }

  const dirty = state !== null &&
    (state.visibility !== visibility || JSON.stringify(state.allowed_emails) !== JSON.stringify(emails));

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={onClose}>
      <div
        className="w-full max-w-md rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] p-5 shadow-[var(--shadow-float)]"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-4 flex items-center gap-2">
          <p className="min-w-0 flex-1 truncate text-sm font-semibold">Partager « {space.name} »</p>
          <button onClick={onClose} className="rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
            <X className="size-4" />
          </button>
        </div>

        {loading ? (
          <div className="flex items-center gap-2 py-6 text-xs text-[var(--color-muted)]">
            <Loader2 className="size-3.5 animate-spin" /> Chargement…
          </div>
        ) : (
          <>
            {/* Public / Privé */}
            <div className="mb-4 grid grid-cols-2 gap-1 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface-2)] p-1">
              {([
                { v: "public" as const, icon: Globe, label: "Public" },
                { v: "private" as const, icon: Lock, label: "Privé" },
              ]).map(({ v, icon: I, label }) => (
                <button
                  key={v}
                  onClick={() => setVisibility(v)}
                  className={cn(
                    "flex items-center justify-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors",
                    visibility === v
                      ? "bg-[var(--color-surface)] text-[var(--color-text)] shadow-sm"
                      : "text-[var(--color-muted)] hover:text-[var(--color-text)]",
                  )}
                >
                  <I className="size-3.5" /> {label}
                </button>
              ))}
            </div>

            <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
              {visibility === "public"
                ? "Toute personne ayant le lien peut consulter ce space (titres, résumés, contenus — jamais les conversations sources)."
                : "Seules les personnes que tu invites ci-dessous peuvent ouvrir le lien, après connexion avec leur email. Personne ne peut s'auto-inviter."}
            </p>

            {/* Invitations (privé) */}
            {visibility === "private" && (
              <div className="mb-4">
                <div className="flex gap-2">
                  <input
                    type="email"
                    value={emailDraft}
                    onChange={(e) => setEmailDraft(e.target.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); addEmail(); } }}
                    placeholder="email@exemple.fr puis Entrée"
                    className="min-w-0 flex-1 rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-xs text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
                  />
                  <button
                    onClick={addEmail}
                    disabled={!EMAIL_RE.test(emailDraft.trim())}
                    className="shrink-0 rounded-lg border border-[var(--color-border)] px-3 py-2 text-xs text-[var(--color-text)] hover:bg-[var(--color-surface-2)] disabled:opacity-40"
                  >
                    Inviter
                  </button>
                </div>
                {emails.length > 0 && (
                  <ul className="mt-2 flex flex-wrap gap-1.5">
                    {emails.map((e) => (
                      <li key={e} className="flex items-center gap-1 rounded-full border border-[var(--color-border)] bg-[var(--color-bg)] py-0.5 pl-2.5 pr-1 text-[11px] text-[var(--color-text)]">
                        {e}
                        <button
                          onClick={() => setEmails((prev) => prev.filter((x) => x !== e))}
                          title="Retirer l'accès"
                          className="rounded-full p-0.5 text-[var(--color-muted)] hover:text-[var(--color-err)]"
                        >
                          <X className="size-3" />
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            )}

            {/* Lien actuel */}
            {state && (
              <div className="mb-4 flex items-center gap-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2">
                <p className="min-w-0 flex-1 truncate font-mono text-[11px] text-[var(--color-muted)]">{state.url}</p>
                <button onClick={handleCopy} title="Copier le lien"
                  className="shrink-0 rounded-md p-1 text-[var(--color-muted)] hover:bg-[var(--color-surface-2)] hover:text-[var(--color-text)]">
                  {copied ? <Check className="size-3.5 text-[var(--color-ok)]" /> : <Copy className="size-3.5" />}
                </button>
              </div>
            )}

            {error && (
              <p className="mb-3 rounded-lg border border-[var(--color-err)]/30 bg-[var(--color-err)]/10 px-3 py-2 text-xs text-[var(--color-err)]">{error}</p>
            )}

            <div className="flex items-center gap-2">
              <button
                onClick={handlePublish}
                disabled={busy !== null || (visibility === "private" && emails.length === 0)}
                title={visibility === "private" && emails.length === 0 ? "Invite au moins un email" : undefined}
                className="flex-1 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40"
              >
                {busy === "publish"
                  ? <Loader2 className="mx-auto size-4 animate-spin" />
                  : state ? (dirty ? "Mettre à jour" : "Republier (contenu à jour)") : "Publier et copier le lien"}
              </button>
              {state && (
                <button
                  onClick={handleUnpublish}
                  disabled={busy !== null}
                  title="Retirer du web"
                  className="shrink-0 rounded-lg border border-[var(--color-border)] p-2 text-[var(--color-muted)] hover:border-[var(--color-err)]/50 hover:text-[var(--color-err)] disabled:opacity-40"
                >
                  {busy === "unpublish" ? <Loader2 className="size-4 animate-spin" /> : <Trash2 className="size-4" />}
                </button>
              )}
            </div>
          </>
        )}
      </div>
    </div>
  );
}
