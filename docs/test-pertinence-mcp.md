# Test de pertinence du MCP — 16 questions

Calibré sur le corpus réel du 2026-08-05 (529 pages). Reprend les catégories de la série
du 2026-08-03 (16/16 avec Claude), qui décide de la recherche sémantique :
`04 Stack technique/Options à évaluer/Recherche du cerveau — mots-clés actuels vs sémantique.md`.

## Pourquoi les questions ont changé

Les deux questions pivots de la série d'origine sont **grillées** : « trésorerie » (mot censé être
absent du corpus) et « taxe foncière » (contrôle négatif) matchent aujourd'hui — non pas dans de
vrais documents, mais dans **les notes qui parlent du test** (ADR-0022, la note ci-dessus, les
sessions Claude Code archivées). Le cerveau s'est auto-contaminé par sa propre méta-documentation.

### Les contrôles négatifs sont à USAGE UNIQUE

Deux vecteurs de contamination, et le second est le plus vicieux :

1. **La documentation du test elle-même.** La note de journal qui raconte une campagne cite les
   mots de contrôle → au scan suivant, ils sont dans le cerveau.
2. **Les sessions Claude Code.** Le connecteur indexe les transcriptions du terminal. Choisir un
   mot de contrôle *en en discutant dans une session Claude Code* le brûle avant même de l'avoir
   utilisé. **Choisis-le sans l'écrire dans une session, ou utilise-le le jour même.**

État au 2026-08-21 (sondé sur `brain.db` en incluant `source_text` — le contenu Obsidian n'est
PAS dans la colonne `content`, une sonde qui l'oublie répond 0 partout et rassure à tort) :

| Mot | Occurrences | État |
|---|---|---|
| `dentiste` | 2 | ❌ grillé par la note du 2026-08-05 |
| `succession` | 2 | ❌ grillé |
| `testament` | 1 | ❌ grillé |
| `hypothèque` | 0 | ✅ (et le piège tient : des dossiers immobiliers existent) |
| `kinésithérapeute` | 0 | ✅ vérifié aussi sur le space déployé le 21/08 |
| `plongée`, `aquarium`, `scooter` | 0 | ✅ en réserve |

Sonde locale (gratuite, instantanée) :

```bash
DB=~/Library/Application\ Support/com.lucidflow.lucid/users/<uuid>/brain.db
sqlite3 "$DB" "select count(*) from nodes where lower(label||' '||coalesce(summary,'')||' '||\
  coalesce(keywords,'')||' '||coalesce(content,'')||' '||coalesce(source_text,'')) like '%<mot>%';"
```

Confirmation sur le space réellement servi au MCP (l'URL se lit dans `.env.mcp`, jamais en
argument — un argument finit dans l'historique du shell) :

```bash
node scripts/measure-mcp-live.ts "<mot>"   # « Aucune des N pages… » = encore utilisable
```

## Protocole

1. Un client MCP = une campagne. Ordre suggéré : **Claude** (référence, déjà 16/16) →
   **Gemma local** ou **Gemini Flash** (le modèle faible qui tranche).
2. Une question = un fil neuf. Sinon le contexte du fil précédent répond à sa place.
3. Aucun indice : pas de nom de dossier, pas d'id dans la question.
4. Noter pour chaque question : ✅/❌, le nombre d'appels d'outils, et si le client a passé
   `query` à `brain_node` (sinon il lit des débuts de page au lieu des passages).

**Critère de réussite** : la réponse est juste **et** sourcée sur le cerveau. Une bonne réponse
inventée compte comme un échec — c'est tout l'objet des contrôles négatifs.

> [!warning] 🔒 = la réponse fait remonter des documents personnels réels
> Ces questions envoient au modèle des factures, contrats et pièces d'identité. Sans objet avec
> le Gemma local ; avec un modèle cloud (Gemini Flash…) c'est une sortie de données assumée.
> Pour une campagne cloud sans exposition, se limiter aux questions non marquées.

## Série factuelle (10)

| # | Question | Ce qu'elle teste | Attendu |
|---|---|---|---|
| 1 | 🔒 Combien de factures Xano ai-je, et sur quelle période ? | agrégation dans un thème (16 pages) | un compte + une plage de dates, pas une page unique |
| 2 | 🔒 Liste tous mes fournisseurs de services web / hébergement et ce que je paie à chacun. | agrégation **inter-dossiers** (Factures Services, Factures Xano, Non triable) | plusieurs fournisseurs venant de dossiers différents |
| 3 | Que dit le PDF du bot sniper BSC à l'étape 1 ? | extraction PDF + lecture d'un passage précis | l'étape d'installation, citée depuis le document |
| 4 | 🔒 Quel est le montant et l'échéance de ma dernière facture d'assurance auto ? | OCR sur PDF scanné + arbitrage entre plusieurs factures | montant + date, avec le document nommé |
| 5 | Quel ADR traite du rangement par embeddings, et quel est son statut ? | coffre Obsidian, frontmatter | ADR-0019, statut `en-cours` |
| 6 | Qu'a-t-on décidé sur la recherche sémantique, et qu'est-ce qui ferait rouvrir la décision ? | note longue, lecture d'un passage précis | différée + le déclencheur « modèle client plus faible » |
| 7 | Quelle est la répartition des parts entre Lucas et moi ? | note unique, réponse courte | 50-50 |
| 8 | Où est rangé le tuto de la chorale, et combien de documents contient-il ? | navigation d'arborescence, pas de contenu | le conteneur + 6 documents |
| 9 | Quel est le montant de mes frais de kinésithérapeute en 2025 ? | **contrôle négatif** (mot absent) | « je ne trouve pas » — aucune invention |
| 10 | À combien s'élève mon hypothèque sur l'appartement de Lyon ? | **contrôle négatif piégé** : `hypothèque` absent, mais un dossier « Immobilier Lyon » existe | « je ne trouve pas », sans broder à partir des documents immobiliers |

## Série conceptuelle (6)

| # | Question | Ce qu'elle teste | Attendu |
|---|---|---|---|
| 11 | Ai-je de quoi organiser ma succession ? | **vocabulaire absent du corpus** (`succession`, `testament`) | traduit le concept en mots présents (assurance, pièces d'identité, documents importants) et répond en inventaire |
| 12 | 🔒 Ai-je un document qui mentionne une vaccination ? | **mot noyé** : une occurrence dans un gros document | le retrouve — c'est aussi le test de la lecture ciblée (`query` + passages) |
| 13 | De quoi ai-je besoin pour constituer un dossier de location, côté locataire ? | **mot dans les MAUVAIS documents** : « location » et « bail » n'existent que dans les documents métier immobilier | répond depuis ses connaissances, utilise le cerveau comme inventaire, **sans** citer les annonces immobilières comme si c'était son dossier |
| 14 | Où en est le développement de Lucid : qu'est-ce qui est livré, qu'est-ce qui reste ? | synthèse multi-sources (journal + ADR + sessions Claude Code) | croise au moins deux types de sources |
| 15 | 🔒 Quels sont mes projets clients actifs, et pour chacun le dernier document en date ? | synthèse transverse sur ~6 thèmes | une liste de projets, chacun rattaché à un document |
| 16 | Ai-je des doublons évidents dans mes documents ? | raisonnement sur la structure plutôt que sur le contenu | s'appuie sur les titres/dossiers (« Non triable », versions de fichiers), admet la limite |

## Grille de score

```
Client : ..................  Date : ..........  Fonction lucid-mcp déployée le : ..........

Factuelles    1 2 3 4 5 6 7 8 9 10   → ..../10
Conceptuelles 11 12 13 14 15 16      → ..../6

Contrôles négatifs (9, 10) tenus sans invention ?   oui / non   ← éliminatoire
`query` passé à brain_node ?                        toujours / parfois / jamais
`full: true` réclamé ?                              jamais / ..... fois
Appels d'outils par question (médiane) :            ....
```

## Campagne 2 — après les correctifs du 2026-08-05

Ce qui a changé côté MCP entre la campagne 1 et maintenant : extraits de recherche **ciblés sur la
requête** (500 car. au lieu des 200 premiers), lecture de page en passages (`query`/`full`), réponse
d'absence explicite, IDF + accents + tokenisation. Tout est déployé.

**Résultats de la campagne 1 (référence)**

| Client | Score | Cause des échecs |
|---|---|---|
| Claude Sonnet 5 (claude.ai, free) | **14/14** (Q15-Q16 non posées, quota) | — |
| ChatGPT | **2/5** (quota épuisé à 6 questions) | ne lisait pas les pages : s'arrêtait à l'index et concluait « information non visible » |

**À refaire en priorité** — pas les 16 questions, seulement les 4 qui discriminent :

| # | Question | Ce qu'on veut voir |
|---|---|---|
| 1 | factures Xano + période | la période, pas seulement le compte (ChatGPT s'arrêtait au compte) |
| 2 | fournisseurs web + montants | des montants réels ; résiste aux deux méthodes de recherche, c'est le cas le plus dur |
| 4 | facture d'assurance | le bon document **et** le bon échéancier (ChatGPT a pris un prélèvement passé pour la prochaine échéance) |
| 9 ou 10 | contrôle négatif | toujours « je ne trouve pas », et en **1-2 recherches** au lieu de 8 |

Métrique à noter en plus du score : **le nombre d'appels d'outils par question**. C'est elle qui
décide de la suite, pas le score — un client à quota est limité par ses allers-retours.

> [!warning] Les mots de contrôle s'usent — et vite
> Au 2026-08-06, **les quatre mots déclarés libres la veille sont grillés** : `hypothèque`,
> `testament`, `succession`, `kitesurf` matchent tous. Ce n'est pas le corpus qui a changé, c'est
> ce fichier-ci et les notes du coffre qui ont été réindexés. La documentation du test contamine
> le test. Corollaire : **ne jamais réutiliser un mot déjà écrit ici** sans le re-sonder.
>
> ```bash
> node scripts/check-mcp-vocab.ts "<url-mcp>" mot1 mot2 mot3   # ABSENT = utilisable
> ```
>
> Sondés libres au 2026-08-06 : `cambriolage`, `obsèques`, `mariage`, `tutelle`, `douane`,
> `naturalisation`, `testamentaire`, `kinésithérapie`, `héritage`, `divorce` — donc à considérer
> comme grillés dès la prochaine réindexation, puisqu'ils sont maintenant écrits ici.
>
> La sonde sert aussi à trouver des **aiguilles** : un mot qui touche exactement 1 page est un bon
> candidat pour le test du mot noyé.

## Série conceptuelle — jeu recalibré du 2026-08-06

Le jeu d'origine (Q11-Q16) est inutilisable tel quel : son vocabulaire est grillé. Quatre questions
le remplacent, calibrées par sonde sur le corpus du 2026-08-06. **C'est cette série qui décide de la
recherche sémantique** — les questions factuelles, elles, sont réglées.

| # | Question | Ce qu'elle teste | Réussite | Échec |
|---|---|---|---|---|
| C1 | Ai-je ce qu'il faut pour déclarer un cambriolage à mon assurance ? | **vocabulaire absent** (`cambriolage` : 0 page) alors que les documents utiles existent | traduit le concept en mots présents (assurance, factures d'achat, pièces d'identité) et répond en inventaire | « je ne trouve rien sur un cambriolage » et s'arrête |
| C2 | De quoi ai-je besoin pour constituer un dossier de location, côté locataire ? | **mot dans les MAUVAIS documents** : `location`, `bail`, `locataire` saturent le top 8, tous professionnels | répond depuis ses connaissances, utilise le cerveau comme inventaire de ce qu'on possède | cite les documents immobiliers métier comme s'ils étaient le dossier personnel |
| C3 | 🔒 Ai-je un document qui mentionne un traitement orthodontique ? | **mot noyé** : `orthodontie` ne touche qu'1 page | le retrouve et le nomme | ne le trouve pas — le budget de passages est trop serré |
| C4 | Où en est mon dossier de naturalisation ? | **contrôle négatif piégé** : `naturalisation` absent, mais un thème « Identité & Papiers » existe | « je ne trouve pas », sans broder à partir des pièces d'identité | invente un dossier à partir des documents d'identité |

C1 est **la** question qui tranche : le mot n'est nulle part, les documents pertinents le sont. Si
elle tombe seule, on sait exactement ce que les embeddings achèteraient. Si C2 tombe, c'est un
problème de discernement du modèle, pas de récupération — aucun vecteur n'y changera rien.

### Résultats — ChatGPT, 2026-08-06

**4/4.** Avec les factuelles du même jour (Q1, Q2, Q10 : 3/3), **7/7 sur la journée**, contre 2/5 en
campagne 1. Les correctifs du 2026-08-06 sur `toolNode` sont entre les deux.

| # | Résultat | Observation |
|---|---|---|
| C1 | ✅ | tableau « trouvé / manquant », construit à partir de recherches sur assurance, RIB, factures |
| C2 | ✅ | liste standard croisée avec ses propres pièces — **aucun document immobilier métier cité** |
| C3 | ✅ | aiguille retrouvée dans un contrat d'assurance, et distinction « mentionne » vs « atteste d'un traitement » |
| C4 | ✅ | refus explicite, en qualifiant les pièces d'identité trouvées comme relevant d'autre chose |

**Le mécanisme de C1 est la vraie information.** `cambriolage` ne touche aucune page : ChatGPT n'a
pas fait de rapprochement sémantique, il a **reformulé la question lui-même** en mots du corpus,
puis cherché ceux-là. La traduction concept → vocabulaire s'est faite dans le modèle, pas dans la
recherche.

D'où la conclusion, et sa limite — le protocole d'origine ne séparait pas ces deux chemins :

- **Chemin MCP (clients cloud) : réglé.** Un client capable reformule seul, la recherche lexicale
  lui suffit, des vecteurs paieraient un travail déjà fait gratuitement.
- **Chemin assistant in-app (Gemma 3 4B local) : non testé, et non transposable.** Un petit modèle
  ne reformule pas avec la même aisance. C'est là que le banc local garde son mot à dire
  (`cargo run --example retrieval` : mots-clés 9/12, hybride 10/12, vecteurs seuls 5/12) — et c'est
  là, et seulement là, que la question de la sémantique reste ouverte.

## Côté assistant local (hors MCP)

Deux mesures reproductibles, sans modèle dans la boucle, sur le vrai cerveau :

```bash
cd src-tauri
cargo run --example retrieval              # mots-clés seuls   → 9/12 au 2026-08-05
HYBRID=1 cargo run --example retrieval     # + vecteurs        → 10/12
SEMANTIC=1 cargo run --example retrieval   # vecteurs seuls    → 5/12
```

Elles mesurent la **récupération** (la bonne page arrive-t-elle dans le contexte injecté ?), pas la
qualité de la réponse. Pour cette dernière il faut poser les questions dans l'app après un rebuild —
et privilégier les questions **factuelles** (montant, date, référence), car c'est l'injection du
passage qui est en test.

## Comment lire le résultat

| Résultat | Conclusion |
|---|---|
| modèle faible ≈ Claude | le cerveau est solide, la sémantique reste un luxe (décision ADR-0022 confirmée) |
| modèle faible s'effondre | la qualité était **empruntée au client** — « marche avec toutes tes IA » ne tient pas, la sémantique devient nécessaire |
| échecs concentrés sur 1-2-15 (agrégation) | ce n'est pas la recherche qui manque, c'est le **plafond de 8 résultats** de `brain_search` |
| échecs concentrés sur 12 (mot noyé) | le découpage en passages est trop strict — élargir le budget ou la fenêtre |
| contrôle négatif raté | problème de prompt/description d'outil, pas de récupération |
