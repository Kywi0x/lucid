# CLAUDE.md — Lucid

> Contexte pour reprendre le dev en session terminal. Vision produit complète dans
> [`second-brain-project.md`](second-brain-project.md). Ce fichier = l'opérationnel.

## C'est quoi

App desktop **Tauri 2 (Rust + React/TS)** qui agrège les conversations de tes outils IA
(Claude Code aujourd'hui) et génère, **100% en local**, un **graphe de connaissances** affiché
en **mind map à bulles** (façon Obsidian) + un `brain.md`. L'IA tourne en local (llama.cpp + Gemma 3 4B).
Positionnement **local-first** (décision Liam 2026-07-17, ex-« 100 % local ») : l'analyse reste sur la
machine, mais le cerveau (résultat, ~2 Mo) se synchronise entre appareils via le compte Supabase
(`src/lib/sync.ts`, bucket `backups`, `users/<uuid>/sync.zip`, last-write-wins).

Principe clé : le **pipeline est agnostique de la source**. Un connecteur produit des
`Conversation` normalisées ; le graphe, le drill-down et les ponts inter-sources marchent tout seuls.

> ✅ **Naming tranché (2026-07-20)** : le produit s'appelle **Lucid** partout — header,
> identifiant Tauri (`com.lucidflow.lucid`), dossier de données. `brainlink` ne subsiste
> que comme nom interne du crate/package Rust (`Cargo.toml`, invisible à l'utilisateur).
> « Second Brain » est l'ancien nom produit, à ne plus utiliser.

## Stack

- **Desktop** : Tauri 2 · **Front** : React 19 + TS + Vite 7 · **Style** : Tailwind v4 (`@import "tailwindcss"`)
  + composants maison (pas de shadcn CLI) · **Graphe** : `@xyflow/react` (React Flow) + `d3-force` · `react-markdown`
- **IA locale** : `llama.cpp` (binaire **`llama-completion`**, PAS `llama-cli` qui est devenu interactif) + **Gemma 3 4B Q4_K_M**
- **Thème** : variables CSS `--color-*` dans `src/index.css`, bascule dark/light native (`prefers-color-scheme`) + override `[data-theme]`

## Lancer / builder (⚠️ gotchas)

```bash
# Toujours charger l'env Rust + Homebrew (cmake) avant cargo/tauri :
. "$HOME/.cargo/env"; export PATH="/opt/homebrew/bin:$PATH"

npm install                     # déps front (déjà fait)
npm run tauri dev               # lance l'app (front sur port 1420)
npx tsc --noEmit                # typecheck front
(cd src-tauri && cargo test --lib)   # tests Rust (9 tests)
```

- **Port 1420 occupé** (« Port 1420 is already in use ») → tuer l'instance restée ouverte :
  `lsof -ti:1420 | xargs kill -9` puis relancer.
- **Lock `target/`** : ne pas lancer `cargo test`/`cargo run` **pendant** que `tauri dev` tourne (les deux cargo se bloquent). Stopper le dev d'abord.

## IA locale — chemins & régénération

- Binaire : `~/Library/Application Support/com.lucidflow.lucid/llama.cpp/build/bin/llama-completion`
- Modèle : `~/Library/Application Support/com.lucidflow.lucid/models/gemma-3-4b-it-Q4_K_M.gguf` (~2.3 Go)
- Override possible : env `SECOND_BRAIN_LLAMA_BIN` / `SECOND_BRAIN_MODEL`
- Sorties persistées dans le même dossier : **`brain.json`** (graphe, lu au démarrage par l'app) + **`brain.md`**

Régénérer le graphe en CLI (utile pour itérer sans l'UI) :
```bash
cd src-tauri
LIMIT=0 cargo run --example brain     # toutes les convs ; LIMIT=4 pour un test rapide
# écrit brain.json + brain.md dans le dossier de données
```
> Si tu modifies `BrainNode`/`BrainGraph`/`SourceRef`, **régénère `brain.json`** sinon l'app affiche des détails vides (les anciens champs ne matchent plus).

## Carte du code

```
src-tauri/src/
  lib.rs                     # commandes Tauri (point d'entrée)
  models.rs                  # structs partagées : Conversation, BrainGraph/Node/Edge, SourceRef, ConnectorStatus
  connectors/claude_code.rs  # lecture/parse JSONL ~/.claude/projects/
  ai/llama.rs                # moteur llama-completion (subprocess, gabarit Gemma)
  ai/pipeline.rs             # extraction par conv + agrégation + synthèse projet + build du graphe + brain.md
  examples/{scan,brain}.rs   # validation CLI (parsing / pipeline E2E)
src/
  App.tsx                    # layout : header (Mind/Dossiers/brain.md + thème), canvas, sidebars flottantes
  components/  BrainMap.tsx (bulles d3-force) · FolderView.tsx · LeftSidebar.tsx (Connecteurs + Assistant chat)
               NodeDetail.tsx (résumé/mots-clés/décisions/sources) · ConversationDetail.tsx · BrainView.tsx (états + markdown) · ThemeToggle.tsx
  lib/  api.ts (pont Tauri) · types.ts · theme.ts · utils.ts
```

**Commandes Tauri exposées** (`lib.rs`) : `list_conversations`, `load_conversation`,
`claude_code_available`, `ai_ready`, `connectors_status`, `ask_brain`, `read_brain_graph`, `generate_brain`.

## Pipeline (résumé)

1. Pour chaque conversation : Gemma extrait `{summary, decisions, patterns, concepts}` (JSON, parsing tolérant).
2. Agrégation **déterministe** en Rust par projet + concepts ; chaque nœud porte ses **`SourceRef`** (provenance).
3. Passe de **synthèse par projet** (objectif + thématiques) via Gemma.
4. Construction du **graphe** (cerveau → projets → concepts ; concept partagé = bulle-pont) + sérialisation `brain.md`.

## Ajouter un connecteur (le multi-sources)

Écrire un module dans `connectors/` qui produit des `Conversation` (avec `source` = id du connecteur,
ex. `"claude"`, `"cowork"`, `"google-drive"`), l'ajouter à l'agrégation (cf. `load_all_conversations`).
Le graphe + drill-down + ponts fonctionnent sans autre changement. Les `SourceRef.link` deviennent
les liens fichiers (chemin local ou URL Drive).

## Règle : parité Mac/Windows (décision Liam, 2026-07-08)

**Toute feature faisable sur Mac doit l'être sur Windows.** En pratique :
- Jamais de dépendance à un binaire système supposé présent (`textutil`, Homebrew…) sans équivalent Windows.
- Binaire externe requis → **sidecar embarqué** (cf. `binaries/windows/`, `tauri.windows.conf.json`) **+ fallback pur Rust** si possible (ex. `pdf-extract` derrière `pdftotext`).
- Si un cas est **réellement impossible** (ex. `.doc` legacy binaire), l'UI le dit explicitement — **jamais d'échec silencieux** (le sync Drive Windows qui skippait tous les PDF sans un mot = l'anti-exemple).

## Prochaines étapes (priorité)

1. **2e connecteur réel** pour valider le multi-sources : claude.ai (import export `.zip`) ou Cowork.
2. **Google Drive OAuth** (flux loopback Tauri ; il faut créer un client OAuth Google Cloud → guider l'user pour client ID/secret).
3. **Watch auto** du dossier Claude Code (MAJ incrémentale).
4. **Packaging** `.dmg`/`.exe` avec llama.cpp en sidecar + download modèle post-install.
5. Polish visuel des bulles ; **fusion de concepts proches** (plus de ponts).
6. ~~Trancher le **naming** (Lucid vs Second Brain) et aligner partout.~~ Fait le 2026-07-20.

## Plugin ponytail

Plugin Claude Code installé **en scope projet** (`~/.claude/plugins/...`, marketplace `DietrichGebert/ponytail`).
Se charge au **démarrage** d'une session terminal `claude` dans ce projet : commandes `/ponytail*`, skills,
**2 hooks Node.js** + un **serveur MCP `ponytail-mcp`**. Philosophie « lazy senior dev » (éviter le sur-engineering).

# LucidFlow — Coffre partagé (second cerveau du SaaS)

Ce dossier est le *coffre Obsidian partagé* entre Lucas (Produit/Business) et Liam (CTO) =
mémoire unique du SaaS LucidFlow ("second cerveau en un clic"). Synchronisé entre les deux.

## Règle d'or n°1 — Consulter AVANT de répondre
Avant toute réponse sur le produit, la stack, l'archi, les décisions ou l'avancement, lis
d'abord les notes pertinentes du coffre (ne devine pas), puis cite-les :
1.⁠ ⁠⁠ 00 Accueil — LucidFlow.md ⁠ (dashboard + liens)
2.⁠ ⁠⁠ 01 Méta & Templates/Comment utiliser ce vault.md ⁠ + ⁠ Conventions & nommage.md ⁠
3.⁠ ⁠Le module concerné (02 Vision → 08 Journal). Tech → ⁠ 04 Stack technique/ ⁠.

## Règle d'or n°2 — Journaliser APRÈS chaque session (anti-perte de contexte)
En fin de session, dépose un compte-rendu daté dans ⁠ 08 Journal de bord/ ⁠
(gabarit ⁠ 01 Méta & Templates/_Templates/Template — Compte-rendu de session.md ⁠) :
objectif, avancées, décisions, questions, prochaines actions.
•⁠  ⁠Décision technique tranchée → ADR dans ⁠ 04 Stack technique/Décisions d'archi (ADR)/ ⁠.
•⁠  ⁠Décision produit/business → ⁠ 07 Gestion de projet/Journal de décisions (co-fondateurs).md ⁠.
•⁠  ⁠Nouvelle tâche → ⁠ 07 Gestion de projet/Tâches & sprints.md ⁠.

## Conventions
•⁠  ⁠Une info = une seule note (atomique), jamais de doublon ; on relie avec des [[wikilinks]].
•⁠  ⁠Frontmatter standard (type/domaine/statut/tags/créé) — voir ⁠ Conventions & nommage ⁠.
•⁠  ⁠Noms de fichiers uniques dans tout le coffre.

## ⚠️ Sécurité des données — NE PAS refaire les erreurs
•⁠  ⁠JAMAIS de coffre/dossier ⁠ LucidFlow ⁠ à l'intérieur du coffre (boucle récursive + perte via Sync).
•⁠  ⁠Avant/après une écriture en masse : ⁠ find . -name '*.md' -not -path '*/.obsidian/*' | wc -l ⁠
  et compare (référence ~55). Si ça baisse → STOP et alerte.
•⁠  ⁠Coffre sous Obsidian Sync (+ git recommandé). Évite les réécritures massives.

## Versionnage git (filet anti-perte)
Si le coffre est sous git, commit en fin de session : ⁠ git add -A && git commit -m "session: <résumé>" ⁠.
Historique complet = rien n'est jamais perdu, conflits visibles.

## Focus de Liam (CTO)
Faire avancer ⁠ 04 Stack technique ⁠ : remplir les premiers ADR (Tauri/Electron, vector store,
LLM local), lancer les POCs du ⁠ Backlog R&D ⁠. Voir ⁠ 07 Gestion de projet/Roadmap.md ⁠ (Phase 0).