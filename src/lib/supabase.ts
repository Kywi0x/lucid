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
  url && key ? createClient(url, key) : null;

/** URL Supabase effective (env ou fallback commité) — à utiliser partout ailleurs
 *  plutôt que `import.meta.env.VITE_SUPABASE_URL` : un build CI sans .env avait
 *  ainsi une URL MCP nulle alors que le client, lui, fonctionnait (bug .dmg 2026-08-05). */
export const SUPABASE_URL = url;

export const BACKUP_BUCKET = "backups";
