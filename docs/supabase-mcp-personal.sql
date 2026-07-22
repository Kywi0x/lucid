-- MCP personnel : un space spécial par compte qui représente TOUT le cerveau
-- (pas un sous-ensemble choisi), pour que Claude Desktop/Code en local et
-- claude.ai à distance se connectent au MÊME serveur MCP avec exactement les
-- mêmes fonctionnalités (décision Liam/Lucas, 2026-07-21). Réutilise
-- `shared_spaces` (nouvelle valeur de `visibility`) et `space_mcp_tokens`
-- (table + policies) tels quels — aucun nouveau concept de token.
--
-- À exécuter dans Supabase → SQL Editor, après supabase-shared-spaces-v2.sql.
-- Si le nom de contrainte ci-dessous ne correspond pas à celui généré chez toi
-- (vérifiable via \d shared_spaces ou l'onglet Table Editor → Constraints),
-- adapte le nom avant de lancer.

alter table public.shared_spaces drop constraint if exists shared_spaces_visibility_check;
alter table public.shared_spaces add constraint shared_spaces_visibility_check
  check (visibility in ('public', 'private', 'personal'));

-- Le space personnel est identifié par un TITRE réservé, jamais montré à
-- l'utilisateur : il sert de clé d'upsert stable (avec l'index unique
-- (owner, title) déjà existant) pour que la sync cloud mette à jour LE même
-- space à chaque fois plutôt que d'en créer un nouveau à chaque cycle.
comment on column public.shared_spaces.title is
  'Titre affiché — sauf pour le space personnel MCP, dont le titre réservé "__lucid_personal_brain__" ne s''affiche jamais (juste une clé d''upsert stable).';

-- Le space personnel n'a pas à apparaître dans le picker "Partager" existant
-- ni dans les vues qui listent les spaces de l'utilisateur.
