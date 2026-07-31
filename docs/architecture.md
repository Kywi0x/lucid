# Architecture — comment chaque brique marche

> Dérivé du code · 2026-07-30 · format constant : 🎯 ce que ça fait · ⚙️ comment · 📁 où.

---

## 1 · Connecteurs — d'où vient le contenu

🎯 Chaque source produit des `Conversation` normalisées → le reste du pipeline ignore la source.

⚙️ Comment
- Le connecteur lit sa source, met en cache (JSON), fait de l'incrémental (mtime local / `modifiedTime` Drive).
- Types de fichiers visés (**même liste pour tous**) : `pdf · docx · pptx · xlsx · csv · txt · md` (+ `doc/rtf` Mac).
- ⚠️ Drive n'accepte aujourd'hui qu'un sous-ensemble → voir `known-gaps.md`.

📁 `src-tauri/src/connectors/` (7 fichiers) · extensions : `local_folder.rs:25`

---

## 2 · Extraction & stockage

🎯 Transformer un fichier en texte exploitable, puis le stocker.

⚙️ Comment
- `file_to_source_text(path)` route par extension → PDF (pdftotext + OCR, fallback pur Rust), docx, pptx, xlsx/csv (→ **table markdown**), txt/md (brut), doc/rtf (`textutil`, Mac).
- Résultat rangé **verbatim** dans `source_text`. Champ séparé `content` = ce que **toi** tu écris (markdown).
- Stockage = **SQLite `brain.db`** (tables `nodes` / `edges` / `meta`). Legacy = `brain.json`.
- Affichage = **markdown rendu** : `content` sinon `source_text` → prose = texte plat, CSV/Excel = tableau, tes notes = markdown riche.

📁 `lib.rs::file_to_source_text` (~1957) · schéma `storage.rs:13` · rendu `components/NodeDetail.tsx`

---

## 3 · Le cerveau (pipeline)

🎯 Construire le graphe à partir de toutes les sources.

⚙️ Comment
1. Gemma extrait `{résumé, décisions, patterns, concepts}` par conversation.
2. Agrégation **déterministe** (Rust) → nœuds (projets/concepts) + provenance.
3. Synthèse par projet (Gemma).
4. Graphe (cerveau → projets → concepts ; concept partagé = pont) + `brain.md`.

📁 `ai/pipeline.rs` · `ai/brain_md.rs` · `models.rs` · commandes `generate_brain`, `read_brain_graph`

---

## 4 · L'Archiviste (rangement auto)

🎯 Ranger / fusionner ce qui arrive en vrac.

⚙️ Comment
- **Doublons de titre exact** → fusion (tombstone sur les absorbés).
- **Clustering par embeddings** (BGE-M3) → k-NN mutuel + cohésion + **garde de domaine**, thèmes nommés par Gemma.
- Le clustering « par mot partagé » a été **abandonné** (fragile).
- Un nœud déjà rangé sous un thème n'est **jamais** repris (stabilité).
- Déclencheurs = **événementiel** (§7), jamais un timer, jamais derrière une écriture MCP.

📁 `archivist.rs` · orchestration `lib.rs::run_archivist_scan_once_in_progress` (~598) · commande `run_archivist`

---

## 5 · IA locale (deux moteurs)

🎯 Générer (résumés, arbitrages) + embedder (vecteurs).

⚙️ Comment
- **Génération** : `llama-server` + Gemma 4 E4B, HTTP loopback + clé API aléatoire.
- **Embeddings** : 2ᵉ `llama-server --embedding` + BGE-M3, port dédié.
- Binaires **statiques self-contained** (`LLAMA_OPENSSL=OFF`).
- **Bootstrap** (1er lancement) : DL auto du modèle recommandé (secours selon RAM) + BGE-M3, zéro choix.
- **Diagnostic** : état stack + logs → Réglages → Compte.

📁 `ai/llama.rs` · `lib.rs` (`ai_bootstrap`, `ai_diagnostics`) · `components/SetupScreen.tsx` · `scripts/bundle-sidecars.*`

---

## 6 · Affichage (carte & pages)

🎯 La carte à bulles + les pages.

⚙️ Comment
- Carte rendue en **Canvas** + `d3-force`.
- Un refresh **de fond ne recentre plus** (préserve pan/zoom) ; re-fit seulement sur action explicite (`revealKey`).
- Pages = éditeur markdown (TipTap) + résumé IA + sources + propriétés.

📁 `components/BrainMap.tsx` (re-fit : effet `[revealKey]`) · `NodeDetail.tsx` · `App.tsx`

---

## 7 · Détection & watch

🎯 Voir les changements de sources et réagir.

⚙️ Comment
- Local / Obsidian / Notes / Claude Code → watcher fs **temps réel** (`notify`), debounce ~3 s.
- Google Drive → **sondage ~5 min** (pas de push possible vers un desktop).
- Changement **document** → régé (cooldown 1×/5 min) + event `archiviste-auto` ; jamais pour Claude Code.
- **Inbox** → buffer des fichiers récents (dédup + ×N), event `inbox-updated`, clic ouvre la page.

📁 `lib.rs::start_watcher` (~5103), `inbox_record`/`inbox_recent` · `components/InboxPanel.tsx`

---

## 8 · Sync cloud & sauvegarde

🎯 Le cerveau suit le compte entre machines.

⚙️ Comment
- Merge **last-write-wins** par nœud (`updated_at`) + tombstones ; via Supabase (bucket `backups`).
- Sauvegarde manuelle + restauration.
- ⚠️ **La sync auto est désactivée aujourd'hui** → voir `known-gaps.md`.

📁 `lib/sync.ts` · `backup.rs` (`merge_in`) · `SettingsModal.tsx` · commandes `export_backup`, `import_backup`

---

## 9 · Sécurité & RGPD

🎯 Garder les données au bon endroit.

⚙️ Comment
- Calcul **100 % local** ; serveur llama fermé (clé API + `127.0.0.1`).
- **Ce qui sort** : le cerveau (labels + extraits) → Supabase (sync + fonction MCP pour Claude Desktop).
- Secrets Google **en clair** sur disque ; credentials bakés au build + PKCE.
- Sentry **opt-in strict** (flag + DSN) + scrubbing.
- Risques détaillés → `known-gaps.md`.

📁 `secrets.rs` · `connectors/google_drive.rs` · `ai/llama.rs` · `supabase/functions/lucid-mcp` · `capabilities/default.json`

---

## 10 · Packaging & distribution

🎯 Livrer `.dmg` / `.exe` avec l'IA embarquée.

⚙️ Comment
- CI (GitHub Actions) build Mac + Windows.
- `bundle-sidecars.{sh,ps1}` embarque llama + poppler + tesseract (statique Mac / DLL Windows).
- Updater signé (minisign), canaux stable/beta.
- Modèles téléchargés au 1er lancement (pas dans l'installeur).

📁 `.github/workflows/release.yml` · `scripts/bundle-sidecars.*` · `tauri*.conf.json` · `binaries/`
