import { useEffect, useState, type ReactNode } from "react";
import type { Session } from "@supabase/supabase-js";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, Loader2, Lock, Mail } from "lucide-react";
import { supabase } from "@/lib/supabase";
import { BrainMap } from "@/components/BrainMap";
import { EMPTY_GRAPH } from "@/lib/types";
import { ThemeToggle } from "@/components/ThemeToggle";
import { cn } from "@/lib/utils";

/** Gate obligatoire : l'app n'est rendue qu'avec une session Supabase active.
 *  La session est persistée en localStorage par supabase-js → une fois connecté,
 *  l'app démarre même hors-ligne (getSession() ne fait pas d'appel réseau). */
export function AuthGate({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [loading, setLoading] = useState(true);
  // user.id déjà transmis au Rust — l'app n'est montée qu'une fois les données
  // locales scopées au bon compte (users/<uuid>/), sinon on lirait le cerveau
  // du compte précédent le temps d'un render.
  const [scopedTo, setScopedTo] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    if (!supabase) { setLoading(false); return; }
    supabase.auth.getSession().then(({ data }) => {
      setSession(data.session);
      setLoading(false);
    });
    const { data: sub } = supabase.auth.onAuthStateChange((_e, s) => setSession(s));
    return () => sub.subscription.unsubscribe();
  }, []);

  const userId = session?.user.id ?? null;
  useEffect(() => {
    if (loading) return;
    invoke<boolean>("set_active_user", { userId })
      .then((fresh) => {
        // Compte neuf sur cette machine : l'onboarding doit rejouer (ces flags
        // vivent en localStorage, donc au niveau machine, pas par compte).
        if (fresh) {
          localStorage.removeItem("lucid.onboarded");
          localStorage.removeItem("lucid.demo");
        }
      })
      .catch((e) => console.error("set_active_user:", e))
      .finally(() => setScopedTo(userId));
  }, [loading, userId]);

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
  if (scopedTo !== userId) {
    return (
      <div className="flex h-screen items-center justify-center bg-[var(--color-bg)]">
        <Loader2 className="size-6 animate-spin text-[var(--color-muted)]" />
      </div>
    );
  }
  return <>{children}</>;
}

/** Écran plein : le vrai canvas (BrainMap, vide) en fond — l'orbe vivante qu'il
 *  affiche EST le root, une seule fois, jamais dupliquée avec un mark statique
 *  dans la carte. `caption` : texte affiché sous cette orbe (ex. « Connecte-toi
 *  pour commencer »), optionnel pour ne rien changer aux écrans annexes
 *  (config manquante, chargement). */
function Shell({ children, caption }: { children: ReactNode; caption?: string }) {
  return (
    <div className="relative flex h-screen items-center justify-center overflow-hidden px-6">
      <div className="absolute inset-0">
        <BrainMap graph={EMPTY_GRAPH} onSelect={() => {}} selectedId={null} query="" caption={caption} />
      </div>
      <div className="absolute right-4 top-4 z-10">
        <ThemeToggle />
      </div>
      <div className="lucid-rise panel relative w-full max-w-sm rounded-2xl p-7">
        <div className="mb-6 flex flex-col items-center text-center">
          <p className="font-mono text-sm font-semibold tracking-[0.28em] text-[var(--color-text)]">
            LUCID
          </p>
          <p className="mt-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-[var(--color-muted)]">
            second cerveau · local-first
          </p>
        </div>
        {children}
      </div>
      <p className="absolute bottom-4 left-1/2 -translate-x-1/2 whitespace-nowrap font-mono text-[10px] uppercase tracking-[0.1em] text-[var(--color-muted)]/70">
        local-first · l'analyse reste sur ta machine, ton cerveau te suit
      </p>
    </div>
  );
}

function Field({
  icon, ...props
}: { icon: ReactNode } & React.InputHTMLAttributes<HTMLInputElement>) {
  return (
    <label className="flex items-center gap-2.5 rounded-xl border border-[var(--color-border)] bg-[var(--color-surface-2)] px-3.5 py-2.5 transition-colors focus-within:border-[var(--color-accent)]">
      <span className="shrink-0 text-[var(--color-muted)]">{icon}</span>
      <input
        {...props}
        className="w-full bg-transparent text-sm text-[var(--color-text)] outline-none placeholder:text-[var(--color-muted)]/70"
      />
    </label>
  );
}

function LoginScreen() {
  const [mode, setMode] = useState<"signin" | "signup">("signin");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ text: string; kind: "err" | "info" } | null>(null);

  async function auth() {
    if (!email.trim() || password.length < 6) {
      setMsg({ text: "Email + mot de passe (6 caractères minimum).", kind: "err" });
      return;
    }
    setBusy(true); setMsg(null);
    try {
      const { error, data } = mode === "signin"
        ? await supabase!.auth.signInWithPassword({ email: email.trim(), password })
        : await supabase!.auth.signUp({ email: email.trim(), password });
      if (error) throw error;
      if (mode === "signup" && !data.session) {
        setMsg({
          text: "Compte créé — vérifie ta boîte mail pour confirmer, puis connecte-toi.",
          kind: "info",
        });
        setMode("signin");
      }
      // Session obtenue → onAuthStateChange du gate débloque l'app.
    } catch (e) {
      setMsg({ text: String((e as Error).message ?? e), kind: "err" });
    } finally { setBusy(false); }
  }

  const signup = mode === "signup";

  return (
    <Shell caption={signup ? "Inscris-toi pour commencer" : "Connecte-toi pour commencer"}>
      <form className="space-y-2.5" onSubmit={(e) => { e.preventDefault(); auth(); }}>
        <Field
          icon={<Mail className="size-4" />}
          type="email" value={email} onChange={(e) => setEmail(e.target.value)}
          placeholder="email@exemple.fr" autoFocus
        />
        <Field
          icon={<Lock className="size-4" />}
          type="password" value={password} onChange={(e) => setPassword(e.target.value)}
          placeholder={signup ? "Choisis un mot de passe (6 min.)" : "Mot de passe"}
        />
        <button
          type="submit" disabled={busy}
          className="group flex w-full items-center justify-center gap-2 rounded-xl bg-[var(--color-accent)] px-4 py-2.5 text-sm font-medium text-white transition-all hover:bg-[var(--color-accent-hover)] hover:shadow-[0_4px_20px_color-mix(in_srgb,var(--color-accent)_40%,transparent)] disabled:opacity-40"
        >
          {busy
            ? <Loader2 className="size-4 animate-spin" />
            : (
              <>
                {signup ? "Créer mon compte" : "Entrer dans Lucid"}
                <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
              </>
            )}
        </button>
      </form>

      {msg && (
        <p className={cn(
          "mt-3 text-center text-xs leading-relaxed",
          msg.kind === "err" ? "text-[var(--color-err)]" : "text-[var(--color-ok)]",
        )}>
          {msg.text}
        </p>
      )}

      <p className="mt-5 text-center text-xs text-[var(--color-muted)]">
        {signup ? "Déjà un compte ?" : "Première fois ?"}{" "}
        <button
          type="button"
          onClick={() => { setMode(signup ? "signin" : "signup"); setMsg(null); }}
          className="font-medium text-[var(--color-accent)] hover:underline"
        >
          {signup ? "Se connecter" : "Créer un compte"}
        </button>
      </p>

      <p className="mt-4 border-t border-[var(--color-border)] pt-3 text-center text-[11px] leading-relaxed text-[var(--color-muted)]">
        Ton compte synchronise ton cerveau entre tes appareils — l'analyse de
        tes données reste 100 % locale.
      </p>
    </Shell>
  );
}
