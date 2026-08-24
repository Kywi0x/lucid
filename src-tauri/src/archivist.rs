//! L'Archiviste — curation locale du cerveau : un script (titres/structure)
//! fait la grosse majorité du rangement, Gemma n'intervient que pour trancher
//! les cas ambigus (fusionner un groupe de titres proches, ou les garder
//! séparés). Aucune écriture directe : tout sort en proposition `mcp_pending/`,
//! comme le MCP — revue/acceptation par l'user, rien n'est jamais perdu.
//!
//! `summary`/`keywords` par page ne sont PAS utilisés : ils sont vides pour
//! toute note créée manuellement ou via MCP (l'extraction IA par page est
//! désactivée, cf. `ai::pipeline::AI_EXTRACTION`). La couche 2 lit donc un
//! extrait du `content` réel à la demande plutôt que de dépendre d'un résumé
//! pré-calculé.

use crate::ai::LlamaEngine;
use crate::models::BrainNode;
use crate::BrainGraph;
use std::collections::{HashMap, HashSet};

fn normalize(label: &str) -> String {
    label.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tokens (mots de 3+ caractères) d'un titre normalisé, pour un score de
/// recouvrement simple — pas de tokenizer, pas de dépendance, juste split.
fn tokens(label: &str) -> HashSet<String> {
    normalize(label)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() > 2)
        .map(str::to_string)
        .collect()
}

/// Deux tokens "matchent" s'ils sont égaux, ou si l'un est un PRÉFIXE/SUFFIXE
/// de l'autre — nécessaire pour les mots composés ("Cybersécurité" doit
/// matcher "Sécurité", accolé en suffixe). Un containment n'importe où dans le
/// mot (pas juste bord à bord) donne trop de faux positifs — ex. "informations"
/// contient "form" en plein milieu (in-FORM-ations) sans aucun rapport de sens ;
/// bug réel trouvé en testant sur un vrai cerveau (2026-07-22).
fn tokens_match(a: &str, b: &str) -> bool {
    // Seuil à 5+ caractères (pas 4) : un radical trop court ("form") revient
    // dans trop de mots français sans rapport ("formation", "formulaire") —
    // bug réel trouvé en testant sur un vrai cerveau (2026-07-22), "Formations"
    // matchait à tort un conteneur "JS Form" sans lien de sens.
    a == b
        || (a.len() >= 5 && (b.starts_with(a) || b.ends_with(a)))
        || (b.len() >= 5 && (a.starts_with(b) || a.ends_with(b)))
}

fn overlap_score(a: &HashSet<String>, b: &HashSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let matched_a = a.iter().filter(|ta| b.iter().any(|tb| tokens_match(ta, tb))).count();
    let matched_b = b.iter().filter(|tb| a.iter().any(|ta| tokens_match(ta, tb))).count();
    let inter = matched_a.max(matched_b) as f32;
    let union = (a.len() + b.len()) as f32 - inter;
    inter / union
}

pub struct MoveSuggestion {
    pub node_id: String,
    pub new_parent_id: String,
    pub node_label: String,
    pub target_label: String,
}

pub struct DuplicateGroup {
    pub label: String,
    pub node_ids: Vec<String>,
}

pub struct ScanResult {
    pub moves: Vec<MoveSuggestion>,
    pub groups: Vec<DuplicateGroup>,
    /// Labels (affichage/rapport) des orphelins sans destination thématique
    /// confiante ET sans bac "Non triable" encore présent DANS CE graphe —
    /// l'orchestrateur (lib.rs) les route vers le bac dans le même passage
    /// (le circuit de résolution des propositions gère déjà "créer un dossier"
    /// + "y déplacer des pages" en un seul lot), cf. `orphans_unresolved_ids`.
    pub orphans_unresolved: Vec<String>,
    /// Ids des mêmes orphelins, même ordre — nécessaires pour les déplacer
    /// (le libellé seul ne suffit pas à écrire une proposition "move").
    pub orphans_unresolved_ids: Vec<String>,
    /// Groupes détectés parmi les enfants du bac "Non triable" — un mot
    /// significatif partagé par 3+ pages (script, zéro IA). Ce que ce script
    /// rate (même sujet, aucun mot partagé) est le travail d'une passe IA
    /// séparée sur les mêmes candidats — cf. `ai_cluster_leftovers`.
    pub theme_clusters: Vec<ThemeCluster>,
    /// Documents au texte IDENTIQUE, premier id = survivant. Bande « certain » :
    /// l'orchestrateur émet directement une fusion, sans passer par Gemma.
    pub exact_duplicates: Vec<Vec<String>>,
    /// Id RÉEL du bac "Non triable" s'il existe déjà dans ce graphe — peut
    /// différer de la constante `CATCHALL_ID` (dossier créé manuellement avant
    /// l'Archiviste, ou par une version antérieure). `None` : à créer sous
    /// `CATCHALL_ID`, cf. `needs_catchall`. Exposé pour que l'orchestrateur
    /// (lib.rs) route vers le bon id sans redupliquer cette recherche.
    pub catchall_id: Option<String>,
}

pub struct ThemeCluster {
    pub label: String,
    pub node_ids: Vec<String>,
}

// Seuils choisis pour être conservateurs : mieux vaut laisser un orphelin de
// côté (log) que le ranger au mauvais endroit sans confirmation humaine.
const MIN_OVERLAP: f32 = 0.34;
const MIN_MARGIN: f32 = 0.15;

/// Bac de repli pour les fichiers sans destination thématique confiante —
/// mieux qu'un aveu passif ("laissé de côté") : ils sortent quand même de
/// Downloads, juste dans une catégorie honnête plutôt que devinée.
pub const CATCHALL_LABEL: &str = "Non triable";
/// Id stable (pas un timestamp) : un run qui retrouve ce nœud (créé par un
/// run précédent) ne le recrée pas — idempotent par construction.
pub const CATCHALL_ID: &str = "arch-non-triable";

/// Sources « à trier » (politique de structure, ADR-0020) : l'Archiviste a le
/// droit de REGARDER leur contenu. Ça ne veut pas dire tout déplacer — ce qui est
/// réellement rangé est protégé juste après par `RESPECT_SUBFOLDERS`.
/// - `local-folder` : dumps Bureau/Documents/Téléchargements.
/// - `apple-notes` : app de notes, pas de vraie hiérarchie (décision Liam 2026-07-30).
/// - `google-drive` : niveau 2 d'ADR-0020, « respecter + compléter » — l'arbo est
///   gardée, mais le vrac posé à la racine du Drive doit être rangeable
///   (décision Liam 2026-08-19 : jusqu'ici Drive était en tout-ou-rien, absent de
///   cette liste, donc même ses fichiers en vrac n'étaient jamais examinés).
const SORTABLE_CONNECTORS: &[&str] = &["local-folder", "apple-notes", "google-drive"];

/// Sources dont les SOUS-dossiers ne sont pas des destinations valables : ranger
/// un document d'un sous-dossier de Téléchargements vers un autre, c'est déplacer
/// du vrac dans du vrac. Sous-ensemble de `SORTABLE_CONNECTORS` — Drive en est
/// absent exprès : `Drive/Ideeri/Factures` est un vrai classement humain, il doit
/// rester une cible (c'est déjà la règle de `RECEIVING_SOURCES`).
const DUMP_CONNECTORS: &[&str] = &["local-folder", "apple-notes"];


/// Sources dont les dossiers peuvent RECEVOIR un document venu d'ailleurs
/// (ADR-0022 phase 1.3). Volontairement restrictif.
///
/// Obsidian et Claude Code en sont exclus : ce ne sont pas des systèmes de
/// classement pour documents arbitraires, ce sont des **corpus autonomes**. Un vault
/// est une base de connaissances curée, un dépôt Claude Code est un projet — y
/// injecter un PDF venu de Téléchargements pollue une structure à forte autorité,
/// même si ça ne touche pas la source réelle. ADR-0020 classe d'ailleurs le MCP en
/// « on ne touche pas », et recevoir est une forme de toucher.
///
/// Apple Notes en est exclu aussi, mais pour une autre raison : ADR-0020 niveau 3
/// le classe « toujours ranger », donc ses dossiers ne sont pas un signal fiable.
///
/// Constaté le 2026-08-03 : aucune contamination ne s'est produite sur la première
/// passe (aucun document local n'était assez proche d'un dossier Obsidian), mais
/// rien ne l'empêchait — le risque était latent, il est maintenant fermé.
pub const RECEIVING_SOURCES: &[&str] = &["local-folder", "google-drive"];

/// Sources triables dont on respecte tout de même les SOUS-dossiers (ADR-0022
/// phase 1.2) — quel que soit le nombre de documents qu'ils contiennent : un
/// dossier nommé avec un seul fichier est un classement qui attend d'être rempli,
/// pas un accident (débat tranché avec Liam le 2026-08-19). Apple Notes en est
/// absent exprès : ADR-0020 le classe niveau 3 (« toujours ranger »), ses dossiers
/// ne sont pas un signal fiable.
const RESPECT_SUBFOLDERS: &[&str] = &["local-folder", "google-drive"];

/// `true` si le bac "Non triable" n'existe pas encore dans ce graphe — à
/// l'orchestrateur (lib.rs) de le créer avant le prochain passage.
/// Peu importe le `kind` : une proposition "create" produit toujours un nœud
/// `note` (limite connue du circuit MCP, pas de type "container" à la création)
/// — on ne teste donc que le libellé, sinon on retente de le créer en boucle.
pub fn needs_catchall(graph: &BrainGraph) -> bool {
    !graph.nodes.iter().any(|n| normalize(&n.label) == normalize(CATCHALL_LABEL))
}

/// Chaîne des ancêtres d'un nœud (parent, grand-parent...), sans la racine.
fn ancestor_chain<'a>(id: &str, parent_of: &HashMap<&'a str, &'a str>) -> Vec<&'a str> {
    let mut chain = Vec::new();
    let mut cur = id.to_string();
    for _ in 0..16 {
        let Some(&p) = parent_of.get(cur.as_str()) else { break };
        chain.push(p);
        cur = p.to_string();
    }
    chain
}

/// Couche script (zéro IA) : doublons de titre + orphelins avec un parent
/// candidat sans ambiguïté. Pur, testable sans I/O.
///
/// Traite aussi les fichiers imbriqués dans un dossier de scan brut (Bureau/
/// Documents/Téléchargements — nom variable selon l'OS/la langue, jamais codé
/// en dur ici) : ces dossiers sont repérés par leur CONTENU — au moins une
/// feuille `connector == "local-folder"` en dessous — pas par leur nom. Tout
/// fichier encore quelque part sous un tel dossier est un candidat au
/// rattachement vers un vrai dossier thématique, à n'importe quelle profondeur.
/// `respect_user_folders` (ADR-0022 phase 1.2) : quand il est vrai, un document
/// d'une source triable rangé dans un SOUS-dossier créé par l'humain n'est plus
/// candidat — seul le vrac posé DIRECTEMENT dans un dossier scanné l'est. Le nom
/// qu'un utilisateur a donné à un dossier est une étiquette sémantique humaine,
/// meilleure que tout ce qu'un modèle de 4 Md peut inférer ; la dissoudre était du
/// gâchis de signal. Piloté par réglage : marche arrière sans recompilation.
pub fn scan(graph: &BrainGraph, respect_user_folders: bool) -> ScanResult {
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());

    let mut by_label: HashMap<String, Vec<&BrainNode>> = HashMap::new();
    for n in &graph.nodes {
        if n.kind == "root" {
            continue;
        }
        by_label.entry(normalize(&n.label)).or_default().push(n);
    }
    // Doublons EXACTS d'abord : la certitude prime sur l'inférence. Les documents
    // absorbés sont retirés des groupes de titres (inutile de faire trancher Gemma
    // sur un cas déjà résolu) ET des candidats au rangement (ils vont disparaître,
    // leur proposer un déplacement en plus créerait une proposition bloquée à vie).
    let exact_duplicates = exact_duplicates(graph);
    let absorbed: HashSet<String> = exact_duplicates.iter()
        .flat_map(|g| g[1..].iter().cloned())
        .collect();

    let groups: Vec<DuplicateGroup> = by_label
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(label, v)| DuplicateGroup {
            label,
            node_ids: v.iter().map(|n| n.id.clone()).filter(|id| !absorbed.contains(id)).collect(),
        })
        .filter(|g| g.node_ids.len() > 1)
        .collect();
    let grouped_ids: HashSet<&str> =
        groups.iter().flat_map(|g| g.node_ids.iter().map(String::as_str)).collect();

    let parent_of: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.parent_id.as_deref().map(|p| (n.id.as_str(), p)))
        .collect();

    // Dossiers de scan « à trier » : conteneurs sous la racine avec au moins une
    // feuille d'une SOURCE À TRIER en dessous (local-folder, apple-notes… cf.
    // SORTABLE_CONNECTORS / ADR-0020). Leur contenu est du vrac → candidat au
    // rangement thématique ; le conteneur lui-même n'est jamais une cible.
    let scan_root_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|c| c.kind == "container" && c.parent_id.as_deref() == root_id.as_deref())
        .filter(|c| {
            graph.nodes.iter().any(|n| {
                n.connector.as_deref().is_some_and(|conn| SORTABLE_CONNECTORS.contains(&conn))
                    && ancestor_chain(&n.id, &parent_of).contains(&c.id.as_str())
            })
        })
        .map(|c| c.id.as_str())
        .collect();
    let under_scan_root = |id: &str| ancestor_chain(id, &parent_of).iter().any(|a| scan_root_ids.contains(a));

    // Sous-ensemble « dump » des dossiers de scan : leurs SOUS-dossiers ne sont
    // pas des destinations (vrac vers vrac). Séparé de `scan_root_ids` depuis que
    // Drive est triable : la racine du Drive n'est pas une cible, mais ses
    // sous-dossiers en sont de vraies (ADR-0022 phase 1.3, `RECEIVING_SOURCES`).
    let dump_root_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|c| scan_root_ids.contains(c.id.as_str()))
        .filter(|c| {
            graph.nodes.iter().any(|n| {
                n.connector.as_deref().is_some_and(|conn| DUMP_CONNECTORS.contains(&conn))
                    && ancestor_chain(&n.id, &parent_of).contains(&c.id.as_str())
            })
        })
        .map(|c| c.id.as_str())
        .collect();
    let under_dump_root = |id: &str| ancestor_chain(id, &parent_of).iter().any(|a| dump_root_ids.contains(a));

    // Conteneurs candidats pour le rattachement — jamais la racine, jamais un
    // dossier de scan brut ni un sous-dossier de dump (on range VERS un vrai
    // dossier thématique, pas d'un sous-dossier de Downloads vers un autre).
    let container_tokens: Vec<(&BrainNode, HashSet<String>)> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "container" && Some(&n.id) != root_id.as_ref())
        .filter(|c| !scan_root_ids.contains(c.id.as_str()) && !under_dump_root(&c.id))
        .map(|n| (n, tokens(&n.label)))
        .collect();

    // Conteneur "attrape-tout" déjà créé par un run précédent (cf. `catchall_name`
    // en constante, jamais deviné : soit il existe déjà, soit on le laisse à
    // l'orchestrateur (lib.rs) de le créer avant le prochain passage).
    let catchall_id: Option<&str> = graph
        .nodes
        .iter()
        .find(|n| normalize(&n.label) == normalize(CATCHALL_LABEL) && !under_scan_root(&n.id))
        .map(|n| n.id.as_str());

    // Un nœud qui a déjà des enfants agit comme un hub (bac, thème créé par un
    // passage précédent...), même en `kind: note` — jamais un orphelin à ranger
    // ailleurs, quel que soit son propre kind. Bug réel : "Medecine" (thème
    // fraîchement créé) se faisait renvoyer vers "Non triable" par cette même
    // boucle avant cette garde (2026-07-22).
    let has_children: HashSet<&str> = graph.nodes.iter().filter_map(|n| n.parent_id.as_deref()).collect();

    let mut moves = Vec::new();
    // Candidats au bac "Non triable" — QUE le bac existe déjà ou pas encore,
    // rassemblés dans UN SEUL pool avant de trancher où ils vont vraiment.
    // Avant ce fix, un orphelin partait par un chemin DIFFÉRENT selon que
    // `catchall_id` était `Some` (déplacement direct vers le bac, jamais vu
    // du clustering) ou `None` (via `orphans_unresolved`, seul chemin
    // effectivement considéré pour le clustering) — dès qu'un premier passage
    // avait déjà créé le bac, tout passage suivant resweepait ses nouveaux
    // orphelins À PLAT dans le bac sans jamais retenter le regroupement par
    // thème (bug réel, 2026-07-23 : 20 factures/devis avec un mot commun dans
    // le titre ne se groupaient plus une fois le bac déjà présent).
    let mut catchall_bound: Vec<(String, String)> = Vec::new(); // (id, label)
    if let Some(root) = &root_id {
        for n in &graph.nodes {
            // Le bac lui-même n'est jamais un candidat (protège aussi le tout
            // premier passage, où il vient d'être créé et n'a encore aucun enfant).
            // Idem pour un nœud-entité (société/client) : c'est un hub relié par
            // des `link`, pas par des `contains`, donc absent de `has_children` —
            // sans ce garde il serait rangé comme un orphelin dans un thème.
            if Some(n.id.as_str()) == catchall_id
                || has_children.contains(n.id.as_str())
                || n.id.starts_with("arch-entity-")
            {
                continue;
            }
            let direct_root_child = n.parent_id.as_ref() == Some(root);
            let nested_in_scan_root = under_scan_root(&n.id);
            if !direct_root_child && !nested_in_scan_root {
                continue; // déjà dans un dossier thématique choisi, on n'y touche pas
            }
            // Structure montée à la main : un fichier dans `Documents/Clients/X/` a
            // été rangé par un humain, contrairement à celui posé dans `Documents/`.
            // On ne garde donc comme candidat que le vrac DIRECTEMENT dans un dossier
            // scanné. Ne s'applique qu'aux sources de `RESPECT_SUBFOLDERS`.
            //
            // Le nombre de documents dans le sous-dossier n'entre PAS en compte
            // (débat du 2026-08-19) : un dossier nommé qui n'en contient qu'un est
            // un classement qui attend d'être rempli — et c'est déjà une
            // destination valable pour l'ancrage par similarité (son centroïde
            // existe), donc il se remplira tout seul. Le dissoudre reviendrait à le
            // casser juste avant qu'il serve.
            if respect_user_folders && !direct_root_child {
                let parent_is_scan_root = n.parent_id.as_deref()
                    .is_some_and(|p| scan_root_ids.contains(p));
                let respects = n.connector.as_deref()
                    .is_some_and(|c| RESPECT_SUBFOLDERS.contains(&c));
                if respects && !parent_is_scan_root { continue; }
            }
            // Un conteneur directement sous la racine est une catégorie
            // volontaire (source détectée, dossier scanné...), pas un orphelin.
            if n.kind != "leaf" && n.kind != "note" {
                continue;
            }
            // Déjà couvert par un groupe de doublons : Gemma tranche, pas le script.
            if grouped_ids.contains(n.id.as_str()) {
                continue;
            }
            // Absorbé par une fusion de doublon exact : il va disparaître.
            if absorbed.contains(&n.id) {
                continue;
            }
            let n_tokens = tokens(&n.label);
            let mut best: Option<(&BrainNode, f32)> = None;
            let mut second_best = 0.0f32;
            for (c, ctoks) in &container_tokens {
                let s = overlap_score(&n_tokens, ctoks);
                let current_best = best.map(|(_, bs)| bs).unwrap_or(0.0);
                if s > current_best {
                    second_best = current_best;
                    best = Some((c, s));
                } else if s > second_best {
                    second_best = s;
                }
            }
            match best {
                Some((c, score)) if score >= MIN_OVERLAP && (score - second_best) >= MIN_MARGIN => {
                    moves.push(MoveSuggestion {
                        node_id: n.id.clone(),
                        new_parent_id: c.id.clone(),
                        node_label: n.label.clone(),
                        target_label: c.label.clone(),
                    });
                }
                // Aucune destination thématique confiante trouvée — direction le
                // bac "Non triable" (existant ou à créer), sauf s'il rejoint
                // d'abord un cluster de thème (tranché juste après la boucle).
                _ => catchall_bound.push((n.id.clone(), n.label.clone())),
            }
        }
    }

    // Cluster par mot partagé AVANT de figer le bac plat, sur l'ensemble unifié
    // (enfants déjà dans un bac existant + candidats tout juste identifiés) —
    // sinon rien ne serait jamais groupé avant un passage suivant qui n'arrive
    // pas forcément.
    let catchall_bound_ids: HashSet<String> = catchall_bound.iter().map(|(id, _)| id.clone()).collect();
    let theme_clusters = cluster_catchall(graph, catchall_id, &catchall_bound_ids);

    // Un candidat qui vient de rejoindre un cluster est géré par
    // `theme_clusters` (qui le route vers son thème, existant ou tout juste
    // créé) — il ne part PAS aussi dans le bac plat. Ce qui reste (candidats
    // non clusterés) va au bac existant (`moves`) ou attend sa création
    // (`orphans_unresolved`), selon que `catchall_id` existe déjà.
    let clustered: HashSet<&str> =
        theme_clusters.iter().flat_map(|c| c.node_ids.iter().map(String::as_str)).collect();
    let mut orphans_unresolved = Vec::new();
    let mut orphans_unresolved_ids = Vec::new();
    for (id, label) in catchall_bound {
        if clustered.contains(id.as_str()) {
            continue;
        }
        match catchall_id {
            Some(cid) => moves.push(MoveSuggestion {
                node_id: id,
                new_parent_id: cid.to_string(),
                node_label: label,
                target_label: CATCHALL_LABEL.to_string(),
            }),
            None => {
                orphans_unresolved.push(label);
                orphans_unresolved_ids.push(id);
            }
        }
    }

    ScanResult {
        moves,
        groups,
        exact_duplicates,
        orphans_unresolved,
        orphans_unresolved_ids,
        theme_clusters,
        catchall_id: catchall_id.map(str::to_string),
    }
}

/// Répartition PAR CAUSE des documents que `scan` n'a jamais routés — purement
/// informatif (rapport de diagnostic). Ne décide rien, ne modifie rien.
#[derive(Default, Debug)]
pub struct SkipBreakdown {
    /// Titre normalisé partagé avec un autre nœud : `scan` réserve ces cas à la
    /// décision de fusion et ne les range JAMAIS thématiquement (`:256`).
    pub duplicate_title: usize,
    /// Le nœud a des enfants (ou c'est le bac, ou une entité) → traité en hub (`:239`).
    pub has_children: usize,
    /// Ni enfant direct de la racine ni sous un dossier de scan → réputé « déjà
    /// dans un dossier thématique choisi » (`:247`).
    pub outside_scan_scope: usize,
    /// Kind hors leaf/note (`:252`). Structurellement 0 quand l'appelant a déjà
    /// filtré sur leaf/note (c'est le cas du rapport) — compté quand même pour
    /// que le total boucle si ce filtre amont change un jour.
    pub wrong_kind: usize,
    /// Rangé à la main par l'utilisateur dans un sous-dossier, et volontairement
    /// respecté (ADR-0022 phase 1.2). Ce n'est ni un échec ni un oubli : c'est une
    /// décision. Sans cette catégorie, ces documents tombaient dans
    /// `routed_pending` et le rapport annonçait un échec là où il y a un choix.
    pub user_filed: usize,
    /// Aucune garde ne s'applique : `scan` a bel et bien routé ce document, mais
    /// il n'est pas à sa place dans le graphe — la proposition correspondante
    /// n'a pas (encore) été appliquée : en attente dans `mcp_pending/`, refusée,
    /// ou bloquée par une dépendance.
    pub routed_pending: usize,
}

/// Attribue à chaque id de `ids` la garde de `scan` qui l'a écarté du rangement.
///
/// ponytail: réplique les prédicats de `scan` (mêmes tests, MÊME ORDRE
/// d'évaluation) au lieu de refactorer `scan` pour les partager — un compteur de
/// diagnostic ne doit pas pouvoir changer le comportement du tri. Les deux
/// vivent côte à côte dans ce fichier : toute modification d'une garde de `scan`
/// doit être répercutée ici. Un écart reste visible plutôt que silencieux — il
/// se déverse dans `routed_pending`, qui gonflerait sans raison.
pub fn skip_breakdown(graph: &BrainGraph, ids: &HashSet<&str>, respect_user_folders: bool) -> SkipBreakdown {
    let mut out = SkipBreakdown::default();
    if ids.is_empty() {
        return out;
    }
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());

    // Occurrences par titre normalisé (≥2 ⇒ le nœud est dans un groupe de doublons).
    let mut label_counts: HashMap<String, usize> = HashMap::new();
    for n in &graph.nodes {
        if n.kind == "root" {
            continue;
        }
        *label_counts.entry(normalize(&n.label)).or_default() += 1;
    }

    let parent_of: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.parent_id.as_deref().map(|p| (n.id.as_str(), p)))
        .collect();

    let scan_root_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|c| c.kind == "container" && c.parent_id.as_deref() == root_id.as_deref())
        .filter(|c| {
            graph.nodes.iter().any(|n| {
                n.connector.as_deref().is_some_and(|conn| SORTABLE_CONNECTORS.contains(&conn))
                    && ancestor_chain(&n.id, &parent_of).contains(&c.id.as_str())
            })
        })
        .map(|c| c.id.as_str())
        .collect();
    let under_scan_root = |id: &str| ancestor_chain(id, &parent_of).iter().any(|a| scan_root_ids.contains(a));

    let catchall_id: Option<&str> = graph
        .nodes
        .iter()
        .find(|n| normalize(&n.label) == normalize(CATCHALL_LABEL) && !under_scan_root(&n.id))
        .map(|n| n.id.as_str());

    let has_children: HashSet<&str> = graph.nodes.iter().filter_map(|n| n.parent_id.as_deref()).collect();

    for n in graph.nodes.iter().filter(|n| ids.contains(n.id.as_str())) {
        // Ordre identique à la boucle de `scan` : chaque document est compté dans
        // la garde qui l'a RÉELLEMENT écarté, pas dans la première qui pourrait
        // s'appliquer. Sans racine, `scan` ne route rien du tout — tout tombe
        // alors dans `outside_scan_scope`, ce que le test d'égalité produit déjà.
        if Some(n.id.as_str()) == catchall_id
            || has_children.contains(n.id.as_str())
            || n.id.starts_with("arch-entity-")
        {
            out.has_children += 1;
        } else if n.parent_id.as_ref() != root_id.as_ref() && !under_scan_root(&n.id) {
            out.outside_scan_scope += 1;
        } else if respect_user_folders
            && n.parent_id.as_ref() != root_id.as_ref()
            && !n.parent_id.as_deref().is_some_and(|p| scan_root_ids.contains(p))
            && n.connector.as_deref().is_some_and(|c| RESPECT_SUBFOLDERS.contains(&c))
        {
            out.user_filed += 1;
        } else if n.kind != "leaf" && n.kind != "note" {
            out.wrong_kind += 1;
        } else if label_counts.get(&normalize(&n.label)).copied().unwrap_or(0) > 1 {
            out.duplicate_title += 1;
        } else {
            out.routed_pending += 1;
        }
    }
    out
}

// ─── Doublons EXACTS (même texte) ───────────────────────────────────────────
// Bande « certain » de l'ADR-0022 : aucune inférence, donc aucun appel au modèle
// et aucune décision à valider. Le même document présent dans Téléchargements ET
// dans Drive doit être UN nœud, pas deux — sinon il occupe deux fois la fenêtre
// de contexte injectée, ce que la note produit désigne comme le réglage clé
// (« trop de contexte = cher + dilue »).
//
// Remplace, pour ces cas, la détection par TITRE qui se trompe dans les deux sens :
// deux « Document-20-01 » différents passaient pour des doublons, un fichier
// renommé passait inaperçu.

/// Texte servant à l'empreinte : espaces normalisés, casse conservée. La
/// normalisation des espaces n'est pas cosmétique — le MÊME PDF extrait par
/// `pdftotext` ou par le repli `pdf-extract` ne produit pas les mêmes sauts de
/// ligne, et on veut que ces deux extractions se reconnaissent.
fn dedup_text(n: &BrainNode) -> String {
    let raw = if !n.content.trim().is_empty() { n.content.as_str() } else { n.source_text.as_str() };
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Groupes de documents au texte IDENTIQUE. Premier id = survivant.
///
/// Survivant = le document rangé le plus PROFONDÉMENT, car c'est celui qui a été
/// placé le plus délibérément (`Drive/Ideeri/Factures/x.pdf` plutôt que
/// `Téléchargements/x.pdf`) ; à profondeur égale, le libellé le plus court
/// (« facture.pdf » plutôt que « facture (1).pdf »), puis l'id pour être
/// déterministe. Le survivant garde son lien vers l'original.
///
/// ponytail: le `SourceRef` des absorbés est perdu par la fusion — « ouvrir
/// l'original » ne pointera que vers une des deux provenances. À corriger si on
/// veut afficher « présent sur le disque ET sur Drive » (cf. ADR-0022).
pub fn exact_duplicates(graph: &BrainGraph) -> Vec<Vec<String>> {
    use sha2::{Digest, Sha256};
    let depth_of = |id: &str| -> usize {
        let parent_of: HashMap<&str, &str> = graph.nodes.iter()
            .filter_map(|n| n.parent_id.as_deref().map(|p| (n.id.as_str(), p)))
            .collect();
        ancestor_chain(id, &parent_of).len()
    };
    let mut by_hash: HashMap<String, Vec<&BrainNode>> = HashMap::new();
    for n in &graph.nodes {
        if n.kind != "leaf" && n.kind != "note" { continue; }
        if n.id.starts_with("arch-") { continue; }
        let text = dedup_text(n);
        // Un texte vide n'est pas une preuve d'identité : deux extractions ratées
        // ne sont pas le même document.
        if text.len() < 32 { continue; }
        let mut h = Sha256::new();
        h.update(text.as_bytes());
        by_hash.entry(format!("{:x}", h.finalize())).or_default().push(n);
    }
    let mut out: Vec<Vec<String>> = by_hash.into_values()
        .filter(|v| v.len() > 1)
        .map(|mut v| {
            v.sort_by(|a, b| {
                depth_of(&b.id).cmp(&depth_of(&a.id))
                    .then(a.label.len().cmp(&b.label.len()))
                    .then(a.id.cmp(&b.id))
            });
            v.into_iter().map(|n| n.id.clone()).collect()
        })
        .collect();
    out.sort_by(|a, b| a[0].cmp(&b[0])); // sortie déterministe
    out
}

// ─── Fusion de dossiers ÉVIDEMMENT redondants ───────────────────────────────
// Le clustering par lots fabrique plusieurs dossiers pour un seul sujet : trois
// dossiers de communes aux noms quasi identiques, deux dossiers de factures du
// même émetteur (constaté sur les données de Liam, 2026-08-03). C'est le défaut le
// plus coûteux en crédibilité : un utilisateur qui voit trois dossiers au même nom
// conclut que l'outil ne sait pas ce qu'il fait.
//
// À ne PAS confondre avec la passe de taxonomie qu'on vient de couper : elle
// INVENTAIT un parent pour réunir des choses différentes (des devis pharma avec
// des notes de physique sous « Santé »), sur le jugement d'un modèle. Ici on
// FUSIONNE deux dossiers qui sont la même chose, sur une preuve mesurable, sans
// créer de nœud ni de niveau. Aucun appel au modèle.

/// Recouvrement de tokens minimal entre deux NOMS pour les tenir pour le même
/// sujet. 0,50 vérifié sur les données réelles (2026-08-03) : les trois dossiers de
/// communes se recouvrent à 0,50 deux à deux et fusionnent ; les quatre dossiers de
/// factures (0,33) et les deux dossiers immobiliers (0,25) restent séparés.
/// Desserrer ce seuil retransformerait la fusion en devinette.
pub const FOLDER_MERGE_OVERLAP: f32 = 0.50;
/// Documents de la même famille de nom exigés DE CHAQUE CÔTÉ : un fichier égaré ne
/// doit jamais souder deux dossiers.
const FAMILY_MIN_PER_FOLDER: usize = 2;

pub struct FolderMerge {
    /// Survivant (le plus gros dossier) — `ids[0]` d'une proposition `merge`.
    pub survivor_id: String,
    pub absorbed_ids: Vec<String>,
    pub reason: String,
}

/// Famille d'un nom de fichier structuré : on retire les groupes de chiffres de
/// FIN (le numéro d'ordre). `Invoice-14545AA1-0036` → `invoice-14545aa1`.
///
/// `None` si le nom n'est pas structuré — il faut au moins deux segments, dix
/// caractères, et un segment contenant un chiffre (donc un code). Sans ces trois
/// gardes, « Contenu site business » ferait famille avec n'importe quelle prose.
pub fn filename_family(label: &str) -> Option<String> {
    let mut segs: Vec<String> = label
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    while segs.last().is_some_and(|s| s.chars().all(|c| c.is_ascii_digit())) {
        segs.pop();
    }
    if segs.len() < 2 { return None; }
    if !segs.iter().any(|s| s.chars().any(|c| c.is_ascii_digit())) { return None; }
    let key = segs.join("-");
    if key.len() < 10 { return None; }
    Some(key)
}

/// Fusions évidentes entre dossiers-thèmes. Deux signaux, tous deux exigeant le
/// MÊME domaine majoritaire (un dossier sans domaine ne fusionne jamais) :
///  1. noms se recouvrant à `FOLDER_MERGE_OVERLAP` ;
///  2. `FAMILY_MIN_PER_FOLDER` documents de la même famille de nom de chaque côté.
///
/// Les deux sont complémentaires sur données réelles : le signal des noms attrape
/// les dossiers de communes (que les familles de noms ne voient pas, leurs
/// libellés étant dérivés différemment), le signal des familles attrape les
/// factures d'un même émetteur (dont les noms ne se recouvrent pas assez).
///
/// Pur : ni I/O, ni LLM. `domains` = `id de document → domaine`.
pub fn folder_merges(graph: &BrainGraph, domains: &HashMap<String, String>) -> Vec<FolderMerge> {
    let mut children: HashMap<&str, Vec<&BrainNode>> = HashMap::new();
    for n in &graph.nodes {
        if let Some(p) = n.parent_id.as_deref() {
            if p.starts_with("arch-theme-") && (n.kind == "leaf" || n.kind == "note") {
                children.entry(p).or_default().push(n);
            }
        }
    }
    let folders: Vec<&str> = {
        let mut v: Vec<&str> = children.keys().copied().collect();
        v.sort_unstable(); // ordre déterministe : la sortie ne doit pas dépendre du HashMap
        v
    };
    if folders.len() < 2 { return Vec::new(); }

    let label_of: HashMap<&str, &str> = graph.nodes.iter().map(|n| (n.id.as_str(), n.label.as_str())).collect();
    let majority_domain = |fid: &str| -> Option<String> {
        let mut c: HashMap<&str, usize> = HashMap::new();
        for n in children.get(fid).into_iter().flatten() {
            if let Some(d) = domains.get(&n.id) { *c.entry(d.as_str()).or_default() += 1; }
        }
        c.into_iter().max_by_key(|(_, n)| *n).map(|(d, _)| d.to_string())
    };
    let families = |fid: &str| -> HashMap<String, usize> {
        let mut m: HashMap<String, usize> = HashMap::new();
        for n in children.get(fid).into_iter().flatten() {
            if let Some(k) = filename_family(&n.label) { *m.entry(k).or_default() += 1; }
        }
        m
    };

    let info: Vec<(&str, Option<String>, HashSet<String>, HashMap<String, usize>, usize)> = folders.iter()
        .map(|&fid| {
            (fid, majority_domain(fid), tokens(label_of.get(fid).copied().unwrap_or("")),
             families(fid), children.get(fid).map(Vec::len).unwrap_or(0))
        })
        .collect();

    // Union-find sur les paires appariées → groupes transitifs.
    let mut parent: Vec<usize> = (0..info.len()).collect();
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x { p[x] = p[p[x]]; x = p[x]; }
        x
    }
    let mut reasons: HashMap<usize, &'static str> = HashMap::new();
    for i in 0..info.len() {
        for j in (i + 1)..info.len() {
            let (_, di, ti, fi, _) = &info[i];
            let (_, dj, tj, fj, _) = &info[j];
            // Garde de domaine : sans domaine des deux côtés, et identique, on ne
            // fusionne pas — même logique que la garde du clustering.
            let (Some(di), Some(dj)) = (di, dj) else { continue };
            if di != dj { continue; }
            let by_name = overlap_score(ti, tj) >= FOLDER_MERGE_OVERLAP;
            let by_family = !by_name && fi.iter().any(|(k, ni)| {
                *ni >= FAMILY_MIN_PER_FOLDER && fj.get(k).copied().unwrap_or(0) >= FAMILY_MIN_PER_FOLDER
            });
            if !by_name && !by_family { continue; }
            let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
            if ri != rj {
                parent[ri] = rj;
                reasons.insert(rj, if by_name { "noms quasi identiques" } else { "même famille de nom de fichier" });
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..info.len() {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    let mut out: Vec<FolderMerge> = groups.into_iter()
        .filter(|(_, m)| m.len() >= 2)
        .map(|(root, mut members)| {
            // Survivant = le plus gros dossier ; à égalité, le plus petit id (stable).
            members.sort_by(|&a, &b| info[b].4.cmp(&info[a].4).then(info[a].0.cmp(info[b].0)));
            FolderMerge {
                survivor_id: info[members[0]].0.to_string(),
                absorbed_ids: members[1..].iter().map(|&i| info[i].0.to_string()).collect(),
                reason: reasons.get(&root).copied().unwrap_or("dossiers redondants").to_string(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.survivor_id.cmp(&b.survivor_id));
    out
}

const MIN_CLUSTER: usize = 3;
/// Mots trop génériques pour porter un thème à eux seuls (connecteurs de
/// remplissage, jamais un vrai sujet).
const CLUSTER_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "les", "des", "une", "pour", "avec",
    "dans", "sur", "par", "aux", "del", "test", "live", "export",
];

fn title_case(word: &str) -> String {
    let mut c = word.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Rejette les tokens purement numériques (années, numéros de séquence) et
/// les hash/ids hexadécimaux (ex. "6a476f7666e5c") : un mot partagé par
/// coïncidence n'est pas un thème, même sur 3+ pages — bruit constaté en
/// testant sur un vrai cerveau (2026-07-22).
fn is_meaningful_word(token: &str) -> bool {
    if token.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let looks_like_hash = token.len() >= 6
        && token.chars().all(|c| c.is_ascii_hexdigit())
        && token.chars().any(|c| c.is_ascii_digit());
    !looks_like_hash
}

/// Regroupe par mot significatif partagé par 3 pages ou plus (mécanique,
/// aucune IA) — les enfants directs de `parent_id` (le bac "Non triable", s'il
/// existe déjà) ET les orphelins de `extra_ids` (candidats au bac dans CE
/// même passage, pas encore déplacés). Sans `extra_ids`, un premier passage —
/// bac tout juste créé, encore vide dans le graphe qu'on regarde — ne
/// clusterait jamais rien avant un second passage. Chaque nœud rejoint au plus
/// UN groupe (le plus grand qui le contient), pour ne jamais le proposer dans
/// deux thèmes à la fois.
fn cluster_catchall(graph: &BrainGraph, parent_id: Option<&str>, extra_ids: &HashSet<String>) -> Vec<ThemeCluster> {
    let children: Vec<(&BrainNode, HashSet<String>)> = graph
        .nodes
        .iter()
        .filter(|n| {
            let already_inside = parent_id.is_some_and(|p| n.parent_id.as_deref() == Some(p));
            (already_inside || extra_ids.contains(&n.id)) && (n.kind == "leaf" || n.kind == "note")
        })
        .map(|n| (n, tokens(&n.label)))
        .collect();

    let mut by_token: HashMap<String, Vec<String>> = HashMap::new();
    for (n, toks) in &children {
        for t in toks {
            if CLUSTER_STOPWORDS.contains(&t.as_str()) || !is_meaningful_word(t) {
                continue;
            }
            by_token.entry(t.clone()).or_default().push(n.id.clone());
        }
    }

    let mut candidates: Vec<(String, Vec<String>)> = by_token.into_iter().collect();
    // Plus gros groupes en premier : un nœud dans plusieurs groupes rejoint le
    // plus significatif plutôt que le premier venu (ordre de HashMap arbitraire).
    candidates.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));

    let mut assigned: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (tok, ids) in candidates {
        let fresh: Vec<String> = ids.into_iter().filter(|id| !assigned.contains(id)).collect();
        if fresh.len() < MIN_CLUSTER {
            continue;
        }
        for id in &fresh {
            assigned.insert(id.clone());
        }
        out.push(ThemeCluster { label: title_case(&tok), node_ids: fresh });
    }
    out
}

/// 2ᵉ filet pour le contenu COURT (notes) que les embeddings ne savent pas
/// clusterer (texte trop court → cohésion peu fiable, cf. session 2026-07-30 :
/// 100 % des notes Apple tombaient en « Non triable »). Regroupe par ENTITÉ
/// distinctive partagée (nom propre / mot rare) plutôt que par cohésion sémantique.
///
/// Garde-fous anti « Lyon → Projets » (l'échec de l'ancien clustering par mot) :
/// 1. **Rareté (IDF)** — le terme doit être rare dans TOUT le corpus (`RARE_MAX`) :
///    un mot fréquent (« facture », « projet », « design », « export ») ne regroupe JAMAIS.
/// 2. **Nom propre** — présent avec une majuscule dans au moins un titre.
/// 3. **`is_meaningful_word`** — ni hash hexadécimal, ni nombre.
/// 4. **`MIN_CLUSTER`** (3) — au moins 3 pages.
///
/// Le label du thème = l'entité (title-case) → produit directement « Papiris ».
/// Pur (pas d'I/O, pas de LLM) → testable, déterministe, rapide (ne tourne que
/// sur les restes). ponytail: pas de garde de domaine ici (les 4 garde-fous
/// ci-dessus suffisent sur du titre court) — à ajouter si des faux positifs
/// inter-domaines apparaissent en usage réel.
pub fn cluster_by_entity(graph: &BrainGraph, leftovers: &[(String, String)]) -> Vec<ThemeCluster> {
    if leftovers.len() < MIN_CLUSTER {
        return Vec::new();
    }
    // Fréquence documentaire de chaque token sur TOUT le corpus (base de l'IDF).
    let mut corpus_df: HashMap<String, usize> = HashMap::new();
    for nd in &graph.nodes {
        for t in tokens(&nd.label) {
            *corpus_df.entry(t).or_default() += 1;
        }
    }
    // Un terme « distinctif » apparaît dans au plus RARE_MAX docs du corpus.
    // ponytail: cap absolu simple ; passer en proportion du corpus si un gros
    // cerveau fait remonter le seuil naturel des vrais clients au-dessus de 6.
    const RARE_MAX: usize = 6;

    // Indice de NOM PROPRE : capitalisé dans la MAJORITÉ (≥ 60 %) de ses
    // occurrences dans TOUT le corpus. Un vrai nom propre (« Papiris ») est quasi
    // toujours en majuscule ; un mot courant (« prix », « site », « plan »)
    // apparaît autant en minuscule → écarté, même s'il est rare par hasard.
    // (Correctif du faux positif « Prix », session 2026-07-30.)
    let mut tok_total: HashMap<String, usize> = HashMap::new();
    let mut tok_cap: HashMap<String, usize> = HashMap::new();
    for nd in &graph.nodes {
        for raw in nd.label.split(|c: char| !c.is_alphanumeric()) {
            if raw.chars().count() <= 2 { continue; }
            *tok_total.entry(raw.to_lowercase()).or_default() += 1;
            if raw.chars().next().is_some_and(|c| c.is_uppercase()) {
                *tok_cap.entry(raw.to_lowercase()).or_default() += 1;
            }
        }
    }
    let is_proper = |t: &str| {
        let total = tok_total.get(t).copied().unwrap_or(0);
        total > 0 && tok_cap.get(t).copied().unwrap_or(0) * 5 >= total * 3 // ≥ 60 %
    };

    // token distinctif → ids des restes qui le portent.
    let mut by_token: HashMap<String, Vec<String>> = HashMap::new();
    for (id, label) in leftovers {
        for t in tokens(label) {
            if CLUSTER_STOPWORDS.contains(&t.as_str()) || !is_meaningful_word(&t) { continue; }
            if !is_proper(&t) { continue; }                                         // nom propre (majoritairement capitalisé)
            if corpus_df.get(&t).copied().unwrap_or(0) > RARE_MAX { continue; }      // rareté (IDF)
            by_token.entry(t).or_default().push(id.clone());
        }
    }

    // Plus gros groupes d'abord ; chaque id rejoint au plus un groupe.
    let mut cands: Vec<(String, Vec<String>)> = by_token.into_iter().collect();
    cands.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let mut assigned: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (tok, ids) in cands {
        let fresh: Vec<String> = ids.into_iter().filter(|id| assigned.insert(id.clone())).collect();
        if fresh.len() < MIN_CLUSTER { continue; }
        out.push(ThemeCluster { label: title_case(&tok), node_ids: fresh });
    }
    out
}

pub struct GroupDecision {
    pub survivor_id: String,
    pub dropped_ids: Vec<String>,
    pub reason: String,
}

fn path_of(node: &BrainNode, by_id: &HashMap<&str, &BrainNode>) -> String {
    let mut parts = Vec::new();
    let mut cur = node.parent_id.as_deref();
    for _ in 0..12 {
        let Some(id) = cur else { break };
        let Some(p) = by_id.get(id) else { break };
        parts.push(p.label.clone());
        cur = p.parent_id.as_deref();
    }
    parts.reverse();
    parts.join(" › ")
}

const SYSTEM_PROMPT: &str =
    "Tu ranges le second cerveau d'un utilisateur. Tu réponds STRICTEMENT en JSON valide, sans texte autour.";

/// Labels des enfants directs d'un nœud, triés — pour les conteneurs (toujours
/// `content` vide par construction), c'est un bien meilleur signal de doublon
/// que le chemin d'origine (qui diffère presque toujours entre deux imports).
fn children_labels(node_id: &str, graph: &BrainGraph) -> Vec<String> {
    let mut v: Vec<String> = graph
        .nodes
        .iter()
        .filter(|n| n.parent_id.as_deref() == Some(node_id))
        .map(|n| n.label.clone())
        .collect();
    v.sort();
    v
}

fn decision_prompt(group: &DuplicateGroup, graph: &BrainGraph) -> String {
    let by_id: HashMap<&str, &BrainNode> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut out = format!(
        "Ces {n} pages ont un titre identique ou quasi identique (« {label} ») dans le cerveau d'un utilisateur.\n\n",
        n = group.node_ids.len(),
        label = group.label,
    );
    for id in &group.node_ids {
        if let Some(n) = by_id.get(id.as_str()) {
            let detail = if n.kind == "container" {
                let kids = children_labels(id, graph);
                if kids.is_empty() {
                    " — conteneur vide (aucune sous-page)".to_string()
                } else {
                    let shown: Vec<&str> = kids.iter().take(15).map(String::as_str).collect();
                    format!(" — {} sous-page(s) : {}{}", kids.len(), shown.join(", "),
                        if kids.len() > 15 { ", …" } else { "" })
                }
            } else if n.content.trim().is_empty() {
                String::new()
            } else {
                let c = n.content.trim();
                let end = c.len().min(200);
                format!(" — extrait : {}", &c[..end])
            };
            out.push_str(&format!(
                "- id `{id}` · type {kind} · chemin : {path}{detail}\n",
                kind = n.kind,
                path = path_of(n, &by_id),
            ));
        }
    }
    out.push_str(
        "\nCe sont probablement des doublons (le même contenu ou dossier importé plusieurs fois). \
         IMPORTANT : le chemin d'origine seul (ex. un import daté vs un rangement thématique) \
         N'EST PAS une preuve de contenu différent — la plupart des vrais doublons ont des chemins \
         différents simplement parce qu'ils ont été importés à des moments différents. \
         Pour un CONTENEUR : compare surtout la liste des sous-pages — des sous-pages identiques ou \
         très proches = quasiment certainement le même dossier importé deux fois, fusionne. \
         Ne garde séparé que si un signal de CONTENU concret l'indique (sous-pages clairement \
         différentes, extrait de contenu différent, dates ou entités explicitement différentes) — \
         pas seulement parce que le chemin ou l'id diffère (ils diffèrent toujours, par construction).\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {\"decision\": \"merge\" ou \"keep_separate\", \"survivor_id\": \"un id ci-dessus, vide si keep_separate\", \"reason\": \"1 phrase\"}\n",
    );
    out
}

pub enum GroupOutcome {
    Merge(GroupDecision),
    KeepSeparate { reason: String },
    /// Réponse du modèle inexploitable (pas de JSON, décision invalide, id
    /// halluciné...) — distingué de `KeepSeparate` pour le diagnostic : ce
    /// n'est pas une décision, c'est un échec de parsing.
    ParseFailed { raw_excerpt: String },
}

// Au-delà, le prompt reste raisonnable pour la fenêtre de contexte de Gemma
// (8192 tokens) sans complexifier avec un découpage en plusieurs appels —
// ponytail: cap simple, le surplus part quand même dans le bac plat (jamais
// perdu), l'appelant (lib.rs) le signale dans le rapport plutôt que de le
// masquer en silence.
const AI_CLUSTER_MAX_CANDIDATES: usize = 100;

/// Court extrait du CONTENU réel d'une page pour les passes IA : `content`
/// (édité par l'user) sinon `source_text` (texte extrait des PDF/CSV/docx à la
/// génération, sans IA). C'est le levier pour ranger par SUJET les fichiers mal
/// nommés (hash, `IMG_1234`, `ACFrOgDw==`) dont le seul libellé ne dit rien.
/// Normalisé sur une ligne, tronqué à `max` caractères.
fn snippet_for(node: &BrainNode, max: usize) -> String {
    let raw = if !node.content.trim().is_empty() { node.content.as_str() } else { node.source_text.as_str() };
    let one_line: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        one_line.chars().take(max).collect::<String>() + "…"
    } else {
        one_line
    }
}

fn ai_cluster_prompt(leftover: &[(String, String, String)]) -> String {
    let mut out = format!(
        "Voici {n} pages du second cerveau d'un utilisateur, actuellement sans dossier clair \
         (aucun mot de titre partagé assez souvent pour un rangement mécanique). Pour chaque page : \
         son titre, puis un extrait de son contenu réel (après le tiret) — le titre peut être \
         inutile (nom de fichier codé), l'extrait dit alors le vrai sujet.\n\n",
        n = leftover.len(),
    );
    for (id, label, snippet) in leftover {
        if snippet.is_empty() {
            out.push_str(&format!("- id `{id}` : {label}\n"));
        } else {
            out.push_str(&format!("- id `{id}` : {label} — {snippet}\n"));
        }
    }
    out.push_str(
        "\nPropose de regrouper par THÈME les pages qui traitent clairement du même sujet concret \
         (même type de document, même projet, même fournisseur...), en te fondant sur le titre ET \
         l'extrait de contenu, MÊME si les titres ne partagent aucun mot — c'est tout l'intérêt de \
         cette passe. Un groupe doit contenir au moins 3 pages et un thème réel, pas un fourre-tout \
         vague (\"Divers\", \"Autres\"...). Laisse de côté (n'inclus dans aucun groupe) toute page isolée \
         ou dont le sujet est incertain — ne devine jamais.\n\n\
         Le \"label\" est un nom de DOSSIER : 1 à 3 mots MAXIMUM, français, court et net \
         (ex. « Factures », « Immobilier », « Papiers »). JAMAIS une phrase ni une description \
         (pas « Documents immobiliers et administratifs »).\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {\"groups\": [{\"label\": \"Nom court\", \"ids\": [\"id ci-dessus\", ...]}]}\n\
         (\"groups\" peut être vide si rien ne se regroupe clairement.)\n",
    );
    out
}

/// Clustering SÉMANTIQUE (Gemma) de ce qui reste après le tri mécanique par
/// mot commun — capte les regroupements qu'un script ne peut pas voir (même
/// sujet, aucun mot partagé). Jamais appelé sur ce que le script a déjà su
/// grouper seul.
pub fn ai_cluster_leftovers(engine: &LlamaEngine, leftover: &[(String, String)], graph: &BrainGraph) -> Vec<ThemeCluster> {
    if leftover.len() < MIN_CLUSTER {
        return Vec::new();
    }
    let capped: Vec<(String, String)> = leftover.iter().take(AI_CLUSTER_MAX_CANDIDATES).cloned().collect();

    // Enrichit chaque candidat d'un extrait de son contenu réel — un fichier mal
    // nommé se range alors par sujet, plus par son titre inexploitable.
    let by_id: HashMap<&str, &BrainNode> = graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut enriched: Vec<(String, String, String)> = capped.iter()
        .map(|(id, label)| {
            let snip = by_id.get(id.as_str()).map(|n| snippet_for(n, 160)).unwrap_or_default();
            (id.clone(), label.clone(), snip)
        })
        .collect();
    // Trie par label : les familles (`export_*`, `Document-*`…) deviennent
    // adjacentes, donc restent dans le MÊME lot ci-dessous → l'IA les voit
    // ensemble et peut les regrouper (un cluster éclaté sur 2 lots serait raté).
    enriched.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    // Découpage en lots qui tiennent dans le contexte : le prompt (65+ items ×
    // extrait de contenu) dépassait les 8192 tokens de Gemma → llama-server
    // répondait 400 et AUCUN cluster n'était produit (bug remonté par Liam le
    // 2026-07-28 : « Non triable » figé à 65). On budgète en CARACTÈRES (approx.
    // sûre ~3,5 char/token) : garder prompt + n_predict(700) + marge template
    // bien sous CONTEXT_TOKENS. Chaque lot < budget → jamais de 400.
    // Budget en CARACTÈRES par lot, converti depuis les tokens dispo avec un
    // ratio PESSIMISTE (2,2 char/token — le réel mesuré sur ids/CSV denses est
    // ~2,44, on sous-remplit pour garder une marge). Réserve n_predict (700) +
    // l'overhead fixe (système + préambule + consignes + template Gemma, ~700
    // tokens). Ce calcul avec un mauvais ratio (÷3) laissait passer un lot de
    // 8300 tokens dans un contexte 8192 → 400 (bug du 2026-07-28).
    // La RÉPONSE (JSON qui ré-écrit les ids groupés) grossit avec le nombre
    // d'items : un lot de 80 dépassait `n_predict=700`, le JSON était TRONQUÉ en
    // plein milieu → parsing échoue → 0 cluster alors que Gemma répondait bien
    // (bug remonté par Liam le 2026-07-28). On réserve donc un vrai budget de
    // réponse ET on plafonne les items/lot pour borner sa longueur.
    const RESPONSE_TOKENS: u32 = 2500;
    const BATCH_MAX_ITEMS: usize = 40;
    let ctx = crate::ai::llama::CONTEXT_TOKENS as usize;
    // Budget items = contexte - réponse - overhead fixe (système+préambule+
    // consignes+template ~700 tokens), converti en chars (ratio pessimiste 2,2).
    let items_token_budget = ctx.saturating_sub(RESPONSE_TOKENS as usize + 700);
    let budget_chars = items_token_budget * 22 / 10;
    let mut out: Vec<ThemeCluster> = Vec::new();
    let mut assigned: HashSet<String> = HashSet::new();
    let mut batch: Vec<(String, String, String)> = Vec::new();
    let mut batch_chars = 0usize;
    let item_chars = |it: &(String, String, String)| it.0.len() + it.1.len() + it.2.len() + 16;

    let flush = |batch: &mut Vec<(String, String, String)>, out: &mut Vec<ThemeCluster>, assigned: &mut HashSet<String>| {
        if batch.len() < MIN_CLUSTER { batch.clear(); return; }
        let valid_ids: HashSet<&str> = batch.iter().map(|(id, _, _)| id.as_str()).collect();
        let prompt = ai_cluster_prompt(batch);
        match engine.complete_json(Some(SYSTEM_PROMPT), &prompt, RESPONSE_TOKENS) {
            Ok(raw) => {
                let parsed = parse_ai_cluster_response(&raw, &valid_ids);
                crate::elog!("🗂️ cluster lot: {} items, {} chars prompt → {} groupe(s) parsé(s).",
                    batch.len(), prompt.len(), parsed.len());
                for cluster in parsed {
                    let fresh: Vec<String> = cluster.node_ids.into_iter().filter(|id| assigned.insert(id.clone())).collect();
                    if fresh.len() >= MIN_CLUSTER {
                        out.push(ThemeCluster { label: cluster.label, node_ids: fresh });
                    }
                }
            }
            Err(e) => crate::elog!("🗂️ cluster lot: {} items → ÉCHEC appel IA: {e}", batch.len()),
        }
        batch.clear();
    };

    for it in enriched {
        let c = item_chars(&it);
        if (batch_chars + c > budget_chars || batch.len() >= BATCH_MAX_ITEMS) && !batch.is_empty() {
            flush(&mut batch, &mut out, &mut assigned);
            batch_chars = 0;
        }
        batch_chars += c;
        batch.push(it);
    }
    flush(&mut batch, &mut out, &mut assigned);
    out
}

/// Partie pure (parsing + validation) de `ai_cluster_leftovers`, séparée pour
/// être testable sans dépendre du vrai moteur IA (binaire + modèle absents en
/// CI). Défensif : ids hallucinés filtrés (hors de `valid_ids`), groupes sous
/// 3 pages écartés, aucune page assignée à plus d'un groupe (le premier gagne).
fn parse_ai_cluster_response(raw: &str, valid_ids: &HashSet<&str>) -> Vec<ThemeCluster> {
    let Some(json_str) = crate::ai::pipeline::extract_json(raw) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return Vec::new();
    };
    let Some(groups) = v.get("groups").and_then(|g| g.as_array()) else {
        return Vec::new();
    };

    let mut assigned: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for g in groups {
        let label = g.get("label").and_then(|l| l.as_str()).unwrap_or("").trim();
        if label.is_empty() {
            continue;
        }
        let Some(ids) = g.get("ids").and_then(|i| i.as_array()) else { continue };
        let fresh: Vec<String> = ids
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|id| valid_ids.contains(id) && !assigned.contains(*id))
            .map(String::from)
            .collect();
        if fresh.len() < MIN_CLUSTER {
            continue; // halluciné, trop petit, ou déjà pris par un groupe précédent
        }
        for id in &fresh {
            assigned.insert(id.clone());
        }
        out.push(ThemeCluster { label: label.to_string(), node_ids: fresh });
    }
    out
}

// ─── Clustering par embeddings (ADR-0019, Phase 1) ──────────────────────────
// Regroupe des documents à partir de leurs VECTEURS (pas du LLM) : déterministe,
// stable, sans hallucination. Union-find single-linkage par seuil de similarité
// cosinus. Le seuil est ABSOLU mais calibré haut (BGE-M3 tasse les scores :
// inter ~0,79 / intra ~0,90 mesurés Phase 0 → 0,86 sépare bien).

/// Seuil de similarité cosinus au-delà duquel deux documents sont « du même
/// groupe ». Calibré sur les données réelles de Liam (Phase 0).
/// Voisins considérés pour le graphe k-NN mutuel.
pub const CLUSTER_K_NEIGHBORS: usize = 6;
/// Plancher de similarité d'un lien : en dessous, deux docs ne sont jamais liés
/// même s'ils figurent dans leurs « plus proches voisins » respectifs (région
/// clairsemée = pas de vrai thème commun).
pub const CLUSTER_MIN_LINK: f32 = 0.80;
/// Cohésion minimale d'un cluster = similarité moyenne des membres à leur
/// centroïde. En dessous, le groupe est un fourre-tout hétérogène → dissous (ses
/// docs repartent en singletons, que l'appelant enverra en « Non triable »).
/// C'est LE garde-fou anti-blob : sur BGE-M3 les cosinus sont tassés dans
/// ~[0.6,1.0], donc un seuil ABSOLU ne sépare pas les thèmes (un cours de
/// physique et une carte grise sont à 0.78) ; la cohésion au centroïde, si.
pub const CLUSTER_COHESION_MIN: f32 = 0.84;
/// Écart au-delà duquel un membre est considéré comme **décroché** du reste de
/// son groupe, et retiré même si la cohésion moyenne est bonne.
///
/// La garde de cohésion raisonne sur une MOYENNE : 4 documents à 0,98 et un
/// intrus à 0,879 donnent 0,959, très au-dessus du plancher — le groupe est
/// déclaré sain et personne ne regarde le membre le plus loin. Un intrus isolé
/// dans un groupe serré est donc structurellement invisible (« Formations
/// Pharmadvance » avait avalé une étude de marché, 2026-08-24).
///
/// Mesuré sur les 12 dossiers du corpus réel de Liam : **2 éjections, dans les
/// 2 seuls dossiers qu'il a signalés, zéro dégât sur les 10 autres.**
pub const CLUSTER_OUTLIER_MARGIN: f32 = 0.05;

/// Groupes d'INDICES (dans l'ordre de `vecs`) par **k-NN mutuel + garde de
/// cohésion**. Remplace le single-linkage par seuil absolu, qui chaînait A~B~C
/// en un seul blob dès qu'UN lien faible existait — sur 202 docs réels ça a
/// soudé 130 documents hétérogènes dans un même dossier (bug 2026-07-29). Ici un
/// lien n'existe que si deux docs sont RÉCIPROQUEMENT dans leurs k plus proches
/// voisins (résiste au chaînage par un doc « hub »), puis chaque composante n'est
/// gardée que si elle est cohésive autour de son centroïde. Pur → testable sans
/// moteur.
pub fn cluster_indices(vecs: &[Vec<f32>]) -> Vec<Vec<usize>> {
    cluster_indices_with(vecs, CLUSTER_COHESION_MIN, CLUSTER_OUTLIER_MARGIN)
}

/// Idem, avec un seuil de cohésion explicite (réglable sans recompilation via
/// `archivist_tuning.json`).
pub fn cluster_indices_with(vecs: &[Vec<f32>], cohesion_min: f32, outlier_margin: f32) -> Vec<Vec<usize>> {
    let n = vecs.len();
    if n == 0 { return Vec::new(); }
    let sim = |i: usize, j: usize| crate::ai::llama::cosine(&vecs[i], &vecs[j]);

    // k plus proches voisins de chaque point (au-dessus du plancher de lien).
    let topk: Vec<Vec<usize>> = (0..n).map(|i| {
        let mut nbrs: Vec<(usize, f32)> = (0..n).filter(|&j| j != i).map(|j| (j, sim(i, j))).collect();
        nbrs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        nbrs.into_iter().take(CLUSTER_K_NEIGHBORS).filter(|&(_, s)| s >= CLUSTER_MIN_LINK).map(|(j, _)| j).collect()
    }).collect();

    // Composantes connexes du graphe k-NN MUTUEL (lien i-j ssi chacun est dans
    // le top-k de l'autre).
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x { parent[x] = parent[parent[x]]; x = parent[x]; }
        x
    }
    for i in 0..n {
        for &j in &topk[i] {
            if j > i && topk[j].contains(&i) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj { parent[ri] = rj; }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }

    // Garde de cohésion sur chaque composante, puis ordre déterministe.
    let mut out: Vec<Vec<usize>> = Vec::new();
    for (_, members) in groups {
        out.extend(cohesive_split(vecs, members, cohesion_min, outlier_margin));
    }
    for g in &mut out { g.sort(); }
    out.sort_by_key(|g| g[0]);
    out
}

/// Centroïde (moyenne non normalisée — `cosine` renormalise) d'un sous-ensemble.
fn centroid_of(vecs: &[Vec<f32>], idx: &[usize]) -> Vec<f32> {
    let dim = idx.first().map(|&i| vecs[i].len()).unwrap_or(0);
    let mut c = vec![0.0f32; dim];
    for &i in idx { for k in 0..dim.min(vecs[i].len()) { c[k] += vecs[i][k]; } }
    for x in &mut c { *x /= idx.len().max(1) as f32; }
    c
}

/// Rend une composante cohésive : retire itérativement le membre le plus loin du
/// centroïde tant que la cohésion moyenne reste sous le seuil. Les membres
/// retirés ET une composante entièrement dissoute reviennent en SINGLETONS —
/// jamais perdus (l'appelant les enverra en « Non triable » via le filtre
/// MIN_CLUSTER). Un groupe déjà cohésif est renvoyé tel quel.
fn cohesive_split(vecs: &[Vec<f32>], mut members: Vec<usize>, cohesion_min: f32, outlier_margin: f32) -> Vec<Vec<usize>> {
    if members.len() < 2 { return vec![members]; }
    let mut dropped: Vec<usize> = Vec::new();
    loop {
        let c = centroid_of(vecs, &members);
        let sims: Vec<(usize, f32)> = members.iter().map(|&i| (i, crate::ai::llama::cosine(&vecs[i], &c))).collect();
        let avg = sims.iter().map(|(_, s)| s).sum::<f32>() / sims.len() as f32;
        if avg >= cohesion_min {
            // Cohésion moyenne atteinte : reste le cas du membre DÉCROCHÉ, que la
            // moyenne ne peut pas voir. On ne descend jamais sous 3 membres — en
            // dessous, l'appelant dissoudrait le groupe, ce qui coûterait plus que
            // l'intrus.
            if members.len() > 3 {
                let worst = members
                    .iter()
                    .map(|&i| {
                        let sum_others: f32 = sims.iter().filter(|(j, _)| *j != i).map(|(_, s)| s).sum();
                        let mean_others = sum_others / (members.len() - 1) as f32;
                        let s = sims.iter().find(|(j, _)| *j == i).map(|(_, s)| *s).unwrap_or(0.0);
                        (i, mean_others - s)
                    })
                    .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                if let Some((i, ecart)) = worst {
                    if ecart > outlier_margin {
                        members.retain(|&m| m != i);
                        dropped.push(i);
                        continue; // un décroché peut en cacher un autre
                    }
                }
            }
            let mut out = vec![members];
            out.extend(dropped.into_iter().map(|m| vec![m]));
            return out;
        }
        if members.len() <= 2 {
            members.extend(dropped);
            return members.into_iter().map(|m| vec![m]).collect(); // tout dissous
        }
        let worst = sims.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).unwrap().0;
        members.retain(|&m| m != worst);
        dropped.push(worst);
    }
}

/// Un nom de dossier ne doit promettre que ce que le dossier contient.
///
/// Gemma nomme volontiers d'après le client dominant — utile, mais il l'applique
/// dès qu'un client **revient**, pas quand il **couvre**. Cas réel du 2026-08-24 :
/// un groupe de 7 devis dont 5 concernent le même client a pris le nom de ce
/// client, et une estimation de réparation automobile s'est retrouvée rangée sous
/// une enseigne qui n'a rien à voir. Le regroupement était bon ; c'est le NOM qui
/// mentait.
///
/// **On vérifie la seule chose que le modèle a affirmée** : l'entité qu'il déclare
/// (`entity`) est-elle vraiment présente dans les documents ? Si elle couvre moins
/// de 80 % de l'échantillon, ses mots sortent du nom — il reste le mot de type
/// (« Devis »), qui est honnête.
///
/// Première version (matin du 24/08) : le contrôle portait sur **tous** les mots
/// du nom. Écarté le soir même, sur mesure — un mot de type présent dans 7 docs
/// sur 11 (64 %) tombait comme un nom d'entité, le nom devenait vide et **le
/// groupe entier repartait en « Non triable »**. Pire, un mot absent partout
/// (jugement thématique, invérifiable) survivait à un mot majoritaire. La
/// couverture ne distingue pas une entité d'un mot commun : 71 % pour un client,
/// 64 % pour un type — c'est le MODÈLE qui doit dire lequel est lequel, le code
/// se contente de vérifier sa déclaration.
///
/// `entity == None` ⇒ rien n'est promis, le nom passe tel quel.
pub fn name_supported_by(name: &str, entity: Option<&str>, samples: &[String]) -> Option<String> {
    if samples.is_empty() { return None; }
    let core = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let Some(entity) = entity.map(str::trim).filter(|e| !e.is_empty() && *e != "null") else {
        return Some(name.trim().to_string()).filter(|n| !n.is_empty());
    };
    let ent_words: Vec<String> = entity.split_whitespace()
        .map(|w| core(w)).filter(|w| w.chars().count() >= 3).collect();
    if ent_words.is_empty() { return Some(name.trim().to_string()); }

    let lowered: Vec<String> = samples.iter().map(|s| s.to_lowercase()).collect();
    // Un document « contient l'entité » dès qu'un de ses mots significatifs y est :
    // « Novolia SAS » se cite aussi bien « Novolia » tout court.
    let covered = lowered.iter().filter(|s| ent_words.iter().any(|w| s.contains(w))).count();
    // 80 % du groupe, **ou** un seul document manquant. Le plancher n'est pas du
    // confort : sur un groupe de 4, 3/4 = 75 % tombait sous les 80 % et le dossier
    // « Véhicule Peugeot 208 » — parfaitement cohérent, cohésion 0,96 — était
    // abandonné parce qu'UN document sur quatre ne citait pas le modèle dans son
    // extrait (mesuré le 2026-08-24). Le cas d'origine reste refusé : 5/7, il en
    // manque deux.
    let missing = samples.len() - covered;
    if missing <= 1 || covered * 5 >= samples.len() * 4 {
        return Some(name.trim().to_string()); // l'entité couvre le dossier : nom conservé
    }

    // Elle n'en couvre qu'une partie : ses mots sortent, le reste du nom demeure.
    let mut kept: Vec<&str> = name.split_whitespace()
        .filter(|w| !ent_words.contains(&core(w)))
        .collect();
    let is_short = |w: &&str| core(w).chars().count() < 3;
    while kept.first().is_some_and(is_short) { kept.remove(0); }
    while kept.last().is_some_and(is_short) { kept.pop(); }
    if kept.is_empty() { return None; } // le nom N'ÉTAIT que l'entité : on renonce
    Some(kept.join(" "))
}

/// Nomme un groupe (Phase 2) : Gemma ne fait QUE ça — donner un nom de dossier
/// court à partir d'un échantillon des documents du groupe. `samples` = titres
/// (+ éventuel extrait). Renvoie un label court, ou `None` si réponse inexploitable.
pub fn ai_name_cluster(engine: &LlamaEngine, samples: &[String]) -> Option<(String, Option<String>)> {
    if samples.is_empty() { return None; }
    let mut prompt = String::from("Voici des documents d'un même dossier (titre — extrait du contenu) :\n");
    for s in samples.iter().take(12) {
        prompt.push_str(&format!("- {}\n", s.chars().take(200).collect::<String>()));
    }
    prompt.push_str(
        "\nDonne un nom de DOSSIER court (2 à 4 mots, français). PRIORITÉ ABSOLUE : si un même CLIENT / \
         ENTREPRISE / PERSONNE apparaît dans PRESQUE TOUS les documents ci-dessus, nomme le dossier d'après \
         lui (ex. « Factures Parabola », « Devis Novolia ») — c'est le nom du client qui compte, PAS un mot \
         de rôle générique comme « Services », « Prestations », « Documents ». S'il n'apparaît que dans une \
         PARTIE des documents, ne l'utilise PAS : le dossier contiendrait des documents qui n'ont rien à \
         voir avec lui. Dans ce cas, ou s'il n'y a aucun client identifiable, donne un nom thématique court \
         (ex. « Factures », « Immobilier »). JAMAIS une phrase ni « Divers ».\n\
         Indique séparément le CLIENT / ENTREPRISE / PERSONNE que tu as utilisé dans le nom, ou null \
         si le nom est purement thématique — il est vérifié dans les documents.\n\
         Renvoie UNIQUEMENT : {\"name\": \"…\", \"entite\": \"…\" ou null}\n",
    );
    let raw = engine.complete_json(Some(SYSTEM_PROMPT), &prompt, 60).ok()?;
    let js = crate::ai::pipeline::extract_json(&raw)?;
    let v: serde_json::Value = serde_json::from_str(js).ok()?;
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").trim();
    if name.is_empty() || name.chars().count() > 40 { return None; }
    // Rejet des noms fourre-tout : si Gemma ne trouve qu'un nom générique, c'est
    // que le groupe n'a pas de thème réel → on renvoie None, l'appelant l'envoie
    // en « Non triable » plutôt que de créer un dossier « Documents variés »
    // trompeur (garde-fou en plus de la cohésion, au cas où un groupe limite
    // passe le seuil).
    const GENERIC: [&str; 8] = ["divers", "varié", "varies", "autre", "document", "fichier", "misc", "général"];
    // Filtre de couverture AVANT le rejet des fourre-tout : retirer un nom
    // d'entité peut laisser un mot générique (« Devis Novolia » → « Devis »
    // reste bon, mais « Dossiers Novolia » → « Dossiers » ne l'est pas).
    let entity = v.get("entite").and_then(|e| e.as_str()).map(str::to_string);
    let Some(name) = name_supported_by(name, entity.as_deref(), samples) else {
        crate::elog!("🗂️ nommage: proposition rejetée (le nom n'était que l'entité, absente d'une partie des documents) — groupe abandonné.");
        return None;
    };
    let low = name.to_lowercase();
    if GENERIC.iter().any(|g| low.contains(g)) {
        crate::elog!("🗂️ nommage: proposition rejetée (nom fourre-tout) — groupe abandonné.");
        return None;
    }
    Some((name, entity))
}

// ─── Tag de domaine (le « sujet », que l'embedding noie sous la forme) ──────
// L'embedding sur texte brut mélange la FORME (long pavé de prose vs doc court
// structuré) et le SUJET, et la forme domine : un journal de trading et une
// proposition immobilière, tous deux « longs pavés français », sont à 0.95 de
// similarité alors qu'ils n'ont aucun rapport de sujet (mesuré le 2026-07-29).
// Un seuil ne sépare jamais ça de façon robuste. On ajoute donc UN signal
// orthogonal : Gemma classe chaque doc dans une liste FERMÉE de domaines (un mot
// stable, pas un mot libre qui dériverait en synonymes). Ce domaine sert de
// GARDE : on ne regroupe/n'ancre jamais deux docs de domaines différents, même
// si l'embedding les croit proches. Grossier exprès — le tri fin (quel client)
// reste fait par l'embedding À L'INTÉRIEUR du domaine.

/// Taxonomie fermée. Plus de repli « Autre » (décision Liam, 2026-08-03) : un
/// document que le modèle ne classe pas reste NON TAGUÉ — absent du cache, donc
/// retenté au scan suivant — au lieu d'être versé dans un fourre-tout. « Autre »
/// agrégeait des documents sans aucun rapport et, comme la garde ne compare que
/// des domaines égaux, elle devenait inopérante ENTRE eux : c'est ce qui laissait
/// des exports de base de données se regrouper avec des pièces d'identité.
pub const DOMAIN_LIST: [&str; 9] = [
    "Facturation", "Devis & Commercial", "Immobilier", "Finance & Trading",
    "Contrats & Juridique", "Identité & Papiers", "Études & Cours", "Santé",
    "Technique & Data",
];

/// Codes de transport (3 lettres) ↔ domaines canoniques. Le modèle ne rend que le
/// code : « FAC » au lieu d'un objet `{"n":1,"domain":"Devis & Commercial"}`
/// divise par ~3 les tokens GÉNÉRÉS, et la génération est le goulot
/// d'étranglement (les prompts, eux, sont traités en parallèle).
///
/// Les codes ne sortent JAMAIS d'ici : `parse_domains` les résout en noms
/// complets avant tout stockage, donc `archivist_domains.json`, la garde de
/// domaine et le rapport continuent de ne voir que les noms complets.
const DOMAIN_CODES: [(&str, &str); 9] = [
    ("FAC", "Facturation"),
    ("DEV", "Devis & Commercial"),
    ("IMM", "Immobilier"),
    ("FIN", "Finance & Trading"),
    ("JUR", "Contrats & Juridique"),
    ("IDE", "Identité & Papiers"),
    ("ETU", "Études & Cours"),
    ("SAN", "Santé"),
    ("TEC", "Technique & Data"),
];
const DOMAIN_BATCH_MAX: usize = 30;

/// Code de 3 lettres → domaine canonique. `None` = inexploitable (code inconnu,
/// halluciné, vide) : l'appelant laisse alors le document NON TAGUÉ, jamais dans
/// un repli. Tolère le nom complet, que le modèle recopie parfois malgré la
/// consigne — c'est du transport, pas une décision.
pub fn domain_from_code(raw: &str) -> Option<&'static str> {
    let key = raw.trim().to_uppercase();
    if key.is_empty() { return None; }
    if let Some((_, d)) = DOMAIN_CODES.iter().find(|(c, _)| *c == key) {
        return Some(d);
    }
    DOMAIN_LIST.iter().find(|d| d.to_uppercase() == key).copied()
}

fn domain_prompt(docs: &[(String, String, String)]) -> String {
    let mut out = String::from("Classe chaque document dans UN de ces domaines. Réponds avec le CODE de 3 lettres :\n");
    for (code, name) in DOMAIN_CODES { out.push_str(&format!("- {code} = {name}\n")); }
    out.push_str("\nDocuments (numéro : titre — extrait) :\n");
    // NB : on NUMÉROTE (1, 2, 3…) au lieu de faire recopier l'id — les ids réels
    // font 60+ caractères, 30 par lot faisaient dépasser la réponse au-delà des
    // tokens de sortie → JSON tronqué → tout retombait « Autre » (bug 2026-07-29).
    for (i, (_, label, snippet)) in docs.iter().enumerate() {
        let n = i + 1;
        if snippet.is_empty() {
            out.push_str(&format!("{n}. {label}\n"));
        } else {
            out.push_str(&format!("{n}. {label} — {}\n", snippet.chars().take(160).collect::<String>()));
        }
    }
    out.push_str(
        "\nFonde-toi sur le SUJET RÉEL (le contenu), pas la forme. Choisis TOUJOURS le code le \
         plus proche, même si l'ajustement est imparfait. N'invente aucun code.\n\
         Renvoie UNIQUEMENT le JSON, une map plate numéro → code :\n\
         {\"1\":\"FAC\",\"2\":\"IMM\",\"3\":\"TEC\"}\n",
    );
    out
}

/// Parse la map plate `{"1":"FAC", …}` en `numéro (1-based) → domaine canonique`.
///
/// Toute entrée douteuse est ABSENTE du résultat plutôt que rabattue sur un
/// repli : numéro hors de la borne du lot (anti-hallucination), valeur non
/// textuelle, ou code inconnu. Un document absent d'ici reste non tagué — donc
/// non mis en cache par l'appelant, donc retenté au scan suivant.
pub fn parse_domains(raw: &str, count: usize) -> std::collections::HashMap<usize, String> {
    let mut out = std::collections::HashMap::new();
    let Some(js) = crate::ai::pipeline::extract_json(raw) else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(js) else { return out };
    let Some(obj) = v.as_object() else { return out };
    for (key, val) in obj {
        let Ok(idx) = key.trim().parse::<usize>() else { continue };
        if idx < 1 || idx > count { continue; }
        let Some(code) = val.as_str() else { continue };
        if let Some(domain) = domain_from_code(code) {
            out.insert(idx, domain.to_string());
        }
    }
    out
}

/// Tag de domaine pour un lot de docs (id, titre, extrait) — par batches pour
/// limiter les appels Gemma. Renvoie `(tags, lots_en_échec)`.
///
/// Un lot dont l'appel ÉCHOUE (erreur moteur) ou dont la réponse est
/// INEXPLOITABLE (pas de JSON, JSON tronqué → aucun numéro parsé) ne produit
/// AUCUNE entrée : ses documents sont absents du résultat, donc jamais mis en
/// cache par l'appelant, donc retentés au prochain scan. Avant ce garde, un
/// `unwrap_or_default()` faisait retomber les 30 docs du lot sur « Autre » et
/// l'appelant gravait ce « Autre » dans le cache SOUS LEUR SIGNATURE COURANTE :
/// ils n'étaient plus jamais reclassés tant que leur contenu ne changeait pas.
/// Une panne de quelques secondes devenait un état permanent (diagnostiqué le
/// 2026-08-02 ; déjà soupçonné le 2026-07-29 avec 202/202 docs en « Autre »).
///
/// À l'INTÉRIEUR d'un lot réussi, un document absent de la réponse garde son
/// repli « Autre » : là, le modèle a bel et bien répondu — c'est une décision,
/// pas une panne, et elle mérite d'être mise en cache.
pub fn ai_domain_tags(
    engine: &LlamaEngine,
    docs: &[(String, String, String)],
) -> (std::collections::HashMap<String, String>, usize) {
    let mut out = std::collections::HashMap::new();
    let mut failed = 0usize;
    for chunk in docs.chunks(DOMAIN_BATCH_MAX) {
        // Map plate de codes à 3 lettres : ~10 tokens par document, donc 300
        // suffisent largement pour 30 items (c'était 900 avec l'ancien format
        // verbeux `{"n":1,"domain":"Devis & Commercial"}`).
        let parsed = match engine.complete_json(Some(SYSTEM_PROMPT), &domain_prompt(chunk), 300) {
            Ok(raw) => parse_domains(&raw, chunk.len()),
            Err(e) => {
                crate::elog!("🗂️ domaines: lot de {} doc(s) → ÉCHEC appel IA ({e}) — lot non taggé, à retenter.", chunk.len());
                failed += 1;
                continue;
            }
        };
        if parsed.is_empty() {
            crate::elog!("🗂️ domaines: lot de {} doc(s) → réponse inexploitable — lot non taggé, à retenter.", chunk.len());
            failed += 1;
            continue;
        }
        // Un document sans code exploitable est simplement absent : plus de repli
        // « Autre » qui le figeait en cache et le retirait de toute reprise.
        for (i, (id, _, _)) in chunk.iter().enumerate() {
            if let Some(domain) = parsed.get(&(i + 1)) {
                out.insert(id.clone(), domain.clone());
            }
        }
    }
    (out, failed)
}

// ─── Ancrage incrémental sur les dossiers existants ─────────────────────────
// Range les pages orphelines dans des dossiers DÉJÀ créés au lieu d'en recréer.
// C'est LA clé de l'incrémental à l'échelle (Liam, 2026-07-28) : quand Drive
// ajoute 500 pages, une nouvelle facture doit rejoindre « Factures » existant,
// PAS fragmenter en un énième bac. Le rattachement mécanique ne visait que les
// `kind:"container"` (dossiers de source) — jamais les dossiers Archiviste, qui
// sont des `kind:"note"` → sans cette passe, tout nouvel apport recréait des
// thèmes doublons.

const ASSIGN_BATCH_MAX: usize = 40;

fn assign_prompt(pages: &[(String, String, String)], folders: &[(String, String, Vec<String>)]) -> String {
    let mut out = String::from("DOSSIERS EXISTANTS où ranger (id : nom — exemples de contenu) :\n");
    for (id, label, sample) in folders {
        if sample.is_empty() {
            out.push_str(&format!("- id `{id}` : {label}\n"));
        } else {
            out.push_str(&format!("- id `{id}` : {label} (ex. : {})\n", sample.join(", ")));
        }
    }
    out.push_str("\nPAGES à classer (id : titre — extrait de contenu) :\n");
    for (id, label, snippet) in pages {
        if snippet.is_empty() {
            out.push_str(&format!("- id `{id}` : {label}\n"));
        } else {
            out.push_str(&format!("- id `{id}` : {label} — {snippet}\n"));
        }
    }
    out.push_str(
        "\nPour chaque page, indique le dossier existant qui lui correspond VRAIMENT (même thème ou même \
         type de document, en te fondant sur le contenu). Si AUCUN dossier ne convient clairement, NE la \
         classe pas (omets-la) — ne force jamais une page dans un dossier à peu près. Ne devine jamais.\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {\"assign\": [{\"page\": \"id de page\", \"folder\": \"id de dossier existant\"}, ...]}\n\
         (\"assign\" peut être vide si aucune page ne rejoint clairement un dossier existant.)\n",
    );
    out
}

/// Assigne des pages orphelines à des dossiers existants (Gemma). Batché sur les
/// pages ; la liste des dossiers (petite) est rappelée dans chaque lot. Renvoie
/// les paires (page_id, folder_id) sûres. `pages`=(id,label,snippet),
/// `folders`=(id,label,échantillon de contenu).
pub fn ai_assign_to_folders(
    engine: &LlamaEngine,
    pages: &[(String, String, String)],
    folders: &[(String, String, Vec<String>)],
) -> Vec<(String, String)> {
    if pages.is_empty() || folders.is_empty() {
        return Vec::new();
    }
    let valid_folders: HashSet<&str> = folders.iter().map(|(id, _, _)| id.as_str()).collect();
    let mut out: Vec<(String, String)> = Vec::new();
    let mut done: HashSet<String> = HashSet::new();
    for chunk in pages.chunks(ASSIGN_BATCH_MAX) {
        let valid_pages: HashSet<&str> = chunk.iter().map(|(id, _, _)| id.as_str()).collect();
        let prompt = assign_prompt(chunk, folders);
        let Ok(raw) = engine.complete_json(Some(SYSTEM_PROMPT), &prompt, 2000) else { continue };
        let assigns = parse_assignments(&raw, &valid_pages, &valid_folders);
        crate::elog!("🗂️ ancrage: {} pages, {} dossiers → {} assignation(s).", chunk.len(), folders.len(), assigns.len());
        for (page, folder) in assigns {
            if done.insert(page.clone()) { out.push((page, folder)); }
        }
    }
    out
}

/// Partie pure (parsing/validation) de `ai_assign_to_folders`. Défensif : page
/// et dossier doivent tous deux être valides ; une page assignée au plus une fois.
fn parse_assignments(raw: &str, valid_pages: &HashSet<&str>, valid_folders: &HashSet<&str>) -> Vec<(String, String)> {
    let Some(js) = crate::ai::pipeline::extract_json(raw) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(js) else { return Vec::new() };
    let Some(arr) = v.get("assign").and_then(|a| a.as_array()) else { return Vec::new() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for a in arr {
        let (Some(page), Some(folder)) = (
            a.get("page").and_then(|p| p.as_str()),
            a.get("folder").and_then(|f| f.as_str()),
        ) else { continue };
        if valid_pages.contains(page) && valid_folders.contains(folder) && seen.insert(page.to_string()) {
            out.push((page.to_string(), folder.to_string()));
        }
    }
    out
}

pub struct TaxonomyGroup {
    /// Chemin hiérarchique des dossiers parents, du plus général au plus précis
    /// (ex. `["Immobilier", "Ventes"]`). Profondeur LIBRE — c'est l'IA qui juge
    /// combien de niveaux sont pertinents (demande de Liam le 2026-07-28).
    pub path: Vec<String>,
    /// Ids des thèmes/dossiers existants à ranger sous le dernier segment du chemin.
    pub theme_ids: Vec<String>,
}

/// Un dossier de regroupement doit chapeauter au moins 2 dossiers — sinon ce
/// n'est pas un regroupement, juste un renommage déguisé.
const MIN_SUPERGROUP: usize = 2;
/// Garde-fou anti-sur-structure : au-delà, l'IA invente des niveaux artificiels.
const MAX_TAXO_DEPTH: usize = 4;

fn taxonomy_prompt(themes: &[(String, String, usize, Vec<String>)]) -> String {
    let mut out = format!(
        "Voici {n} dossiers thématiques du second cerveau d'un utilisateur, tous au même niveau \
         (à la racine). Pour chacun : son nom, son nombre de pages, et un ÉCHANTILLON de son contenu \
         (entre parenthèses). Beaucoup se recouvrent (plusieurs dossiers du même domaine créés \
         séparément) ou pourraient s'imbriquer.\n\n",
        n = themes.len(),
    );
    for (id, label, size, sample) in themes {
        if sample.is_empty() {
            out.push_str(&format!("- id `{id}` : {label} ({size} pages)\n"));
        } else {
            out.push_str(&format!("- id `{id}` : {label} ({size} pages — ex. : {})\n", sample.join(", ")));
        }
    }
    out.push_str(
        "\nOrganise ces dossiers en une ARBORESCENCE claire et lisible. IMPORTANT : fie-toi au CONTENU \
         (l'échantillon), pas seulement au nom qui peut être trompeur — ex. un dossier « Carte » contenant \
         « carte_identité, carte_grise » relève des PAPIERS/Administration, PAS de la géographie. Regroupe \
         les dossiers d'un même domaine sous un dossier parent au titre COURT (1 à 3 mots, français). Tu peux \
         créer PLUSIEURS NIVEAUX si c'est vraiment justifié (ex. « Immobilier » puis « Immobilier / Ventes »), \
         mais reste SOBRE : ne crée un niveau que s'il clarifie réellement, jamais de hiérarchie artificielle. \
         Exprime la profondeur par un chemin séparé par « / ». Un dossier parent doit chapeauter AU MOINS 2 \
         dossiers. Laisse à la racine (n'inclus nulle part) tout dossier isolé ou dont le rapprochement est \
         incertain. Ne devine jamais.\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {\"groups\": [{\"path\": \"Parent\" ou \"Parent/Sous-dossier\", \"themes\": [\"id ci-dessus\", ...]}]}\n\
         (\"groups\" peut être vide si rien ne se regroupe clairement.)\n",
    );
    out
}

/// Consolidation SÉMANTIQUE des dossiers-thèmes en une arborescence multi-niveaux
/// (Gemma) — corrige la fragmentation du clustering par lots (« Immobilier ×3 »,
/// « Administratif ×2 »…) en les chapeautant sous des parents communs, profondeur
/// jugée par l'IA (Liam, 2026-07-28). Ne touche qu'aux dossiers fournis, jamais
/// aux pages. `themes` = (id, label, nb_pages).
pub fn ai_group_themes(engine: &LlamaEngine, themes: &[(String, String, usize, Vec<String>)]) -> Vec<TaxonomyGroup> {
    if themes.len() <= MIN_SUPERGROUP {
        return Vec::new();
    }
    let valid_ids: HashSet<&str> = themes.iter().map(|(id, _, _, _)| id.as_str()).collect();
    let prompt = taxonomy_prompt(themes);
    // Réponse potentiellement longue (chemins + ids) → budget large, comme le
    // clustering, pour ne pas tronquer le JSON (cf. bug troncature du 2026-07-28).
    let Ok(raw) = engine.complete_json(Some(SYSTEM_PROMPT), &prompt, 2000) else {
        return Vec::new();
    };
    collapse_taxonomy(parse_taxonomy(&raw, &valid_ids))
}

/// Aplatit les niveaux intermédiaires INUTILES : Gemma sur-structure souvent
/// (`Administration / Documents` où « Administration » ne contient QUE
/// « Documents ») → un nœud dans un nœud pour rien (remonté par Liam le
/// 2026-07-28). On garde un niveau supérieur SEULEMENT s'il regroupe réellement
/// ≥2 branches (préfixe partagé par ≥2 groupes) ; sinon on réduit au 1er segment
/// (le plus général : « Administration », pas « Documents »). Pur → testable.
fn collapse_taxonomy(mut groups: Vec<TaxonomyGroup>) -> Vec<TaxonomyGroup> {
    let mut first_count: HashMap<String, usize> = HashMap::new();
    for g in &groups {
        if let Some(first) = g.path.first() {
            *first_count.entry(first.clone()).or_default() += 1;
        }
    }
    for g in &mut groups {
        if g.path.len() >= 2 {
            let shared = g.path.first().is_some_and(|f| first_count.get(f).copied().unwrap_or(0) >= 2);
            if !shared { g.path.truncate(1); }
        }
    }
    groups
}

/// Partie pure (parsing + validation) de `ai_group_themes`, testable sans le
/// moteur IA. Défensif : ids hallucinés filtrés, chemins vides/trop profonds
/// écartés, groupes sous MIN_SUPERGROUP écartés, un thème dans au plus un groupe.
fn parse_taxonomy(raw: &str, valid_ids: &HashSet<&str>) -> Vec<TaxonomyGroup> {
    let Some(json_str) = crate::ai::pipeline::extract_json(raw) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else { return Vec::new() };
    let Some(groups) = v.get("groups").and_then(|g| g.as_array()) else { return Vec::new() };

    let mut assigned: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for g in groups {
        let path: Vec<String> = g.get("path").and_then(|p| p.as_str()).unwrap_or("")
            .split('/')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if path.is_empty() || path.len() > MAX_TAXO_DEPTH { continue; }
        let Some(ids) = g.get("themes").and_then(|i| i.as_array()) else { continue };
        let fresh: Vec<String> = ids.iter()
            .filter_map(|v| v.as_str())
            .filter(|id| valid_ids.contains(id) && !assigned.contains(*id))
            .map(String::from)
            .collect();
        if fresh.len() < MIN_SUPERGROUP { continue; }
        for id in &fresh { assigned.insert(id.clone()); }
        out.push(TaxonomyGroup { path, theme_ids: fresh });
    }
    out
}

// ─── Couche entités (sociétés / clients) ────────────────────────────────────
// Extrait les organisations nommées dans le CONTENU d'un document et les
// matérialise en nœuds-entités reliés aux docs (wikilinks doc↔client). C'est
// ce qui fait passer Lucid d'un arbre de dossiers à un vrai graphe de
// connaissances : cliquer un client montre tous ses docs, où qu'ils soient
// rangés. Sort en propositions (create + link), jamais d'écriture directe.

/// On lit BEAUCOUP plus que les 200 car du clustering (le nom du client est
/// souvent en en-tête ou en pied de page), mais borné : un devis/facture/contrat
/// tient largement là-dedans, et ça garde le prompt raisonnable pour Gemma 3 4B.
const ENTITY_MAX_CHARS: usize = 4000;
/// Fenêtre de taille "document business" : sous ENTITY_MIN = trop court pour
/// contenir une entité fiable ; au-dessus de ENTITY_MAX = data-dump (CSV/export
/// géant) où extraire des entités n'a pas de sens et coûte cher pour rien.
const ENTITY_MIN_TEXT: usize = 120;
const ENTITY_MAX_TEXT: usize = 60_000;

fn entity_prompt(label: &str, text: &str) -> String {
    format!(
        "Document : « {label} »\n\nExtrait de son contenu :\n{text}\n\n\
         Extrais les ORGANISATIONS et CLIENTS nommés dans ce document (sociétés, entreprises, \
         cabinets, administrations, fournisseurs...). Uniquement des entités RÉELLES et NOMMÉES — \
         jamais un terme générique (« le client », « la société »), jamais une personne physique \
         seule, jamais un simple lieu. Si aucune organisation nommée n'apparaît clairement, renvoie \
         une liste vide. Ne devine jamais.\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {{\"entities\": [\"Nom exact de l'organisation\", ...]}}\n"
    )
}

/// Organisations nommées dans un document (via Gemma). Vide si le texte est
/// hors de la fenêtre "document business" ou si l'IA ne trouve rien de sûr.
pub fn extract_doc_entities(engine: &LlamaEngine, node: &BrainNode) -> Vec<String> {
    let text = if !node.content.trim().is_empty() { node.content.as_str() } else { node.source_text.as_str() };
    let len = text.chars().count();
    if len < ENTITY_MIN_TEXT || len > ENTITY_MAX_TEXT {
        return Vec::new();
    }
    let snippet: String = text.chars().take(ENTITY_MAX_CHARS).collect();
    let prompt = entity_prompt(&node.label, &snippet);
    let Ok(raw) = engine.complete_json(Some(SYSTEM_PROMPT), &prompt, 300) else {
        return Vec::new();
    };
    parse_entities(&raw)
}

fn is_generic_entity(name: &str) -> bool {
    let l = name.to_lowercase();
    let l = l.trim();
    matches!(l,
        "client" | "le client" | "la société" | "la societe" | "société" | "societe"
        | "entreprise" | "l'entreprise" | "fournisseur" | "n/a" | "na" | "inconnu"
        | "divers" | "autres" | "sarl" | "sas" | "sa"
    )
}

/// Clé de résolution d'entités : minuscule, suffixes juridiques retirés (SARL,
/// SAS, SA, EURL...), ponctuation ôtée → « Société Y », « Y SARL », « Y. » donnent
/// le MÊME slug « societe-y » / « y ». Sert d'id (`arch-entity-<slug>`) et de clé
/// de dédup entre documents (deux orthographes → un seul nœud-entité).
pub fn entity_slug(name: &str) -> String {
    const LEGAL: &[&str] = &["sarl", "sas", "sasu", "sa", "eurl", "sci", "scop", "gmbh", "ltd", "llc", "inc", "&", "et"];
    let lower = name.to_lowercase();
    let cleaned: String = lower.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect();
    cleaned
        .split_whitespace()
        .filter(|w| !LEGAL.contains(w))
        .collect::<Vec<_>>()
        .join("-")
}

/// Partie pure (parsing/validation), testable sans moteur IA. Défensif :
/// génériques filtrés, dédup par slug DANS le doc, JSON invalide toléré.
fn parse_entities(raw: &str) -> Vec<String> {
    let Some(js) = crate::ai::pipeline::extract_json(raw) else { return Vec::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(js) else { return Vec::new() };
    let Some(arr) = v.get("entities").and_then(|e| e.as_array()) else { return Vec::new() };
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for x in arr {
        let Some(name) = x.as_str() else { continue };
        let name = name.trim();
        if name.chars().count() < 2 || is_generic_entity(name) { continue; }
        let slug = entity_slug(name);
        if slug.is_empty() || !seen.insert(slug) { continue; }
        out.push(name.to_string());
    }
    out
}

/// Décision Gemma pour UN groupe de titres proches — jamais appelé pour les
/// cas que le script tranche seul. Ne fusionne jamais sur un doute (id
/// halluciné, JSON invalide, décision inattendue → `ParseFailed`, pas `Merge`).
pub fn decide_group(engine: &LlamaEngine, group: &DuplicateGroup, graph: &BrainGraph) -> GroupOutcome {
    let prompt = decision_prompt(group, graph);
    let raw = match engine.complete_json(Some(SYSTEM_PROMPT), &prompt, 200) {
        Ok(r) => r,
        Err(e) => return GroupOutcome::ParseFailed { raw_excerpt: format!("(erreur moteur: {e})") },
    };
    let excerpt = || raw.chars().take(300).collect::<String>();
    let Some(json_str) = crate::ai::pipeline::extract_json(&raw) else {
        return GroupOutcome::ParseFailed { raw_excerpt: excerpt() };
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return GroupOutcome::ParseFailed { raw_excerpt: excerpt() };
    };
    match v.get("decision").and_then(|d| d.as_str()) {
        Some("keep_separate") => {
            GroupOutcome::KeepSeparate { reason: v.get("reason").and_then(|s| s.as_str()).unwrap_or("").to_string() }
        }
        Some("merge") => {
            let Some(survivor_id) = v.get("survivor_id").and_then(|s| s.as_str()).map(String::from) else {
                return GroupOutcome::ParseFailed { raw_excerpt: excerpt() };
            };
            if !group.node_ids.contains(&survivor_id) {
                return GroupOutcome::ParseFailed { raw_excerpt: excerpt() }; // id halluciné
            }
            let dropped_ids: Vec<String> = group.node_ids.iter().filter(|id| **id != survivor_id).cloned().collect();
            let reason = v.get("reason").and_then(|s| s.as_str()).unwrap_or("").to_string();
            GroupOutcome::Merge(GroupDecision { survivor_id, dropped_ids, reason })
        }
        _ => GroupOutcome::ParseFailed { raw_excerpt: excerpt() },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, label: &str, kind: &str, parent: Option<&str>) -> BrainNode {
        BrainNode {
            id: id.into(), label: label.into(), kind: kind.into(), weight: 0,
            summary: String::new(), keywords: vec![], decisions: vec![], patterns: vec![],
            community: 0, parent_id: parent.map(String::from), synthesized_at: None, date: None,
            content: String::new(), connector: None, source_id: None, source_project: None,
            source_text: String::new(), updated_at: None,
        }
    }

    fn graph(nodes: Vec<BrainNode>) -> BrainGraph {
        BrainGraph { nodes, edges: vec![], markdown: String::new(), report: String::new(), generated_at: String::new() }
    }

    fn local_leaf(id: &str, label: &str, parent: &str) -> BrainNode {
        let mut n = node(id, label, "leaf", Some(parent));
        n.connector = Some("local-folder".into());
        n
    }

    /// Le rapport annonçait « Hors périmètre : 160 » sans dire pourquoi (retour
    /// Liam, 2026-08-03). Les causes doivent être EXCLUSIVES (chaque document
    /// compté une fois, dans la garde qui l'a réellement écarté) et leur somme
    /// doit faire le total, sinon la répartition ment.
    #[test]
    fn skip_breakdown_attribue_une_seule_cause_par_document_et_boucle_sur_le_total() {
        let container = |id: &str| node(id, id, "container", Some("root"));
        let nodes = vec![
            node("root", "Cerveau", "root", None),
            // Dossier de scan : reconnu comme tel parce qu'une feuille local-folder vit dessous.
            container("p:Documents"),
            // Conteneur d'une source NON triable (Obsidian) : aucune feuille
            // local-folder/apple-notes dessous → n'est pas un dossier de scan, son
            // arborescence est respectée telle quelle (ADR-0020).
            container("p:Ailleurs"),
            // Passe toutes les gardes → `scan` l'a routé, la proposition n'est pas appliquée.
            local_leaf("leaf:a", "Titre unique A", "p:Documents"),
            // Deux titres identiques → réservés à la décision de fusion.
            local_leaf("leaf:dup1", "Facture", "p:Documents"),
            local_leaf("leaf:dup2", "Facture", "p:Documents"),
            // A une sous-page → traité en hub, jamais rangé comme document.
            local_leaf("leaf:hub", "Dossier client", "p:Documents"),
            local_leaf("leaf:kid", "Sous-page unique", "leaf:hub"),
            // Hors racine ET hors dossier de scan → réputé déjà rangé. Il FAUT une
            // source non triable ici : avec un connecteur `local-folder`, son
            // conteneur deviendrait lui-même un dossier de scan et le document
            // repasserait dans le périmètre.
            {
                let mut n = node("leaf:ailleurs", "Titre unique B", "leaf", Some("p:Ailleurs"));
                n.connector = Some("obsidian".into());
                n
            },
        ];
        let g = graph(nodes);
        let ids: HashSet<&str> = g.nodes.iter()
            .filter(|n| n.kind == "leaf" || n.kind == "note")
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(ids.len(), 6, "6 documents dans la population analysée");

        let b = skip_breakdown(&g, &ids, false);
        assert_eq!(b.duplicate_title, 2, "les deux « Facture »");
        assert_eq!(b.has_children, 1, "le hub, pas sa sous-page");
        assert_eq!(b.outside_scan_scope, 1, "celui rangé sous un conteneur non scanné");
        assert_eq!(b.wrong_kind, 0, "population déjà filtrée sur leaf/note");
        assert_eq!(b.routed_pending, 2, "leaf:a et la sous-page du hub");
        assert_eq!(
            b.duplicate_title + b.has_children + b.outside_scan_scope + b.wrong_kind
                + b.user_filed + b.routed_pending,
            ids.len(),
            "les causes sont exclusives et couvrent tout le total",
        );
    }

    #[test]
    fn cluster_by_entity_groupe_entite_rare_pas_mot_frequent() {
        let mut nodes = Vec::new();
        // "Facture" fréquent dans le corpus (7 docs) → générique, doit être écarté.
        for i in 0..7 { nodes.push(node(&format!("f{i}"), &format!("Facture {i}"), "note", None)); }
        // 3 notes courtes partageant l'entité RARE "Papiris" (nom propre).
        nodes.push(node("p1", "Papiris design", "note", None));
        nodes.push(node("p2", "KPI Papiris", "note", None));
        nodes.push(node("p3", "Papiris (Design)", "note", None));
        // 3 notes partageant le mot FRÉQUENT "Facture" → ne doit PAS grouper (rareté).
        nodes.push(node("g1", "Facture eau", "note", None));
        nodes.push(node("g2", "Facture gaz", "note", None));
        nodes.push(node("g3", "Facture edf", "note", None));
        // 3 notes partageant le mot COURANT "prix" en casse mixte (1 Maj / 2 min)
        // → rare mais PAS un nom propre → doit être écarté (correctif faux positif).
        nodes.push(node("x1", "Prix maison", "note", None));
        nodes.push(node("x2", "prix communes", "note", None));
        nodes.push(node("x3", "le prix", "note", None));
        let g = graph(nodes);
        let leftovers: Vec<(String, String)> = ["p1", "p2", "p3", "g1", "g2", "g3", "x1", "x2", "x3"].iter()
            .map(|id| (id.to_string(), g.nodes.iter().find(|n| n.id == *id).unwrap().label.clone()))
            .collect();
        let clusters = super::cluster_by_entity(&g, &leftovers);
        assert_eq!(clusters.len(), 1, "seule l'entité rare Papiris doit grouper (ni Facture ni prix)");
        assert_eq!(clusters[0].label, "Papiris");
        assert_eq!(clusters[0].node_ids.len(), 3);
    }

    #[test]
    fn routes_unmatched_orphan_to_existing_catchall() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
            node("catchall", CATCHALL_LABEL, "container", Some("root")),
            local_leaf("orphan", "Invoice-14545AA1-0052", "downloads"),
        ]);
        assert!(!needs_catchall(&g));
        let r = scan(&g, false);
        assert_eq!(r.moves.len(), 1);
        assert_eq!(r.moves[0].new_parent_id, "catchall");
        assert!(r.orphans_unresolved.is_empty());
    }

    #[test]
    fn a_hub_with_children_is_never_treated_as_an_orphan_itself() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            // Thème créé par un passage précédent — en `note` (limite connue
            // du circuit MCP), direct enfant de root, avec déjà des enfants.
            node("theme", "Medecine", "note", Some("root")),
            local_leaf("m1", "medecine_anatomie", "theme"),
            local_leaf("m2", "medecine_nutrition", "theme"),
            local_leaf("m3", "medecine_maladies", "theme"),
        ]);
        let r = scan(&g, false);
        assert!(r.moves.is_empty(), "le hub lui-même ne doit jamais être proposé au déplacement");
    }

    #[test]
    fn catchall_never_proposes_to_move_itself() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
            // Le bac existe déjà, en `note` (comme le produit réellement une
            // proposition "create" — jamais "container").
            node(CATCHALL_ID, CATCHALL_LABEL, "note", Some("root")),
            local_leaf("orphan", "Invoice-14545AA1-0052", "downloads"),
        ]);
        let r = scan(&g, false);
        assert_eq!(r.moves.len(), 1, "seul l'orphelin doit bouger, pas le bac lui-même");
        assert_eq!(r.moves[0].node_id, "orphan");
    }

    #[test]
    fn reports_catchall_still_needed_when_absent() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
            local_leaf("orphan", "Invoice-14545AA1-0052", "downloads"),
        ]);
        assert!(needs_catchall(&g));
        let r = scan(&g, false);
        assert!(r.moves.is_empty());
        assert_eq!(r.orphans_unresolved.len(), 1);
    }

    #[test]
    fn clusters_catchall_children_sharing_a_significant_word() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("catchall", CATCHALL_LABEL, "note", Some("root")),
            local_leaf("i1", "Invoice-14545AA1-0052", "catchall"),
            local_leaf("i2", "Invoice-21A6E515-0006", "catchall"),
            local_leaf("i3", "Invoice-XG1G5TIQ-0002", "catchall"),
            local_leaf("other", "Repas", "catchall"),
        ]);
        let r = scan(&g, false);
        assert_eq!(r.theme_clusters.len(), 1);
        assert_eq!(r.theme_clusters[0].label, "Invoice");
        assert_eq!(r.theme_clusters[0].node_ids.len(), 3);
    }

    #[test]
    fn ignores_hash_and_numeric_coincidences() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("catchall", CATCHALL_LABEL, "note", Some("root")),
            local_leaf("h1", "export_biens__6a476f7666e5c", "catchall"),
            local_leaf("h2", "export_actions__6a476f7666e5c", "catchall"),
            local_leaf("h3", "export_photos__6a476f7666e5c", "catchall"),
            local_leaf("y1", "301 communes - webflow-301-redirects-v2", "catchall"),
            local_leaf("y2", "301 estimations - webflow-301-redirects-estimation", "catchall"),
            local_leaf("y3", "fs-301-import-template", "catchall"),
        ]);
        let r = scan(&g, false);
        // "6a476f7666e5c" (hash) et "301" (numérique) ne doivent jamais former
        // un thème, même partagés par 3 pages — mais "webflow" (2 pages) non
        // plus (sous le seuil), donc aucun cluster ici du tout.
        assert!(r.theme_clusters.is_empty());
    }

    #[test]
    fn does_not_cluster_below_minimum_size() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("catchall", CATCHALL_LABEL, "note", Some("root")),
            local_leaf("c1", "chimie_elements", "catchall"),
            local_leaf("c2", "chimie_reactions", "catchall"),
        ]);
        let r = scan(&g, false);
        assert!(r.theme_clusters.is_empty(), "2 pages ne suffisent pas (seuil à 3)");
    }

    #[test]
    fn finds_exact_duplicate_titles() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("a", "RIB", "leaf", Some("root")),
            node("b", "RIB", "leaf", Some("root")),
        ]);
        let r = scan(&g, false);
        assert_eq!(r.groups.len(), 1);
        assert_eq!(r.groups[0].node_ids.len(), 2);
    }

    #[test]
    fn moves_orphan_to_best_matching_container_only_when_unambiguous() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("c1", "Sécurité & Confiance", "container", Some("root")),
            node("c2", "Business", "container", Some("root")),
            node("orphan", "Cybersécurité", "leaf", Some("root")),
        ]);
        let r = scan(&g, false);
        assert_eq!(r.moves.len(), 1);
        assert_eq!(r.moves[0].new_parent_id, "c1");
    }

    #[test]
    fn leaves_ambiguous_orphan_unresolved_rather_than_guessing() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("c1", "Sport football", "container", Some("root")),
            node("c2", "Sport nutrition", "container", Some("root")),
            node("orphan", "Sport général", "leaf", Some("root")),
        ]);
        let r = scan(&g, false);
        assert!(r.moves.is_empty());
        assert_eq!(r.orphans_unresolved.len(), 1);
    }

    #[test]
    fn reaches_into_scan_root_folders_to_reattach_nested_files() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            // Dossier de scan brut : détecté par sa feuille local-folder, pas par son nom.
            node("downloads", "Downloads", "container", Some("root")),
            node("downloads/assurances", "Assurances Dossier", "container", Some("downloads")),
            local_leaf("orphan", "Cybersécurité", "downloads/assurances"),
            // Vraie destination thématique, hors de l'arborescence de scan.
            node("c1", "Sécurité & Confiance", "container", Some("root")),
            node("c2", "Business", "container", Some("root")),
        ]);
        let r = scan(&g, false);
        assert_eq!(r.moves.len(), 1, "le fichier imbriqué doit être proposé au rattachement");
        assert_eq!(r.moves[0].new_parent_id, "c1");
    }

    #[test]
    fn never_suggests_moving_between_two_scan_root_subfolders() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
            // Un conteneur au nom proche, mais LUI AUSSI sous Downloads — ne
            // doit jamais être choisi comme destination (pas un vrai rangement).
            node("downloads/secu", "Sécurité Info", "container", Some("downloads")),
            local_leaf("leaf1", "Cybersécurité", "downloads"),
        ]);
        let r = scan(&g, false);
        assert!(r.moves.is_empty());
        assert_eq!(r.orphans_unresolved.len(), 1);
    }

    #[test]
    fn top_level_container_is_never_treated_as_orphan() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
        ]);
        let r = scan(&g, false);
        assert!(r.moves.is_empty());
        assert!(r.orphans_unresolved.is_empty());
    }

    // ── parse_ai_cluster_response : validation de la réponse Gemma ──────────
    // Pas de test appelant le vrai moteur (binaire + modèle absents en CI,
    // comme pour `decide_group`) — seule la partie pure (parsing/validation)
    // est testable, et c'est elle qui porte toute la logique défensive.

    #[test]
    fn parse_ai_cluster_accepte_un_groupe_valide() {
        let valid: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        let raw = r#"{"groups": [{"label": "Invoice", "ids": ["a", "b", "c"]}]}"#;
        let out = parse_ai_cluster_response(raw, &valid);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "Invoice");
        assert_eq!(out[0].node_ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_ai_cluster_filtre_les_ids_hallucines() {
        let valid: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        // "zzz" n'existe pas dans les candidats fournis au modèle.
        let raw = r#"{"groups": [{"label": "Invoice", "ids": ["a", "b", "zzz"]}]}"#;
        let out = parse_ai_cluster_response(raw, &valid);
        assert!(out.is_empty(), "seuls 2 ids valides restent (< 3, seuil de cluster) : {:?}",
            out.iter().map(|c| &c.node_ids).collect::<Vec<_>>());
    }

    #[test]
    fn parse_ai_cluster_rejette_un_groupe_sous_le_seuil() {
        let valid: HashSet<&str> = ["a", "b"].into_iter().collect();
        let raw = r#"{"groups": [{"label": "Invoice", "ids": ["a", "b"]}]}"#;
        let out = parse_ai_cluster_response(raw, &valid);
        assert!(out.is_empty(), "2 pages ne suffisent pas (seuil à 3)");
    }

    #[test]
    fn parse_ai_cluster_rejette_un_libelle_vide() {
        let valid: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        let raw = r#"{"groups": [{"label": "", "ids": ["a", "b", "c"]}]}"#;
        let out = parse_ai_cluster_response(raw, &valid);
        assert!(out.is_empty());
    }

    #[test]
    fn parse_ai_cluster_une_page_ne_rejoint_pas_deux_groupes() {
        let valid: HashSet<&str> = ["a", "b", "c", "d", "e", "f"].into_iter().collect();
        // "c" apparaît dans les deux groupes — seul le premier doit le garder ;
        // le 2e groupe garde 3 ids valides même sans "c" (au-dessus du seuil).
        let raw = r#"{"groups": [
            {"label": "Invoice", "ids": ["a", "b", "c"]},
            {"label": "Devis", "ids": ["c", "d", "e", "f"]}
        ]}"#;
        let out = parse_ai_cluster_response(raw, &valid);
        assert_eq!(out.len(), 2);
        assert!(out[0].node_ids.contains(&"c".to_string()));
        assert!(!out[1].node_ids.contains(&"c".to_string()), "« c » ne doit pas être aussi dans le 2e groupe");
        assert_eq!(out[1].node_ids.len(), 3, "« c » retiré du 2e groupe, les 3 autres restent : {:?}", out[1].node_ids);
    }

    #[test]
    fn parse_ai_cluster_tolere_une_reponse_inexploitable() {
        let valid: HashSet<&str> = ["a", "b", "c"].into_iter().collect();
        assert!(parse_ai_cluster_response("ceci n'est pas du JSON", &valid).is_empty());
        assert!(parse_ai_cluster_response(r#"{"groups": []}"#, &valid).is_empty());
        assert!(parse_ai_cluster_response(r#"{"autre_chose": 1}"#, &valid).is_empty());
    }

    #[test]
    fn parse_taxonomy_chapeaute_et_gere_la_profondeur() {
        let valid: HashSet<&str> = ["arch-theme-devis", "arch-theme-invoice", "arch-theme-estimation", "arch-theme-ventes", "arch-theme-lyon"]
            .into_iter().collect();
        // 1 niveau + 1 niveau imbriqué (Immobilier/Ventes).
        let raw = r#"{"groups": [
            {"path": "Facturation", "themes": ["arch-theme-devis", "arch-theme-invoice", "arch-theme-estimation"]},
            {"path": "Immobilier/Ventes", "themes": ["arch-theme-ventes", "arch-theme-lyon"]}
        ]}"#;
        let out = parse_taxonomy(raw, &valid);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].path, vec!["Facturation"]);
        assert_eq!(out[0].theme_ids.len(), 3);
        assert_eq!(out[1].path, vec!["Immobilier", "Ventes"]);
        assert_eq!(out[1].theme_ids.len(), 2);
    }

    #[test]
    fn entity_slug_resout_les_variantes() {
        // Suffixe juridique + ponctuation ne doivent pas produire des entités distinctes.
        assert_eq!(entity_slug("Y SARL"), entity_slug("Y."));
        assert_eq!(entity_slug("Y SARL"), "y");
        assert_eq!(entity_slug("Acme Corp SAS"), "acme-corp");
        // Conservateur : on ne fusionne PAS sur un mot commun générique ("société").
        assert_ne!(entity_slug("Société Y"), entity_slug("Y SARL"));
    }

    #[test]
    fn parse_entities_filtre_generiques_et_dedup() {
        let raw = r#"{"entities": ["Acme SARL", "le client", "Acme", "Beta Corp"]}"#;
        let out = parse_entities(raw);
        // "Acme SARL" et "Acme" → même slug (une seule), "le client" générique écarté.
        assert_eq!(out.len(), 2, "attendu Acme + Beta Corp : {out:?}");
        assert!(out.iter().any(|e| e.starts_with("Acme")));
        assert!(out.contains(&"Beta Corp".to_string()));
    }

    #[test]
    fn parse_entities_tolere_l_inexploitable() {
        assert!(parse_entities("pas du json").is_empty());
        assert!(parse_entities(r#"{"entities": []}"#).is_empty());
        assert!(parse_entities(r#"{"autre": 1}"#).is_empty());
    }

    #[test]
    fn parse_assignments_ancre_sur_dossiers_existants() {
        let pages: HashSet<&str> = ["leaf:facture-1", "leaf:facture-2", "leaf:zzz"].into_iter().collect();
        let folders: HashSet<&str> = ["arch-theme-factures", "arch-cat-immobilier"].into_iter().collect();
        let raw = r#"{"assign": [
            {"page": "leaf:facture-1", "folder": "arch-theme-factures"},
            {"page": "leaf:facture-2", "folder": "arch-theme-factures"},
            {"page": "leaf:hallucinée", "folder": "arch-theme-factures"},
            {"page": "leaf:zzz", "folder": "dossier-inexistant"}
        ]}"#;
        let out = parse_assignments(raw, &pages, &folders);
        // Les 2 factures valides passent ; page hallucinée et dossier inexistant filtrés.
        assert_eq!(out.len(), 2, "{out:?}");
        assert!(out.iter().all(|(_, f)| f == "arch-theme-factures"));
    }

    #[test]
    fn parse_assignments_defensif() {
        let pages: HashSet<&str> = ["leaf:a"].into_iter().collect();
        let folders: HashSet<&str> = ["arch-theme-x"].into_iter().collect();
        assert!(parse_assignments("pas du json", &pages, &folders).is_empty());
        assert!(parse_assignments(r#"{"assign": []}"#, &pages, &folders).is_empty());
        assert!(parse_assignments(r#"{"autre": 1}"#, &pages, &folders).is_empty());
    }

    #[test]
    fn cluster_indices_regroupe_les_vecteurs_proches() {
        // 2 vecteurs quasi colinéaires (groupe A) + 2 autres orthogonaux (groupe B).
        let a1 = vec![1.0, 0.0, 0.0];
        let a2 = vec![0.98, 0.02, 0.0];
        let b1 = vec![0.0, 1.0, 0.0];
        let b2 = vec![0.0, 0.97, 0.03];
        let groups = cluster_indices(&[a1, a2, b1, b2]);
        assert_eq!(groups.len(), 2, "attendu 2 groupes : {groups:?}");
        assert!(groups.contains(&vec![0, 1]));
        assert!(groups.contains(&vec![2, 3]));
    }

    /// Régression 2026-07-29 : le single-linkage soudait une CHAÎNE de docs
    /// (voisins proches deux à deux mais extrêmes sans rapport) en un seul blob
    /// « Documents variés ». Avec k-NN mutuel + garde de cohésion, une telle
    /// chaîne ne doit JAMAIS ressortir en un seul groupe — elle est cassée /
    /// dissoute (ses docs iront en « Non triable »).
    #[test]
    fn ne_chaine_pas_une_serie_de_docs_disparates_en_un_blob() {
        // 5 vecteurs à 30° d'écart : chaque voisin est à cos(30°)=0.866 (lien),
        // mais les extrêmes sont à cos(120°)=-0.5 (aucun rapport). Single-linkage
        // les aurait tous fusionnés.
        let chain = vec![
            vec![1.0, 0.0], vec![0.866, 0.5], vec![0.5, 0.866], vec![0.0, 1.0], vec![-0.5, 0.866],
        ];
        let groups = cluster_indices(&chain);
        assert!(
            !groups.iter().any(|g| g.len() >= 4),
            "la chaîne hétérogène ne doit pas former un gros groupe : {groups:?}",
        );
    }

    /// ADR-0022 phase 1.3 — un corpus autonome ne doit pas RECEVOIR. Un dossier
    /// Obsidian ou un projet Claude Code est une base curée : y verser un PDF venu
    /// de Téléchargements pollue une structure à forte autorité. Le 2026-08-03,
    /// aucune contamination ne s'était produite, mais rien ne l'empêchait.
    #[test]
    fn seules_les_sources_autorisees_peuvent_recevoir_un_document() {
        assert!(RECEIVING_SOURCES.contains(&"local-folder"));
        assert!(RECEIVING_SOURCES.contains(&"google-drive"));
        for interdit in ["obsidian", "claude-code", "apple-notes", "chatgpt", "claude-ai"] {
            assert!(!RECEIVING_SOURCES.contains(&interdit),
                "{interdit} ne doit jamais être une destination d'ancrage");
        }
    }

    /// ADR-0022 phase 1.2 : ce que l'humain a rangé ne doit plus être dissous. Le
    /// vrac posé directement dans un dossier scanné reste candidat, le contenu d'un
    /// sous-dossier monté à la main ne l'est plus. Et le drapeau doit vraiment
    /// commander les deux comportements — c'est la marche arrière si la décision
    /// produit change (ADR-0022 encore `proposé`).
    #[test]
    fn respecte_les_sous_dossiers_montes_a_la_main_mais_pas_le_vrac() {
        let nodes = vec![
            node("root", "Cerveau", "root", None),
            node("p:Documents", "Documents", "container", Some("root")),
            node("p:Documents/Clients", "Clients", "container", Some("p:Documents")),
            // Rangé à la main dans un sous-dossier → doit être laissé tranquille.
            // UN SEUL document dedans, exprès : un dossier nommé qui n'en contient
            // qu'un reste un classement (débat tranché le 2026-08-19).
            local_leaf("leaf:range", "proposition-2026", "p:Documents/Clients"),
            // Posé en vrac directement dans Documents → doit rester candidat.
            local_leaf("leaf:vrac", "scan001", "p:Documents"),
        ];
        let g = graph(nodes);

        let respecte = scan(&g, true);
        let touches: Vec<&str> = respecte.moves.iter().map(|m| m.node_id.as_str())
            .chain(respecte.orphans_unresolved_ids.iter().map(String::as_str))
            .chain(respecte.theme_clusters.iter().flat_map(|c| c.node_ids.iter().map(String::as_str)))
            .collect();
        assert!(touches.contains(&"leaf:vrac"), "le vrac reste candidat : {touches:?}");
        assert!(!touches.contains(&"leaf:range"),
            "un document rangé à la main ne doit plus être candidat : {touches:?}");

        // Drapeau à false : ancien comportement, les deux sont candidats.
        let avant = scan(&g, false);
        let touches_avant: Vec<&str> = avant.moves.iter().map(|m| m.node_id.as_str())
            .chain(avant.orphans_unresolved_ids.iter().map(String::as_str))
            .chain(avant.theme_clusters.iter().flat_map(|c| c.node_ids.iter().map(String::as_str)))
            .collect();
        assert!(touches_avant.contains(&"leaf:range"),
            "drapeau à false = comportement d'avant, donc marche arrière réelle : {touches_avant:?}");
    }

    /// ADR-0020 niveau 2 pour Drive (décision Liam 2026-08-19) : l'arbo Drive est
    /// gardée, mais le vrac posé à la racine du Drive devient rangeable. Avant,
    /// `google-drive` était absent de `SORTABLE_CONNECTORS` : rien de Drive n'était
    /// jamais examiné, vrac compris.
    #[test]
    fn drive_arbo_gardee_mais_vrac_de_la_racine_rangeable() {
        let drive_leaf = |id: &str, label: &str, parent: &str| {
            let mut n = node(id, label, "leaf", Some(parent));
            n.connector = Some("google-drive".into());
            n
        };
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("p:Drive", "Google Drive", "container", Some("root")),
            node("p:Drive/Factures", "Factures", "container", Some("p:Drive")),
            node("p:Drive/Assurance", "Assurance habitation", "container", Some("p:Drive")),
            // Classement humain → intouchable.
            drive_leaf("d:range", "facture-ideeri-01", "p:Drive/Factures"),
            drive_leaf("d:range2", "facture-ideeri-02", "p:Drive/Factures"),
            // Dossier nommé avec UN SEUL document : classement qui attend d'être
            // rempli, protégé au même titre (débat tranché le 2026-08-19).
            drive_leaf("d:seul", "attestation-2026", "p:Drive/Assurance"),
            // Vrac à la racine du Drive → candidat, comme le vrac dans Documents.
            drive_leaf("d:vrac", "SCAN_20260414_0001", "p:Drive"),
        ]);
        let r = scan(&g, true);
        let touches: Vec<&str> = r.moves.iter().map(|m| m.node_id.as_str())
            .chain(r.orphans_unresolved_ids.iter().map(String::as_str))
            .chain(r.theme_clusters.iter().flat_map(|c| c.node_ids.iter().map(String::as_str)))
            .collect();
        assert!(touches.contains(&"d:vrac"), "le vrac Drive doit être candidat : {touches:?}");
        assert!(!touches.contains(&"d:range") && !touches.contains(&"d:range2"),
            "l'arbo Drive reste intouchée : {touches:?}");
        assert!(!touches.contains(&"d:seul"),
            "un dossier Drive nommé à 1 document reste un classement : {touches:?}");
    }

    /// Bande « certain » (ADR-0022) : deux documents au même texte sont le MÊME
    /// document, quels que soient leur nom et leur source. Aucune inférence, donc
    /// aucun modèle — mais c'est une fusion, donc elle doit être exacte.
    #[test]
    fn exact_duplicates_reconnait_le_meme_texte_et_garde_le_mieux_range() {
        let long = "Facture numéro 42 émise le 3 août 2026 pour la prestation de développement, montant total hors taxes de mille deux cents euros.";
        let mut doc = |id: &str, label: &str, parent: &str, text: &str| {
            let mut n = node(id, label, "leaf", Some(parent));
            n.source_text = text.into();
            n
        };
        let nodes = vec![
            node("root", "Cerveau", "root", None),
            node("p:Téléchargements", "Téléchargements", "container", Some("root")),
            node("p:Drive", "Drive", "container", Some("root")),
            node("p:Drive/Ideeri", "Ideeri", "container", Some("p:Drive")),
            node("p:Drive/Ideeri/Factures", "Factures", "container", Some("p:Drive/Ideeri")),
            // Même texte, mais espaces différents : c'est le cas réel de deux
            // extractions PDF différentes du même fichier.
            doc("leaf:vrac", "facture-042 (1)", "p:Téléchargements", &long.replace(' ', "\n  ")),
            doc("leaf:drive", "facture-042", "p:Drive/Ideeri/Factures", long),
            // Texte différent → jamais groupé.
            doc("leaf:autre", "facture-043", "p:Téléchargements", "Facture numéro 43 émise le 4 août 2026, montant de deux mille euros hors taxes pour du conseil."),
            // Texte trop court → pas une preuve d'identité (deux extractions ratées
            // ne sont pas le même document).
            doc("leaf:court-a", "vide-a", "p:Téléchargements", "n/a"),
            doc("leaf:court-b", "vide-b", "p:Téléchargements", "n/a"),
        ];
        let groups = exact_duplicates(&graph(nodes));
        assert_eq!(groups.len(), 1, "un seul groupe : {groups:?}");
        assert_eq!(groups[0].len(), 2);
        assert_eq!(groups[0][0], "leaf:drive",
            "le survivant est celui rangé le plus profondément, pas celui du vrac");
        assert_eq!(groups[0][1], "leaf:vrac");
    }

    /// Un doublon exact ne doit PAS être soumis en plus à la décision de Gemma, ni
    /// recevoir un déplacement — sinon la fusion s'applique, sa cible disparaît, et
    /// le déplacement reste bloqué à vie dans `mcp_pending/`.
    #[test]
    fn un_doublon_exact_est_retire_des_groupes_de_titres_et_des_candidats() {
        let long = "Attestation sur l'honneur établie le trois août deux mille vingt-six certifiant l'exactitude des informations déclarées par le signataire.";
        let mut doc = |id: &str, text: &str| {
            let mut n = node(id, "Attestation", "leaf", Some("p:Documents"));
            n.source_text = text.into();
            n.connector = Some("local-folder".into());
            n
        };
        let nodes = vec![
            node("root", "Cerveau", "root", None),
            node("p:Documents", "Documents", "container", Some("root")),
            doc("leaf:a", long),
            doc("leaf:b", long),
        ];
        let g = graph(nodes);
        let r = scan(&g, false);
        assert_eq!(r.exact_duplicates.len(), 1, "le doublon exact est détecté");
        assert!(r.groups.is_empty(),
            "les deux titres identiques ne forment plus un groupe à faire trancher : {:?}",
            r.groups.iter().map(|x| &x.label).collect::<Vec<_>>());
        let absorbed = &r.exact_duplicates[0][1];
        assert!(!r.moves.iter().any(|m| &m.node_id == absorbed),
            "l'absorbé ne reçoit aucun déplacement");
        assert!(!r.orphans_unresolved_ids.contains(absorbed),
            "l'absorbé n'est pas non plus routé vers le bac");
    }

    #[test]
    fn filename_family_ne_retient_que_les_noms_structures() {
        // Numéro d'ordre retiré, code de l'émetteur conservé → deux émetteurs
        // différents donnent deux familles différentes.
        assert_eq!(filename_family("Invoice-14545AA1-0036").as_deref(), Some("invoice-14545aa1"));
        assert_eq!(filename_family("Invoice-14545AA1-0044").as_deref(), Some("invoice-14545aa1"));
        assert_eq!(filename_family("Invoice-21A6E515-0003").as_deref(), Some("invoice-21a6e515"));
        assert_eq!(filename_family("dbo-commune_france-41-live.1765355792").as_deref(),
            Some("dbo-commune-france-41-live"));
        // De la prose : aucun chiffre → pas une famille, sinon n'importe quel
        // document « métier » ferait famille avec n'importe quel autre.
        assert_eq!(filename_family("Contenu site business"), None);
        assert_eq!(filename_family("Village Immobilier Mions"), None);
        // Réduit à un seul segment après retrait des chiffres de fin → trop pauvre.
        assert_eq!(filename_family("Document-20-01"), None);
        assert_eq!(filename_family("RIB"), None);
    }

    /// La fusion doit se déclencher sur une PREUVE et jamais au-delà — c'est ce qui
    /// la distingue de la passe de taxonomie qu'on a coupée (2026-08-03).
    #[test]
    fn folder_merges_fusionne_les_redondances_et_epargne_le_reste() {
        let mut nodes = vec![node("root", "Cerveau", "root", None)];
        let mut domains: HashMap<String, String> = HashMap::new();
        let add = |nodes: &mut Vec<BrainNode>, domains: &mut HashMap<String, String>,
                       folder: &str, label: &str, docs: &[&str], dom: &str| {
            nodes.push(node(folder, label, "note", Some("root")));
            for (i, d) in docs.iter().enumerate() {
                let id = format!("{folder}::{i}");
                nodes.push(node(&id, d, "leaf", Some(folder)));
                domains.insert(id, dom.to_string());
            }
        };
        // Trois dossiers de communes aux noms proches, même domaine → un seul.
        add(&mut nodes, &mut domains, "arch-theme-a", "Données Communales France",
            &["dbo-commune_france-41-live.1765355792", "dbo-commune_france-41-live.1766567021"], "Technique & Data");
        add(&mut nodes, &mut domains, "arch-theme-b", "Données Communes France",
            &["communes_location_only", "communes_location_only_v2"], "Technique & Data");
        add(&mut nodes, &mut domains, "arch-theme-c", "Données Géographiques Communes",
            &["api-transaction - Communes - 6620d8", "api-transaction - Communes - 6620d9"], "Technique & Data");
        // Deux dossiers de factures du MÊME émetteur : noms trop éloignés (0,33),
        // mais même famille de nom de fichier → fusionnés par le 2ᵉ signal.
        add(&mut nodes, &mut domains, "arch-theme-d", "Factures Services",
            &["Invoice-14545AA1-0044", "Invoice-14545AA1-0046"], "Facturation");
        add(&mut nodes, &mut domains, "arch-theme-e", "Factures Parabola",
            &["Invoice-14545AA1-0036", "Invoice-14545AA1-0038"], "Facturation");
        // Émetteur DIFFÉRENT : même préfixe « Invoice » mais autre code → intact.
        add(&mut nodes, &mut domains, "arch-theme-f", "Factures Xano",
            &["Invoice-21A6E515-0003", "Invoice-21A6E515-0004"], "Facturation");
        // Même premier mot, sujets distincts → jamais fusionnés (recouvrement 0,25).
        add(&mut nodes, &mut domains, "arch-theme-g", "Immobilier Saint-Jean",
            &["webflow-301-redirects", "webflow-301-redirects-v2"], "Immobilier");
        add(&mut nodes, &mut domains, "arch-theme-h", "Immobilier Lyon",
            &["Lyon_8e_Arrondissement", "Mions"], "Immobilier");

        let g = graph(nodes);
        let merges = folder_merges(&g, &domains);
        let groups: Vec<HashSet<&str>> = merges.iter()
            .map(|m| {
                let mut s: HashSet<&str> = m.absorbed_ids.iter().map(String::as_str).collect();
                s.insert(m.survivor_id.as_str());
                s
            })
            .collect();

        assert!(groups.iter().any(|s| s.len() == 3
            && s.contains("arch-theme-a") && s.contains("arch-theme-b") && s.contains("arch-theme-c")),
            "les trois dossiers de communes fusionnent : {groups:?}");
        assert!(groups.iter().any(|s| s.len() == 2
            && s.contains("arch-theme-d") && s.contains("arch-theme-e")),
            "les deux dossiers du même émetteur fusionnent : {groups:?}");
        let touched: HashSet<&str> = groups.iter().flatten().copied().collect();
        for intact in ["arch-theme-f", "arch-theme-g", "arch-theme-h"] {
            assert!(!touched.contains(intact), "{intact} ne doit PAS être fusionné");
        }
        assert_eq!(groups.len(), 2, "exactement deux fusions : {groups:?}");
    }

    /// La garde de domaine doit primer : deux noms identiques mais deux domaines
    /// différents restent séparés (c'est le garde-fou anti-devinette).
    #[test]
    fn folder_merges_refuse_de_traverser_deux_domaines() {
        let mut domains: HashMap<String, String> = HashMap::new();
        let nodes = vec![
            node("root", "Cerveau", "root", None),
            node("arch-theme-x", "Dossier Estimation", "note", Some("root")),
            node("arch-theme-x::0", "estimation_a", "leaf", Some("arch-theme-x")),
            node("arch-theme-y", "Dossier Estimations", "note", Some("root")),
            node("arch-theme-y::0", "estimation_b", "leaf", Some("arch-theme-y")),
        ];
        domains.insert("arch-theme-x::0".into(), "Immobilier".into());
        domains.insert("arch-theme-y::0".into(), "Devis & Commercial".into());
        assert!(folder_merges(&graph(nodes), &domains).is_empty());
    }

    #[test]
    fn domain_from_code_resout_les_codes_et_rejette_le_reste() {
        assert_eq!(domain_from_code("IMM"), Some("Immobilier"));
        assert_eq!(domain_from_code("fin"), Some("Finance & Trading")); // casse tolérée
        assert_eq!(domain_from_code(" JUR "), Some("Contrats & Juridique")); // espaces
        // Tolérance : le modèle recopie parfois le nom complet malgré la consigne.
        assert_eq!(domain_from_code("Immobilier"), Some("Immobilier"));
        // Rien d'exploitable ⇒ None, JAMAIS un repli : le document restera non
        // tagué et sera retenté au scan suivant (« Autre » n'existe plus).
        assert_eq!(domain_from_code("XYZ"), None);
        assert_eq!(domain_from_code("banane"), None);
        assert_eq!(domain_from_code(""), None);
        assert!(!DOMAIN_LIST.contains(&"Autre"), "« Autre » ne doit plus être un domaine");
    }

    /// Format compact (2026-08-03) : map plate numéro → code de 3 lettres, ~3× moins
    /// de tokens générés que l'ancien `{"tags":[{"n":1,"domain":"…"}]}`. Les codes
    /// sont résolus en noms complets AVANT de sortir, pour que le cache et la garde
    /// de domaine ne voient jamais un code.
    #[test]
    fn parse_domains_lit_le_format_compact_et_resout_les_codes() {
        let raw = r#"{"1":"FIN","2":"IMM","3":"TEC","9":"FAC"}"#;
        let out = parse_domains(raw, 3); // lot de 3 docs
        assert_eq!(out.get(&1).map(|s| s.as_str()), Some("Finance & Trading"));
        assert_eq!(out.get(&2).map(|s| s.as_str()), Some("Immobilier"));
        assert_eq!(out.get(&3).map(|s| s.as_str()), Some("Technique & Data"));
        assert!(!out.contains_key(&9), "un numéro hors borne (halluciné) doit être ignoré");
    }

    /// Un code inconnu ne doit RIEN produire — surtout pas un repli. Le document
    /// reste absent du résultat, donc absent du cache, donc retenté au scan suivant.
    #[test]
    fn parse_domains_laisse_non_tague_un_code_inconnu() {
        let raw = r#"{"1":"FAC","2":"ZZZ","3":"","4":42}"#;
        let out = parse_domains(raw, 4);
        assert_eq!(out.get(&1).map(|s| s.as_str()), Some("Facturation"));
        assert!(!out.contains_key(&2), "code inconnu → non tagué");
        assert!(!out.contains_key(&3), "code vide → non tagué");
        assert!(!out.contains_key(&4), "valeur non textuelle → non taguée");
        assert_eq!(out.len(), 1, "seul le document classé sûrement ressort");
    }

    /// Une réponse dans l'ANCIEN format ne doit plus rien produire : le lot est
    /// alors compté en échec par `ai_domain_tags` (`parsed.is_empty()`) et retenté,
    /// au lieu d'être silencieusement mal interprété.
    #[test]
    fn parse_domains_ne_lit_plus_lancien_format_verbeux() {
        let raw = r#"{"tags":[{"n":1,"domain":"Immobilier"}]}"#;
        assert!(parse_domains(raw, 1).is_empty());
    }

    #[test]
    fn cluster_indices_isole_un_vecteur_sans_voisin() {
        let a1 = vec![1.0, 0.0];
        let a2 = vec![0.99, 0.01];
        let lonely = vec![0.0, 1.0];
        let groups = cluster_indices(&[a1, a2, lonely]);
        // le solitaire forme son propre groupe (taille 1) — l'appelant filtrera sur MIN_CLUSTER
        assert!(groups.iter().any(|g| g == &vec![0, 1]));
        assert!(groups.iter().any(|g| g == &vec![2]));
    }

    #[test]
    fn collapse_taxonomy_aplati_les_niveaux_inutiles() {
        let g = |p: &[&str]| TaxonomyGroup { path: p.iter().map(|s| s.to_string()).collect(), theme_ids: vec!["x".into(), "y".into()] };
        // Chemins à 2 niveaux dont le 1er segment n'est PAS partagé → aplatis au 1er.
        let out = collapse_taxonomy(vec![g(&["Administration", "Documents"]), g(&["Gestion", "Processus"])]);
        assert_eq!(out[0].path, vec!["Administration"]);
        assert_eq!(out[1].path, vec!["Gestion"]);
        // Préfixe partagé par 2 groupes → hiérarchie justifiée, on GARDE 2 niveaux.
        let out2 = collapse_taxonomy(vec![g(&["Immobilier", "Ventes"]), g(&["Immobilier", "Locations"])]);
        assert_eq!(out2[0].path, vec!["Immobilier", "Ventes"]);
        assert_eq!(out2[1].path, vec!["Immobilier", "Locations"]);
    }

    #[test]
    fn parse_taxonomy_defensif() {
        let valid: HashSet<&str> = ["arch-theme-devis", "arch-theme-invoice"].into_iter().collect();
        // chemin chapeautant < MIN_SUPERGROUP (2) thèmes valides → écarté
        assert!(parse_taxonomy(r#"{"groups": [{"path": "X", "themes": ["arch-theme-devis", "zzz"]}]}"#, &valid).is_empty());
        // chemin vide écarté ; JSON invalide toléré
        assert!(parse_taxonomy(r#"{"groups": [{"path": "", "themes": ["arch-theme-devis", "arch-theme-invoice"]}]}"#, &valid).is_empty());
        assert!(parse_taxonomy("pas du json", &valid).is_empty());
        // profondeur excessive (> MAX_TAXO_DEPTH) écartée
        assert!(parse_taxonomy(r#"{"groups": [{"path": "a/b/c/d/e", "themes": ["arch-theme-devis", "arch-theme-invoice"]}]}"#, &valid).is_empty());
    }
    // ── Nom de dossier : ce qu'il promet doit être dans le dossier ────────────
    // Cas réel du 2026-08-24 (noms fictifs ici) : 7 devis, 5 pour le même client,
    // le dossier prenait le nom du client et avalait une estimation auto.

    fn devis_samples() -> Vec<String> {
        let mut v: Vec<String> = (0..5)
            .map(|i| format!("Devis {i} — Proposition commerciale pour Novolia SAS, prestation annuelle."))
            .collect();
        v.push("Estimation remplacement radar avant — véhicule, pièces et main d'oeuvre.".to_string());
        v.push("Devis toiture — remplacement de tuiles, échafaudage compris.".to_string());
        v
    }

    #[test]
    fn nom_de_dossier_retire_le_client_qui_ne_couvre_pas_le_groupe() {
        let s = devis_samples(); // « novolia » : 5/7 = 71 %, sous le seuil de 80 %
        assert_eq!(name_supported_by("Devis Novolia", Some("Novolia"), &s).as_deref(), Some("Devis"));
    }

    /// Cas réel du 2026-08-24 : « Formations Pharmadvance » avait avalé une étude
    /// de marché. 4 documents à ~0,98 du centroïde, l'intrus à 0,879 — la MOYENNE
    /// (0,959) passait le plancher de cohésion sans qu'on regarde le décroché.
    #[test]
    fn un_membre_decroche_est_retire_meme_si_la_moyenne_est_bonne() {
        // 4 vecteurs quasi identiques + 1 nettement à l'écart, en 3 dimensions.
        let serre = vec![
            vec![1.0, 0.02, 0.0],
            vec![1.0, 0.00, 0.02],
            vec![0.99, 0.03, 0.01],
            vec![1.0, 0.01, 0.03],
        ];
        let mut vecs = serre.clone();
        // L'intrus doit être assez proche pour ÊTRE lié (plancher 0,80) mais assez
        // loin pour décrocher — sinon le test passerait pour la mauvaise raison.
        vecs.push(vec![1.0, 0.50, 0.0]); // ~0,89 avec les autres
        let groupes = cluster_indices_with(&vecs, CLUSTER_COHESION_MIN, CLUSTER_OUTLIER_MARGIN);
        let garde: Vec<&Vec<usize>> = groupes.iter().filter(|g| g.len() >= 3).collect();
        assert_eq!(garde.len(), 1, "un seul vrai groupe attendu : {groupes:?}");
        assert_eq!(garde[0], &vec![0, 1, 2, 3], "l'intrus (index 4) doit être sorti");

        // Marge à 1.0 = garde désactivée : l'intrus reste, preuve que c'est bien
        // ce critère-là qui l'a retiré (et qu'il est réglable sans recompiler).
        let sans_garde = cluster_indices_with(&vecs, CLUSTER_COHESION_MIN, 1.0);
        assert!(sans_garde.iter().any(|g| g.len() == 5), "sans la garde, le groupe reste entier");
    }

    /// Un groupe de 3 ne se fait jamais amputer : à 2 membres l'appelant le
    /// dissoudrait, ce qui coûterait plus cher que l'intrus.
    #[test]
    fn un_groupe_de_trois_nest_jamais_ampute() {
        let vecs = vec![
            vec![1.0, 0.02, 0.0],
            vec![1.0, 0.00, 0.02],
            vec![1.0, 0.50, 0.0], // décroché (~0,89), mais on est trois
        ];
        let groupes = cluster_indices_with(&vecs, 0.5, CLUSTER_OUTLIER_MARGIN);
        assert!(groupes.iter().any(|g| g.len() == 3), "les 3 restent ensemble : {groupes:?}");
    }

    #[test]
    fn nom_de_dossier_tolere_un_seul_document_sans_lentite() {
        // Groupe voiture réel : 4 documents, 3 citent le modèle dans leur extrait,
        // le quatrième a un en-tête d'assureur qui mange les 200 premiers caractères.
        // 75 % < 80 %, mais un seul manquant : le dossier doit vivre.
        let mut s: Vec<String> = (0..3).map(|i| format!("Document {i} — Peugeot 208, immatriculation AB-123-CD.")).collect();
        s.push("Attestation — Assurances Meridian, 3 place du Marché, contrat 447-882-19.".to_string());
        assert_eq!(name_supported_by("Véhicule Peugeot 208", Some("Peugeot 208"), &s).as_deref(),
                   Some("Véhicule Peugeot 208"));
    }

    #[test]
    fn nom_de_dossier_garde_le_client_qui_couvre_tout() {
        let s: Vec<String> = (0..6)
            .map(|i| format!("Facture {i} — Parabola SARL, prestation mensuelle."))
            .collect();
        assert_eq!(name_supported_by("Factures Parabola", Some("Parabola SARL"), &s).as_deref(), Some("Factures Parabola"));
    }

    #[test]
    fn nom_de_dossier_garde_un_mot_purement_thematique() {
        // « immobilier » n'apparaît dans AUCUN document : c'est un jugement du
        // modèle, pas une promesse d'entité. Mesuré : 8 clusters sur 19 en ont un.
        let s: Vec<String> = ["Compromis de vente — appartement T3, 62 m2.",
                              "Acte notarié — cession de lot, copropriété."]
            .iter().map(|x| x.to_string()).collect();
        assert_eq!(name_supported_by("Immobilier", None, &s).as_deref(), Some("Immobilier"));
    }

    #[test]
    fn nom_de_dossier_tolere_un_intrus_isole_sur_un_grand_groupe() {
        // 9/10 = 90 % : un seul document hors sujet ne fait pas tomber le nom.
        let mut s: Vec<String> = (0..9).map(|i| format!("Bail {i} — Résidence Kestrel, loyer mensuel.")).collect();
        s.push("Attestation d'assurance — habitation.".to_string());
        assert_eq!(name_supported_by("Baux Kestrel", Some("Résidence Kestrel"), &s).as_deref(), Some("Baux Kestrel"));
    }

    #[test]
    fn nom_de_dossier_sans_rien_de_soutenu_renonce() {
        // Les deux mots viennent d'un seul document sur cinq : on ne nomme pas.
        let mut s: Vec<String> = (0..4).map(|i| format!("Note de service {i} — organisation interne.")).collect();
        s.push("Kestrel Novolia — protocole d'accord.".to_string());
        assert_eq!(name_supported_by("Kestrel Novolia", Some("Kestrel Novolia"), &s), None);
    }

    #[test]
    fn nom_de_dossier_ne_retire_jamais_le_mot_de_type() {
        // Régression du 24/08 : « devis » présent dans 7 documents sur 11 (64 %)
        // tombait comme un nom d'entité, le nom devenait vide et LE GROUPE ENTIER
        // repartait en « Non triable ». Seule l'entité déclarée est vérifiée.
        let mut s: Vec<String> = (0..7).map(|i| format!("Devis {i} — proposition commerciale.")).collect();
        s.extend((0..4).map(|i| format!("Note interne {i} — organisation du service.")));
        assert_eq!(name_supported_by("Devis Novolia", Some("Novolia"), &s).as_deref(), Some("Devis"));
        // Et sans entité déclarée, le nom passe tel quel : on ne devine pas.
        assert_eq!(name_supported_by("Devis Commerciaux", None, &s).as_deref(), Some("Devis Commerciaux"));
    }

    #[test]
    fn nom_de_dossier_ne_laisse_pas_un_mot_outil_orphelin() {
        let s: Vec<String> = (0..5).map(|i| format!("Contrat {i} — prestation de service.")).collect();
        // « novolia » absent de tous les documents SAUF un : retiré, et le « de »
        // qui le précédait ne doit pas rester à traîner en fin de nom.
        let mut s2 = s.clone();
        s2[0] = "Contrat 0 — prestation Novolia.".to_string();
        assert_eq!(name_supported_by("Contrats de Novolia", Some("Novolia"), &s2).as_deref(), Some("Contrats"));
    }
}
