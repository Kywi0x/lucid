-- Active Supabase Realtime sur mcp_proposals : le front s'abonne aux nouvelles
-- lignes (INSERT) au lieu de sonder la table toutes les 10s en continu — même
-- mécanisme déjà utilisé pour la sync cloud (shared_spaces). Coût réseau
-- continu remonté par Liam le 2026-07-21.
--
-- À exécuter dans Supabase → SQL Editor, après supabase-mcp-proposals-v2.sql.

alter publication supabase_realtime add table public.mcp_proposals;
