import type { ConnectorStatus } from "@/lib/types";
import { relativeDate } from "@/lib/utils";
import { iconOf } from "@/lib/connector-icons";

/** Tuile d'une source + lecture de son état. Vit à part du Launchpad : le
 *  sélecteur de dossiers Drive s'affiche DANS le Launchpad et a besoin de la
 *  tuile — l'importer depuis Launchpad créait un cycle. */

/** État d'une source : rouge = il faut reconnecter, ambre = branchée mais rien
 *  à lire, vert = elle alimente le cerveau.
 *  L'ambre ne regarde PAS l'âge du contenu : `last_sync` porte la date du
 *  document le plus récent, pas celle du dernier passage de Lucid — une source
 *  parfaitement à jour dont rien n'a bougé depuis trois jours passait au orange
 *  pour rien. Quand le Rust exposera une vraie date de vérification, c'est
 *  ICI qu'elle remplacera le compte. */
export type ConnState = "on" | "stale" | "dead";

export function connState(c: ConnectorStatus): ConnState {
  if (c.reconnect_reason || !c.connected) return "dead";
  return c.conversation_count === 0 ? "stale" : "on";
}

export function stateColor(s: ConnState): string {
  return s === "dead" ? "var(--color-err)" : s === "stale" ? "var(--color-warn)" : "var(--color-ok)";
}

/** « 218 documents · plus récent il y a 6 min » — jamais une date absolue, et
 *  jamais le mot « sync » : `last_sync` ne dit pas quand Lucid est allé voir,
 *  mais la date du contenu le plus récent qu'il connaît pour cette source
 *  (Obsidian ne la fournit même pas). Le vrai horodatage de vérification
 *  n'existe pas encore côté Rust. */
export function connMeta(c: ConnectorStatus): string {
  if (c.reconnect_reason) return "accès expiré · reconnecter";
  if (!c.connected) return "non connectée";
  const n = c.conversation_count;
  const count = n ? `${n} document${n > 1 ? "s" : ""}` : "connectée";
  return c.last_sync ? `${count} · plus récent ${relativeDate(c.last_sync)}` : count;
}

export function Tile({ id, size, badge }: { id: string; size?: "sm" | "md" | "big"; badge?: boolean }) {
  const icon = iconOf(id);
  return (
    <span className={`tile ${icon.plate ? "plate" : "bleed"}${size ? ` ${size}` : ""}`}>
      <img src={icon.src} alt="" />
      {badge && <span className="ok">✓</span>}
    </span>
  );
}

