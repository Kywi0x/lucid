-- mcp_proposals généralisé : 5 formes d'action (create/update/move/merge/link)
-- au lieu d'une seule ("propose une note") — un seul circuit de propositions,
-- quel que soit le type d'action (décision 2026-07-21, plan Archiviste étape 2).
--
-- À exécuter dans Supabase → SQL Editor, après supabase-mcp-proposals.sql.

-- parent_id/label n'ont de sens que pour "create" (et label pour "merge",
-- optionnel) — plus jamais NOT NULL pour couvrir les 4 autres actions.
alter table public.mcp_proposals
  alter column parent_id drop not null,
  alter column label drop not null,
  add column if not exists action text not null default 'create',
  add column if not exists target_id text,
  add column if not exists new_parent_id text,
  add column if not exists merge_ids text[],
  add column if not exists link_target text,
  add column if not exists relation text;

alter table public.mcp_proposals drop constraint if exists mcp_proposals_label_check;
alter table public.mcp_proposals add constraint mcp_proposals_label_check
  check (label is null or char_length(label) <= 200);

-- Note : la policy insert "proposer sur un space public" (visibility='public')
-- n'est plus le point d'application — la fonction distante écrit désormais
-- avec la clé service (déjà validée en amont par le token MCP). Elle reste en
-- place sans effet, pas la peine d'y toucher.
