-- ─── Lucid — setup Supabase (sauvegarde cloud) ─────────────────────────────
-- À coller dans le SQL Editor du dashboard Supabase (Run).
-- Crée le bucket privé `backups` + les policies RLS : chaque utilisateur
-- authentifié ne peut lire/écrire QUE dans son dossier {user_id}/...

insert into storage.buckets (id, name, public)
values ('backups', 'backups', false)
on conflict (id) do nothing;

create policy "backups_select_own"
on storage.objects for select to authenticated
using (bucket_id = 'backups' and (storage.foldername(name))[1] = auth.uid()::text);

create policy "backups_insert_own"
on storage.objects for insert to authenticated
with check (bucket_id = 'backups' and (storage.foldername(name))[1] = auth.uid()::text);

create policy "backups_delete_own"
on storage.objects for delete to authenticated
using (bucket_id = 'backups' and (storage.foldername(name))[1] = auth.uid()::text);
