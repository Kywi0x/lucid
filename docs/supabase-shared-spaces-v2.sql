-- Partage v2 : visibilité public/privé + invitations par email.
-- À exécuter dans Supabase → SQL Editor (après supabase-shared-spaces.sql).

alter table public.shared_spaces
  add column if not exists visibility text not null default 'public'
    check (visibility in ('public', 'private')),
  add column if not exists allowed_emails text[] not null default '{}';

-- Lecture : public = tout le monde ; privé = propriétaire ou email invité.
-- L'invitation ne vient QUE du propriétaire (colonne modifiable par lui seul,
-- via la policy update existante) — pas d'auto-invitation possible.
drop policy if exists "lecture publique par lien" on public.shared_spaces;
create policy "lecture selon visibilite"
  on public.shared_spaces for select using (
    visibility = 'public'
    or auth.uid() = owner
    or lower(coalesce(auth.jwt() ->> 'email', '')) = any (allowed_emails)
  );
