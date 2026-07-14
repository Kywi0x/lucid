-- Partage public de spaces (v1) — à exécuter dans Supabase → SQL Editor.
-- Lecture publique par lien (l'id uuid fait office de capability),
-- écriture réservée au propriétaire authentifié.

create table if not exists public.shared_spaces (
  id         uuid primary key default gen_random_uuid(),
  owner      uuid not null references auth.users(id) on delete cascade,
  title      text not null,
  data       jsonb not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

-- Republier le même space le met à jour au lieu de créer un doublon.
create unique index if not exists shared_spaces_owner_title on public.shared_spaces (owner, title);

alter table public.shared_spaces enable row level security;

create policy "lecture publique par lien"
  on public.shared_spaces for select using (true);

create policy "publier son propre space"
  on public.shared_spaces for insert with check (auth.uid() = owner);

create policy "mettre a jour son space"
  on public.shared_spaces for update using (auth.uid() = owner);

create policy "supprimer son space"
  on public.shared_spaces for delete using (auth.uid() = owner);
