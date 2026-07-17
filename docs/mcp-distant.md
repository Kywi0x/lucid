# MCP distant — le cerveau accessible à toutes les IA

Un space **publié en public** devient interrogeable par n'importe quel client MCP
(claude.ai web/mobile, ChatGPT, agents…) via une edge function Supabase, avec les
mêmes 3 tools que le MCP local (`brain_overview`, `brain_search`, `brain_node`).

Deux capabilities distinctes (décision Liam 2026-07-16) :
- **lien de partage** (`?id=<uuid du space>`) → viewer web, **lecture seule** ;
- **URL MCP** (`?token=<token>`) → lecture + `brain_add_note` (propositions à valider).
  Le token vit dans `space_mcp_tokens`, lisible par le propriétaire seul ; le lien
  public ne permet PAS de le déduire.

```
Client MCP ──POST JSON-RPC──▶ /functions/v1/lucid-mcp?token=<token>
                                   │  clé service : token → space_id
                                   │  clé anon + RLS : space public uniquement
                                   ▼
                    shared_spaces.data (lecture) · mcp_proposals (propositions)
                                   ▼ polling 10 s par l'app du propriétaire
                    mcp_pending/ local → bulles fantômes → Accepter / Refuser
```

## Déployer (une fois)

```bash
npm i -g supabase          # ou brew install supabase/tap/supabase
supabase login
supabase link --project-ref <ref-du-projet>   # le ref de VITE_SUPABASE_URL
supabase functions deploy lucid-mcp --no-verify-jwt
```

`--no-verify-jwt` est nécessaire : les clients MCP n'envoient pas de JWT Supabase.
La sécurité vient de la RLS (`lecture selon visibilite`) : la clé anon utilisée par
la function ne peut lire que les spaces `visibility = 'public'`. Un space privé ou
dépublié → « introuvable ».

## Utiliser

1. Dans Lucid : Partager un space → **Public** → cocher **« Inclure le texte des
   sources »** (sinon les IA ne voient que titres/résumés) → Publier.
2. Copier l'**URL MCP** affichée dans la modale
   (`https://<ref>.supabase.co/functions/v1/lucid-mcp?space=<uuid>`).
3. claude.ai → Settings → Connectors → **Add custom connector** → coller l'URL.
   (ChatGPT : Settings → Connectors, même principe.)

## Tester en curl

```bash
URL="https://<ref>.supabase.co/functions/v1/lucid-mcp?space=<uuid>"
# initialize
curl -s "$URL" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}'
# tools/list
curl -s "$URL" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
# overview
curl -s "$URL" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"brain_overview","arguments":{}}}'
```

## Prérequis SQL

Exécuter `docs/supabase-mcp-proposals.sql` dans le SQL Editor (tables
`space_mcp_tokens` + `mcp_proposals` et leurs policies).

## Limites v1 (assumées)

- **Spaces publics uniquement** côté lecture (le token sépare les usages, il ne rend
  pas un space privé lisible) — accès privé complet en v2.
- **Écriture = propositions seulement** : `brain_add_note` dépose dans `mcp_proposals`,
  l'app du propriétaire rapatrie (polling 10 s) et l'utilisateur valide dans Lucid.
  Pas de chaînage de propositions en distant (le parent doit exister dans le space).
- **Collaboration** : quelqu'un qui veut travailler sur un space partagé le copie dans
  SON cerveau et génère SON propre MCP — on ne modifie jamais le cerveau d'autrui.
- **Stateless** : pas de flux SSE (GET → 405), conforme au transport streamable HTTP ;
  tous les clients récents s'en contentent.
- Le contenu servi est **exactement** ce qui a été publié (snapshot au moment du
  « Publier ») — republier pour rafraîchir, comme pour le viewer web.
