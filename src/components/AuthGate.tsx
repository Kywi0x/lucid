import { useEffect, useState, type ReactNode } from "react";
import type { Session } from "@supabase/supabase-js";
import { Brain, Loader2 } from "lucide-react";
import { supabase } from "@/lib/supabase";

/** Gate obligatoire : l'app n'est rendue qu'avec une session Supabase active.
 *  La session est persistée en localStorage par supabase-js → une fois connecté,
 *  l'app démarre même hors-ligne (getSession() ne fait pas d'appel réseau). */
export function AuthGate({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    if (!supabase) { setLoading(false); return; }
    supabase.auth.getSession().then(({ data }) => {
      setSession(data.session);
      setLoading(false);
    });
    const { data: sub } = supabase.auth.onAuthStateChange((_e, s) => setSession(s));
    return () => sub.subscription.unsubscribe();
  }, []);

  if (!supabase) {
    return (
      <Shell>
        <p className="text-sm font-semibold text-[var(--color-err)]">Configuration manquante</p>
        <p className="mt-2 text-xs leading-relaxed text-[var(--color-muted)]">
          La connexion est requise mais Supabase n'est pas configuré : remplis{" "}
          <code>VITE_SUPABASE_URL</code> et <code>VITE_SUPABASE_ANON_KEY</code> dans{" "}
          <code>.env</code> puis relance l'app.
        </p>
      </Shell>
    );
  }

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)]">
        <Loader2 className="size-6 animate-spin text-[var(--color-muted)]" />
      </div>
    );
  }

  if (!session) return <LoginScreen />;
  return <>{children}</>;
}

function Shell({ children }: { children: ReactNode }) {
  return (
    <div className="flex h-screen items-center justify-center bg-[var(--color-bg)] px-6">
      <div className="w-full max-w-sm rounded-2xl border border-[var(--color-border)] bg-[var(--color-surface)] p-6 shadow-xl">
        <div className="mb-5 flex items-center gap-2.5">
          <span className="flex size-9 items-center justify-center rounded-xl bg-[var(--color-accent-soft)]">
            <Brain className="size-5 text-[var(--color-accent)]" />
          </span>
          <p className="text-base font-semibold text-[var(--color-text)]">Lucid</p>
        </div>
        {children}
      </div>
    </div>
  );
}

function LoginScreen() {
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState<"signin" | "signup" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  async function auth(mode: "signin" | "signup") {
    if (!email.trim() || password.length < 6) {
      setMsg("Email + mot de passe (6 caractères min).");
      return;
    }
    setBusy(mode); setMsg(null);
    try {
      const { error, data } = mode === "signin"
        ? await supabase!.auth.signInWithPassword({ email: email.trim(), password })
        : await supabase!.auth.signUp({ email: email.trim(), password });
      if (error) throw error;
      if (mode === "signup" && !data.session) {
        setMsg("Compte créé — vérifie ta boîte mail pour confirmer, puis connecte-toi.");
      }
      // Session obtenue → onAuthStateChange du gate débloque l'app.
    } catch (e) {
      setMsg(String((e as Error).message ?? e));
    } finally { setBusy(null); }
  }

  return (
    <Shell>
      <p className="mb-4 text-xs leading-relaxed text-[var(--color-muted)]">
        Connecte-toi pour utiliser Lucid. Tes données restent traitées en local ;
        le compte sert à l'accès et aux sauvegardes cloud.
      </p>
      <form
        className="space-y-2"
        onSubmit={(e) => { e.preventDefault(); auth("signin"); }}
      >
        <input
          type="email" value={email} onChange={(e) => setEmail(e.target.value)}
          placeholder="email@exemple.fr" autoFocus
          className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
        />
        <input
          type="password" value={password} onChange={(e) => setPassword(e.target.value)}
          placeholder="Mot de passe (6 caractères min)"
          className="w-full rounded-lg border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3 py-2 text-sm text-[var(--color-text)] outline-none focus:border-[var(--color-accent)]"
        />
        <div className="flex gap-2 pt-1">
          <button type="submit" disabled={busy !== null}
            className="flex-1 rounded-lg bg-[var(--color-accent)] px-3 py-2 text-sm text-white hover:bg-[var(--color-accent-hover)] disabled:opacity-40">
            {busy === "signin" ? <Loader2 className="mx-auto size-4 animate-spin" /> : "Se connecter"}
          </button>
          <button type="button" onClick={() => auth("signup")} disabled={busy !== null}
            className="flex-1 rounded-lg border border-[var(--color-border)] px-3 py-2 text-sm text-[var(--color-text)] hover:bg-[var(--color-surface-2)] disabled:opacity-40">
            {busy === "signup" ? <Loader2 className="mx-auto size-4 animate-spin" /> : "Créer un compte"}
          </button>
        </div>
      </form>
      {msg && <p className="mt-3 text-xs text-[var(--color-muted)]">{msg}</p>}
    </Shell>
  );
}
