import { supabase } from "./supabase";
import { readBrainGraph } from "./api";
import type { BrainGraph, Space } from "./types";

/** Titre réservé du space "MCP personnel" (jamais affiché) — sert de clé
 *  d'upsert stable via l'index unique (owner, title) déjà utilisé par les
 *  spaces publiés classiques. */
export const PERSONAL_SPACE_TITLE = "__lucid_personal_brain__";

export interface ShareState {
  id: string;
  visibility: "public" | "private";
  allowed_emails: string[];
}

export interface ShareOptions {
  visibility: "public" | "private";
  /** Emails invités (mode privé) — normalisés en minuscules. */
  allowedEmails: string[];
  /** Opt-in explicite : embarquer le texte des sources (fichiers, conversations)
   *  dans le space publié — sinon seuls titres/résumés/notes éditées sortent. */
  includeSources?: boolean;
}

function shareUrl(id: string): string {
  const base = import.meta.env.VITE_SHARE_URL as string | undefined;
  return base ? `${base.replace(/\/+$/, "")}/?id=${id}` : id;
}

/** URL du connecteur MCP distant — construite sur le TOKEN MCP (capability
 *  séparée du lien de partage : le lien public ne donne pas accès au MCP). */
export function mcpUrl(token: string): string | null {
  const base = import.meta.env.VITE_SUPABASE_URL as string | undefined;
  return base ? `${base.replace(/\/+$/, "")}/functions/v1/lucid-mcp?token=${token}` : null;
}

/** Récupère (ou crée) le token MCP d'un space publié. Nul si la table n'existe
 *  pas encore (SQL pas appliqué) — l'UI masque alors la ligne MCP. */
export async function ensureMcpToken(spaceRowId: string): Promise<string | null> {
  try {
    const { data } = await supabase!
      .from("space_mcp_tokens").select("token").eq("space_id", spaceRowId).maybeSingle();
    if (data?.token) return data.token as string;
    const { data: created, error } = await supabase!
      .from("space_mcp_tokens").insert({ space_id: spaceRowId }).select("token").single();
    if (error) return null;
    return created.token as string;
  } catch {
    return null;
  }
}

async function uid(): Promise<string> {
  if (!supabase) throw new Error("Supabase non configuré (.env).");
  const { data } = await supabase.auth.getSession();
  const id = data.session?.user.id;
  if (!id) throw new Error("Connecte-toi pour publier.");
  return id;
}

/** État de partage actuel du space (null = jamais publié). */
export async function fetchShareState(space: Space): Promise<(ShareState & { url: string }) | null> {
  const owner = await uid();
  const { data, error } = await supabase!
    .from("shared_spaces")
    .select("id, visibility, allowed_emails")
    .eq("owner", owner)
    .eq("title", space.name)
    .maybeSingle();
  if (error) throw new Error(error.message);
  return data ? { ...(data as ShareState), url: shareUrl(data.id) } : null;
}

/** Publie (ou met à jour) un space. Par défaut n'embarque QUE l'essentiel :
 *  labels, résumés, contenus édités. Le texte des sources ne sort que sur
 *  opt-in explicite (`includeSources`) — jamais la provenance. */
export async function publishSpace(
  space: Space,
  subgraph: BrainGraph,
  opts: ShareOptions,
): Promise<ShareState & { url: string }> {
  const owner = await uid();
  const payload = {
    title: space.name,
    nodes: subgraph.nodes.map((n) => ({
      id: n.id, label: n.label, kind: n.kind, weight: n.weight,
      summary: n.summary, keywords: n.keywords, parent_id: n.parent_id ?? null,
      date: n.date ?? null,
      content: n.content || (opts.includeSources ? n.source_text ?? "" : ""),
    })),
    edges: subgraph.edges.filter((e) => e.kind === "contains" || e.kind === "link"),
  };

  const { data, error } = await supabase!
    .from("shared_spaces")
    .upsert(
      {
        owner,
        title: space.name,
        data: payload,
        visibility: opts.visibility,
        allowed_emails: opts.allowedEmails.map((e) => e.trim().toLowerCase()).filter(Boolean),
        updated_at: new Date().toISOString(),
      },
      { onConflict: "owner,title" },
    )
    .select("id, visibility, allowed_emails")
    .single();
  if (error) throw new Error(error.message);
  return { ...(data as ShareState), url: shareUrl(data.id) };
}

/** Le space "MCP personnel" : TOUT le cerveau, tenu à jour à chaque sync cloud
 *  (pas un choix éditorial de l'utilisateur, contrairement à un space publié
 *  classique) — pour que Claude Desktop/Code (locaux) et claude.ai (distant)
 *  se connectent au même serveur MCP avec les mêmes fonctionnalités (décision
 *  2026-07-21). Toujours avec le texte des sources : c'est le compte du
 *  propriétaire, pas un partage vers un tiers — pas d'opt-in à demander.
 *  Best-effort : n'importe quelle erreur ne doit jamais bloquer la sync cloud
 *  elle-même (appelant : `push()` dans sync.ts).
 */
// Même plafond que la lecture d'une page côté MCP (`brain_node`/`toolNode`,
// 24 000 caractères ≈ 6 300 tokens) — un cerveau avec des PDF entiers embarqués
// peut peser des dizaines de Mo si on envoie chaque contenu intégralement ;
// sans ce plafond, l'upload (JSON.stringify + requête HTTP d'un coup dans la
// fenêtre) pouvait faire planter l'app (bug remonté par Liam le 2026-07-21).
const MAX_NODE_CONTENT = 24_000;
function truncateForSync(s: string): string {
  return s.length <= MAX_NODE_CONTENT ? s : s.slice(0, MAX_NODE_CONTENT) + "\n\n*[… tronqué]*";
}

export async function ensurePersonalMcpSpace(): Promise<void> {
  if (!supabase) return;
  const owner = await uid();
  const graph = await readBrainGraph();
  if (!graph) return;
  const payload = {
    title: PERSONAL_SPACE_TITLE,
    nodes: graph.nodes.map((n) => ({
      id: n.id, label: n.label, kind: n.kind, weight: n.weight,
      summary: n.summary, keywords: n.keywords, parent_id: n.parent_id ?? null,
      date: n.date ?? null,
      content: truncateForSync(n.content || n.source_text || ""),
    })),
    edges: graph.edges.filter((e) => e.kind === "contains" || e.kind === "link"),
  };
  const { error } = await supabase
    .from("shared_spaces")
    .upsert(
      { owner, title: PERSONAL_SPACE_TITLE, data: payload, visibility: "personal", updated_at: new Date().toISOString() },
      { onConflict: "owner,title" },
    );
  if (error) throw new Error(error.message);
}

/** URL MCP personnelle (crée le space + son token au besoin). Nul si Supabase
 *  n'est pas configuré ou si la migration SQL n'est pas encore appliquée. */
export async function ensurePersonalMcpUrl(): Promise<string | null> {
  if (!supabase) return null;
  try {
    await ensurePersonalMcpSpace();
    const owner = await uid();
    const { data } = await supabase
      .from("shared_spaces").select("id")
      .eq("owner", owner).eq("title", PERSONAL_SPACE_TITLE).maybeSingle();
    if (!data) return null;
    const token = await ensureMcpToken(data.id as string);
    return token ? mcpUrl(token) : null;
  } catch {
    return null;
  }
}

export interface SharedWithMe {
  id: string;
  title: string;
  updated_at: string;
}

/** Spaces que d'autres ont partagés avec moi (mon email dans allowed_emails). */
export async function fetchSharedWithMe(): Promise<SharedWithMe[]> {
  if (!supabase) return [];
  const { data: sess } = await supabase.auth.getSession();
  const email = sess.session?.user.email?.toLowerCase();
  const me = sess.session?.user.id;
  if (!email || !me) return [];
  const { data, error } = await supabase
    .from("shared_spaces")
    .select("id, title, updated_at")
    .contains("allowed_emails", [email])
    .neq("owner", me)
    .order("updated_at", { ascending: false });
  if (error) throw new Error(error.message);
  return (data ?? []) as SharedWithMe[];
}

/** Charge le contenu d'un space partagé (le RLS vérifie l'accès). */
export async function fetchSharedSpace(id: string): Promise<{ title: string; updated_at: string; data: never }> {
  if (!supabase) throw new Error("Supabase non configuré.");
  const { data, error } = await supabase
    .from("shared_spaces")
    .select("title, data, updated_at")
    .eq("id", id)
    .maybeSingle();
  if (error) throw new Error(error.message);
  if (!data) throw new Error("Ce space n'est plus partagé avec toi.");
  return data as { title: string; updated_at: string; data: never };
}

/** URL navigateur d'un space partagé. */
export function sharedSpaceUrl(id: string): string {
  return shareUrl(id);
}

/** Retire le space du web (supprime la ligne — le lien meurt). */
export async function unpublishSpace(space: Space): Promise<void> {
  const owner = await uid();
  const { error } = await supabase!
    .from("shared_spaces")
    .delete()
    .eq("owner", owner)
    .eq("title", space.name);
  if (error) throw new Error(error.message);
}
