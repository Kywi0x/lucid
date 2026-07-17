-- ─── Lucid — sync auto du cerveau (2026-07-17) ─────────────────────────────
-- À coller dans le SQL Editor du dashboard Supabase (Run), APRÈS supabase-setup.sql.
-- La sync réécrit `{user_id}/sync.zip` à chaque push → il faut la policy UPDATE
-- (le setup initial n'avait que select/insert/delete pour les backups datés).

create policy "backups_update_own"
on storage.objects for update to authenticated
using (bucket_id = 'backups' and (storage.foldername(name))[1] = auth.uid()::text)
with check (bucket_id = 'backups' and (storage.foldername(name))[1] = auth.uid()::text);
