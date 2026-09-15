/**
 * Centre de notifications — construction des cartes à partir de l'état RÉEL de
 * l'app (maquette « l'app vivante », section « Centre de notifications »).
 *
 * Règle reprise de la maquette : trois natures, dans cet ordre — ce qui demande
 * une décision, ce que Lucid a fait tout seul, ce qui s'est passé sans
 * conséquence. Une notification qui n'appelle aucune action n'a pas de bouton,
 * et un échec en a toujours un.
 *
 * Aucun chiffre n'est inventé : tout vient de `connectors_status`, du graphe,
 * du relevé MCP, des compteurs de l'Archiviste, des snapshots et du dernier
 * rapport de scan. Ce qu'on ne sait pas mesurer ne produit pas de carte.
 */
import type { BrainGraph, ConnectorStatus, McpProposal, SnapshotInfo } from "./types";
import type { BrainStats } from "./stats";
import type { ArchivistStats } from "./api";
import { CONNECTOR_ICON } from "./connector-icons";

export type NotifSection = "À traiter" | "Depuis ta dernière visite" | "Cette semaine";
export type NotifTone = "err" | "warn" | "ok" | null;

export interface NotifAction {
  label: string;
  primary?: boolean;
  /** Verbe:argument — interprété par l'app (`node:`, `connect:`, `stats`, `tidy:`, `skipped`, `history`). */
  go?: string;
}

export interface NotifGroupItem {
  icon: string;
  title: string;
  text: string;
  go?: string;
}

export interface NotifItem {
  id: string;
  sec: NotifSection;
  tone: NotifTone;
  /** Clé d'icône : un id de connecteur, ou "brain" pour Lucid lui-même. */
  icon: string;
  when: string;
  title: string;
  text?: string;
  group?: NotifGroupItem[];
  actions?: NotifAction[];
}

// ── Marques locales : ce que l'utilisateur a déjà vu ─────────────────────────
const K_VISIT = "lucid.notif.lastVisit";
const K_ARCH = "lucid.notif.archivist";
const K_VERSION = "lucid.notif.version";
const K_DISMISSED = "lucid.notif.dismissed";
const K_SKIPPED = "lucid.notif.skipped";
const K_SEEN = "lucid.notif.seenIds";

function read<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : (JSON.parse(raw) as T);
  } catch { return fallback; }
}
function write(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* quota : tant pis */ }
}

/** Fichiers illisibles du dernier scan — écrits par le scan, lus ici. */
export function rememberSkipped(files: string[]) { write(K_SKIPPED, files); }

export function dismissedIds(): string[] { return read<string[]>(K_DISMISSED, []); }
export function dismissNotif(id: string) {
  const all = new Set(dismissedIds());
  all.add(id);
  // Les ids portent un compteur (ex. "arch-12") : une carte effacée revient
  // dès que le compteur bouge, sans jamais faire grossir la liste indéfiniment.
  write(K_DISMISSED, [...all].slice(-200));
}
export function clearDismissable(items: NotifItem[]) {
  for (const n of items) if (n.sec !== "À traiter") dismissNotif(n.id);
}

/** Début de la « visite » courante. Figé au démarrage : ce qui est arrivé
 *  pendant que l'app était fermée se lit comme nouveau, pas ce qu'on vient de
 *  voir défiler à l'écran. */
export function openVisit(): number {
  const prev = read<number>(K_VISIT, 0);
  write(K_VISIT, Date.now());
  return prev || Date.now() - 7 * 86400_000; // 1re fois : la dernière semaine
}

/**
 * Première fois que Lucid a VU chaque nœud, par id.
 *
 * Ni `date` (= date du fichier : iCloud qui touche une note la rajeunit) ni
 * `updated_at` (= estampille de réécriture : une régénération ré-estampille
 * des centaines de nœuds dans la même seconde — mesuré sur le cerveau de Liam
 * le 2026-09-15 : 241 nœuds à la même seconde) ne disent « nouveau ». Un id
 * jamais vu, si.
 *
 * Premier lancement : on amorce en silence. Annoncer « 542 documents ajoutés »
 * parce qu'on vient d'ouvrir un carnet serait le même mensonge à l'envers.
 */
function trackFirstSeen(ids: string[]): { firstSeen: Record<string, number>; seeded: boolean } {
  const known = read<Record<string, number>>(K_SEEN, {});
  const seeded = Object.keys(known).length === 0;
  const now = Date.now();
  const next: Record<string, number> = {};
  for (const id of ids) next[id] = known[id] ?? now; // les ids disparus sortent de la mémoire
  write(K_SEEN, next);
  return { firstSeen: next, seeded };
}

/** Un jour de calendrier, jamais une heure : `date` n'en porte pas. */
function dayLabel(iso: string): string {
  const d = new Date(`${iso}T00:00:00`);
  const days = Math.round(
    (new Date().setHours(0, 0, 0, 0) - d.setHours(0, 0, 0, 0)) / 86400_000,
  );
  if (days <= 0) return "aujourd'hui";
  if (days === 1) return "hier";
  if (days < 7) return fmtDay.format(d);
  return `il y a ${days} j`;
}

/** Compteurs Archiviste déjà vus (pour ne compter que le delta). */
export function seenArchivist(): { moved: number; merged: number } {
  return read(K_ARCH, { moved: 0, merged: 0 });
}
export function markArchivistSeen(s: ArchivistStats) {
  write(K_ARCH, { moved: s.moved, merged: s.merged });
}
export function seenVersion(): string { return read<string>(K_VERSION, ""); }
export function markVersionSeen(v: string) { write(K_VERSION, v); }

// ── Formatage ───────────────────────────────────────────────────────────────
const fmtDay = new Intl.DateTimeFormat("fr-FR", { weekday: "long" });

/** « il y a 6 min », « hier », « mardi » — jamais une date absolue dans une
 *  carte : c'est la fraîcheur qui compte, pas l'horodatage. */
export function whenLabel(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < 60_000) return "à l'instant";
  if (diff < 3600_000) return `il y a ${Math.round(diff / 60_000)} min`;
  if (diff < 86400_000) return `il y a ${Math.round(diff / 3600_000)} h`;
  if (diff < 2 * 86400_000) return "hier";
  if (diff < 7 * 86400_000) return fmtDay.format(new Date(ms));
  return `il y a ${Math.round(diff / 86400_000)} j`;
}

function plural(n: number, one: string, many = `${one}s`) {
  return `${n} ${n > 1 ? many : one}`;
}

/** Nœuds de structure : un dossier qui gagne un enfant n'est pas un document
 *  ajouté — ses enfants, eux, apparaissent déjà dans la pile. */
const STRUCTURAL = new Set(["root", "container", "group", "espace", "source", "pending"]);

export interface NotifInput {
  connectors: ConnectorStatus[];
  graph: BrainGraph | null;
  stats: BrainStats | null;
  archivist: ArchivistStats | null;
  proposals: McpProposal[];
  snapshots: SnapshotInfo[];
  version: string;
  /** Début de la visite courante (epoch ms) — cf. `openVisit`. */
  visitStart: number;
}

export function buildNotifications(input: NotifInput): NotifItem[] {
  const out: NotifItem[] = [];
  const skipped = read<string[]>(K_SKIPPED, []);

  // ── 1. À traiter : ce qui attend une décision ou un geste ──
  for (const c of input.connectors) {
    if (!c.reconnect_reason) continue;
    out.push({
      id: `dead-${c.id}`,
      sec: "À traiter",
      tone: "err",
      icon: c.id,
      when: c.last_sync ? whenLabel(Date.parse(c.last_sync)) : "",
      title: `${c.name} — accès expiré`,
      text: c.reconnect_reason,
      actions: [{ label: "Reconnecter", primary: true, go: `connect:${c.id}` }],
    });
  }

  if (skipped.length) {
    out.push({
      id: `skipped-${skipped.length}`,
      sec: "À traiter",
      tone: "warn",
      icon: "local-folder",
      when: "",
      title: `${plural(skipped.length, "fichier illisible", "fichiers illisibles")}`,
      // Jamais « ignorés » : ils sont comptés, on sait lesquels, on peut les voir.
      text: "Lucid n'a pas su en extraire le texte. Ils sont comptés comme illisibles, pas ignorés.",
      actions: [{ label: "Voir lesquels", go: "skipped" }],
    });
  }

  if (input.proposals.length) {
    const n = input.proposals.length;
    out.push({
      id: `tidy-${input.proposals.map((p) => p.id).join("|").slice(0, 60)}`,
      sec: "À traiter",
      tone: "warn",
      icon: "brain",
      when: whenLabel(Date.parse(input.proposals[0].created_at) || Date.now()),
      title: `${plural(n, "proposition")} de ton IA`,
      text: input.proposals.slice(0, 3).map((p) => describeProposal(p, input.graph)).join(" · ")
        + (n > 3 ? " …" : ""),
      actions: [
        { label: "Tout accepter", primary: true, go: "tidy:yes" },
        { label: "Refuser", go: "tidy:no" },
      ],
    });
  }

  // ── 2. Depuis ta dernière visite : ce que Lucid a fait tout seul ──
  const since = input.visitStart;
  const docs = (input.graph?.nodes ?? []).filter((n) => !STRUCTURAL.has(n.kind));
  const { firstSeen, seeded } = trackFirstSeen(docs.map((n) => n.id));
  // `seeded` = on vient de découvrir le cerveau : rien n'est « nouveau ».
  const fresh = seeded
    ? []
    : docs
        .filter((n) => firstSeen[n.id] >= since)
        .sort((a, b) => firstSeen[b.id] - firstSeen[a.id])
        .slice(0, 12);
  if (fresh.length) {
    const newest = fresh[0];
    // L'heure affichée est celle du DOCUMENT quand il la porte (au jour, jamais
    // une heure inventée) ; à défaut, le moment où Lucid l'a découvert.
    const label = (n: typeof newest) => (n.date ? dayLabel(n.date) : whenLabel(firstSeen[n.id]));
    out.push({
      id: `new-${fresh.length}-${newest.id}-${firstSeen[newest.id]}`,
      sec: "Depuis ta dernière visite",
      tone: null,
      icon: newest.connector ?? "brain",
      when: label(newest),
      title: `${plural(fresh.length, "document ajouté", "documents ajoutés")}`,
      group: fresh.map((n) => ({
        icon: n.connector ?? "brain",
        title: n.label,
        text: `${connectorName(input.connectors, n.connector)} · ${label(n)}`,
        go: `node:${n.id}`,
      })),
    });
  }

  if (input.archivist) {
    const seen = seenArchivist();
    const delta = (input.archivist.moved - seen.moved) + (input.archivist.merged - seen.merged);
    if (delta > 0) {
      // « Annuler » = restaurer le snapshot pris AVANT l'application (reason
      // "mcp_accept"). S'il n'y en a pas, pas de bouton : jamais une action
      // affichée qu'on ne sait pas tenir.
      const snap = input.snapshots.find((s) => s.reason === "mcp_accept");
      out.push({
        id: `arch-${input.archivist.moved}-${input.archivist.merged}`,
        sec: "Depuis ta dernière visite",
        tone: "ok",
        icon: "brain",
        when: snap ? whenLabel(snap.created_at * 1000) : "",
        title: `${plural(delta, "fichier rangé", "fichiers rangés")} sans toi`,
        text: `${plural(input.archivist.moved - seen.moved, "page déplacée", "pages déplacées")}, ` +
              `${plural(input.archivist.merged - seen.merged, "doublon fusionné", "doublons fusionnés")}.`,
        actions: snap ? [{ label: "Annuler", go: `restore:${snap.id}` }] : undefined,
      });
    }
  }

  const calls = (input.stats?.calls ?? []).reduce((a, b) => a + b, 0);
  if (calls > 0) {
    const saved = input.stats?.tokensSaved ?? 0;
    out.push({
      id: `mcp-${calls}`,
      sec: "Depuis ta dernière visite",
      tone: null,
      icon: "brain",
      when: "",
      title: `${plural(calls, "consultation")} par tes IA`,
      text: saved > 0
        ? `Environ ${new Intl.NumberFormat("fr-FR").format(Math.round(saved))} tokens évités.`
        : undefined,
      actions: [{ label: "Voir le relevé", go: "stats" }],
    });
  }

  // ── 3. Cette semaine : ce qui s'est passé sans conséquence ──
  if (input.version && input.version !== seenVersion()) {
    out.push({
      id: `ver-${input.version}`,
      sec: "Cette semaine",
      tone: null,
      icon: "brain",
      when: "",
      title: "Mise à jour installée",
      text: `Version ${input.version}.`,
    });
  }

  const weekAgo = Date.now() - 7 * 86400_000;
  const lastSnap = input.snapshots.find((s) => s.created_at * 1000 > weekAgo);
  if (lastSnap) {
    out.push({
      id: `snap-${lastSnap.id}`,
      sec: "Cette semaine",
      tone: null,
      icon: "brain",
      when: whenLabel(lastSnap.created_at * 1000),
      title: "Sauvegarde locale créée",
      text: `${new Intl.NumberFormat("fr-FR").format(lastSnap.node_count)} pages, conservée sur cette machine.`,
      actions: [{ label: "Historique", go: "history" }],
    });
  }

  const hidden = new Set(dismissedIds());
  const kept = out.filter((n) => !hidden.has(n.id));
  // Ordre des sections = ordre de lecture ; à l'intérieur, l'ordre de
  // construction (le plus urgent d'abord) suffit.
  const order: NotifSection[] = ["À traiter", "Depuis ta dernière visite", "Cette semaine"];
  return kept.sort((a, b) => order.indexOf(a.sec) - order.indexOf(b.sec));
}

/** Ce qu'une proposition fera, en français, avec les vrais titres de pages. */
function describeProposal(p: McpProposal, graph: BrainGraph | null): string {
  const labelOf = (id: string) => {
    if (!id) return id;
    const n = graph?.nodes.find((x) => x.id === id);
    return n ? (n.kind === "root" ? "Lucid (racine)" : n.label) : id;
  };
  switch (p.action) {
    case "update": return `Modifier « ${labelOf(p.target_id)} »`;
    case "move":   return `Déplacer « ${labelOf(p.target_id)} » → ${labelOf(p.new_parent_id)}`;
    case "merge":  return `Fusionner ${p.merge_ids.length} pages`;
    case "link":   return `Lier « ${labelOf(p.target_id)} » ↔ « ${labelOf(p.link_target)} »`;
    default:       return `Créer « ${p.label} »`;
  }
}

function connectorName(connectors: ConnectorStatus[], id?: string | null): string {
  if (!id) return "Lucid";
  return connectors.find((c) => c.id === id)?.name
    ?? (CONNECTOR_ICON[id] ? id : "Lucid");
}
