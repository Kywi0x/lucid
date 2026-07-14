import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Nom court d'un projet à partir de son chemin (ex. /a/b/brainlink → brainlink). */
export function projectName(pathOrSlug: string): string {
  const parts = pathOrSlug.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || pathOrSlug;
}

/** Date relative compacte en français (ex. "il y a 3 j"). */
export function relativeDate(iso: string | null): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "";
  const diff = Date.now() - then;
  const min = Math.round(diff / 60000);
  if (min < 1) return "à l'instant";
  if (min < 60) return `il y a ${min} min`;
  const h = Math.round(min / 60);
  if (h < 24) return `il y a ${h} h`;
  const d = Math.round(h / 24);
  if (d < 30) return `il y a ${d} j`;
  const mo = Math.round(d / 30);
  return `il y a ${mo} mois`;
}

/** Copie dans le presse-papier. Le plugin Tauri d'abord (les API web —
 *  navigator.clipboard ET execCommand — échouent en silence dans la WKWebView),
 *  puis les replis web pour un éventuel contexte navigateur. */
export async function copyText(text: string): Promise<boolean> {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
    return true;
  } catch { /* hors Tauri ou plugin absent → replis web */ }
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    return ok;
  }
}
