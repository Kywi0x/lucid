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
        match engine.complete(Some(SYSTEM_PROMPT), &prompt, RESPONSE_TOKENS) {
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

/// Groupes d'INDICES (dans l'ordre de `vecs`) par **k-NN mutuel + garde de
/// cohésion**. Remplace le single-linkage par seuil absolu, qui chaînait A~B~C
/// en un seul blob dès qu'UN lien faible existait — sur 202 docs réels ça a
/// soudé 130 documents hétérogènes dans un même dossier (bug 2026-07-29). Ici un
/// lien n'existe que si deux docs sont RÉCIPROQUEMENT dans leurs k plus proches
/// voisins (résiste au chaînage par un doc « hub »), puis chaque composante n'est
/// gardée que si elle est cohésive autour de son centroïde. Pur → testable sans
/// moteur.
pub fn cluster_indices(vecs: &[Vec<f32>]) -> Vec<Vec<usize>> {
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
        out.extend(cohesive_split(vecs, members));
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
fn cohesive_split(vecs: &[Vec<f32>], mut members: Vec<usize>) -> Vec<Vec<usize>> {
    if members.len() < 2 { return vec![members]; }
    let mut dropped: Vec<usize> = Vec::new();
    loop {
        let c = centroid_of(vecs, &members);
        let sims: Vec<(usize, f32)> = members.iter().map(|&i| (i, crate::ai::llama::cosine(&vecs[i], &c))).collect();
        let avg = sims.iter().map(|(_, s)| s).sum::<f32>() / sims.len() as f32;
        if avg >= CLUSTER_COHESION_MIN {
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

/// Nomme un groupe (Phase 2) : Gemma ne fait QUE ça — donner un nom de dossier
/// court à partir d'un échantillon des documents du groupe. `samples` = titres
/// (+ éventuel extrait). Renvoie un label court, ou `None` si réponse inexploitable.
pub fn ai_name_cluster(engine: &LlamaEngine, samples: &[String]) -> Option<String> {
    if samples.is_empty() { return None; }
    let mut prompt = String::from("Voici des documents d'un même dossier (titre — extrait du contenu) :\n");
    for s in samples.iter().take(12) {
        prompt.push_str(&format!("- {}\n", s.chars().take(200).collect::<String>()));
    }
    prompt.push_str(
        "\nDonne un nom de DOSSIER court (2 à 4 mots, français). PRIORITÉ ABSOLUE : si un même CLIENT / \
         ENTREPRISE / PERSONNE revient dans ces documents, nomme le dossier d'après lui (ex. « Factures \
         Parabola », « Devis Novolia ») — c'est le nom du client qui compte, PAS un mot de rôle générique \
         comme « Services », « Prestations », « Documents ». Seulement s'il n'y a AUCUN client identifiable, \
         donne un nom thématique court (ex. « Factures », « Immobilier »). JAMAIS une phrase ni « Divers ».\n\
         Renvoie UNIQUEMENT : {\"name\": \"…\"}\n",
    );
    let raw = engine.complete(Some(SYSTEM_PROMPT), &prompt, 60).ok()?;
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
    let low = name.to_lowercase();
    if GENERIC.iter().any(|g| low.contains(g)) { return None; }
    Some(name.to_string())
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

/// Taxonomie fermée. « Autre » = repli sûr (retombe sur le clustering embedding
/// puis le bac « Non triable »). À ajuster après essai réel — volontairement court.
pub const DOMAIN_LIST: [&str; 10] = [
    "Facturation", "Devis & Commercial", "Immobilier", "Finance & Trading",
    "Contrats & Juridique", "Identité & Papiers", "Études & Cours", "Santé",
    "Technique & Data", "Autre",
];
const DOMAIN_BATCH_MAX: usize = 30;

/// Ramène une chaîne libre au domaine canonique le plus proche, sinon « Autre ».
/// Robuste aux variantes de casse/formulation de Gemma.
pub fn normalize_domain(raw: &str) -> String {
    let low = raw.trim().to_lowercase();
    if low.is_empty() { return "Autre".to_string(); }
    // Match exact d'abord, puis inclusion dans un sens ou l'autre (ex. « finance »
    // → « Finance & Trading », « trading » aussi).
    for d in DOMAIN_LIST {
        let dl = d.to_lowercase();
        if dl == low { return d.to_string(); }
    }
    for d in DOMAIN_LIST {
        let dl = d.to_lowercase();
        if dl.split(&[' ', '&'][..]).any(|w| !w.trim().is_empty() && low.contains(w.trim()))
            || low.split(&[' ', '&'][..]).any(|w| !w.trim().is_empty() && dl.contains(w.trim()))
        {
            return d.to_string();
        }
    }
    "Autre".to_string()
}

fn domain_prompt(docs: &[(String, String, String)]) -> String {
    let mut out = String::from("Classe chaque document dans UN de ces domaines (recopie le nom EXACT) :\n");
    for d in DOMAIN_LIST { out.push_str(&format!("- {d}\n")); }
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
        "\nFonde-toi sur le SUJET RÉEL (le contenu), pas la forme. En cas de doute, « Autre ».\n\
         Renvoie UNIQUEMENT le JSON :\n{\"tags\": [{\"n\": 1, \"domain\": \"…\"}, ...]}\n",
    );
    out
}

/// Parse la réponse de classification en `numéro (1-based) → domaine canonique`.
/// Ne garde que les numéros dans la borne du lot (anti-hallucination), normalise.
pub fn parse_domains(raw: &str, count: usize) -> std::collections::HashMap<usize, String> {
    let mut out = std::collections::HashMap::new();
    let Some(js) = crate::ai::pipeline::extract_json(raw) else { return out };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(js) else { return out };
    let Some(arr) = v.get("tags").and_then(|t| t.as_array()) else { return out };
    for e in arr {
        let (Some(n), Some(dom)) = (e.get("n").and_then(|x| x.as_u64()), e.get("domain").and_then(|x| x.as_str())) else { continue };
        let idx = n as usize;
        if idx >= 1 && idx <= count { out.insert(idx, normalize_domain(dom)); }
    }
    out
}

/// Tag de domaine pour un lot de docs (id, titre, extrait) — par batches pour
/// limiter les appels Gemma. Les docs sans réponse exploitable retombent sur
/// « Autre » (jamais perdus). L'appelant met le résultat en cache.
pub fn ai_domain_tags(engine: &LlamaEngine, docs: &[(String, String, String)]) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for chunk in docs.chunks(DOMAIN_BATCH_MAX) {
        // Réponse courte (numéro + domaine) → 900 tokens laissent large pour 30 items.
        let parsed = engine
            .complete(Some(SYSTEM_PROMPT), &domain_prompt(chunk), 900)
            .ok()
            .map(|raw| parse_domains(&raw, chunk.len()))
            .unwrap_or_default();
        for (i, (id, _, _)) in chunk.iter().enumerate() {
            out.insert(id.clone(), parsed.get(&(i + 1)).cloned().unwrap_or_else(|| "Autre".to_string()));
        }
    }
    out
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
        let Ok(raw) = engine.complete(Some(SYSTEM_PROMPT), &prompt, 2000) else { continue };
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
    let Ok(raw) = engine.complete(Some(SYSTEM_PROMPT), &prompt, 2000) else {
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
    let Ok(raw) = engine.complete(Some(SYSTEM_PROMPT), &prompt, 300) else {
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

    #[test]
    fn normalize_domain_ramene_a_la_taxonomie() {
        assert_eq!(normalize_domain("Immobilier"), "Immobilier");
        assert_eq!(normalize_domain("trading"), "Finance & Trading");   // mot partiel
        assert_eq!(normalize_domain("FINANCE"), "Finance & Trading");   // casse
        assert_eq!(normalize_domain("banane"), "Autre");                // hors liste
        assert_eq!(normalize_domain(""), "Autre");
    }

    #[test]
    fn parse_domains_filtre_les_numeros_hors_borne_et_normalise() {
        let raw = r#"{"tags":[{"n":1,"domain":"Trading"},{"n":2,"domain":"Immobilier"},{"n":9,"domain":"Immobilier"}]}"#;
        let out = parse_domains(raw, 2); // lot de 2 docs
        assert_eq!(out.get(&1).map(|s| s.as_str()), Some("Finance & Trading"));
        assert_eq!(out.get(&2).map(|s| s.as_str()), Some("Immobilier"));
        assert!(!out.contains_key(&9), "un numéro hors borne (halluciné) doit être ignoré");
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
}
