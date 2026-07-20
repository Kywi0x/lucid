//! Dépose la note « Parcours de démo » dans le cerveau réel (id stable
//! `note-tour`, ré-exécutable). Lancement : `cargo run --example seed_note`.

const CONTENT: &str = r#"Une note pour parcourir Lucid dans l'ordre avant de le montrer à Lucas — environ 15 minutes, sur mes vraies données (pas de démo bidon). Pas besoin de connecter tous les connecteurs : UNE seule source déjà connectée suffit à raconter toute l'histoire (Claude Code est le moins parlant pour lui — préférer Notion, Drive ou un dossier local, plus proche de son quotidien business).

## 1. Le point de départ : le Mind

Vue par défaut à l'ouverture. Les bulles = ton cerveau : le centre, puis les projets, puis les concepts qui les relient. Zoom, déplace-toi, clique une bulle pour l'ouvrir. C'est la vue qui vend le produit d'un coup d'œil — commence toujours par là avec Lucas.

## 2. Montrer une vraie source connectée

Réglages (roue crantée, en haut à droite) → **Connexions**. Montre la source déjà connectée (peu importe laquelle) — le point à faire passer : zéro configuration côté user, l'app fait le travail toute seule. Pas besoin d'en ajouter d'autres pour la démo.

## 3. Régénérer le cerveau

Bouton **Régénérer** (icône ↻, en haut à droite, à côté de l'historique) : ça resynchronise les sources connectées et reconstruit le graphe. Regarde les bulles arriver en cascade — c'est le pipeline qui tourne en local, rien ne part sur un serveur.

## 4. Ouvrir un nœud

Clique n'importe quelle bulle : résumé généré par l'IA, mots-clés, décisions extraites, et les sources d'origine (avec lien vers l'original). C'est la valeur du produit en une seule vue — Lucas doit voir qu'on ne se contente pas de lister des fichiers, on en extrait du sens.

## 5. Les deux autres vues

En haut : **Mind** (les bulles) / **Dossiers** (arborescence classique, pour ceux qui préfèrent) / **brain.md** (le même cerveau en un seul markdown lisible et exportable — la preuve que rien n'est enfermé dans l'app).

## 6. Chercher — ⌘K

Raccourci **⌘K** : recherche instantanée dans tout le cerveau, sans quitter le clavier.

## 7. Écrire une note (comme celle-ci)

Bouton **+** en haut : crée une note manuelle, éditeur riche (titres, listes, tableaux, `/` pour l'IA, `[[` pour lier une autre page). Montre qu'on n'est pas juste un visualiseur passif : on prend aussi des notes ici.

## 8. Revenir en arrière — Historique

Icône horloge (à côté de Régénérer) : tous les snapshots pris avant chaque régénération/suppression. Un clic restaure. Bon argument sécurité/confiance pour Lucas : rien n'est jamais perdu en silence.

## 9. Organiser — Espaces

Rail gauche → **Spaces** : sous-ensembles du cerveau (ex. juste les nœuds d'un projet client). Crée-en un, ajoute quelques nœuds. C'est la brique qui permettra un jour de partager UN espace précis sans exposer tout le cerveau.

## 10. Discuter avec Lucid IA

Rail gauche → **Lucid IA** : chat qui répond en s'appuyant sur le contenu du cerveau (pas un LLM générique — il cite les sources). Bon moment pour montrer une vraie question métier plutôt qu'un test bidon.

## 11. Le compte — sync cloud

Réglages → **Compte** : connexion (email + mot de passe), et le cerveau se synchronise tout seul entre machines — utile pour le pitch "commence sur ton laptop, continue sur ton fixe". L'état de la sync (à jour / en cours / erreur) est visible en direct, jamais un échec silencieux.

## 12. Les mises à jour automatiques

Dernier point technique mais qui compte pour la crédibilité : Lucid se met à jour tout seul (bannière discrète en bas à droite quand une nouvelle version existe). Pas besoin de renvoyer un installeur à chaque correctif.

---

**Pitch de clôture pour Lucas** : Lucid transforme des conversations et documents éparpillés en un cerveau structuré et navigable, 100 % local pour l'analyse, synchronisé entre machines pour le confort — sans dépendre d'un serveur qui lit tes données. Le connecteur montré aujourd'hui n'est qu'un exemple : le pipeline est agnostique de la source, chaque nouveau connecteur branche la même mécanique sans rien changer au graphe ni à l'UI."#;

fn main() {
    match brainlink_lib::seed_walkthrough_note(CONTENT) {
        Ok(id) => eprintln!("✓ Note « Parcours de démo » prête (id: {id}). Ouvre l'app et cherche 🧭 dans ⌘K."),
        Err(e) => eprintln!("Erreur : {e}"),
    }
}
