// Types de propriété (façon Notion/Obsidian). Le .md ne stocke que des VALEURS
// (frontmatter portable) ; le TYPE est associé au nom de la propriété — comme le
// fait Obsidian (.obsidian/types.json). Ici : registre front par nom + inférence
// par défaut depuis la valeur (bon défaut pour les notes importées).
// ponytail: registre en localStorage — les valeurs, elles, vivent dans le .md et
// survivent toujours (le type se ré-infère si le registre est perdu). Passer à un
// store persisté (dossier de données, inclus au backup) si on veut le partager entre machines.

import type { PropValue } from "./frontmatter";

export type PropType = "text" | "tags" | "date" | "number" | "checkbox" | "url";

const RE_DATE = /^\d{4}-\d{2}-\d{2}$/;
const RE_NUM = /^-?\d+(\.\d+)?$/;
const RE_URL = /^https?:\/\//i;

/** Type déduit de la forme de la valeur (défaut quand aucun type explicite). */
export function inferType(v: PropValue): PropType {
  if (Array.isArray(v)) return "tags";
  const s = String(v).trim();
  if (s === "true" || s === "false") return "checkbox";
  if (RE_DATE.test(s)) return "date";
  if (RE_URL.test(s)) return "url";
  if (s !== "" && RE_NUM.test(s)) return "number";
  return "text";
}

/** Convertit une valeur vers la forme attendue par un type (au changement de type). */
export function coerce(value: PropValue, type: PropType): PropValue {
  const asText = Array.isArray(value) ? value.join(", ") : String(value ?? "");
  switch (type) {
    case "tags":     return Array.isArray(value) ? value : asText.split(",").map((s) => s.trim()).filter(Boolean);
    case "checkbox": return asText === "true" ? "true" : "false";
    case "number":   return asText.replace(/[^0-9.\-]/g, "");
    default:         return asText; // text, date, url
  }
}

// ── Registre des types par nom de propriété (localStorage) ──────────────────
const STORE_KEY = "lucid.propertyTypes";

export function loadTypeRegistry(): Record<string, PropType> {
  try { return JSON.parse(localStorage.getItem(STORE_KEY) || "{}"); } catch { return {}; }
}

export function saveTypeRegistry(reg: Record<string, PropType>): void {
  try { localStorage.setItem(STORE_KEY, JSON.stringify(reg)); } catch { /* quota/private mode : on ignore */ }
}

/** Type effectif : override du registre si présent, sinon inféré. */
export function typeOf(name: string, value: PropValue, reg: Record<string, PropType>): PropType {
  return reg[name] ?? inferType(value);
}
