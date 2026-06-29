# Architecture des bulles — rendre la génération scalable (multi-connecteurs)

> Audit + diagnostic + plan pour que **ajouter un connecteur (Drive, Notion, Gmail, Cowork…)
> soit trivial** et produise des bulles cohérentes, sans retoucher le cœur du pipeline.
> Projet : Lucid / Second Brain (`brainlink`) — Tauri 2 (Rust + React/TS), IA locale Gemma 3 4B.

---

## TL;DR

- Les bulles de **GROUPE** ne sont **pas** clusterisées par le LLM : elles sont déterministes en Rust,
  à partir d'un seul `String project`. **C'est la racine du bug Drive.**
- Cette primitive (une string plate) est trop faible pour une hiérarchie → chaque connecteur l'aplatit
  de façon lossy et incohérente. Claude Code marche par chance (`project = cwd` = chemin unique).
- **Fix** : remplacer la primitive par un **chemin** (`container_path: string[]`) que chaque connecteur
  remplit depuis son arbo native. Le cœur construit les groupes génériquement.
- Pour relier une entité (client, matière, projet…) **à travers les sources**, ajouter une
  **couche entité** : extraction typée par le 4B + **résolution déterministe en Rust** (la fiabilité
  ne vient jamais du 4B seul).
- Un seul moteur, trois signaux (**structurel + entité + concepts**) qui se pondèrent automatiquement
  selon le profil utilisateur. Aucun code « par persona ».

---

## 1. AS-IS (état actuel du code)

| Question | Réponse | Référence |
|---|---|---|
| Unité envoyée au modèle | Une **`Conversation` entière** (1 session Claude Code / 1 fichier Drive / 1 conv claude.ai), condensée à 6000 chars. Pas de chunking. | `pipeline.rs:42`, `pipeline.rs:218` |
| Bulles de GROUPE | **Structurelles, déterministes en Rust** : `proj_key = norm(conv.summary.project)`, 1 bulle par clé distincte. Le LLM n'y touche jamais. | `pipeline.rs:235`, `pipeline.rs:446` |
| Bulles CONCEPT / PONTS | Sémantiques : concept présent dans ≥2 groupes → bulle-pont. | `pipeline.rs:479` |
| Arbo Drive | **Jetée** : seul le **nom du 1er parent immédiat** est gardé (1 niveau), clé = nom (pas id), **pas de pagination** (`nextPageToken` ignoré). Fichiers sans parent mappé → fourre-tout `"Google Drive"`. | `google_drive.rs:265` |
| Structure BrainGraph | Nœuds `root \| project \| concept` ; arêtes `contains` (root→project) et `uses \| bridges` (project→concept) ; `community` = 0 pour ponts/root, 1..n par projet. | `models.rs:70-114` |

---

## 2. Diagnostic

**L'hypothèse « le pipeline demande au LLM de re-dériver les groupes » est fausse dans le mécanisme**
(le LLM ne regroupe rien), **mais juste sur le fond** :

- Claude Code marche car `project = cwd` = chemin complet unique par repo (`claude_code.rs:135`).
- Drive échoue car son mapper **aplatit un arbre en une seule string plate**, de façon lossy.
- Preuve que la racine est générale : **claude.ai a le même bug** — `project = "claude.ai"` en dur
  pour toutes les convs (`claude_ai.rs:91`) → une seule méga-bulle.

➡️ **Racine = la primitive de groupe est un seul `String project`.** Trop faible pour une hiérarchie.

### Où la généricité fuit aujourd'hui

1. **Primitive de groupe = `String` plate** (fuite principale).
2. **`proj_key = norm(display_name)`** → collisions de noms entre branches/connecteurs.
3. **`assemble_markdown` code en dur `(source: Claude Code)`** (`pipeline.rs:639-641`) → faux pour les autres sources.
4. **`extraction_prompt` a un `match source` "google-drive"** (`pipeline.rs:61`) — petite fuite tolérable.
5. **Câblage manuel des connecteurs dans `lib.rs`** (`load_all`, `list`, `load_by_id`, `status`) — fuite de *registration*, distincte du bug des bulles.

---

## 3. Contrat connecteur (la clé de la scalabilité)

**Recommandation : additif, pas de type parallèle.** Ajouter un champ à l'existant plutôt qu'introduire
un `NormalizedItem` distinct (même sémantique, ~5× le diff, rename dans 8 fichiers + types front → YAGNI).

```rust
// models.rs — ConversationSummary
#[serde(default)]
pub container_path: Vec<String>,   // hiérarchie native, ex ["Marketing","Q3"]. Vide = "Non classé".
```

- `#[serde(default)]` → les `brain.json` / caches existants restent lisibles.
- `project` devient **affichage seulement** (`container_path.join(" / ")` ou fallback), plus une clé de groupe.
- Le `kind` (file/email/note/conversation) existe déjà de fait via `source`.

**Le cœur change UNE fois** (`pipeline.rs`) : construire les groupes à partir de `container_path`.
Ensuite, **ajouter une source = écrire seulement le mapper qui remplit `container_path`.**

| Connecteur | `container_path` rempli depuis |
|---|---|
| Google Drive | chaîne complète des dossiers parents (par id) |
| Claude Code | `[repo_name]` |
| Gmail | labels |
| Notion | `db > page` (+ relations natives) |
| Cowork | `[]` (→ bac "Non classé" ou entité) |

### Les 4 garde-fous génériques (dans l'agrégation `pipeline.rs`)

1. **Profondeur plafonnée** : ne garder que `container_path[..2]` comme niveaux de groupe.
2. **Min items/bulle** : conteneur à 1 item → fusionné dans le parent.
3. **Bac "Non classé"** : `container_path` vide → groupe par défaut (remplace les fourre-tout).
4. **Normalisation concepts** : `norm()` (lowercase) + table d'alias. Items individuels = jamais des bulles.

---

## 4. Couche entité (le différenciateur produit)

Relier « Béaux Électricité » / « une matière » / « un projet » **à travers les sources**, c'est un
**axe différent** du `container_path` (qui, lui, est par source).

### Vérité architecturale

Le modèle **ne voit qu'un item à la fois** → il ne peut PAS décider seul qu'une entité récurre à
travers les sources. **Cette décision doit être déterministe en Rust** (comme les ponts actuels).
La fiabilité ne vient jamais du 4B seul.

### Recette minimale

1. **Champ typé `entities`** séparé de `concepts` : `[{name, type: client|personne|entreprise|lieu}]`.
   Le 4B est bien meilleur sur « liste les entités » que sur des concepts en vrac.
2. **Canonicalisation en Rust** : pliage d'accents + lowercase + **table d'alias** (le bouton de réglage
   humain) + fuzzy léger (chevauchement de tokens / Levenshtein) avec **seuil réglable**.
3. **Entité dans ≥2 items (surtout ≥2 sources) → bulle-entité** contenant ces items en drill-down.
4. **Métadonnées connecteur > NER** : domaine email (Gmail), nom de repo (Code), relation native (Notion)
   sont des ancres bien plus fiables que le texte.
5. **(Optionnel) 1 seul appel LLM global** : « voici 200 noms d'entités, lesquels = même entité réelle ? »
   — cheap (que des strings), contourne le « un item à la fois ».

### Limites honnêtes

- Rappel 4B ~70-85 %, pas 100 %.
- Le fuzzy sur-fusionne (`Béaux électricité` vs `Béaux plomberie`) ou sous-fusionne → seuil + alias.
- Bulle-entité ≠ bulle-pont actuelle : faire d'une entité un **conteneur** d'items = ajouter
  un `kind:"entity"` au graphe (changement borné dans `pipeline.rs` + `models.rs` + front).

---

## 5. Exemple — Client éparpillé (cas dur, multi-sources)

Données fictives. Le même client apparaît sous des noms incohérents et dans des arbos sans rapport.

| Source | Item | `container_path` | Entité 4B (brut) |
|---|---|---|---|
| Drive | `Devis_2024.pdf` | `["Clients","Béaux Electricité"]` *(sans accent)* | Béaux Electricité |
| Drive | `Contrat signé.docx` | `["Clients","Béaux Electricité"]` | SARL Béaux |
| Drive | `FAC-2041_beaux.pdf` | `["Comptabilité","Factures Q3"]` | Béaux |
| Notion | `Béaux Électricité` (fiche) | `["CRM"]` | Béaux Électricité |
| Gmail | `Devis mise aux normes` | `["Clients","Béaux"]` (label) | `beaux-elec.fr` |
| Cowork | `Préparer réunion Béaux électricité` | `[]` | Béaux électricité |
| Notion | `Rénovation tableau électrique` | `["Projets"]` | **Béaux Plomberie** ⚠️ |

**Résolution Rust :**

```
beaux electricite  ─┐
beaux elec          ├─► canonique  ent:beaux-electricite ("Béaux Électricité")
sarl beaux          ├─   alias forts: [beaux-elec.fr]
beaux               ┤
beaux electricité   ─┘
beaux plomberie  ───►  ent:beaux-plomberie   (token "plomberie" ≠ → NON fusionné)
```

**Graphe :**

```
                              🧠 Brain
        ┌────────────────┬───────┴───────┬──────────────┐
   Drive:Clients    Notion:CRM      Gmail:Factures   devis-generator
        └──────┐    ┌────┘          ┌────┘                │
               ▼    ▼               ▼                      ▼
        ╔═══════════════════════════════════════════════════════╗
        ║   🏢  Béaux Électricité   (client)                     ║  ◄── bulle inter-sources
        ║   ├─ Drive  : Devis_2024, Contrat, FAC-2041            ║
        ║   ├─ Notion : Fiche CRM, CR visite chantier            ║
        ║   ├─ Gmail  : Devis mise aux normes, FAC-2041 réglée   ║
        ║   ├─ Cowork : Réunion, Relecture contrat               ║
        ║   └─ Code   : endpoint PDF devis                       ║
        ╚═══════════════════════════════════════════════════════╝
               │                 │                  │
        💡 devis          💡 mise aux normes   💡 FAC-2041
     (Drive+Gmail+Code)   (Drive+Gmail)        (Drive+Gmail)

   🏢 Béaux Plomberie  ← entité distincte, NON fusionnée (garde-fou)
```

Un même item (`Devis_2024.pdf`) vit **à la fois** sous `Drive:Clients` (structurel) et sous
`Béaux Électricité` (entité). Deux arêtes, pas un conflit.

---

## 6. Exemple — Étudiant (cas facile, structurel fort)

L'arbo Drive **est** déjà l'architecture. Le moteur nettoie et récupère les égarés.

| Source | Item | `container_path` | Diff. |
|---|---|---|---|
| Drive | `Cours_algo_chap3.pdf` | `["Fac","L3 Info","Algorithmique"]` | 🟢 |
| Drive | `slides_BDD.pdf` | `["Fac","L3 Info","Bases de données"]` | 🟢 |
| Drive | `algo_revisions.pdf` | `["fac","algo"]` *(doublon, autre nommage)* | 🟡 |
| Drive | `TD3_recursivite.pdf` | `["Téléchargements"]` *(non rangé)* | 🔴 |
| Drive | `IMG_4421.jpg` (photo tableau) | `["Téléchargements"]` | 🔴 |
| Notion | `Fiche révision SQL` | `["Études"]` | 🟡 |
| Gmail | `Projet de groupe - rendu lundi` | label `["Fac"]` | 🟡 |

- 🟢 `Fac/L3 Info/Algorithmique` → bulle matière directe, gratuite.
- 🟡 `fac/algo` → canonicalisation (alias `algo → Algorithmique`) fusionne le doublon.
- 🟡 Profondeur : la bulle utile = **la matière**, `L3 Info` reste parent regroupeur (garde-fou profondeur).
- 🔴 `TD3_recursivite.pdf` à la racine → bac "Non classé", **mais** rattaché à `Algorithmique` par le
  concept `récursivité` (le sémantique sauve le structurel).
- 🔴 Photo tableau → OCR (tesseract, déjà câblé) ; si illisible → reste "Non classé" (on ne devine pas).

```
                          🧠 Mes études — L3 Info
            ┌──────────────┬───────────┴─────────┬─────────────┐
        📁 Algorithmique  📁 Bases de données  📁 [autres]   🗃 Non classé
        (Drive+racine)    (Drive+Notion)                     (photos illisibles)
         │   │   │            │      │
      TD2  Cours  TD3      slides  Fiche SQL
            chap3  récursiv.       (Notion)
         │                              │
         └──── 💡 récursivité ──────────┘    ◄── concept-pont
                          │
                  💡 projet de groupe (Algo + Gmail rendu lundi)
```

---

## 7. Exemple — Freelance (cas dur, entité dispersée)

L'arbo ne suffit pas (compta rangée par date, mails par type). La couche entité reconstruit le client.

| Source | Item | `container_path` | Entité 4B | Diff. |
|---|---|---|---|---|
| Drive | `maquette_v3.fig` | `["Clients","Café Pixel"]` | Café Pixel | 🟢 |
| Drive | `FAC-2041.pdf` | `["Compta","2024","Q3"]` *(par date)* | Pixel | 🔴 |
| Gmail | `Refonte site` | label `["Café Pixel"]` | contact@cafe-pixel.fr | 🟡 |
| Gmail | `Relance facture` | label `["Factures"]` | "le café" 😬 | 🔴 |
| Notion | `Café Pixel — specs` | `["CRM"]` | Café Pixel | 🟢 |
| Notion | `Backlog sprint` | `["Projets"]` | CP *(abrév.)* | 🔴 |
| Cowork | `Préparer call Pixel` | `[]` | Pixel | 🟡 |
| Claude Code | `intégration Stripe` | `["cafe-pixel-site"]` | — | 🟡 |

- 🟡 Domaine `cafe-pixel.fr` + repo `cafe-pixel-site` → ancres fortes (métadonnée > NER).
- 🔴 `FAC-2041` dans `Compta/2024/Q3` → l'arbo ne dit rien ; sauvée par le contenu (cite « Café Pixel »).
- 🔴 `"le café"` / `"CP"` → trop ambigus pour le 4B ; rattachés via `container_path` co-occurrent ou
  la passe LLM globale. Sinon ratés (le 10-15 % qu'on assume).

```
                            🧠 Activité freelance
        ┌───────────────┬──────────────┴──────────┬──────────────────┐
   Compta          Stack perso              🏢 clients (entités)   admin
   (par date)      (React, Stripe…)         ┌────────┼────────┐    (URSSAF…)
                                         🏢 Café   🏢 Studio  ...
                                          Pixel    Lumen
        ╔═════════════════════════════════════════════════════════╗
        ║  🏢  Café Pixel  (client)                                ║
        ║   ├─ Drive  : maquette_v3, devis, FAC-2041 ◄─ récupérée   ║
        ║   ├─ Gmail  : Refonte site, Relance facture               ║
        ║   ├─ Notion : specs, Backlog sprint                       ║
        ║   ├─ Cowork : Préparer call                               ║
        ║   └─ Code   : intégration Stripe                          ║
        ╚═════════════════════════════════════════════════════════╝
                  │                  │
            💡 Stripe          💡 SEO / refonte (plusieurs clients)
         (compétence transverse, pont entre clients)
```

### Pourquoi c'est « costaud »

| | Étudiant | Freelance | Client éparpillé |
|---|---|---|---|
| Signal dominant | **Structurel** | **Entité** | **Entité** |
| Rôle du 4B | mineur | central | central |
| Ce qui sauve | concepts-ponts | métadonnées connecteur | métadonnées + passe globale |

**Le même pipeline sert tous les profils** : `container_path` (squelette) + entités (transverse) +
concepts (ponts). Aucun code « par persona » : on écrit des **signaux génériques**, le graphe s'auto-adapte.
Le 10-15 % vraiment ambigu finit en **"Non classé"** (visible, corrigeable) plutôt que mal classé.

---

## 8. Plan d'implémentation

### Fichiers à modifier

| # | Fichier | Changement |
|---|---|---|
| 1 | `models.rs` | + `container_path: Vec<String>` (`#[serde(default)]`) sur `ConversationSummary`. |
| 2 | `ai/pipeline.rs` | Grouper depuis `container_path` (clé = chemin tronqué) + 4 garde-fous. Dé-hardcoder `(source: Claude Code)` → `src.connector`. |
| 3 | `connectors/google_drive.rs` | Chaîne complète des parents (clé = id) + **pagination** `nextPageToken`. Remplir `container_path`. |
| 4 | `connectors/claude_code.rs` | Remplir `container_path = [repo_name]`. |
| 5 | `connectors/claude_ai.rs` | Remplir `container_path`. |
| 6 | `src/lib/types.ts` | Refléter `container_path` (drill-down / labels). |
| 7 *(chantier 3)* | `models.rs` + `pipeline.rs` | Champ `entities` typé + résolution (canonicalisation, alias, fuzzy) + `kind:"entity"`. |

### Ordre

1. **(1)** champ `container_path` — socle.
2. **(2)** agrégation générique + garde-fous, en utilisant `[short_project]` comme `container_path`
   provisoire (ne casse rien, valide le cœur isolément).
3. **(3)** mapper Drive complet (chaîne + pagination) → **régénérer `brain.json`** → le bug des bulles disparaît.
4. **(4)(5)** mappers Claude Code / claude.ai pour cohérence.
5. **(6)** front.
6. **Chantier entité (7)** — après que le squelette structurel est stable.

### Critère de réussite du contrat

Ajouter un 4ᵉ connecteur (Gmail/Notion) = écrire **uniquement** son mapper `→ container_path`.
Si `pipeline.rs` doit changer pour ça, la généricité fuit encore (probablement dans le mapping
`container_path → clé de groupe`, à rendre indépendant de la source).

---

*Document de travail — Lucid / Second Brain. Données d'exemple fictives.*
