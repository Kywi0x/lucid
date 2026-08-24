---
name: journal-commits
description: Prépare les messages de commit d'une feature validée par Liam et les dépose dans le compte-rendu du jour (08 Journal de bord du coffre LucidFlow). À lancer quand Liam dit qu'une feature/un fix marche, qu'il a testé dans l'app, qu'il valide, ou quand il demande « le nom des commits », « prépare les commits », « journalise ça ». Ne commite jamais.
---

# Commits + journal, à la validation

Déclencheur : **Liam a validé** (« ça marche », « c'est bon », « j'ai testé, ça tourne »).
Pas avant : un message de commit écrit sur du code non testé décrit une intention, pas un
changement.

## 1. Lire l'état réel, ne pas se souvenir

```bash
git log --oneline -10   # la convention se relit, elle ne se suppose pas
git status --short
git diff --stat
```

La convention du dépôt est `type:slug-en-kebab` (`fix:`, `feat:`, `update:`), sans espace
après les deux-points. **Vérifier à chaque fois** : si l'historique a bougé, suivre
l'historique, pas cette ligne.

## 2. Découper par sujet, pas par fichier

Un commit = un sujet. Deux chantiers sans rapport dans le working tree = deux commits,
même si Liam n'en a validé qu'un — le dire, et donner les deux.
Pour chacun : la ligne `git add <fichiers exacts>` puis le message.

Corps du message (2-5 lignes, seulement s'il apporte quelque chose) :
- **la cause réelle, pas le symptôme** — c'est ce qu'on viendra rechercher dans
  l'historique dans six mois (« accept() sans échéance », pas « la connexion buggait ») ;
- ce qui le prouve : tests verts, mesure, vérification dans l'app.
- Jamais de données personnelles ni de noms de dossiers clients (règle RGPD de l'orga) :
  parler de « un dossier de 7 .docx », pas du nom réel.

## 3. Déposer dans le compte-rendu du jour

Note du jour : `~/Documents/LucidFlow/08 Journal de bord/AAAA-MM-JJ — <sujet>.md`.
Absente → la créer depuis
`01 Méta & Templates/_Templates/Template — Compte-rendu de session.md`.

Y ajouter (ou compléter) une section, juste avant `## 🔗 Notes touchées` :

~~~markdown
## 🧾 Commits à passer

**1. <sujet>** — `git add <fichiers exacts>`

```
type:slug

Cause réelle, en 2-5 lignes. Preuve (tests, mesure, vérif dans l'app).
```
~~~

Un commit déjà passé : le laisser, ajouter son hash court devant le sujet. L'historique
du journal doit rester lisible en une passe.

## 4. Ne jamais commiter

Liam gère ses commits. Fournir le `git add` et le message, s'arrêter là — même si tout
est vert, même s'il ne reste « qu'à » lancer la commande.

## Après écriture dans le coffre

```bash
cd ~/Documents/LucidFlow && find . -name '*.md' -not -path '*/.obsidian/*' | wc -l
```
Le compte doit avoir monté ou être stable (jamais baissé). S'il baisse → STOP et alerter.
