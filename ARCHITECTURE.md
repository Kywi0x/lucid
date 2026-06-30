# Architecture — Lucid

> Décisions techniques actées. Pour la vision produit : `second-brain-project.md`. Pour l'opérationnel : `CLAUDE.md`.

## Modèle de données

### Hiérarchie des nœuds

```
Lucid (root, unique)
  └── Espace        ← conteneur thématique, porte un résumé
        └── Page    ← unité de connaissance
              └── Sous-page (récursif, profondeur illimitée)
```

`BrainNode.kind` : `"root"` | `"espace"` | `"page"`
`BrainNode.parent_id: Option<String>` — navigation récursive (export, traversal)

### Correspondance ancien → nouveau

| Ancien `kind` | Nouveau `kind` | Affiché |
|---|---|---|
| `root` | `root` | Lucid |
| `project` | `espace` | nom de l'espace |
| `concept` | `page` | nom de la page |

---

## Stockage

### brain.md — source de vérité

Fichier unique, markdown structuré. Contient **toutes les informations de l'app**.
Lisible par un humain, un éditeur markdown, et directement injectable comme contexte LLM.

```markdown
# Lucid

## Espace: Développement Tauri
<!-- id: espace-dev-tauri | weight: 12 | community: 1 -->

Résumé de l'espace.

**Mots-clés** : Tauri, Rust, React

### Page: Pipeline IA
<!-- id: page-pipeline-ia | parent: espace-dev-tauri -->

Contenu.

**Décisions** :
- Utiliser Gemma 3 4B Q4_K_M

#### Sous-page: Extraction Gemma
<!-- id: page-extraction-gemma | parent: page-pipeline-ia -->
...
```

`##` = espace · `###` = page · `####`+ = sous-pages (récursif)

### brain.json — cache dérivé

Regénéré automatiquement si `brain.md` est plus récent. Utilisé uniquement pour le rendu du graphe (xyflow/d3). Ne jamais éditer à la main.

---

## Pipeline

```
Connecteur (Claude Code / Google Drive / Notion)
    │
    ├─ Parsing contenu brut : Rust + binaires système
    │   ├── Claude Code  → lecture JSONL
    │   ├── Google Drive → pdftotext (poppler) + OCR (tesseract)
    │   └── Notion       → fetch API
    │
    ↓ produit des `Conversation` (texte brut)
    │
    LLM (Gemma 3 4B, local) → extraction JSON interne
    │   { summary, decisions, patterns, concepts }
    │   (JSON = format intermédiaire fiable pour parsing Rust)
    │
    Rust agrège + structure → sérialise
    │
    ├─ brain.md  ← SOURCE DE VÉRITÉ
    └─ brain.json ← cache dérivé
```

**Pourquoi JSON en extraction LLM et pas markdown** : le LLM est plus fiable pour produire du JSON contraint. Le JSON est interne et éphémère ; le markdown est la sortie persistée et éditable.

---

## Logique cache au démarrage

```rust
if !brain_json_exists() || brain_md_newer_than_brain_json() {
    let graph = parse_brain_md(brain_md_content);  // brain_md.rs
    write_brain_json(graph);
}
```

---

## Export markdown par bulle

Chaque espace/page expose un bouton "Télécharger .md".
Le fichier généré contient le nœud courant + tous ses enfants récursivement (via `parent_id`).
Pas d'enfants = juste le nœud.

---

## Évolution future : LLM en écriture

L'architecture markdown permet au LLM local de créer/modifier des pages :

```
User : "Génère des fiches de révision sur l'espace Physique Chimie"
  → App injecte l'espace depuis brain.md (contexte léger)
  → Gemma génère des pages markdown
  → App écrit dans brain.md
  → brain.json invalidé + regénéré
  → Graphe mis à jour
```

Limites Gemma 3 4B : fenêtre ~8k tokens (chunking pour grands espaces), qualité correcte mais pas GPT-4.

---

## Ajouter un connecteur

1. Créer `src-tauri/src/connectors/mon-connecteur.rs`
2. Produire des `Conversation` avec `source = "mon-connecteur"`
3. L'ajouter à `load_all_conversations()` dans `lib.rs`
4. Graphe + drill-down + export fonctionnent sans autre changement

---

*Décision formalisée dans le vault Obsidian : ADR-0008*
