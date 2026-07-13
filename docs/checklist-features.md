# Checklist features — Lucid

> Liste de tout ce qui existe dans l'app et de quoi vérifier avant une release.
> Règle : chaque feature se teste **sur Mac ET sur Windows** (parité obligatoire, cf. CLAUDE.md).
> Cocher = testé et fonctionnel sur les deux plateformes (ou noter `mac only ⚠️` en attendant).

## 🔐 Compte (Supabase — obligatoire)

- [ ] Écran de connexion s'affiche au lancement si pas de session (AuthGate)
- [ ] Création de compte (email + mot de passe ≥ 6 caractères)
- [ ] Message « vérifie ta boîte mail » si la confirmation email est activée côté Supabase
- [ ] Connexion avec un compte existant
- [ ] Mauvais mot de passe → message d'erreur lisible (pas de « Load failed »)
- [ ] Session persistée : relancer l'app → pas de re-login demandé
- [ ] Démarrage **hors-ligne** avec une session existante → l'app s'ouvre quand même
- [ ] Déconnexion (Réglages → Compte) → retour à l'écran de login
- [ ] `.env` sans clés Supabase → écran « Configuration manquante » explicite

## 🧠 IA locale

- [ ] Premier lancement : SetupScreen propose le téléchargement du modèle
- [ ] Téléchargement du modèle avec barre de progression
- [ ] Installation depuis un fichier `.gguf` local
- [ ] Changement de modèle actif (Réglages → Modèle)
- [ ] « Continuer sans IA » → l'app marche, les features IA sont grisées (pas d'échec silencieux)
- [ ] Barre de statut IA (AiStatusBar) reflète l'état réel

## 🔌 Connecteurs

- [ ] **Claude Code** : détection auto de `~/.claude/projects/`, connexion/déconnexion/reconnexion
- [ ] **claude.ai** : import de l'export `.zip`
- [ ] **ChatGPT** : import de l'export
- [ ] **Google Drive** : saisie credentials OAuth, connexion, sync, déconnexion
- [ ] Google Drive Windows : les PDF sont bien synchronisés (régression connue à surveiller)
- [ ] **Notion** : connexion, sync, chargement de pages, déconnexion
- [ ] **Obsidian** : choix du vault, déconnexion
- [ ] **Dossier local** : choix du dossier, sync, déconnexion
- [ ] Statut des connecteurs correct dans Réglages → Connecteurs
- [ ] **Clients IA externes** : statut / connexion / déconnexion

## 🕸️ Graphe & génération

- [ ] Génération du cerveau (bouton + progression affichée)
- [ ] `brain.json` + `brain.md` écrits dans le dossier de données
- [ ] Relance de l'app → le graphe se recharge depuis `brain.json`
- [ ] Bulles : cerveau → projets → concepts, ponts entre projets (concepts partagés)
- [ ] Drill-down sur un nœud : résumé, mots-clés, décisions, **sources cliquables** (SourceRef)
- [ ] Déplacer un nœud vers un autre parent (set_node_parent)
- [ ] Renommer un nœud
- [ ] Vue Dossiers (FolderView) cohérente avec le graphe
- [ ] Vue brain.md (markdown rendu)

## ✍️ Contenu & notes

- [ ] Créer une note (create_note_node) + éditeur markdown
- [ ] Sauvegarde / rechargement du contenu d'un nœud
- [ ] Synthèse IA d'un nœud (synthesize_node)
- [ ] Génération de contenu IA (AiDraft)
- [ ] Création de structure (create_structure)
- [ ] Import de fichier (import_file) — PDF, docx… ; `.doc` legacy → message explicite
- [ ] Coller une image (save_pasted_image)
- [ ] Export d'un nœud en `.md`
- [ ] Propriétés d'un nœud (Properties)

## 🗂️ Spaces

- [ ] Créer / renommer / supprimer un space
- [ ] Ajouter / retirer un nœud d'un space
- [ ] Filtrage du graphe par space (ancêtres inclus)
- [ ] Export d'un space en `.md`

## 💬 Assistant

- [ ] Question au cerveau (ask_brain) avec réponse sourcée
- [ ] Question sur un nœud précis (ask_node)

## 🤖 MCP

- [ ] Propositions MCP listées (list_mcp_proposals)
- [ ] Accepter / refuser une proposition
- [ ] MCP fonctionne sur Windows (fix récent `fix:mcp-windows` — re-tester)

## 💾 Sauvegardes & historique

- [ ] Backup cloud (Réglages → Compte) : upload vers le bucket `backups`
- [ ] Liste des sauvegardes + restauration (copie locale de brain.json gardée)
- [ ] Snapshots locaux : liste + restauration
- [ ] Snapshots par nœud : liste + lecture

## 🎨 UI / UX

- [ ] Onboarding au premier lancement
- [ ] Checklist de démarrage (StarterChecklist)
- [ ] Mode démo : seed + reset
- [ ] Palette de commandes ⌘K
- [ ] Thème clair / sombre : bascule + suivi du thème système — **chaque écran lisible dans les deux**
- [ ] Recherche
- [ ] Écran de login correct dans les deux thèmes

## 📦 Release (à chaque build)

- [ ] `npx tsc --noEmit` sans erreur
- [ ] `cd src-tauri && cargo test --lib` vert
- [ ] Build `.dmg` Mac ok
- [ ] Build `.exe` Windows ok (sidecars `binaries/windows/` embarqués)
- [ ] Modifs de `BrainNode`/`BrainGraph`/`SourceRef` → `brain.json` régénéré
- [ ] Aucune clé/secret dans le bundle ni dans git (`.env` ignoré)
