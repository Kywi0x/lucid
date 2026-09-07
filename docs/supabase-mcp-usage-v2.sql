-- Relevé d'activité v2 — compter les ÉCHANGES, pas les appels.
-- Cf. ADR-0025 et coffre LucidFlow « Relevé d'activité ». À exécuter après
-- supabase-mcp-usage.sql. N'exige AUCUN redéploiement de l'edge function :
-- la signature de mcp_usage_record ne change pas.
--
-- Problème mesuré (Liam, 2026-09-05) : une seule question produit 1 brain_search
-- + N brain_node. Compter les appels gonfle l'activité d'un facteur ~4 et rend
-- la grille illisible. Un « échange » approche la question réellement posée.
--
-- Méthode : repli par inactivité, comme prévu dans la note. Le MCP en HTTP est
-- SANS ÉTAT — un `tools/call` arrive nu. On regroupe donc les appels d'un même
-- space séparés de moins de GAP. C'est une HEURISTIQUE, pas une frontière
-- exacte : l'UI doit le dire (« regroupé par inactivité »), jamais laisser
-- croire à une mesure exacte.

alter table public.mcp_usage
  add column if not exists exchanges integer not null default 0;

comment on column public.mcp_usage.exchanges is
  'Rafales d''appels séparées de plus de 2 min — approximation d''une question posée, pas une mesure exacte.';

-- Le dernier appel se suit par SPACE, pas par (space, jour, outil) : une même
-- question traverse plusieurs outils et peut franchir minuit.
create table if not exists public.mcp_last_call (
  space_id uuid primary key references public.shared_spaces(id) on delete cascade,
  at       timestamptz not null
);

alter table public.mcp_last_call enable row level security;
-- Aucune policy : seule l'edge function y touche, via la clé service.

create or replace function public.mcp_usage_record(
  p_space_id uuid,
  p_tool     text,
  p_raw      bigint,
  p_served   bigint
) returns void
language plpgsql
security definer
set search_path = public
as $$
declare
  -- Une question tient en quelques secondes d'appels ; l'utilisateur met au
  -- moins le temps de lire avant la suivante. 2 min sépare les deux sans
  -- découper une rafale légitime.
  gap      constant interval := interval '2 minutes';
  last_at  timestamptz;
  is_new   integer;
begin
  select at into last_at from public.mcp_last_call where space_id = p_space_id for update;
  is_new := case when last_at is null or now() - last_at > gap then 1 else 0 end;

  insert into public.mcp_last_call (space_id, at) values (p_space_id, now())
    on conflict (space_id) do update set at = now();

  insert into public.mcp_usage (space_id, day, tool, calls, exchanges, chars_raw, chars_served)
  values (
    p_space_id, (now() at time zone 'utc')::date, p_tool,
    1, is_new, greatest(p_raw, 0), greatest(p_served, 0)
  )
  on conflict (space_id, day, tool) do update
    set calls        = public.mcp_usage.calls + 1,
        exchanges    = public.mcp_usage.exchanges + excluded.exchanges,
        chars_raw    = public.mcp_usage.chars_raw + excluded.chars_raw,
        chars_served = public.mcp_usage.chars_served + excluded.chars_served;
end;
$$;

revoke all on function public.mcp_usage_record(uuid, text, bigint, bigint) from public, anon, authenticated;
