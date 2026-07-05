import { createClient, type SupabaseClient } from "@supabase/supabase-js";

/** Client Supabase — null si les variables d'env ne sont pas remplies
 *  (l'app fonctionne alors 100 % en local, le compte est optionnel). */
const url = import.meta.env.VITE_SUPABASE_URL as string | undefined;
const key = import.meta.env.VITE_SUPABASE_ANON_KEY as string | undefined;

export const supabase: SupabaseClient | null =
  url && key && !key.includes("A_REMPLIR") ? createClient(url, key) : null;

export const BACKUP_BUCKET = "backups";
