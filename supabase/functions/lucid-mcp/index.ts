// Lucid — MCP distant (streamable HTTP, stateless).
// Sert un space PARTAGÉ (public) à n'importe quel client MCP : claude.ai (web/mobile),
// ChatGPT, agents… Mêmes tools que le MCP local (brain_overview/search/node), scopés
// au space passé en `?space=<uuid>`.
//
// Déploiement : supabase functions deploy lucid-mcp --no-verify-jwt
// (--no-verify-jwt : les clients MCP ne portent pas de JWT Supabase ; l'accès est
//  contrôlé par la visibilité du space — v1 ne sert QUE les spaces publics.)
//
// URL connecteur : https://<ref>.supabase.co/functions/v1/lucid-mcp?space=<uuid>

type Node = {
  id: string; label: string; kind: string; weight?: number;
  summary?: string; keywords?: string[]; parent_id?: string | null;
  date?: string | null; content?: string;
};
type Payload = { title: string; nodes: Node[]; edges?: { source: string; target: string; kind: string }[] };

// ── Tools (miroir de lucid_mcp.rs, en pur — testable sans réseau) ─────────────

function pathOf(n: Node, byId: Map<string, Node>): string {
  const parts: string[] = [];
  let cur = n.parent_id;
  for (let guard = 0; cur && guard < 12; guard++) {
    const p = byId.get(cur);
    if (!p) break;
    parts.push(p.label);
    cur = p.parent_id;
  }
  return parts.reverse().join(" › ");
}

function truncate(s: string, max: number): string {
  return s.length <= max ? s : s.slice(0, max) + "\n\n*[… tronqué]*";
}

// ── Lecture ciblée d'une page ─────────────────────────────────────────────────
// `brain_node` renvoyait `truncate(content, 24_000)` ≈ 7 000 tokens PAR page.
// Mesuré le 2026-08-03 sur une question type (1 overview + 3 search + 2 lectures) :
// la lecture de page pèse 93 % du coût en tokens — très au-delà de ce que la
// méthode de recherche peut économiser. On renvoie donc les passages qui matchent
// la requête ; `full: true` reste l'échappatoire vers l'ancien comportement.
const TARGETED_BUDGET = 3_000; // avec `query` : passages retenus
const BLIND_BUDGET = 8_000; // sans `query` : début de page, moins agressif
const FULL_MAX = 24_000; // `full: true` : plafond historique
const BLOCK_SIZE = 800; // granularité d'un passage
const SEARCH_EXCERPT = 500; // extrait par résultat de recherche
const CHILD_EXCERPT = 400; // aperçu par sous-page dans une page-conteneur
const CHILDREN_BUDGET = 8_000; // ≈ 2 000 tokens : 20 sous-pages servies, le reste listé

/// Un montant, symbole AVANT (`$25.00`, facture américaine) ou APRÈS (`16,19 €`).
/// N'accepter que la seconde forme laissait les factures Xano sans montant alors
/// que Parabola passait, son OCR portant « USD » derrière le nombre (2026-08-06).
/// Volontairement large : c'est un indice de position dans le document, pas un
/// extracteur de montant.
const MONEY = /[€$£]\s?\d|\d[\d\s.,]*(?:€|£|\$|eur|usd|chf|ttc|\bht\b)/i;

/// Aperçu d'un document quand le client ne passe pas de `query` — le cas réel,
/// ChatGPT n'en passe pas. Le fournisseur et la date sont en tête, mais le total
/// est au MILIEU, suivi des mentions légales : ni « les N premiers caractères »
/// ni « début + fin » ne l'attrapent (mesuré le 2026-08-06 — trois fournisseurs
/// identifiés, un seul montant). On sert donc le début + le passage qui porte un
/// montant, et on retombe sur début + fin quand il n'y en a pas.
function blindPeek(s: string, budget: number): string {
  const t = s.replace(/\s+/g, " ").trim();
  if (t.length <= budget) return t;
  const head = Math.ceil(budget * 0.5);
  const rest = budget - head;
  const m = t.slice(head).match(MONEY);
  const from = m?.index === undefined
    ? t.length - rest // pas de montant : la fin, faute de mieux
    : Math.max(head, head + m.index - 40); // un peu de contexte avant le chiffre
  return `${t.slice(0, head)} […] ${t.slice(from, from + rest)}`;
}
// Plafond de résultats : c'était un `8` en dur, sans justification écrite. Le coût
// d'un cran de plus vaut MAX_RESULTS × SEARCH_EXCERPT caractères — à rouvrir si une
// campagne montre la bonne page systématiquement en 9ᵉ position.
const MAX_RESULTS = 8;

/** Minuscules + accents pliés : « geographique » doit matcher « géographique ».
 *  ponytail: `brain_search` gagnerait à s'en servir aussi (pas de tokenisation ni
 *  de pliage aujourd'hui) — chantier séparé, il change le classement des résultats. */
function fold(s: string): string {
  return s.normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase();
}

/// Découpe en blocs d'environ `size` caractères, aux sauts de ligne quand c'est
/// possible. Une ligne plus longue que la fenêtre (export CSV, JSON minifié) est
/// coupée en dur : sans ça un tableur entier ne formerait qu'un seul bloc.
function blocks(s: string, size: number): string[] {
  const lines: string[] = [];
  for (const line of s.split("\n")) {
    if (line.length <= size) lines.push(line);
    else for (let i = 0; i < line.length; i += size) lines.push(line.slice(i, i + size));
  }
  const out: string[] = [];
  let cur = "";
  for (const line of lines) {
    if (cur && cur.length + line.length + 1 > size) { out.push(cur); cur = ""; }
    cur += (cur ? "\n" : "") + line;
  }
  if (cur.trim()) out.push(cur);
  return out;
}

/// Passages de `content` qui matchent `query`, dans l'ordre du document, sous le
/// budget de caractères. `null` = rien ne matche (l'appelant renvoie le début de
/// page). Pur — vérifié par `scripts/check-mcp-passages.ts`.
export function relevantPassages(content: string, query: string, budget = TARGETED_BUDGET): string | null {
  const terms = fold(query).split(/[^\p{L}\p{N}]+/u).filter((t) => t.length > 2);
  if (!terms.length) return null;
  const scored = blocks(content, BLOCK_SIZE)
    .map((text, i) => {
      const f = fold(text);
      let score = 0;
      for (const t of terms) score += f.split(t).length - 1;
      return { i, text, score };
    })
    .filter((b) => b.score > 0)
    .sort((a, b) => b.score - a.score || a.i - b.i);
  if (!scored.length) return null;

  const kept: typeof scored = [];
  let used = 0;
  for (const b of scored) {
    if (kept.length && used + b.text.length > budget) break;
    kept.push(b);
    used += b.text.length;
  }
  kept.sort((a, b) => a.i - b.i);

  let out = "";
  let prev = -1;
  for (const b of kept) {
    if (prev >= 0) out += b.i === prev + 1 ? "\n" : "\n\n*[…]*\n\n";
    out += b.text.trim();
    prev = b.i;
  }
  return out;
}

/// Rend la section « Contenu » : passages ciblés si `query`, début de page sinon,
/// page entière (plafonnée) si `full`. Une page déjà courte part telle quelle —
/// il n'y a rien à économiser.
function contentSection(content: string, opts: { query?: string; full?: boolean }): string {
  if (opts.full) return `\n${truncate(content, FULL_MAX)}`;
  if (content.length <= TARGETED_BUDGET) return `\n${content}`;
  const passages = opts.query ? relevantPassages(content, opts.query) : null;
  if (passages) {
    return ` — passages pertinents (${passages.length} des ${content.length} caractères)\n${passages}\n\n` +
      `*Extraits ciblés sur « ${opts.query} ». S'il te manque du contexte, rappelle \`brain_node\` avec \`full: true\`.*`;
  }
  const why = opts.query
    ? `Aucun passage ne matche « ${opts.query} » — début de page renvoyé.`
    : "Passe `query` (les mots de ta recherche) pour recevoir les passages pertinents au lieu du début de page.";
  return ` — début de page (${Math.min(content.length, BLIND_BUDGET)} des ${content.length} caractères)\n` +
    `${truncate(content, BLIND_BUDGET)}\n\n*${why} \`full: true\` pour la page entière.*`;
}

export function toolOverview(p: Payload): string {
  const ids = new Set(p.nodes.map((n) => n.id));
  const tops = p.nodes.filter((n) => !n.parent_id || !ids.has(n.parent_id));
  let out = `# ${p.title} — vue d'ensemble du space\n`;
  const listUnder = (parent: Node) => {
    const kids = p.nodes.filter((n) => n.parent_id === parent.id);
    out += `\n## ${parent.label} (\`${parent.id}\`, ${kids.length} sous-pages)\n`;
    if (parent.summary) out += `${parent.summary}\n`;
    if (parent.keywords?.length) out += `Mots-clés : ${parent.keywords.join(", ")}\n`;
  };
  // Un seul top (racine du space) → on présente ses enfants ; sinon les tops.
  if (tops.length === 1) {
    for (const kid of p.nodes.filter((n) => n.parent_id === tops[0].id)) listUnder(kid);
  } else {
    for (const t of tops) listUnder(t);
  }
  out += `\n${p.nodes.length} pages au total. Utilise \`brain_search\` pour chercher un sujet, \`brain_node\` avec un id pour lire une page.\n`;
  return out;
}

export function toolSearch(p: Payload, query: string): string {
  // Tokenisation sur les non-alphanumériques (« l'assurance », « loyer, » →
  // « assurance », « loyer ») + accents pliés. Avant, `split(/\s+/)` gardait la
  // ponctuation collée au mot et « geographique » ne trouvait pas « géographique ».
  const terms = [...new Set(fold(query).split(/[^\p{L}\p{N}]+/u).filter((t) => t.length >= 2))];
  if (!terms.length) throw new Error("query vide");
  const byId = new Map(p.nodes.map((n) => [n.id, n]));

  const docs = p.nodes.map((n) => ({
    n,
    label: fold(n.label),
    kw: fold((n.keywords ?? []).join(" ")),
    summary: fold(n.summary ?? ""),
    content: fold(n.content ?? ""),
  }));

  // IDF : un mot pèse selon sa RARETÉ. « document » est dans des centaines de
  // pages et ne décide rien ; « vaccination » n'est que dans une et décide tout.
  // Sans ça, l'aiguille se retrouvait à égalité avec le bruit et sortait du top 8
  // (mesuré le 2026-08-05, `cargo run --example retrieval`). Effet de bord utile :
  // les mots vides (« de », « pour », « sur ») s'annulent tout seuls, plus besoin
  // d'une liste à maintenir.
  const idf = new Map(terms.map((t) => {
    const df = docs.filter((d) =>
      d.label.includes(t) || d.kw.includes(t) || d.summary.includes(t) || d.content.includes(t)
    ).length;
    return [t, df === 0 ? 0 : Math.log(1 + p.nodes.length / df)];
  }));

  const scored = docs.flatMap((d) => {
    let score = 0;
    for (const t of terms) {
      const w = idf.get(t) ?? 0;
      if (w === 0) continue;
      if (d.label.includes(t)) score += 5 * w;
      if (d.kw.includes(t)) score += 3 * w;
      if (d.summary.includes(t)) score += 2 * w;
      if (d.content.includes(t)) score += 1 * w;
    }
    return score > 0 ? [{ score, n: d.n }] : [];
  }).sort((a, b) => b.score - a.score);

  // Réponse d'absence EXPLICITE : « Aucun résultat pour X » invitait à réessayer,
  // et les clients enchaînaient jusqu'à 8 recherches de synonymes avant d'oser
  // dire « je ne trouve pas » (mesuré le 2026-08-05 : 21 recherches pour 3
  // questions d'absence). Chaque tour coûte un aller-retour complet à
  // l'utilisateur — le pire poste de tout le parcours.
  if (!scored.length) {
    return `Aucune des ${p.nodes.length} pages du cerveau ne contient « ${query} » — la recherche a couvert les titres, les mots-clés, les résumés et le contenu de chaque page.\n\n` +
      `Si tu as déjà essayé une ou deux reformulations, arrête là : l'information n'est pas dans ce cerveau. Dis-le franchement plutôt que d'énumérer d'autres synonymes.\n`;
  }
  let out = `Résultats pour « ${query} » :\n`;
  for (const { n } of scored.slice(0, MAX_RESULTS)) {
    // Extrait CIBLÉ sur la requête, pas les 200 premiers caractères : un client
    // faible s'arrête à la recherche et conclut « l'information n'est pas
    // visible » sans jamais appeler brain_node (constaté avec ChatGPT le
    // 2026-08-05 : 2/5 contre 5/5 pour Claude, uniquement faute de chaînage).
    // L'extrait doit donc porter la réponse, pas seulement le début de la page.
    const body = n.content ?? "";
    const excerpt = (relevantPassages(body, query, SEARCH_EXCERPT) ??
      truncate(n.summary || body, SEARCH_EXCERPT)).replace(/\n+/g, " ");
    out += `\n- **${n.label}** (\`${n.id}\`, ${n.kind})${n.date ? ` · ${n.date}` : ""} — ${pathOf(n, byId) || "racine"}\n  ${excerpt}\n`;
  }
  out += "\nLis une page complète avec `brain_node`.\n";
  return out;
}

/// Les clients MCP passent un tableau tantôt comme tableau, tantôt comme chaîne
/// JSON, tantôt comme liste séparée par des virgules. On accepte les trois plutôt
/// que de punir le client qui n'a pas deviné la bonne forme (leçon du 06/08).
export function idList(raw: unknown): string[] {
  if (Array.isArray(raw)) return raw.map(String);
  if (typeof raw !== "string") return [];
  const t = raw.trim();
  if (t.startsWith("[")) {
    try { const j = JSON.parse(t); if (Array.isArray(j)) return j.map(String); } catch { /* pas du JSON : on tombe plus bas */ }
  }
  return t.includes(",") ? t.split(",").map((x) => x.trim()).filter(Boolean) : (t ? [t] : []);
}

/// B4 — plusieurs pages en UN appel. Un appel d'outil ne coûte pas que sa
/// réponse : il impose un tour de modèle complet qui relit tout le contexte, deux
/// fois. Trois lectures en un appel, c'est deux tours de modèle économisés — et
/// c'est le cas fréquent quand les documents sont éparpillés hors d'un conteneur
/// (quand ils sont DANS un conteneur, la lecture du conteneur suffit déjà).
export const MAX_NODES_PER_CALL = 5;

export function toolNodes(p: Payload, nodeIds: string[], opts: { query?: string; full?: boolean } = {}): string {
  const ids = nodeIds.map((s) => String(s).trim()).filter(Boolean);
  if (!ids.length) throw new Error("aucun id fourni (utilise brain_search pour en trouver)");
  if (ids.length === 1) return toolNode(p, ids[0], opts);
  const kept = ids.slice(0, MAX_NODES_PER_CALL);
  // Une page introuvable ne doit pas faire échouer les autres : on le dit et on
  // continue, sinon un seul mauvais id perd les quatre bonnes lectures.
  const parts = kept.map((nodeId) => {
    try { return toolNode(p, nodeId, opts); }
    catch (e) { return `# (page illisible)\n\n- id : \`${nodeId}\`\n- erreur : ${(e as Error).message}\n`; }
  });
  let out = parts.join("\n\n---\n\n");
  // Jamais de troncature muette : le modèle doit savoir qu'il lui reste des pages.
  if (ids.length > kept.length) {
    out += `\n\n---\n\n*${ids.length - kept.length} page(s) non lue(s) : ${MAX_NODES_PER_CALL} maximum par appel. Rappelle \`brain_node\` avec le reste.*\n`;
  }
  return out;
}

export function toolNode(p: Payload, nodeId: string, opts: { query?: string; full?: boolean } = {}): string {
  const byId = new Map(p.nodes.map((n) => [n.id, n]));
  const n = byId.get(nodeId);
  if (!n) throw new Error(`nœud \`${nodeId}\` introuvable (utilise brain_search pour trouver un id)`);
  let out = `# ${n.label}\n\n- id : \`${n.id}\` · type : ${n.kind}\n`;
  const path = pathOf(n, byId);
  if (path) out += `- chemin : ${path}\n`;
  if (n.date) out += `- date : ${n.date}\n`;
  if (n.keywords?.length) out += `- mots-clés : ${n.keywords.join(", ")}\n`;
  if (n.summary) out += `\n## Résumé\n${n.summary}\n`;
  if (n.content) out += `\n## Contenu${contentSection(n.content, opts)}\n`;
  const kids = p.nodes.filter((c) => c.parent_id === n.id);
  if (kids.length) {
    // Une page-conteneur ne listait que des titres : « sur quelle période ? » sur un
    // dossier de 16 factures demandait 16 appels `brain_node`, et « ce que je paie à
    // chacun » sur deux dossiers, 29. Un client à quota renonce avant (ChatGPT,
    // 2026-08-06 : s'arrête à l'index et déclare l'information inaccessible — il
    // avait raison, elle n'y était pas). On descend donc date + passage ciblé de
    // chaque enfant : la question d'agrégation tient en UN appel.
    out += "\n## Sous-pages\n";
    let budget = CHILDREN_BUDGET;
    let skipped = 0;
    for (const c of kids) {
      out += `- **${c.label}** (\`${c.id}\`)${c.date ? ` · ${c.date}` : ""}\n`;
      if (budget <= 0) { skipped++; continue; }
      // L'aperçu ne peut PAS dépendre de `query` : mesuré le 2026-08-06, ChatGPT
      // appelle `brain_node` sans, et repartait avec « je n'ai accès qu'à
      // l'organisation, pas au contenu » — alors que les dates, elles,
      // remontaient. Avec `query` on cible ; sinon on descend quand même le
      // résumé, ou le début du document. Budget 1 → `relevantPassages` garde son
      // meilleur bloc. ponytail: coupe au caractère, un montant peut tomber en
      // bordure — l'id est là, `brain_node` donne la page entière.
      // `||` et pas `??` : le payload sérialise les champs Rust vides en `""`, pas
      // en `null` (cf. `share.ts`, `summary: n.summary`). Avec `??` la chaîne vide
      // passait pour une valeur et bloquait le repli — aperçu vide, sous-page nue.
      const targeted = opts.query ? relevantPassages(c.content ?? "", opts.query, 1) : null;
      const peek = targeted
        ? targeted.slice(0, CHILD_EXCERPT).replace(/\s+/g, " ").trim()
        : blindPeek(c.summary || c.content || "", CHILD_EXCERPT);
      if (peek) { budget -= peek.length; out += `  ${peek}\n`; }
    }
    if (skipped) out += `\n*${skipped} sous-pages listées sans extrait (budget atteint) — lis-les avec \`brain_node\`.*\n`;
  }
  return out;
}

// ── Chargement du space ───────────────────────────────────────────────────────
// Clé service partout ici (pas la clé anon) : l'accès est déjà tranché en amont
// par `spaceIdFromToken` (token MCP valide → space_id), qu'il s'agisse d'un
// space publié classique OU du space "personnel" (visibility='personal', tout
// le cerveau du compte — cf. décision 2026-07-21 : mêmes outils, locaux ou
// distants). La RLS anon (spaces publics uniquement) reste en place pour le
// viewer web, mais ne concerne plus ce chemin MCP.

function env(k: string): string | undefined {
  // Edge runtime = Deno.env ; tests locaux (tsx/node) = process.env.
  return typeof Deno !== "undefined" && Deno.env
    ? Deno.env.get(k)
    : (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env[k];
}

/// Le token MCP est une capability SÉPARÉE du lien de partage : il n'est lisible
/// que par le propriétaire (RLS) et par cette function via la clé service.
async function spaceIdFromToken(token: string): Promise<string> {
  if (!/^[0-9a-f-]{36}$/i.test(token)) throw new Error("token MCP invalide — utilise l'URL affichée dans la modale Partager de Lucid");
  const key = env("SUPABASE_SERVICE_ROLE_KEY")!;
  const r = await fetch(
    `${env("SUPABASE_URL")}/rest/v1/space_mcp_tokens?token=eq.${encodeURIComponent(token)}&select=space_id`,
    { headers: { apikey: key, Authorization: `Bearer ${key}` } },
  );
  if (!r.ok) throw new Error(`stockage indisponible (${r.status})`);
  const rows = await r.json();
  if (!rows.length) throw new Error("token MCP inconnu ou révoqué — republie le space dans Lucid pour en obtenir un");
  return rows[0].space_id as string;
}

async function loadSpace(spaceId: string): Promise<Payload> {
  const base = env("SUPABASE_URL")!;
  const key = env("SUPABASE_SERVICE_ROLE_KEY")!;
  const r = await fetch(
    `${base}/rest/v1/shared_spaces?id=eq.${encodeURIComponent(spaceId)}&select=title,data`,
    { headers: { apikey: key, Authorization: `Bearer ${key}` } },
  );
  if (!r.ok) throw new Error(`stockage indisponible (${r.status})`);
  const rows = await r.json();
  if (!rows.length) throw new Error("space introuvable — le lien MCP a peut-être été révoqué, republie-le dans Lucid");
  return rows[0].data as Payload;
}

/// Une proposition précédente existe-t-elle dans CE space ? Clé service : la
/// RLS interdit — à raison — la lecture anon des propositions ; ici on n'en
/// tire qu'un booléen, rien ne fuite. Permet le chaînage parent→sous-pages.
async function proposalExists(id: string, spaceId: string): Promise<boolean> {
  if (!/^[0-9a-f-]{36}$/i.test(id)) return false;
  const key = env("SUPABASE_SERVICE_ROLE_KEY")!;
  const r = await fetch(
    `${env("SUPABASE_URL")}/rest/v1/mcp_proposals?id=eq.${encodeURIComponent(id)}&space_id=eq.${encodeURIComponent(spaceId)}&select=id&limit=1`,
    { headers: { apikey: key, Authorization: `Bearer ${key}` } },
  );
  if (!r.ok) return false;
  return ((await r.json()) as unknown[]).length > 0;
}

/// Vérifie que `id` est une page du space OU une proposition déjà déposée —
/// réutilisée pour valider n'importe quelle référence à un nœud existant
/// (parent d'une création, ou cible d'une modification/déplacement/fusion/lien).
async function assertParent(spaceId: string, id: string): Promise<void> {
  const payload = await loadSpace(spaceId);
  if (!payload.nodes.some((n) => n.id === id) && !(await proposalExists(id, spaceId))) {
    throw new Error(`\`${id}\` introuvable — utilise brain_overview/brain_search pour l'id d'une page existante, ou l'id renvoyé par un dépôt précédent`);
  }
}

async function insertProposals(rows: ProposalRow[]): Promise<void> {
  // Clé service : l'accès est déjà tranché par le token MCP (spaceIdFromToken)
  // et par assertParent — pas besoin de la RLS anon pour ce POST.
  const key = env("SUPABASE_SERVICE_ROLE_KEY")!;
  const r = await fetch(`${env("SUPABASE_URL")}/rest/v1/mcp_proposals`, {
    method: "POST",
    headers: { apikey: key, Authorization: `Bearer ${key}`, "content-type": "application/json", Prefer: "return=minimal" },
    body: JSON.stringify(rows),
  });
  if (!r.ok) throw new Error(`dépôt refusé (${r.status})`);
}

// Le serveur ne connaît jamais le mode local (autonome/validation manuelle) —
// formulation neutre, valable dans les deux cas, sur tous les messages de dépôt.
const PENDING_NOTE = "En attente de validation dans Lucid, ou appliqué tout de suite si le mode autonome est actif.";

/// Dépose une PROPOSITION de note (jamais d'écriture directe) : elle transite
/// par la table mcp_proposals, l'app du propriétaire la rapatrie dans son
/// circuit local de validation (bulles fantômes/badges), puis la supprime.
async function addProposal(spaceId: string, parentId: string, label: string, content: string): Promise<string> {
  if (!label.trim()) throw new Error("label vide");
  await assertParent(spaceId, parentId);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, action: "create", parent_id: parentId, label: label.trim(), content }]);
  return `Proposition \`${proposalId}\` déposée. ${PENDING_NOTE} Pour proposer une sous-page de celle-ci, rappelle brain_add_note avec parent_id="${proposalId}".`;
}

/// PROPOSE l'écrasement du contenu d'une page EXISTANTE (jamais la création).
async function updateNode(spaceId: string, targetId: string, content: string): Promise<string> {
  if (!targetId.trim()) throw new Error("node_id vide");
  await assertParent(spaceId, targetId);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, action: "update", target_id: targetId, content }]);
  return `Proposition \`${proposalId}\` déposée : remplace le contenu de \`${targetId}\`. ${PENDING_NOTE}`;
}

/// PROPOSE de reparenter une page existante sous une autre.
async function moveNode(spaceId: string, targetId: string, newParentId: string): Promise<string> {
  if (!targetId.trim() || !newParentId.trim()) throw new Error("node_id et new_parent_id requis");
  await assertParent(spaceId, targetId);
  await assertParent(spaceId, newParentId);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, action: "move", target_id: targetId, new_parent_id: newParentId }]);
  return `Proposition \`${proposalId}\` déposée : déplace \`${targetId}\` sous \`${newParentId}\`. ${PENDING_NOTE}`;
}

/// PROPOSE la fusion de 2+ pages existantes (la première de `nodeIds` survit).
async function mergeNodes(spaceId: string, nodeIds: string[], label?: string): Promise<string> {
  if (nodeIds.length < 2) throw new Error("node_ids demande au moins 2 ids");
  for (const nid of nodeIds) await assertParent(spaceId, nid);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, action: "merge", merge_ids: nodeIds, label: label?.trim() ?? "" }]);
  return `Proposition \`${proposalId}\` déposée : fusionne ${nodeIds.length} pages (\`${nodeIds[0]}\` survit, les autres seront retirées). ${PENDING_NOTE}`;
}

/// PROPOSE un pont conceptuel entre deux pages existantes.
async function linkNodes(spaceId: string, a: string, b: string, relation?: string): Promise<string> {
  if (!a.trim() || !b.trim()) throw new Error("node_id_a et node_id_b requis");
  if (a === b) throw new Error("une page ne peut pas être liée à elle-même");
  await assertParent(spaceId, a);
  await assertParent(spaceId, b);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, action: "link", target_id: a, link_target: b, relation: relation?.trim() ?? "" }]);
  return `Proposition \`${proposalId}\` déposée : lie \`${a}\` et \`${b}\`. ${PENDING_NOTE}`;
}

// ── Arbre de propositions (plusieurs nœuds/sous-nœuds en UN appel) ────────────

type NoteTree = { label: string; content?: string; children?: NoteTree[] };
type ProposalRow = {
  id: string; space_id: string; action: string;
  parent_id?: string; label?: string; content?: string;
  target_id?: string; new_parent_id?: string; merge_ids?: string[]; link_target?: string; relation?: string;
};

/// Aplati un arbre imbriqué en lignes mcp_proposals chaînées par parent_id.
/// Pur (hors uuid) — exporté pour les tests. Bornes anti-abus alignées sur les
/// checks SQL (label ≤ 200, content ≤ 100k) + caps arbre.
export function flattenTree(spaceId: string, rootParentId: string, nodes: NoteTree[]): { rows: ProposalRow[]; outline: string } {
  const rows: ProposalRow[] = [];
  let outline = "";
  const walk = (list: NoteTree[], parent: string, depth: number) => {
    if (depth >= 8) throw new Error("profondeur max : 8 niveaux");
    for (const n of list) {
      const label = (n.label ?? "").trim();
      if (!label || label.length > 200) throw new Error(`label invalide (1–200 caractères) : « ${label.slice(0, 40)} »`);
      const content = n.content ?? "";
      if (content.length > 100_000) throw new Error(`content trop long pour « ${label} » (100k max)`);
      if (rows.length >= 60) throw new Error("60 notes max par appel — découpe l'arbre en plusieurs appels");
      const id = crypto.randomUUID();
      rows.push({ id, space_id: spaceId, action: "create", parent_id: parent, label, content });
      outline += `${"  ".repeat(depth)}- ${label} → \`${id}\`\n`;
      if (n.children?.length) walk(n.children, id, depth + 1);
    }
  };
  walk(nodes, rootParentId, 0);
  if (!rows.length) throw new Error("nodes vide — passe au moins une note");
  return { rows, outline };
}

async function addTree(spaceId: string, parentId: string, nodes: NoteTree[]): Promise<string> {
  await assertParent(spaceId, parentId);
  const { rows, outline } = flattenTree(spaceId, parentId, nodes);
  await insertProposals(rows);
  return `${rows.length} proposition(s) déposée(s) sous \`${parentId}\` :\n${outline}\n${PENDING_NOTE} Accepter une note accepte automatiquement ses parents. Pour prolonger une branche, rappelle brain_add_tree (ou brain_add_note) avec l'id de la note concernée.`;
}

// ── Protocole MCP (JSON-RPC 2.0 sur POST, stateless) ─────────────────────────

// annotations : readOnlyHint=true sur les 3 lectures → les clients MCP (Claude
// Desktop/Code) peuvent proposer "toujours autoriser" au lieu de reconfirmer à
// chaque appel. Les propositions restent readOnlyHint=false (elles ÉCRIVENT
// une ligne mcp_proposals, même si brain.json n'est jamais touché directement)
// mais destructiveHint=false (rien n'est jamais perdu tant que ce n'est pas
// accepté). Miroir des annotations déjà en place côté MCP local historique.
const TOOLS = [
  {
    name: "brain_overview",
    description: "Vue d'ensemble du second cerveau de l'utilisateur : sections, résumés, compteurs. Utilise cet outil SYSTÉMATIQUEMENT avant de répondre à toute question sur son travail, ses projets ou ses notes — même si tu penses déjà savoir. Commence toujours par ça.",
    inputSchema: { type: "object", properties: {} },
    annotations: { title: "Vue d'ensemble", readOnlyHint: true, openWorldHint: false },
  },
  {
    name: "brain_search",
    description: "Recherche des pages dans le cerveau de l'utilisateur. Renvoie les 8 meilleures avec un extrait ciblé sur ta requête (pas le début de la page). Utilise-le systématiquement dès qu'une question touche un sujet précis. Si l'extrait ne suffit pas — un montant, une date, un détail —, appelle `brain_node` avec l'id ET ta requête : ne réponds JAMAIS que l'information « n'est pas visible » sans avoir lu la page, la recherche ne renvoie qu'un aperçu.",
    inputSchema: { type: "object", properties: { query: { type: "string" } }, required: ["query"] },
    annotations: { title: "Recherche", readOnlyHint: true, openWorldHint: false },
  },
  {
    name: "brain_node",
    description: "Lit une ou PLUSIEURS pages (contenu, chemin, sous-pages) à partir de leur id. À appeler après brain_search dès qu'une page semble pertinente, pour lire son contenu réel avant de répondre. Si plusieurs pages t'intéressent, passe-les toutes d'un coup dans `node_ids` (jusqu'à 5) au lieu d'enchaîner les appels. Passe TOUJOURS `query` (les mots que tu cherches) : les pages sont alors réduites aux passages utiles au lieu de leur début — une page peut peser plusieurs milliers de mots. Utilise `full` seulement s'il te manque du contexte.",
    inputSchema: {
      type: "object",
      properties: {
        node_id: { type: "string", description: "id d'une page (ou utilise node_ids pour en lire plusieurs)" },
        node_ids: { type: "array", items: { type: "string" }, description: "plusieurs ids à lire en un seul appel (5 maximum)" },
        query: { type: "string", description: "ce que tu cherches dans ces pages (mots-clés ou question) — renvoie les passages pertinents" },
        full: { type: "boolean", description: "true = pages entières (coûteux, à réserver aux cas où les passages ne suffisent pas)" },
      },
    },
    annotations: { title: "Lire une page", readOnlyHint: true, openWorldHint: false },
  },
  {
    name: "brain_add_note",
    description: "PROPOSE une seule note sous une page existante (parent_id via brain_overview/brain_search) ou sous une proposition précédente. Pour plusieurs nœuds/sous-nœuds, préfère brain_add_tree. N'écrit jamais directement : le propriétaire valide dans Lucid.",
    inputSchema: {
      type: "object",
      properties: {
        parent_id: { type: "string" },
        label: { type: "string", description: "titre de la note" },
        content: { type: "string", description: "contenu markdown" },
      },
      required: ["parent_id", "label"],
    },
    annotations: { title: "Proposer une page", readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  {
    name: "brain_add_tree",
    description: "PROPOSE une arborescence complète de notes (nœuds + sous-nœuds, jusqu'à 8 niveaux / 60 notes) en UN appel, sous une page existante ou une proposition précédente. Chaque nœud : {label, content?, children?} — children a la même forme, récursivement. N'écrit jamais directement : le propriétaire valide dans Lucid.",
    inputSchema: {
      type: "object",
      properties: {
        parent_id: { type: "string", description: "id de la page (ou proposition) sous laquelle greffer l'arbre" },
        nodes: {
          type: "array",
          description: "arbre imbriqué : [{label, content?, children?: [même forme]}]",
          items: {
            type: "object",
            properties: {
              label: { type: "string", description: "titre de la note" },
              content: { type: "string", description: "contenu markdown" },
              children: { type: "array", description: "sous-notes, même forme récursive", items: { type: "object" } },
            },
            required: ["label"],
          },
        },
      },
      required: ["parent_id", "nodes"],
    },
    annotations: { title: "Proposer une arborescence", readOnlyHint: false, destructiveHint: false, idempotentHint: false, openWorldHint: false },
  },
  {
    name: "update_node",
    description: "PROPOSE de remplacer le contenu d'une page EXISTANTE (obsolète, à corriger) sans changer sa place dans l'arbre. Pour créer une nouvelle page, utilise brain_add_note. N'écrit jamais directement : le propriétaire valide dans Lucid (ou c'est appliqué aussitôt s'il a activé le mode autonome).",
    inputSchema: {
      type: "object",
      properties: {
        node_id: { type: "string", description: "id de la page à modifier (via brain_search/brain_overview)" },
        content: { type: "string", description: "nouveau contenu markdown, remplace l'ancien" },
      },
      required: ["node_id", "content"],
    },
    annotations: { title: "Proposer une modification", readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
  {
    name: "move_node",
    description: "PROPOSE de déplacer une page existante sous un nouveau parent (reparenter dans l'arbre). N'écrit jamais directement.",
    inputSchema: {
      type: "object",
      properties: {
        node_id: { type: "string", description: "id de la page à déplacer" },
        new_parent_id: { type: "string", description: "id du nouveau parent" },
      },
      required: ["node_id", "new_parent_id"],
    },
    annotations: { title: "Proposer un déplacement", readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
  {
    name: "merge_nodes",
    description: "PROPOSE de fusionner 2+ pages en une seule (doublons, sujets redondants). La PREMIÈRE page de node_ids survit : elle garde les enfants et le contenu concaténé des autres, qui disparaissent. N'écrit jamais directement.",
    inputSchema: {
      type: "object",
      properties: {
        node_ids: { type: "array", items: { type: "string" }, description: "2+ ids ; le premier survit" },
        label: { type: "string", description: "titre final du survivant (optionnel, garde son titre actuel sinon)" },
      },
      required: ["node_ids"],
    },
    annotations: { title: "Proposer une fusion", readOnlyHint: false, destructiveHint: true, idempotentHint: false, openWorldHint: false },
  },
  {
    name: "link_nodes",
    description: "PROPOSE un pont conceptuel entre deux pages existantes, sans les fusionner ni changer l'arbre (deux sujets liés, par exemple). N'écrit jamais directement.",
    inputSchema: {
      type: "object",
      properties: {
        node_id_a: { type: "string" },
        node_id_b: { type: "string" },
        relation: { type: "string", description: "nature du lien (optionnel, ex. \"voir aussi\")" },
      },
      required: ["node_id_a", "node_id_b"],
    },
    annotations: { title: "Proposer un lien", readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
  },
];

const CORS = {
  "Access-Control-Allow-Origin": "*",
  "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type, mcp-session-id, mcp-protocol-version",
  "Access-Control-Allow-Methods": "GET, POST, OPTIONS, DELETE",
};

function rpcResult(id: unknown, result: unknown): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, result }), {
    headers: { ...CORS, "Content-Type": "application/json" },
  });
}
function rpcError(id: unknown, code: number, message: string): Response {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id, error: { code, message } }), {
    headers: { ...CORS, "Content-Type": "application/json" },
  });
}

// Garde : permet d'importer handler + tools en Node pour les tests (pas de runtime Deno).
declare const Deno: { serve: (h: (req: Request) => Promise<Response> | Response) => void; env: { get: (k: string) => string | undefined } };

export async function handler(req: Request): Promise<Response> {
  if (req.method === "OPTIONS") return new Response(null, { headers: CORS });
  // Pas de flux SSE en v1 (stateless) : GET → 405, conforme au transport streamable HTTP.
  if (req.method !== "POST") {
    return new Response(JSON.stringify({ name: "lucid-brain-remote", hint: "POST JSON-RPC (MCP streamable HTTP)" }), {
      status: 405, headers: { ...CORS, "Content-Type": "application/json" },
    });
  }

  const mcpToken = new URL(req.url).searchParams.get("token") ?? "";
  let msg: { jsonrpc?: string; id?: unknown; method?: string; params?: Record<string, unknown> };
  try { msg = await req.json(); } catch { return rpcError(null, -32700, "JSON invalide"); }
  const { id, method, params } = msg;

  // Notifications (pas d'id) : accusé sans corps.
  if (id === undefined || method?.startsWith("notifications/")) {
    return new Response(null, { status: 202, headers: CORS });
  }

  switch (method) {
    // B1 (vue d'ensemble servie une fois dans `instructions`) a été implémenté
    // puis RETIRÉ le 2026-08-21, sur mesure : ChatGPT a continué d'appeler
    // `brain_overview` dans 4 questions sur 4, connecteur reconnecté pour forcer
    // un `initialize` neuf. La session payait donc DEUX fois — 745 tokens
    // d'instructions ignorées + 949 tokens d'appel. Ne pas réessayer sans un
    // client dont on a vérifié qu'il honore ce champ.
    case "initialize":
      return rpcResult(id, {
        protocolVersion: (params?.protocolVersion as string) ?? "2025-03-26",
        capabilities: { tools: {} },
        serverInfo: { name: "lucid-brain-remote", version: "0.1.0" },
      });
    case "ping":
      return rpcResult(id, {});
    case "tools/list":
      return rpcResult(id, { tools: TOOLS });
    case "tools/call": {
      const name = params?.name as string;
      // deno-lint-ignore no-explicit-any
      const args = (params?.arguments ?? {}) as Record<string, any>;
      try {
        if (!mcpToken) throw new Error("paramètre ?token=<token-mcp> manquant — copie l'URL MCP depuis la modale Partager de Lucid (le lien de partage public ne donne pas accès au MCP)");
        const spaceId = await spaceIdFromToken(mcpToken);
        if (name === "brain_add_note") {
          const text = await addProposal(spaceId, args.parent_id ?? "", args.label ?? "", args.content ?? "");
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        if (name === "brain_add_tree") {
          // Certains clients sérialisent l'argument array en string JSON.
          const nodes = typeof args.nodes === "string" ? JSON.parse(args.nodes) : args.nodes;
          if (!Array.isArray(nodes)) throw new Error("nodes doit être un tableau [{label, content?, children?}]");
          const text = await addTree(spaceId, args.parent_id ?? "", nodes as NoteTree[]);
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        if (name === "update_node") {
          const text = await updateNode(spaceId, args.node_id ?? "", args.content ?? "");
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        if (name === "move_node") {
          const text = await moveNode(spaceId, args.node_id ?? "", args.new_parent_id ?? "");
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        if (name === "merge_nodes") {
          const nodeIds = typeof args.node_ids === "string" ? JSON.parse(args.node_ids) : args.node_ids;
          if (!Array.isArray(nodeIds)) throw new Error("node_ids doit être un tableau d'ids");
          const text = await mergeNodes(spaceId, nodeIds as string[], args.label);
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        if (name === "link_nodes") {
          const text = await linkNodes(spaceId, args.node_id_a ?? "", args.node_id_b ?? "", args.relation);
          return rpcResult(id, { content: [{ type: "text", text }] });
        }
        const payload = await loadSpace(spaceId);
        const text =
          name === "brain_overview" ? toolOverview(payload) :
          name === "brain_search" ? toolSearch(payload, args.query ?? "") :
          name === "brain_node" ? toolNodes(payload, idList(args.node_ids ?? args.node_id), { query: args.query, full: args.full === true || args.full === "true" }) :
          (() => { throw new Error(`tool inconnu : ${name}`); })();
        return rpcResult(id, { content: [{ type: "text", text }] });
      } catch (e) {
        return rpcResult(id, { content: [{ type: "text", text: `Erreur : ${(e as Error).message}` }], isError: true });
      }
    }
    default:
      return rpcError(id, -32601, `méthode inconnue : ${method}`);
  }
}

if (typeof Deno !== "undefined" && Deno.serve) Deno.serve(handler);
