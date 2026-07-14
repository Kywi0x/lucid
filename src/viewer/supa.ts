import { createClient } from "@supabase/supabase-js";
import { SUPABASE_URL, SUPABASE_ANON_KEY } from "./config";

/** Client du viewer : sert uniquement à prouver son identité (magic link)
 *  pour les spaces privés — l'accès reste décidé par les policies RLS. */
export const supa = createClient(SUPABASE_URL, SUPABASE_ANON_KEY);
