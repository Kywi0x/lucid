# Second Brain — Document de projet

> Une app desktop qui connecte tes outils IA et génère automatiquement un second cerveau structuré, sous forme de mind map en Markdown, alimentée par une IA embarquée.

---

## Vision produit

Les devs et knowledge workers accumulent des centaines de conversations dans Claude Code, Cursor, Codex, Google Drive, WhatsApp — et tout ce contexte disparaît. **Second Brain** agrège, analyse et structure automatiquement tout ce contenu en un graphe de connaissances personnel, local, navigable.

**Principe fondamental : plug & play.**
Tu télécharges l'app, tu connectes tes outils, l'IA embarquée fait le reste. Zéro configuration manuelle, zéro clé API requise.

### Ce que l'user voit

1. Télécharge le `.dmg` ou `.exe`
2. Connecte ses outils (Claude Code en 1 clic, OAuth pour les autres)
3. L'IA analyse tout son contenu et génère une **mind map à bulles** (projets, concepts, connexions)
4. La mind map se met à jour automatiquement à chaque nouveau contenu

### Principe de drill-down & traçabilité (cœur du produit)

Le second cerveau n'est utile que si on peut **remonter à la source**. Chaque bulle est navigable :

- **Clic sur une bulle projet** → résumé synthétique : **objectif**, **thématiques**, **mots-clés**,
  décisions clés. Puis on déroule les **sous-bulles sources** : les conversations / fichiers
  qui ont servi à générer ce résumé.
- **Clic sur une bulle concept** → où le concept apparaît (projets, conversations) et pourquoi.
- **Sources affichées explicitement** : chaque élément indique **de quel connecteur** il vient
  (Claude Code, Cowork, Claude, Google Drive…) et, si l'IA a rattaché des fichiers, **les liens
  cliquables** (chemin local ou URL).

> **Le but : l'app résume et organise *toutes* tes apps comme un second cerveau** — pas juste
> Claude Code. La mind map agrège plusieurs sources en un seul graphe, et on peut toujours
> tracer d'où vient chaque bulle.

### Architecture multi-connecteurs (anticipée dès le modèle)

Chaque nœud du graphe porte une liste de **`SourceRef`** : `{ connecteur, titre, id, lien, date }`.
Le pipeline est agnostique de la source — un connecteur n'a qu'à produire des `Conversation`
normalisées (texte + métadonnées). Ajouter Cowork, Claude, Google Drive, WhatsApp… revient à
écrire un connecteur ; le graphe, le drill-down et l'affichage des sources fonctionnent sans
changement. Le connecteur d'origine est conservé sur chaque source pour l'afficher (badge/icône)
et ouvrir le bon viewer (conversation locale, fichier, URL Drive…).

---

## Stack technique

### App desktop

| Couche | Choix | Raison |
|---|---|---|
| Framework | **Tauri 2** (Rust + WebView) | ~8 Mo vs ~150 Mo Electron, packaging natif `.dmg`/`.exe`, performances |
| Frontend | **React + TypeScript** | Composants mind map, UI des connecteurs |
| UI components | **shadcn/ui + Tailwind** | Design system rapide, accessible |
| Mind map viewer | **React Flow** | Rendu de graphes interactif, performant |

### IA embarquée

| Couche | Choix | Raison |
|---|---|---|
| Runtime | **llama.cpp** (binaire `llama-completion` embarqué) | Zéro dépendance externe, tourne sur CPU/GPU, cross-platform. ⚠️ API récente : `llama-cli` est devenu interactif, le one-shot passe par `llama-completion` |
| Modèle | **Gemma 3 4B Q4_K_M** (~2.3 Go) | Variante Gemma 3 réelle la plus proche (le « 2B » du doc initial n'existe pas en Gemma 3). Tourne sur 8 Go RAM, ~77 tok/s sur Metal. Meilleure qualité de clustering |
| Packaging | Binaire + modèle bundlé dans l'installeur | L'user ne voit rien, ça tourne dès l'install |
| Interface Rust→IA | Appel subprocess ou binding C via Tauri | Simple, stable |

> **Pourquoi pas Ollama ?** Ollama nécessite une installation séparée par l'user. llama.cpp embarqué = vraie expérience plug & play.

### Connecteurs (MVP → V2)

| Connecteur | Méthode d'accès | Priorité |
|---|---|---|
| **Claude Code** | Lecture fichiers JSONL `~/.claude/projects/` | MVP |
| Claude (claude.ai) | Export / API conversations | V2 |
| Cowork | Sessions & artefacts Cowork | V2 |
| Google Drive | OAuth2 + Drive API (fichiers rattachés → liens) | V2 |
| WhatsApp | Parsing export `.zip` local | V2 |
| Codex / OpenAI | API (clé user optionnelle) | V2 |
| Notion | OAuth2 + Notion API | V3 |
| Slack | OAuth2 + Slack API | V3 |

> Chaque connecteur ne fait qu'une chose : transformer sa source en `Conversation` normalisée.
> La provenance (`SourceRef`) est attachée à chaque nœud → le drill-down et les liens fichiers
> fonctionnent quel que soit le connecteur.

### Stockage & sync

| Mode | Technologie | Déclencheur |
|---|---|---|
| Local (défaut) | **SQLite** + fichier `brain.md` | Toujours actif |
| Cloud (opt-in) | **Supabase** (PostgreSQL + Storage) | Activé par l'user dans les settings |

Le fichier `brain.md` est la source de vérité. Supabase stocke une copie pour backup et multi-device. L'user garde toujours accès à son `.md` sans abonnement.

### Format du `brain.md`

```markdown
# 🧠 Second Brain — [date de mise à jour]

## 🗂 Projets actifs
### Nom du projet
- Décision clé 1 (source: Claude Code · 2025-06-10)
- Pattern récurrent identifié
- ...

## 💡 Concepts récurrents
### Nom du concept
- Contexte et occurrences
- ...

## 🔗 Connexions identifiées
- [Projet A] ↔ [Concept B] : raison du lien
- ...
```

### Monétisation

| Plan | Prix | Limites |
|---|---|---|
| **Free** | Gratuit | 1 connecteur, sync locale uniquement |
| **Pro** | 9 $/mois | Connecteurs illimités, sync Supabase, historique complet |
| **Team** | 25 $/user/mois | Mind map partagée, espaces collaboratifs |

Stack : **Stripe Billing** + **Supabase Auth** + vérification de licence côté Tauri au démarrage.

---

## Architecture du pipeline IA

```
[Connecteur] → [Ingestion] → [Chunking] → [Analyse Gemma 3 2B] → [Clustering] → [brain.md]
     ↑                                                                                  ↓
[Nouveau fichier détecté]                                              [Diff + mise à jour incrémentale]
```

**Prompt système de l'IA embarquée (principe) :**
- Extraire les thèmes, décisions, patterns récurrents
- Regrouper par projet et concept
- Identifier les connexions entre sujets
- Produire du Markdown structuré, concis, navigable

**⚠️ Correction de scope (28 juin 2026)** — La mind map est la **sortie principale** de l'IA,
pas le `brain.md`. Le pipeline produit un **graphe de connaissances** (`BrainGraph` :
nœuds cerveau→projets→concepts + liens d'appartenance et ponts inter-projets) qui alimente
une **mind map à bulles façon Obsidian** (layout physique d3-force, `BrainMap.tsx`). Le
`brain.md` n'est qu'une **sérialisation Markdown** du même graphe (onglet secondaire).
Persistance : `brain.json` (graphe) + `brain.md` dans le dossier de données applicatif.
Le pipeline est **résilient** : une conversation qui échoue à l'analyse est ignorée, pas fatale.

**Choix d'implémentation — pipeline hybride** (de-risque le risque #1, qualité du clustering) :
- Le **modèle** fait l'extraction *par conversation* → JSON `{summary, decisions, patterns, concepts}`
  (parsing tolérant : extraction du 1er objet JSON équilibré, défaut si invalide).
- L'**agrégation, le clustering par projet et les connexions** sont assemblés
  *déterministiquement* en Rust (concepts ≥ 2 occurrences = récurrents ; concepts présents
  dans ≥ 2 projets = connexions). → Résultat reproductible, robuste au bruit du petit modèle.
- Code : `src-tauri/src/ai/{llama.rs,pipeline.rs}`. Modèle & binaire résolus depuis le dossier
  de données applicatif (`~/Library/Application Support/fr.ideeri.brainlink/`) ou via les
  variables `SECOND_BRAIN_MODEL` / `SECOND_BRAIN_LLAMA_BIN`.

---

## Roadmap

### Phase 0 — Preuve de concept ✅ (réalisée)
- [x] Lire les fichiers JSONL de Claude Code localement — *17 convs / 1052 msgs parsés*
- [x] Appeler llama.cpp via subprocess Rust pour analyser des conversations — *`llama-completion` + Gemma 3 4B*
- [x] Générer un `brain.md` de démonstration — *pipeline hybride : extraction IA par conv + assemblage déterministe*
- [x] Valider la qualité du clustering thématique — *bonne qualité observée : décisions/patterns/concepts pertinents par projet*

### Phase 1 — MVP

**Fait ✅**
- [x] App Tauri 2 (React/TS) — layout flottant, thème dark/light natif
- [x] Connecteur Claude Code (lecture + parsing JSONL) — 17 convs / 1052 msgs validés
- [x] IA locale llama.cpp (`llama-completion`) + Gemma 3 4B Q4 — ~77 tok/s sur Metal
- [x] Pipeline hybride → **graphe de connaissances** (`BrainGraph`) + `brain.md`
- [x] **Mind map à bulles** (d3-force, fond dots, couleurs par catégorie, hover animé)
- [x] **Folder view** + **recherche globale** dans les deux vues
- [x] **Drill-down / provenance** : clic bulle → résumé (objectif/thématiques) + mots-clés + décisions + **sources** (connecteur, lien fichier) → ouvre la conversation
- [x] **Sidebar flottante** : onglet Connecteurs (statut + dernière synchro) + onglet Assistant (chat local sur le `brain.md`)
- [x] Résilience pipeline (une conversation en échec n'interrompt plus tout)

**Reste à faire ⏳**
- [ ] **2e connecteur réel** pour valider le multi-sources (claude.ai via export `.zip` ou Cowork) → faire apparaître les ponts inter-sources
- [ ] **Google Drive OAuth** (flux loopback Tauri ; nécessite un client OAuth Google Cloud)
- [ ] **Watch auto** du dossier Claude Code (mise à jour incrémentale du graphe)
- [ ] **Packaging** `.dmg` (Intel + Apple Silicon) / `.exe` — avec llama.cpp en **sidecar bundlé** + download du modèle post-install
- [ ] **Polish visuel** des bulles (animations, layout, lisibilité)
- [ ] **Fusion de concepts proches** (« Supabase » / « Supabase Auth » → plus de ponts)
- [ ] Landing page + waitlist

> **Naming à trancher** : le header affiche **« Lucid »** ; le projet/identifiant Tauri reste
> `brainlink` / `fr.ideeri.brainlink` / produit « Second Brain ». Décider d'un nom unique puis
> aligner `package.json`, `Cargo.toml`, `tauri.conf.json` (productName + identifier), le header.

### Phase 2 — Produit (3 mois)
- [ ] Connecteurs Google Drive, WhatsApp, Codex
- [ ] Sync Supabase (opt-in)
- [ ] Intégration Stripe (plans Free / Pro)
- [ ] Recherche sémantique dans le mind map
- [ ] Onboarding guidé

### Phase 3 — Scale (6 mois)
- [ ] Plan Team + mind map partagée
- [ ] Connecteurs Notion, Slack, Obsidian
- [ ] Mobile companion (iOS/Android, lecture seule)
- [ ] API publique pour connecteurs tiers
- [ ] Analytics usage (opt-in, anonymisé)

---

## Risques & mitigations

| Risque | Probabilité | Mitigation |
|---|---|---|
| Qualité du clustering IA insuffisante | Élevée | Itérations intensives sur les prompts en Phase 0 |
| llama.cpp trop lent sur machines anciennes | Moyenne | Mode "cloud" optionnel via API Anthropic |
| Taille de l'installeur trop lourde (~1.5 Go) | Moyenne | Téléchargement du modèle post-install, en arrière-plan |
| Accès aux fichiers refusé par l'OS | Faible | Permissions explicites demandées à l'install (macOS sandbox) |
| Concurrence (Mem.ai, Rewind, etc.) | Moyenne | Différenciation : 100% local, dev-first, format ouvert (.md) |

---

## Différenciateurs clés

- **100% local par défaut** — aucune donnée ne quitte la machine sans consentement explicite
- **Format ouvert** — le `brain.md` appartient à l'user, lisible partout
- **Dev-first** — Claude Code comme connecteur natif, là où les concurrents ignorent les outils IA
- **Zéro friction** — aucune configuration, l'IA tourne sans setup utilisateur

---

*Document généré le 28 juin 2026 — v0.1*
