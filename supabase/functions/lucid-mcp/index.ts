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

// ── Chargement du space (RLS : la clé anon ne voit que les spaces publics) ────

function env(k: string): string | undefined {
  // Edge runtime = Deno.env ; tests locaux (tsx/node) = process.env.
  return typeof Deno !== "undefined" && Deno.env
    ? Deno.env.get(k)
    : (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env[k];
}

function anonHeaders(): Record<string, string> {
  const anon = env("SUPABASE_ANON_KEY")!;
  return { apikey: anon, Authorization: `Bearer ${anon}` };
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
  const r = await fetch(
    `${base}/rest/v1/shared_spaces?id=eq.${encodeURIComponent(spaceId)}&select=title,data,visibility`,
    { headers: anonHeaders() },
  );
  if (!r.ok) throw new Error(`stockage indisponible (${r.status})`);
  const rows = await r.json();
  if (!rows.length) throw new Error("space introuvable ou privé — seuls les spaces publics sont accessibles via le MCP distant (v1)");
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

/// Vérifie que `parentId` est une page du space OU une proposition déjà déposée.
async function assertParent(spaceId: string, parentId: string): Promise<void> {
  const payload = await loadSpace(spaceId); // vérifie aussi que le space est public
  if (!payload.nodes.some((n) => n.id === parentId) && !(await proposalExists(parentId, spaceId))) {
    throw new Error(`parent \`${parentId}\` introuvable — utilise brain_overview/brain_search pour l'id d'une page existante, ou l'id renvoyé par un dépôt précédent pour créer une sous-page`);
  }
}

async function insertProposals(rows: ProposalRow[]): Promise<void> {
  // Uuids générés côté function : pas de RETURNING (la RLS interdit — à raison —
  // la relecture des propositions à l'anon, et PostgREST exige un droit select
  // pour representation).
  const r = await fetch(`${env("SUPABASE_URL")}/rest/v1/mcp_proposals`, {
    method: "POST",
    headers: { ...anonHeaders(), "content-type": "application/json", Prefer: "return=minimal" },
    body: JSON.stringify(rows),
  });
  if (!r.ok) throw new Error(`dépôt refusé (${r.status}) — le space est-il toujours public ?`);
}

/// Dépose une PROPOSITION de note (jamais d'écriture directe) : elle transite
/// par la table mcp_proposals, l'app du propriétaire la rapatrie dans son
/// circuit local de validation (bulles fantômes), puis la supprime.
async function addProposal(spaceId: string, parentId: string, label: string, content: string): Promise<string> {
  if (!label.trim()) throw new Error("label vide");
  await assertParent(spaceId, parentId);
  const proposalId = crypto.randomUUID();
  await insertProposals([{ id: proposalId, space_id: spaceId, parent_id: parentId, label: label.trim(), content }]);
  return `Proposition \`${proposalId}\` déposée. Elle apparaîtra dans l'app Lucid du propriétaire (bulle en attente) et sera visible dans le cerveau seulement s'il l'accepte. Pour proposer une sous-page de celle-ci, rappelle brain_add_note avec parent_id="${proposalId}".`;
}

// ── Arbre de propositions (plusieurs nœuds/sous-nœuds en UN appel) ────────────

type NoteTree = { label: string; content?: string; children?: NoteTree[] };
type ProposalRow = { id: string; space_id: string; parent_id: string; label: string; content: string };

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
      rows.push({ id, space_id: spaceId, parent_id: parent, label, content });
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
  return `${rows.length} proposition(s) déposée(s) sous \`${parentId}\` :\n${outline}\nElles apparaîtront dans l'app Lucid du propriétaire (bulles en attente) — accepter une note accepte automatiquement ses parents. Pour prolonger une branche, rappelle brain_add_tree (ou brain_add_note) avec l'id de la note concernée.`;
}

// ── Protocole MCP (JSON-RPC 2.0 sur POST, stateless) ─────────────────────────

const TOOLS = [
  {
    name: "brain_overview",
    description: "Vue d'ensemble du second cerveau partagé : sections, résumés, compteurs. Commence toujours par ça.",
    inputSchema: { type: "object", properties: {} },
  },
  {
    name: "brain_search",
    description: "Recherche des pages par mots-clés dans le cerveau partagé. Renvoie les 8 meilleures avec extraits.",
    inputSchema: { type: "object", properties: { query: { type: "string" } }, required: ["query"] },
  },
  {
    name: "brain_node",
    description: "Lit une page complète (contenu, chemin, sous-pages) à partir de son id.",
    inputSchema: { type: "object", properties: { node_id: { type: "string" } }, required: ["node_id"] },
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
