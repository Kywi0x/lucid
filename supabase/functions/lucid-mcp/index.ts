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
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) throw new Error("query vide");
  const byId = new Map(p.nodes.map((n) => [n.id, n]));
  const scored = p.nodes.flatMap((n) => {
    const label = n.label.toLowerCase();
    const kw = (n.keywords ?? []).join(" ").toLowerCase();
    const summary = (n.summary ?? "").toLowerCase();
    const content = (n.content ?? "").toLowerCase();
    let score = 0;
    for (const t of terms) {
      if (label.includes(t)) score += 5;
      if (kw.includes(t)) score += 3;
      if (summary.includes(t)) score += 2;
      if (content.includes(t)) score += 1;
    }
    return score > 0 ? [{ score, n }] : [];
  }).sort((a, b) => b.score - a.score);

  if (!scored.length) return `Aucun résultat pour « ${query} ».`;
  let out = `Résultats pour « ${query} » :\n`;
  for (const { n } of scored.slice(0, 8)) {
    const excerpt = truncate(n.summary || n.content || "", 200).replace(/\n/g, " ");
    out += `\n- **${n.label}** (\`${n.id}\`, ${n.kind}) — ${pathOf(n, byId) || "racine"}\n  ${excerpt}\n`;
  }
  out += "\nLis une page complète avec `brain_node`.\n";
  return out;
}

export function toolNode(p: Payload, nodeId: string): string {
  const byId = new Map(p.nodes.map((n) => [n.id, n]));
  const n = byId.get(nodeId);
  if (!n) throw new Error(`nœud \`${nodeId}\` introuvable (utilise brain_search pour trouver un id)`);
  let out = `# ${n.label}\n\n- id : \`${n.id}\` · type : ${n.kind}\n`;
  const path = pathOf(n, byId);
  if (path) out += `- chemin : ${path}\n`;
  if (n.keywords?.length) out += `- mots-clés : ${n.keywords.join(", ")}\n`;
  if (n.summary) out += `\n## Résumé\n${n.summary}\n`;
  if (n.content) out += `\n## Contenu\n${truncate(n.content, 24_000)}\n`;
  const kids = p.nodes.filter((c) => c.parent_id === n.id);
  if (kids.length) {
    out += "\n## Sous-pages\n";
    for (const c of kids) out += `- ${c.label} (\`${c.id}\`)\n`;
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
    description: "Recherche des pages par mots-clés dans le cerveau de l'utilisateur. Renvoie les 8 meilleures avec extraits. Utilise-le systématiquement dès qu'une question touche un sujet précis, avant de répondre depuis ta seule mémoire.",
    inputSchema: { type: "object", properties: { query: { type: "string" } }, required: ["query"] },
    annotations: { title: "Recherche", readOnlyHint: true, openWorldHint: false },
  },
  {
    name: "brain_node",
    description: "Lit une page complète (contenu, chemin, sous-pages) à partir de son id. À appeler après brain_overview/brain_search dès qu'une page semble pertinente, pour lire son contenu réel avant de répondre.",
    inputSchema: { type: "object", properties: { node_id: { type: "string" } }, required: ["node_id"] },
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
          name === "brain_node" ? toolNode(payload, args.node_id ?? "") :
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
