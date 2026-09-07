import { supabase } from "./supabase";
import { personalSpaceId } from "./share";

/** Fenêtre du relevé, en jours. */
export const DAYS = 14;

/** Caractères par token — hypothèse déjà portée par la note du coffre et par
 *  `share.ts` (« 24 000 caractères ≈ 6 300 tokens »). Une seule valeur, ici. */
export const CHARS_PER_TOKEN = 3.81;

interface UsageRow {
  day: string;          // YYYY-MM-DD (UTC, comme l'écrit la fonction SQL)
  calls: number;
  /** Rafales de moins de 2 min. Vaut 0 sur les lignes ANTÉRIEURES à la
   *  migration v2 (un `default` ne rétro-remplit pas), et `null` si la
   *  migration n'est pas appliquée du tout. */
  exchanges: number | null;
  chars_raw: number;
  chars_served: number;
}

export interface BrainStats {
  /** Tokens évités sur la fenêtre — le chiffre n°1 de la note. */
  tokensSaved: number | null;
  /** % de tokens évités sur la fenêtre, ou null tant qu'il n'y a rien à mesurer. */
  savedPct: number | null;
  /** Échanges par jour (rafales ≈ questions posées), le plus ancien en premier. */
  days: number[];
  /** Appels d'outils par jour — une seule question en produit plusieurs. */
  calls: number[];
}

/** Jour UTC d'il y a `back` jours, au format YYYY-MM-DD (même base que le SQL). */
export function dayKey(back: number): string {
  const d = new Date();
  d.setUTCDate(d.getUTCDate() - back);
  return d.toISOString().slice(0, 10);
}

/** DAYS cases, la plus ancienne à gauche, complétées à zéro si l'historique est court. */
export function toCells(days: number[]): number[] {
  const recent = days.slice(-DAYS);
  return [...Array(Math.max(0, DAYS - recent.length)).fill(0), ...recent];
}

/**
 * Relevé des `DAYS` derniers jours. Renvoie `null` s'il n'y a rien à mesurer —
 * la carte affiche alors « — » plutôt qu'un chiffre inventé (règle de la note
 * « Relevé d'activité » : des chiffres calculés, jamais estimés).
 */
export async function fetchBrainStats(): Promise<BrainStats | null> {
  if (!supabase) return null;
  const spaceId = await personalSpaceId();
  if (!spaceId) return null;

  const from = dayKey(DAYS - 1);
  const { data, error } = await supabase
    .from("mcp_usage")
    .select("day, calls, exchanges, chars_raw, chars_served")
    .eq("space_id", spaceId)
    .gte("day", from);
  // Table absente (migration non appliquée) ou RLS : pas de relevé, pas de
  // plantage — la carte reste sur son état « — ».
  if (error || !data?.length) return null;

  const rows = data as UsageRow[];
  let raw = 0, served = 0;
  // Plusieurs outils par jour → on somme les lignes du même jour.
  const exPerDay = new Map<string, number>();
  const callsPerDay = new Map<string, number>();
  for (const r of rows) {
    raw += r.chars_raw;
    served += r.chars_served;
    exPerDay.set(r.day, (exPerDay.get(r.day) ?? 0) + (r.exchanges ?? 0));
    callsPerDay.set(r.day, (callsPerDay.get(r.day) ?? 0) + r.calls);
  }

  const days: number[] = [];
  const calls: number[] = [];
  for (let i = DAYS - 1; i >= 0; i--) {
    const k = dayKey(i);
    const ex = exPerDay.get(k) ?? 0;
    const c = callsPerDay.get(k) ?? 0;
    // Un jour avec des appels mais AUCUN échange n'existe pas dans les faits :
    // une rafale dure 2 min, il en faut au moins une pour qu'un appel ait eu
    // lieu. C'est donc une ligne écrite avant la migration v2, dont la rafale
    // n'est pas reconstituable (aucun horodatage n'a été conservé). On retombe
    // sur les appels pour ce jour-là plutôt que d'effacer son historique — et
    // ça se résorbe tout seul à mesure que les jours défilent.
    days.push(ex > 0 || c === 0 ? ex : c);
    calls.push(c);
  }

  // Aucun caractère candidat = rien à économiser : on ne fabrique pas un 0 %
  // qui laisserait croire que Lucid n'a rien fait.
  const savedPct = raw > 0 ? (1 - served / raw) * 100 : null;
  const tokensSaved = raw > 0 ? Math.max(0, Math.round((raw - served) / CHARS_PER_TOKEN)) : null;
  return { tokensSaved, savedPct, days, calls };
}
