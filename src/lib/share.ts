import { supabase } from "./supabase";
import type { BrainGraph, Space } from "./types";

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
