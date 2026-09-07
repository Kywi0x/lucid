import { createClient, type SupabaseClient } from "@supabase/supabase-js";
import { DEFAULT_SUPABASE_URL, DEFAULT_SUPABASE_ANON_KEY } from "./supabase-config";

/** Client Supabase. Les VITE_* du .env priment (dev / autre projet), sinon
 *  fallback sur les valeurs commitées — un build sans .env (CI, packaging
 *  Windows) fonctionne, jamais d'écran « Configuration manquante » en prod. */
const envUrl = import.meta.env.VITE_SUPABASE_URL as string | undefined;
const envKey = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined;
const url = envUrl && !envUrl.includes("VOTRE") ? envUrl : DEFAULT_SUPABASE_URL;
const key = envKey && !envKey.includes("VOTRE") && !envKey.includes("A_REMPLIR") ? envKey : DEFAULT_SUPABASE_ANON_KEY;

export const supabase: SupabaseClient | null =
  url && key
    ? createClient(url, key, {
        // PKCE et pas implicite : le retour OAuth est capté par un serveur
        // loopback côté Rust, qui ne voit que la query string — le fragment
        // `#access_token` du flux implicite ne lui parviendrait jamais.
        auth: { flowType: "pkce" },
      })
    : null;

/** Port du retour OAuth (connexion Google / Apple). Doit rester égal à
 *  `AUTH_OAUTH_PORT` (src-tauri/src/lib.rs) et figurer tel quel dans les
 *  « Redirect URLs » Supabase : leur joker ne couvre pas le port. */
const OAUTH_PORT = 45711;
const OAUTH_REDIRECT = `http://localhost:${OAUTH_PORT}`;

/** Connexion via un fournisseur : ouvre le navigateur système, attend le retour
 *  sur le loopback, échange le code contre une session (que le gate détecte).
 *  Navigateur système et pas la webview : Google refuse ses écrans de connexion
 *  dans une webview embarquée (« disallowed_useragent »). */
export async function signInWithProvider(provider: "google" | "apple") {
  const [{ invoke }, { openUrl }] = await Promise.all([
    import("@tauri-apps/api/core"),
    import("@tauri-apps/plugin-opener"),
  ]);

  // L'écoute démarre avant l'ouverture du navigateur : l'invoke n'est pas attendu ici.
  const pending = invoke<string>("auth_await_oauth_code");

  const { data, error } = await supabase!.auth.signInWithOAuth({
    provider,
    options: { redirectTo: OAUTH_REDIRECT, skipBrowserRedirect: true },
  });
  if (error || !data.url) {
    void pending.catch(() => {});
    throw error ?? new Error("URL d'autorisation absente.");
  }

  await openUrl(data.url);
  const code = await pending;
  const { error: exchangeError } = await supabase!.auth.exchangeCodeForSession(code);
  if (exchangeError) throw exchangeError;
}

/** URL Supabase effective (env ou fallback commité) — à utiliser partout ailleurs
 *  plutôt que `import.meta.env.VITE_SUPABASE_URL` : un build CI sans .env avait
 *  ainsi une URL MCP nulle alors que le client, lui, fonctionnait (bug .dmg 2026-08-05). */
export const SUPABASE_URL = url;

export const BACKUP_BUCKET = "backups";
