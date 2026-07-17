-- MCP distant : tokens d'accès + propositions — à exécuter dans Supabase → SQL Editor.
--
-- Deux capabilities distinctes :
--   · lien de partage (uuid du space)  → LECTURE seule (viewer web)
--   · token MCP (table ci-dessous)     → lecture + PROPOSITION via le connecteur
-- Le token n'est lisible que par le propriétaire (et l'edge function via la clé
-- service). Une proposition ne modifie jamais rien : l'app du propriétaire la
-- rapatrie dans son circuit local mcp_pending/ (bulles fantômes) et la supprime.

-- ── Token MCP par space (capability séparée du lien public) ──
create table if not exists public.space_mcp_tokens (
  token      uuid primary key default gen_random_uuid(),
  space_id   uuid not null unique references public.shared_spaces(id) on delete cascade,
  created_at timestamptz not null default now()
);

alter table public.space_mcp_tokens enable row level security;

-- Seul le propriétaire du space voit / crée / révoque son token.
-- (L'edge function le lit avec la clé service, qui contourne la RLS —
--  jamais la clé anon : le viewer ne peut PAS lire le token.)
drop policy if exists "owner lit son token" on public.space_mcp_tokens;
create policy "owner lit son token"
  on public.space_mcp_tokens for select using (
    exists (select 1 from public.shared_spaces s where s.id = space_id and s.owner = auth.uid())
  );
drop policy if exists "owner cree son token" on public.space_mcp_tokens;
create policy "owner cree son token"
  on public.space_mcp_tokens for insert with check (
    exists (select 1 from public.shared_spaces s where s.id = space_id and s.owner = auth.uid())
  );
drop policy if exists "owner revoque son token" on public.space_mcp_tokens;
create policy "owner revoque son token"
  on public.space_mcp_tokens for delete using (
    exists (select 1 from public.shared_spaces s where s.id = space_id and s.owner = auth.uid())
  );

-- ── Propositions ──

create table if not exists public.mcp_proposals (
  id         uuid primary key default gen_random_uuid(),
  space_id   uuid not null references public.shared_spaces(id) on delete cascade,
  parent_id  text not null,
  label      text not null check (char_length(label) between 1 and 200),
  content    text not null default '' check (char_length(content) <= 100000),
  created_at timestamptz not null default now()
);

alter table public.mcp_proposals enable row level security;

-- Déposer : quiconque a le lien d'un space PUBLIC (l'uuid = la capability,
-- même niveau d'accès que la lecture). Bornes anti-abus via les checks ci-dessus.
drop policy if exists "proposer sur un space public" on public.mcp_proposals;
create policy "proposer sur un space public"
  on public.mcp_proposals for insert
  with check (
    exists (
      select 1 from public.shared_spaces s
      where s.id = space_id and s.visibility = 'public'
    )
  );

-- Lire / supprimer : uniquement le propriétaire du space (l'app connectée).
drop policy if exists "le proprietaire lit ses propositions" on public.mcp_proposals;
create policy "le proprietaire lit ses propositions"
  on public.mcp_proposals for select using (
    exists (
      select 1 from public.shared_spaces s
      where s.id = space_id and s.owner = auth.uid()
    )
  );
drop policy if exists "le proprietaire supprime ses propositions" on public.mcp_proposals;
create policy "le proprietaire supprime ses propositions"
  on public.mcp_proposals for delete using (
    exists (
      select 1 from public.shared_spaces s
      where s.id = space_id and s.owner = auth.uid()
    )
  );
