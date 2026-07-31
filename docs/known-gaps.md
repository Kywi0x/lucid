# Trous connus, dette technique & évolutions

> Audit direct du code · 2026-07-30 · sévérité indicative.

## 🔴 Perte de données / sync

| Problème | Où | Évolution possible |
|---|---|---|
| **Sync auto désactivée** — pas de synchro temps réel entre machines ; un pull ne rafraîchit pas l'UI | `App.tsx` (`startAutoSync` court-circuité), `lib/sync.ts` | Réactiver + brancher le refresh au pull |
| **Restore n'invalide pas le cache mémoire** — restauration sans effet jusqu'au redémarrage | `backup.rs::import_in` | `clear_cache` après restore |
| **LWW nœud-entier** — éditions concurrentes du même nœud : l'une écrase l'autre ; delete gagne toujours ; tombstones jamais purgées | `backup.rs::merge_in` | Merge par champ + purge tombstones + horloge logique |
| **Backups manuels sans rotation** — stockage cloud qui gonfle | `SettingsModal.tsx` | Quota / rotation |

## 🟠 Échecs silencieux connecteurs (violent ADR-0015)

| Problème | Où | Évolution possible |
|---|---|---|
| **Drive jette Docs/Sheets/.docx/.xlsx/.csv sans un mot** (l'anti-exemple du CLAUDE.md) | `google_drive.rs:448` | Liste canonique + export types Google + liste « skipped » |
| **OCR Drive cassé sur Windows** (chemins `/opt/homebrew` en dur) | `google_drive.rs:764` | Résoudre le sidecar déjà embarqué (`bundle-sidecars.ps1`) |
| **Token Drive expiré = détection morte en silence** | `google_drive.rs:339` | Proposer une reconnexion |
| **Dossier bloqué TCC macOS = 0 fichier, aucun message** | `local_folder.rs:144`, `obsidian.rs:95` | Modal « autoriser dans Réglages Système » |

## 🔵 Sécurité / RGPD (données clients finaux)

| Problème | Où | Évolution possible |
|---|---|---|
| **Cerveau (labels + extraits docs) auto-uploadé vers Supabase** (by-design MCP, à assumer) | `lib/sync.ts` → `lib/share.ts` | Documenter / durcir le périmètre exposé |
| **Tokens OAuth + client_secret en clair** sur disque | `connectors/google_drive.rs` | Keychain OS |
| **`lucid.log` expose des noms de fichiers/dossiers (= clients) via le Diagnostic** | `lib.rs::ai_diagnostics` | Scrub du tail de log |
| **MCP distant : token en URL, CORS `*`, service-role bypass RLS** | `supabase/functions/lucid-mcp` | Token en header + révocable |
| **Fixture test = vraies données perso** | `tests/fixtures/brain.liam.json` | Fixture fictif |
| **Capability `opener` = `path:"**"`** (tout chemin) | `capabilities/default.json` | Restreindre |

## 🟢 Robustesse / échelle

| Problème | Où | Évolution possible |
|---|---|---|
| **Drive : pas de `changes` token** → re-liste tout à chaque sync + poll | `google_drive.rs` | Delta-token Drive |
| **Apple Notes : import tout-ou-rien** (une note verrouillée casse tout), pas de timeout | `apple_notes.rs` | Par lots + timeout + skip note KO |
| **Symlinks cycliques → récursion infinie** | `local_folder.rs:143`, `obsidian.rs` | Set `visited` |
| **`free_port` fait `kill -9`** sur tout PID du port | `ai/llama.rs` | Vérifier que c'est bien Lucid |

## 🟣 Vision / features à trancher ou finir

| Sujet | État | Note |
|---|---|---|
| **Politique de structure par source** | Proposé (ADR-0020) | Apple Notes/Drive pas rangés thématiquement |
| **Modif d'une source déjà fusionnée = perte silencieuse** | À faire | Tombstone à **signature de contenu** (ressuscite si le contenu change) |
| **Extraction structurée** (DOCX → markdown riche) | Optionnel | Seulement si l'affichage devient un besoin ; PDF non fiable |
| **`container_path` vide** (claude.ai / ChatGPT) | Dette | Pas de hiérarchie projet — `claude_ai.rs:94` |
| **Bootstrap 1er lancement + build Windows** | Codés, non testés | À valider en réel |
| **Inbox** | Limite | Ne capte pas les changements Drive au fichier |

## Résidus de code

- Bloc `— DEBUG temporaire —` → `google_drive.rs:435`
- `// TODO chantier 5` → `claude_ai.rs:94`
