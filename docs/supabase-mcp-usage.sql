-- Relevé d'activité — compteurs d'économie du MCP.
-- Cf. coffre LucidFlow, « Relevé d'activité (valeur mesurée du cerveau) » (2026-08-25)
-- et ADR-0025. À exécuter dans Supabase → SQL Editor, après supabase-mcp-personal.sql.
--
-- Ce qu'on stocke : des COMPTEURS, agrégés par jour et par outil.
-- Ce qu'on ne stocke JAMAIS : le texte de la requête, le contenu servi, le
-- libellé des pages consultées. La ligne à ne pas franchir est dans la note.

create table if not exists public.mcp_usage (
  space_id     uuid   not null references public.shared_spaces(id) on delete cascade,
  day          date   not null,
  tool         text   not null,
  calls        integer not null default 0,
  -- `chars_raw`    : ce qu'aurait coûté le collage brut des pages candidates.
  -- `chars_served` : ce que le MCP a réellement renvoyé.
  -- L'économie est une soustraction, pas une estimation.
  chars_raw    bigint not null default 0,
  chars_served bigint not null default 0,
  primary key (space_id, day, tool)
);

alter table public.mcp_usage enable row level security;

-- Lecture : le propriétaire du space, et lui seul. Aucune policy d'écriture —
-- seule l'edge function écrit, avec la clé service (qui contourne la RLS).
drop policy if exists mcp_usage_owner_select on public.mcp_usage;
create policy mcp_usage_owner_select on public.mcp_usage
  for select using (
    exists (
      select 1 from public.shared_spaces s
      where s.id = public.mcp_usage.space_id and s.owner = auth.uid()
    )
  );

-- Un seul aller-retour par appel MCP : upsert additif sur la ligne du jour.
create or replace function public.mcp_usage_record(
  p_space_id uuid,
  p_tool     text,
  p_raw      bigint,
  p_served   bigint
) returns void
language sql
security definer
set search_path = public
as $$
  insert into public.mcp_usage (space_id, day, tool, calls, chars_raw, chars_served)
  values (p_space_id, (now() at time zone 'utc')::date, p_tool, 1, greatest(p_raw, 0), greatest(p_served, 0))
  on conflict (space_id, day, tool) do update
    set calls        = public.mcp_usage.calls + 1,
        chars_raw    = public.mcp_usage.chars_raw + excluded.chars_raw,
        chars_served = public.mcp_usage.chars_served + excluded.chars_served;
$$;

revoke all on function public.mcp_usage_record(uuid, text, bigint, bigint) from public, anon, authenticated;
