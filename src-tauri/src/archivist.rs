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
pub fn scan(graph: &BrainGraph) -> ScanResult {
    let root_id = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());

    let mut by_label: HashMap<String, Vec<&BrainNode>> = HashMap::new();
    for n in &graph.nodes {
        if n.kind == "root" {
            continue;
        }
        by_label.entry(normalize(&n.label)).or_default().push(n);
    }
    let groups: Vec<DuplicateGroup> = by_label
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(label, v)| DuplicateGroup { label, node_ids: v.iter().map(|n| n.id.clone()).collect() })
        .collect();
    let grouped_ids: HashSet<&str> =
        groups.iter().flat_map(|g| g.node_ids.iter().map(String::as_str)).collect();

    let parent_of: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .filter_map(|n| n.parent_id.as_deref().map(|p| (n.id.as_str(), p)))
        .collect();

    // Dossiers de scan brut : conteneurs sous la racine avec au moins une
    // feuille locale quelque part en dessous.
    let scan_root_ids: HashSet<&str> = graph
        .nodes
        .iter()
        .filter(|c| c.kind == "container" && c.parent_id.as_deref() == root_id.as_deref())
        .filter(|c| {
            graph.nodes.iter().any(|n| {
                n.connector.as_deref() == Some("local-folder")
                    && ancestor_chain(&n.id, &parent_of).contains(&c.id.as_str())
            })
        })
        .map(|c| c.id.as_str())
        .collect();
    let under_scan_root = |id: &str| ancestor_chain(id, &parent_of).iter().any(|a| scan_root_ids.contains(a));

    // Conteneurs candidats pour le rattachement — jamais la racine, jamais un
    // dossier de scan brut ni l'un de ses sous-dossiers (on range VERS un vrai
    // dossier thématique, pas d'un sous-dossier de Downloads vers un autre).
    let container_tokens: Vec<(&BrainNode, HashSet<String>)> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "container" && Some(&n.id) != root_id.as_ref())
        .filter(|c| !scan_root_ids.contains(c.id.as_str()) && !under_scan_root(&c.id))
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
            if Some(n.id.as_str()) == catchall_id || has_children.contains(n.id.as_str()) {
                continue;
            }
            let direct_root_child = n.parent_id.as_ref() == Some(root);
            let nested_in_scan_root = under_scan_root(&n.id);
            if !direct_root_child && !nested_in_scan_root {
                continue; // déjà dans un dossier thématique choisi, on n'y touche pas
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
        orphans_unresolved,
        orphans_unresolved_ids,
        theme_clusters,
        catchall_id: catchall_id.map(str::to_string),
    }
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

fn ai_cluster_prompt(leftover: &[(String, String)]) -> String {
    let mut out = format!(
        "Voici {n} pages du second cerveau d'un utilisateur, actuellement sans dossier clair \
         (aucun mot de titre partagé assez souvent pour un rangement mécanique).\n\n",
        n = leftover.len(),
    );
    for (id, label) in leftover {
        out.push_str(&format!("- id `{id}` : {label}\n"));
    }
    out.push_str(
        "\nPropose de regrouper par THÈME les pages qui traitent clairement du même sujet concret \
         (même type de document, même projet, même fournisseur...), MÊME si leurs titres ne partagent \
         aucun mot — c'est tout l'intérêt de cette passe, le tri mécanique par mot commun a déjà eu sa \
         chance avant toi. Un groupe doit contenir au moins 3 pages et un thème réel, pas un fourre-tout \
         vague (\"Divers\", \"Autres\"...). Laisse de côté (n'inclus dans aucun groupe) toute page isolée \
         ou dont le sujet est incertain — ne devine jamais.\n\n\
         Renvoie UNIQUEMENT un JSON :\n\
         {\"groups\": [{\"label\": \"Nom du thème, 1 à 3 mots, en français\", \"ids\": [\"id ci-dessus\", ...]}]}\n\
         (\"groups\" peut être vide si rien ne se regroupe clairement.)\n",
    );
    out
}

/// Clustering SÉMANTIQUE (Gemma) de ce qui reste après le tri mécanique par
/// mot commun — capte les regroupements qu'un script ne peut pas voir (même
/// sujet, aucun mot partagé). Jamais appelé sur ce que le script a déjà su
/// grouper seul.
pub fn ai_cluster_leftovers(engine: &LlamaEngine, leftover: &[(String, String)]) -> Vec<ThemeCluster> {
    if leftover.len() < MIN_CLUSTER {
        return Vec::new();
    }
    let capped: Vec<(String, String)> = leftover.iter().take(AI_CLUSTER_MAX_CANDIDATES).cloned().collect();
    let valid_ids: HashSet<&str> = capped.iter().map(|(id, _)| id.as_str()).collect();

    let prompt = ai_cluster_prompt(&capped);
    let Ok(raw) = engine.complete(Some(SYSTEM_PROMPT), &prompt, 700) else {
        return Vec::new();
    };
    parse_ai_cluster_response(&raw, &valid_ids)
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

/// Décision Gemma pour UN groupe de titres proches — jamais appelé pour les
/// cas que le script tranche seul. Ne fusionne jamais sur un doute (id
/// halluciné, JSON invalide, décision inattendue → `ParseFailed`, pas `Merge`).
pub fn decide_group(engine: &LlamaEngine, group: &DuplicateGroup, graph: &BrainGraph) -> GroupOutcome {
    let prompt = decision_prompt(group, graph);
    let raw = match engine.complete(Some(SYSTEM_PROMPT), &prompt, 200) {
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

    #[test]
    fn routes_unmatched_orphan_to_existing_catchall() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
            node("catchall", CATCHALL_LABEL, "container", Some("root")),
            local_leaf("orphan", "Invoice-14545AA1-0052", "downloads"),
        ]);
        assert!(!needs_catchall(&g));
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
        assert!(r.theme_clusters.is_empty(), "2 pages ne suffisent pas (seuil à 3)");
    }

    #[test]
    fn finds_exact_duplicate_titles() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("a", "RIB", "leaf", Some("root")),
            node("b", "RIB", "leaf", Some("root")),
        ]);
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
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
        let r = scan(&g);
        assert!(r.moves.is_empty());
        assert_eq!(r.orphans_unresolved.len(), 1);
    }

    #[test]
    fn top_level_container_is_never_treated_as_orphan() {
        let g = graph(vec![
            node("root", "Cerveau", "root", None),
            node("downloads", "Downloads", "container", Some("root")),
        ]);
        let r = scan(&g);
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
}
