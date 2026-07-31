# Lucid — documentation technique

> **Source de vérité = le code.** Cette doc vit dans le repo, versionnée avec le code : quand le
> code change, la doc change dans le même commit. Elle est dérivée d'une lecture directe du code
> (pas des notes Obsidian, qui décrivent la *vision* et peuvent diverger de l'app réelle).
>
> Dernière passe : 2026-07-30.

## C'est quoi
App **desktop Tauri 2** (Rust + React/TS) qui agrège tes sources (fichiers, Notes, Drive, sessions IA),
en extrait le texte **100 % en local**, en construit un **graphe de connaissances** (« le cerveau »)
affiché en carte à bulles, et le sert à tes IA. Local-first : le calcul reste sur la machine, seul le
cerveau (~2 Mo) se synchronise via le compte.

## Le flux
```
Sources            Extraction              Cerveau                Sorties
fichiers, Notes,  → texte (source_text)  → graphe brain.db     → • carte à bulles (Canvas)
Drive, Claude…      + IA (résumé/thèmes)    (nœuds + liens)        • pages markdown
                                                                   • chat IA local
                                                                   • MCP (Claude Desktop)
```

## Stack (réelle, d'après les manifests)
- **Desktop** : Tauri 2. **Front** : React 19 + TS + Vite, Tailwind v4, `@xyflow/react` + `d3-force` (carte), TipTap + react-markdown (éditeur).
- **Rust** : `rusqlite` (SQLite embarqué, `bundled`), `reqwest` (rustls), `notify` (watch fs), `calamine` (.xlsx), `pdf-extract` (PDF pur Rust), `zip`, `getrandom`.
- **IA locale** : `llama-server` (llama.cpp) + **Gemma 4 E4B** (génération) + **BGE-M3** (embeddings).
- **Cloud (optionnel)** : Supabase (auth + sync du cerveau + fonction MCP distante).

## Carte des modules

**Rust — `src-tauri/src/`**
| Fichier | Rôle |
|---|---|
| `lib.rs` | Point d'entrée : ~90 commandes Tauri, watchers fs, Inbox, bootstrap IA, extraction (`file_to_source_text`) |
| `models.rs` | Structs partagées (`BrainNode`, `BrainGraph`, `Conversation`…) |
| `storage.rs` | SQLite `brain.db` (schéma `nodes`/`edges`/`meta`) |
| `backup.rs` | Sérialisation cerveau, sync/merge last-write-wins, tombstones, export/import |
| `archivist.rs` | Rangement : dédup titres + clustering embeddings + domaines |
| `ai/llama.rs` | Moteurs llama-server (génération + embeddings), catalogue/DL modèles |
| `ai/pipeline.rs` | Extraction par conv → agrégation → graphe |
| `ai/brain_md.rs` | Sérialisation `brain.md` |
| `secrets.rs` | Masquage de secrets dans le contenu |
| `connectors/*` | 7 sources : `claude_code`, `claude_ai`, `chatgpt`, `google_drive`, `obsidian`, `apple_notes`, `local_folder` |

**Front — `src/`**
| Fichier | Rôle |
|---|---|
| `App.tsx` | Orchestration : état global, watchers d'events, déclencheurs Archiviste |
| `lib/api.ts` | Pont vers les commandes Tauri |
| `lib/sync.ts` | Sync cloud (⚠️ auto désactivée aujourd'hui) |
| `lib/share.ts` | Spaces partagés + MCP personnel |
| `components/BrainMap.tsx` | Carte à bulles (Canvas + d3-force) |
| `components/NodeDetail.tsx` | Page d'un nœud (éditeur markdown, résumé, sources) |
| `components/SettingsModal.tsx` | Connecteurs, modèle IA, compte, diagnostic |
| `components/SetupScreen.tsx` | Bootstrap IA au 1er lancement |
| `components/InboxPanel.tsx` | Flux « Fichiers récents » |

## Où lire quoi
- **[architecture.md](architecture.md)** — comment chaque brique marche + *où dans le code*.
- **[known-gaps.md](known-gaps.md)** — trous, dette technique et évolutions (audit du code).

*(La vision produit/business et les décisions d'archi (ADR) vivent dans le vault Obsidian LucidFlow. Ici = le technique « comment ça marche vraiment ».)*
