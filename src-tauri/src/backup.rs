//! Sauvegarde / restauration du cerveau : zip en mémoire des données utiles
//! (~2 Mo — brain.json, notes, historique, images…), à destination du cloud
//! (Supabase Storage, uploadé côté front). Les modèles IA et llama.cpp sont
//! exclus : retéléchargeables, et ~10 Go.

use std::io::{Read, Write};
use std::path::Path;

/// Fichiers / dossiers embarqués dans la sauvegarde (relatifs au dossier de données).
/// `brain.db` (Phase 8) coexiste avec `brain.json` : un compte migré n'a plus
/// que le premier, un compte pas encore migré n'a que le second — `is_file()`
/// dans `export_in` ignore silencieusement celui qui est absent.
pub const FILES: &[&str] = &[
    "brain.json",
    "brain.db",
    "brain.md",
    "spaces.json",
    "deleted_nodes.json",
    "deleted_spaces.json",
    "brain_cache.json",
    "google_drive_conversations.json",
];
pub const DIRS: &[&str] = &["snapshots", "node_history", "assets", "mcp_pending"];

// ── Cache mémoire de brain.json ──────────────────────────────────────────────
// Chaque commande relisait le fichier entier depuis le disque, même pour une
// action minime — coûteux sur un cerveau de plusieurs dizaines de Mo, et
// aggravé par le mode autonome (MCP) qui enchaîne des actions rapidement (bug
// remonté par Liam le 2026-07-21 : "il doit relire constamment tout, il l'a
// pas en mémoire ?"). `write_brain` et `merge_in` sont les deux seuls writers
// de brain.json (le second, seul, écrit hors de `write_brain` — sync cloud) ;
// tous deux mettent ce cache à jour juste après avoir écrit, donc une lecture
// via `load_brain_cached` n'est jamais périmée tant qu'un des deux chemins a
// bien été utilisé pour la dernière écriture.
static BRAIN_CACHE: std::sync::Mutex<Option<(std::path::PathBuf, crate::models::BrainGraph)>> =
    std::sync::Mutex::new(None);

fn cached_for(dir: &Path) -> Option<crate::models::BrainGraph> {
    let cache = BRAIN_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    match cache.as_ref() {
        Some((cached_dir, graph)) if cached_dir == dir => Some(graph.clone()),
        _ => None,
    }
}

fn set_cache(dir: &Path, graph: &crate::models::BrainGraph) {
    let mut cache = BRAIN_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *cache = Some((dir.to_path_buf(), graph.clone()));
}

/// Invalide le cache mémoire — nécessaire après toute suppression de fichiers
/// qui ne passe pas par `write_brain`/`merge_in` (ex. `reset_environment`) :
/// le process Tauri ne redémarre pas quand le front recharge la page, donc
/// sans ça une prochaine lecture renvoie l'ancien cerveau depuis la RAM même
/// si le disque est vide (bug remonté par Liam le 2026-07-22 : reset → data
/// toujours là après reconnexion, alors que disque ET cloud étaient vides).
pub fn clear_cache() {
    let mut cache = BRAIN_CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *cache = None;
}

/// Charge le cerveau — depuis le cache mémoire s'il correspond au même dossier
/// (compte actif inchangé), sinon relit le disque et peuple le cache. Lit
/// `brain.db` s'il existe (post-migration SQLite, Phase 3a), sinon `brain.json`
/// (comportement historique — tant que la migration n'est pas déclenchée au
/// démarrage, cf. Phase 4, tous les comptes existants restent sur ce chemin).
pub fn load_brain_cached(dir: &Path) -> Result<crate::models::BrainGraph, String> {
    if let Some(g) = cached_for(dir) { return Ok(g); }
    let graph = if dir.join("brain.db").exists() {
        crate::storage::load_brain_graph(dir)?
    } else {
        let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())?
    };
    set_cache(dir, &graph);
    Ok(graph)
}

/// Nom de fichier/dossier sûr multi-OS. Les ids de nœuds contiennent `:`
/// (`leaf:<conv>`, `p:<projet>`) — légal sur macOS, interdit sur Windows
/// (os error 123). Les caractères interdits sont encodés en %XX. Idempotent
/// (le `%` n'est pas ré-encodé) : on n'a jamais besoin de décoder, les lecteurs
/// encodent l'id brut pour retrouver le chemin.
pub fn safe_component(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => {
                out.push_str(&format!("%{:02X}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

/// Renomme les dossiers/fichiers hérités dont le nom contient des caractères
/// interdits sur Windows (créés par d'anciens builds Mac). Idempotent, appelé
/// avant chaque export et fusion.
pub fn migrate_unsafe_names(dir: &Path) {
    for sub in DIRS {
        let root = dir.join(sub);
        for e in std::fs::read_dir(&root).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let safe = safe_component(&name);
            if safe != name {
                let _ = std::fs::rename(e.path(), root.join(safe));
            }
        }
    }
}

/// Force l'écriture du WAL dans `brain.db` avant de le zipper (Phase 8).
/// En mode WAL, un commit récent peut n'exister que dans `brain.db-wal` —
/// zipper `brain.db` seul sans checkpoint produirait une sauvegarde qui a
/// l'air correcte mais qui manque les toutes dernières écritures (bien pire
/// qu'un simple fichier texte, qui n'a pas ce problème). No-op si pas migré.
fn checkpoint_wal(dir: &Path) {
    if !dir.join("brain.db").exists() { return; }
    if let Ok(conn) = crate::storage::open_or_init(dir) {
        let _ = conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", []);
    }
}

/// Chemin relatif d'une entrée de zip, chaque composant assaini — les zips
/// produits par d'anciens builds Mac contiennent des `:` qui feraient planter
/// toute écriture sur Windows.
fn safe_rel(rel: &Path) -> std::path::PathBuf {
    rel.components()
        .map(|c| safe_component(&c.as_os_str().to_string_lossy()))
        .collect()
}

/// Zippe les données du cerveau. Renvoie les octets du zip.
pub fn export_in(dir: &Path) -> Result<Vec<u8>, String> {
    migrate_unsafe_names(dir);
    checkpoint_wal(dir);
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);

        let mut add_file = |zip: &mut zip::ZipWriter<&mut std::io::Cursor<Vec<u8>>>, rel: &str, abs: &Path| -> Result<(), String> {
            let bytes = std::fs::read(abs).map_err(|e| format!("{rel} : {e}"))?;
            zip.start_file(rel, opts).map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())
        };

        for f in FILES {
            let p = dir.join(f);
            if p.is_file() { add_file(&mut zip, f, &p)?; }
        }
        for d in DIRS {
            let root = dir.join(d);
            if !root.is_dir() { continue; }
            // Un seul niveau de profondeur suffit pour nos dossiers actuels,
            // mais node_history contient des sous-dossiers par nœud → 2 niveaux.
            for entry in walk(&root) {
                let rel = entry.strip_prefix(dir).map_err(|e| e.to_string())?
                    .to_string_lossy().replace('\\', "/");
                add_file(&mut zip, &rel, &entry)?;
            }
        }
        zip.finish().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

/// Empreinte des données user : mtime le plus récent (secs epoch) du périmètre
/// de sauvegarde. La sync cloud pousse quand elle change — une suppression seule
/// ne la fait pas bouger, mais toute suppression réécrit brain.json à côté.
pub fn fingerprint_in(dir: &Path) -> u64 {
    // Contenu d'exemple (demo.flag) : empreinte 0 → la sync ne pousse jamais la
    // démo vers le cloud, et le cloud la remplace (0 = « rien qui compte ici »).
    if dir.join("demo.flag").exists() {
        return 0;
    }
    let mtime = |p: &Path| -> u64 {
        p.metadata().ok()
            .and_then(|md| md.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    };
    let files = FILES.iter().map(|f| dir.join(f));
    let dirs = DIRS.iter().flat_map(|d| walk(&dir.join(d)));
    // brain.db-wal : en mode WAL, un commit récent peut n'exister que là — sans
    // le compter, l'empreinte ne bougerait pas juste après une écriture tant
    // qu'aucun checkpoint n'a eu lieu, et la sync croirait que rien n'a changé.
    let wal = std::iter::once(dir.join("brain.db-wal"));
    files.chain(dirs).chain(wal).map(|p| mtime(&p)).max().unwrap_or(0)
}

/// Fichiers d'une arborescence (récursif, fichiers seulement).
fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p); } else if p.is_file() { out.push(p); }
        }
    }
    out
}

/// Restaure une sauvegarde : sécurise les chemins (pas de `..` ni d'absolu),
/// garde une copie locale de brain.json avant écrasement.
pub fn import_in(dir: &Path, bytes: &[u8]) -> Result<usize, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Archive illisible : {e}"))?;

    // Filet : état actuel sauvegardé avant écrasement (brain.json ou brain.db
    // selon ce que le compte utilise).
    for name in ["brain.json", "brain.db"] {
        let p = dir.join(name);
        if p.exists() {
            let _ = std::fs::copy(&p, dir.join(format!("{name}.avant-restauration")));
        }
    }

    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        // enclosed_name refuse `..` et les chemins absolus.
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("Chemin suspect dans l'archive : {}", entry.name()));
        };
        let dest = dir.join(safe_rel(&rel));
        if entry.is_dir() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(&dest, bytes).map_err(|e| e.to_string())?;
        count += 1;
    }
    if count == 0 { return Err("Archive vide.".into()); }
    // Les données restaurées sont un vrai cerveau : l'état démo ne s'applique plus
    // (sinon la bannière « Contenu d'exemple » resterait par-dessus les vraies données).
    let _ = std::fs::remove_file(dir.join("demo.flag"));
    Ok(count)
}

// ── Fusion (sync cloud) ───────────────────────────────────────────────────────
// Le remplacement intégral (import_in) perd des données dès que deux machines
// vivent en même temps : chacune écrase le cerveau de l'autre. La sync passe
// donc par une FUSION : nœud par nœud (le plus récent gagne via `updated_at`),
// union des espaces/edges, tombstones honorées, fichiers annexes additifs.
// import_in reste le remplacement explicite (bouton « Restaurer »).

use crate::models::{BrainEdge, BrainGraph};

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Compare deux nœuds SANS leur `updated_at` — pour détecter si un nœud a
/// vraiment changé sans que l'estampille elle-même compte comme une
/// différence. Comparaison directe des champs, pas de sérialisation JSON :
/// l'ancienne version (`node_sig`, un `serde_json::to_value` par nœud) coûtait
/// cher sur les 415 nœuds à CHAQUE écriture, avec des `source_text` moyens de
/// ~100+ Ko — délai perceptible remonté par Liam le 2026-07-22 sur la simple
/// création d'une note (~1s rien que pour ce diff, avant même la persistance).
fn node_unchanged(a: &crate::models::BrainNode, b: &crate::models::BrainNode) -> bool {
    a.label == b.label
        && a.kind == b.kind
        && a.weight == b.weight
        && a.summary == b.summary
        && a.keywords == b.keywords
        && a.decisions == b.decisions
        && a.patterns == b.patterns
        && a.community == b.community
        && a.parent_id == b.parent_id
        && a.synthesized_at == b.synthesized_at
        && a.date == b.date
        && a.content == b.content
        && a.connector == b.connector
        && a.source_id == b.source_id
        && a.source_project == b.source_project
        && a.source_text == b.source_text
}

/// Écrit brain.json en estampillant `updated_at` sur chaque nœud nouveau ou
/// modifié par rapport à la version précédente du fichier. Point de passage
/// unique : tous les writers de brain.json doivent l'utiliser, sinon les
/// modifications ne se propagent pas dans la fusion de sync.
pub fn write_brain(dir: &Path, graph: &mut BrainGraph) -> Result<(), String> {
    if dir.join("brain.db").exists() {
        return write_brain_sqlite(dir, graph);
    }
    // "old" vient du cache mémoire quand possible — évite une lecture complète
    // de plus du disque en plus de celle déjà faite par l'appelant pour
    // préparer `graph` (cf. `load_brain_cached`).
    let old: Option<BrainGraph> = cached_for(dir).or_else(|| {
        std::fs::read_to_string(dir.join("brain.json")).ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    });
    let old_by_id: std::collections::HashMap<String, (Option<u64>, crate::models::BrainNode)> = old
        .map(|g| g.nodes.into_iter().map(|n| (n.id.clone(), (n.updated_at, n))).collect())
        .unwrap_or_default();
    let now = now_secs();
    for n in &mut graph.nodes {
        n.updated_at = match old_by_id.get(&n.id) {
            Some((stamp, old_n)) if node_unchanged(old_n, n) => stamp.or(Some(now)),
            _ => Some(now),
        };
    }
    std::fs::write(
        dir.join("brain.json"),
        serde_json::to_string_pretty(graph).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    set_cache(dir, graph);
    Ok(())
}

/// Variante SQLite de `write_brain` (Phase 4). Même règle d'estampillage
/// (`node_unchanged`, comparaison à l'ancien graphe), mais la persistance devient un
/// `UPDATE`/`INSERT` ciblé par nœud réellement modifié + un `DELETE` par nœud
/// disparu, dans une seule transaction — plus de sérialisation d'un blob de
/// plusieurs dizaines de Mo à chaque mutation (c'est ce qui causait le freeze
/// du lot de 44 créations MCP, 2026-07-21). Les arêtes suivent le même diff
/// ciblé (clé naturelle source/target/kind/relation, pas d'id stable dans le
/// modèle actuel). `markdown`/`report` (le texte complet du cerveau, plusieurs
/// Mo, pas encore recalculés depuis SQL — Phase 6) ne sont ré-écrits QUE s'ils
/// ont réellement changé : les réécrire à chaque mutation (constaté le
/// 2026-07-22 : ~18 ms sur ~28 ms pour l'ajout d'une seule note, l'essentiel
/// du coût restant) recréait exactement le problème que SQLite devait éliminer.
fn write_brain_sqlite(dir: &Path, graph: &mut BrainGraph) -> Result<(), String> {
    let old: Option<BrainGraph> = cached_for(dir).or_else(|| crate::storage::load_brain_graph(dir).ok());
    let (old_by_id, old_edges, old_meta): (
        std::collections::HashMap<String, (Option<u64>, crate::models::BrainNode)>,
        std::collections::HashSet<(String, String, String, String)>,
        Option<(String, String, String)>,
    ) = match old {
        Some(BrainGraph { nodes, edges, markdown, report, generated_at }) => {
            let edges_set = edges.into_iter().map(|e| (e.source, e.target, e.kind, e.relation)).collect();
            let by_id = nodes.into_iter().map(|n| (n.id.clone(), (n.updated_at, n))).collect();
            (by_id, edges_set, Some((markdown, report, generated_at)))
        }
        None => (Default::default(), Default::default(), None),
    };
    let old_ids: std::collections::HashSet<String> = old_by_id.keys().cloned().collect();
    let now = now_secs();

    let mut changed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for n in &mut graph.nodes {
        match old_by_id.get(&n.id) {
            Some((stamp, old_n)) if node_unchanged(old_n, n) => {
                if stamp.is_none() { changed_ids.insert(n.id.clone()); }
                n.updated_at = stamp.or(Some(now));
            }
            _ => {
                n.updated_at = Some(now);
                changed_ids.insert(n.id.clone());
            }
        }
    }
    let new_ids: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let removed_ids: Vec<String> = old_ids.difference(&new_ids).cloned().collect();

    let new_edges: std::collections::HashSet<(String, String, String, String)> = graph.edges.iter()
        .map(|e| (e.source.clone(), e.target.clone(), e.kind.clone(), e.relation.clone()))
        .collect();
    let edges_to_add: Vec<&(String, String, String, String)> = new_edges.difference(&old_edges).collect();
    let edges_to_remove: Vec<&(String, String, String, String)> = old_edges.difference(&new_edges).collect();

    let (old_markdown, old_report, old_generated_at) = match &old_meta {
        Some((m, r, g)) => (Some(m.as_str()), Some(r.as_str()), Some(g.as_str())),
        None => (None, None, None),
    };

    let mut conn = crate::storage::open_or_init(dir).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Un lot peut contenir un enfant AVANT son parent tout juste créé dans le
    // même lot (ex. l'Archiviste qui crée un dossier de thème et y déplace ses
    // pages en une seule résolution) — sans ça, `nodes.parent_id REFERENCES
    // nodes(id)` échoue en "FOREIGN KEY constraint failed" dès que l'ordre
    // d'itération de `graph.nodes` traite l'enfant avant le parent (bug réel
    // remonté par Liam le 2026-07-23 : 244 propositions marquées résolues côté
    // fichiers, mais AUCUNE persistée dans brain.db — l'écriture finale
    // échouait silencieusement après coup). Reporte la vérification des clés
    // étrangères au COMMIT : l'ordre d'insertion dans CE lot n'a plus besoin
    // d'être topologique tant que l'ensemble est cohérent à la fin.
    tx.pragma_update(None, "defer_foreign_keys", true).map_err(|e| e.to_string())?;
    for n in graph.nodes.iter().filter(|n| changed_ids.contains(&n.id)) {
        crate::storage::upsert_node(&tx, n).map_err(|e| format!("upsert nœud {} : {e}", n.id))?;
    }
    for id in &removed_ids {
        tx.execute("DELETE FROM nodes WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    }
    sync_edges_tx(&tx, &edges_to_add, &edges_to_remove)?;
    if old_markdown != Some(graph.markdown.as_str()) {
        crate::storage::set_meta(&tx, "markdown", &graph.markdown).map_err(|e| e.to_string())?;
    }
    if old_report != Some(graph.report.as_str()) {
        crate::storage::set_meta(&tx, "report", &graph.report).map_err(|e| e.to_string())?;
    }
    if old_generated_at != Some(graph.generated_at.as_str()) {
        crate::storage::set_meta(&tx, "generated_at", &graph.generated_at).map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;

    set_cache(dir, graph);
    Ok(())
}

type EdgeKey = (String, String, String, String);

fn sync_edges_tx(tx: &rusqlite::Transaction, to_add: &[&EdgeKey], to_remove: &[&EdgeKey]) -> Result<(), String> {
    if !to_remove.is_empty() {
        let mut del = tx
            .prepare("DELETE FROM edges WHERE source = ?1 AND target = ?2 AND kind = ?3 AND relation = ?4")
            .map_err(|e| e.to_string())?;
        for (s, t, k, r) in to_remove {
            del.execute(rusqlite::params![s, t, k, r]).map_err(|e| e.to_string())?;
        }
    }
    if !to_add.is_empty() {
        let mut ins = tx
            .prepare("INSERT INTO edges (source, target, kind, relation) VALUES (?1,?2,?3,?4)")
            .map_err(|e| e.to_string())?;
        for (s, t, k, r) in to_add {
            ins.execute(rusqlite::params![s, t, k, r]).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Variante ciblée : pour les appelants qui savent PRÉCISÉMENT quels nœuds ils
/// viennent de créer/modifier/supprimer (les 5 fonctions `_on` et
/// `resolve_all_pending_in`) — aucune lecture ni comparaison du graphe entier,
/// juste l'écriture des lignes concernées. Le diff générique de
/// `write_brain_sqlite` (charger l'ancien graphe, comparer les 415 nœuds un
/// par un) n'a de sens que quand l'appelant ne sait pas ce qui a changé
/// (régénération complète du pipeline IA) ; ici on le sait déjà, le déduire à
/// nouveau par comparaison est un travail inutile (constaté avec Liam le
/// 2026-07-22). Les arêtes restent diffées, mais contre une lecture légère
/// (`storage::load_edges`, pas de gros `source_text` à charger) — les `_on`
/// ne signalent pas encore précisément quelles arêtes elles ont touchées.
/// `markdown`/`report`/`generated_at` ne sont jamais écrits ici : seule la
/// régénération complète les modifie, et elle passe par `write_brain`.
pub fn write_brain_touching(
    dir: &Path,
    graph: &mut BrainGraph,
    touched_ids: &[String],
    removed_ids: &[String],
) -> Result<(), String> {
    if !dir.join("brain.db").exists() {
        // Pas encore migré vers SQLite : le fichier JSON n'a pas de notion de
        // ligne — retombe sur l'écriture pleine (diff générique, sur un
        // fichier et non une base, comportement inchangé).
        return write_brain(dir, graph);
    }
    let now = now_secs();
    let touched: std::collections::HashSet<&str> = touched_ids.iter().map(|s| s.as_str()).collect();
    for n in graph.nodes.iter_mut() {
        if touched.contains(n.id.as_str()) {
            n.updated_at = Some(now);
        }
    }

    let old_edges: std::collections::HashSet<EdgeKey> = crate::storage::load_edges(dir)?
        .into_iter()
        .map(|e| (e.source, e.target, e.kind, e.relation))
        .collect();
    let new_edges: std::collections::HashSet<EdgeKey> = graph.edges.iter()
        .map(|e| (e.source.clone(), e.target.clone(), e.kind.clone(), e.relation.clone()))
        .collect();
    let edges_to_add: Vec<&EdgeKey> = new_edges.difference(&old_edges).collect();
    let edges_to_remove: Vec<&EdgeKey> = old_edges.difference(&new_edges).collect();

    let mut conn = crate::storage::open_or_init(dir).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // Même raison que dans `write_brain_sqlite` : un lot peut créer un parent
    // ET déplacer ses enfants dedans en une seule résolution, dans un ordre
    // qui n'est pas forcément topologique — cf. commentaire là-bas (bug réel
    // du 2026-07-23, "FOREIGN KEY constraint failed" silencieux).
    tx.pragma_update(None, "defer_foreign_keys", true).map_err(|e| e.to_string())?;
    for n in graph.nodes.iter().filter(|n| touched.contains(n.id.as_str())) {
        crate::storage::upsert_node(&tx, n).map_err(|e| format!("upsert nœud {} : {e}", n.id))?;
    }
    for id in removed_ids {
        tx.execute("DELETE FROM nodes WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
    }
    sync_edges_tx(&tx, &edges_to_add, &edges_to_remove)?;
    tx.commit().map_err(|e| e.to_string())?;

    set_cache(dir, graph);
    Ok(())
}

/// La structure de l'arbre a UNE seule vérité : `parent_id`. Les arêtes
/// `contains` sont re-dérivées d'elle — jamais fusionnées : l'union des arêtes
/// de deux régénérations différentes emmêle la hiérarchie (doubles parents).
/// Les autres arêtes (wikilinks, ponts) sont conservées, dédupliquées, et les
/// parents orphelins sont rattachés à la racine.
pub fn rebuild_tree_edges(graph: &mut BrainGraph) {
    let ids: std::collections::HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    let root = graph.nodes.iter().find(|n| n.kind == "root").map(|n| n.id.clone());
    for n in &mut graph.nodes {
        if let Some(p) = &n.parent_id {
            if !ids.contains(p) && n.kind != "root" {
                n.parent_id = root.clone();
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut edges: Vec<BrainEdge> = vec![];
    for e in graph.edges.drain(..) {
        if e.kind != "contains"
            && ids.contains(&e.source) && ids.contains(&e.target)
            && seen.insert(format!("{}|{}|{}|{}", e.source, e.target, e.kind, e.relation))
        {
            edges.push(e);
        }
    }
    for n in &graph.nodes {
        if let Some(p) = &n.parent_id {
            if ids.contains(p) {
                edges.push(BrainEdge {
                    source: p.clone(), target: n.id.clone(),
                    kind: "contains".into(), relation: "contains".into(),
                });
            }
        }
    }
    graph.edges = edges;
}

/// Résultat d'une fusion : l'appelant repousse vers le cloud si le local
/// contenait des choses que le distant n'avait pas.
#[derive(serde::Serialize)]
pub struct MergeReport {
    pub files: usize,
    pub local_extra: bool,
}

fn zip_json<T: serde::de::DeserializeOwned>(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Option<T> {
    let mut entry = archive.by_name(name).ok()?;
    let mut raw = String::new();
    entry.read_to_string(&mut raw).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Pendant binaire de `zip_json` pour "brain.db" (Phase 8) : extrait l'entrée
/// vers un fichier temporaire (SQLite n'ouvre pas une base depuis un buffer
/// mémoire) et la lit comme un graphe.
fn zip_brain_db(archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Option<BrainGraph> {
    let mut entry = archive.by_name("brain.db").ok()?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).ok()?;
    drop(entry);
    let tmp = std::env::temp_dir().join(format!("lucid_merge_remote_{}_{}.db", now_secs(), std::process::id()));
    std::fs::write(&tmp, &buf).ok()?;
    let graph = crate::storage::load_brain_graph_from_file(&tmp).ok();
    let _ = std::fs::remove_file(&tmp);
    graph
}

/// Persiste un graphe déjà fusionné — les estampilles ont été décidées par la
/// fusion LWW (celles du camp qui gagne) et ne doivent JAMAIS être recalculées
/// à l'écriture (contrairement à `write_brain`) : SQLite si le compte y est
/// déjà, JSON sinon.
fn persist_merged(dir: &Path, graph: &BrainGraph) -> Result<(), String> {
    if dir.join("brain.db").exists() {
        crate::storage::replace_graph(dir, graph)?;
    } else {
        std::fs::write(
            dir.join("brain.json"),
            serde_json::to_string_pretty(graph).map_err(|e| e.to_string())?,
        ).map_err(|e| e.to_string())?;
    }
    set_cache(dir, graph);
    Ok(())
}

/// Fusionne une sauvegarde distante dans les données locales (sync cloud).
pub fn merge_in(dir: &Path, bytes: &[u8]) -> Result<MergeReport, String> {
    migrate_unsafe_names(dir);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Archive illisible : {e}"))?;

    // Filet : état actuel gardé avant fusion (brain.json ou brain.db selon le compte).
    for name in ["brain.json", "brain.db"] {
        let p = dir.join(name);
        if p.exists() {
            let _ = std::fs::copy(&p, dir.join(format!("{name}.avant-restauration")));
        }
    }

    let mut local_extra = false;

    // Tombstones : union des deux côtés — une suppression faite n'importe où tient partout.
    let mut deleted: std::collections::HashSet<String> =
        std::fs::read_to_string(dir.join("deleted_nodes.json"))
            .ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    let remote_deleted: std::collections::HashSet<String> =
        zip_json(&mut archive, "deleted_nodes.json").unwrap_or_default();
    if !deleted.is_subset(&remote_deleted) { local_extra = true; }
    deleted.extend(remote_deleted);
    if let Ok(json) = serde_json::to_string_pretty(&deleted) {
        let _ = std::fs::write(dir.join("deleted_nodes.json"), json);
    }

    // Nœud par nœud, le plus récent gagne ; union des deux côtés, moins les
    // tombstones. Écrit tel quel (les estampilles fusionnées font foi) — lu
    // depuis brain.db si le compte est migré, brain.json sinon (Phase 8) ;
    // même chose côté distant selon ce que contient l'archive.
    let local_brain: Option<BrainGraph> = load_brain_cached(dir).ok();
    let remote_brain: Option<BrainGraph> = zip_brain_db(&mut archive)
        .or_else(|| zip_json(&mut archive, "brain.json"));
    match (local_brain, remote_brain) {
        (Some(local), Some(remote)) => {
            let mut by_id: std::collections::HashMap<String, crate::models::BrainNode> =
                remote.nodes.into_iter().map(|n| (n.id.clone(), n)).collect();
            for ln in local.nodes {
                // Distant strictement plus récent → il gagne ; sinon (égalité
                // comprise) le local gagne : c'est ce que l'utilisateur regarde.
                let keep_local = match by_id.get(&ln.id) {
                    Some(rn) => {
                        let remote_newer = rn.updated_at.unwrap_or(0) > ln.updated_at.unwrap_or(0);
                        if !remote_newer && !node_unchanged(rn, &ln) { local_extra = true; }
                        !remote_newer
                    }
                    None => { local_extra = true; true }
                };
                if keep_local { by_id.insert(ln.id.clone(), ln); }
            }
            for id in &deleted { by_id.remove(id); }
            // Seuls les wikilinks/ponts s'unionnent — l'arbre est reconstruit
            // depuis parent_id (rebuild_tree_edges), jamais fusionné.
            let edges: Vec<BrainEdge> = remote.edges.into_iter().chain(local.edges.into_iter()).collect();
            // markdown/report : régénérables — on garde le plus récemment généré.
            let (markdown, report, generated_at) = if remote.generated_at > local.generated_at {
                (remote.markdown, remote.report, remote.generated_at)
            } else {
                (local.markdown, local.report, local.generated_at)
            };
            let mut merged = BrainGraph {
                nodes: by_id.into_values().collect(),
                edges,
                markdown,
                report,
                generated_at,
            };
            rebuild_tree_edges(&mut merged);
            persist_merged(dir, &merged)?;
        }
        (None, Some(remote)) => {
            persist_merged(dir, &remote)?;
        }
        (Some(_), None) => { local_extra = true; }
        (None, None) => {}
    }

    // spaces.json : même mécanique que les nœuds — union par id, le plus récent
    // gagne (`updated_at` estampillé par save_spaces), tombstones honorées.
    let mut deleted_spaces: std::collections::HashSet<String> =
        std::fs::read_to_string(dir.join("deleted_spaces.json"))
            .ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    let remote_deleted_spaces: std::collections::HashSet<String> =
        zip_json(&mut archive, "deleted_spaces.json").unwrap_or_default();
    if !deleted_spaces.is_subset(&remote_deleted_spaces) { local_extra = true; }
    deleted_spaces.extend(remote_deleted_spaces);
    if let Ok(json) = serde_json::to_string_pretty(&deleted_spaces) {
        let _ = std::fs::write(dir.join("deleted_spaces.json"), json);
    }

    let sid = |s: &serde_json::Value| s.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let stamp = |s: &serde_json::Value| s.get("updated_at").and_then(|v| v.as_u64()).unwrap_or(0);
    let local_spaces: Vec<serde_json::Value> = std::fs::read_to_string(dir.join("spaces.json"))
        .ok().and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
    let remote_spaces: Vec<serde_json::Value> = zip_json(&mut archive, "spaces.json").unwrap_or_default();
    let mut by_id: std::collections::HashMap<String, serde_json::Value> =
        remote_spaces.into_iter().map(|s| (sid(&s), s)).collect();
    for ls in local_spaces {
        let id = sid(&ls);
        match by_id.get(&id) {
            Some(rs) if stamp(rs) > stamp(&ls) => {}
            Some(rs) => {
                if rs != &ls { local_extra = true; }
                by_id.insert(id, ls);
            }
            None => { local_extra = true; by_id.insert(id, ls); }
        }
    }
    for id in &deleted_spaces { by_id.remove(id); }
    let spaces: Vec<serde_json::Value> = by_id.into_values().collect();
    if let Ok(json) = serde_json::to_string_pretty(&spaces) {
        let _ = std::fs::write(dir.join("spaces.json"), json);
    }

    // Fichiers annexes (historique, images, snapshots…) : additifs — on n'écrase
    // jamais un fichier local existant, on ajoute ce qui manque.
    let merged_by_hand = ["brain.json", "brain.db", "spaces.json", "deleted_nodes.json"];
    let mut files = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(format!("Chemin suspect dans l'archive : {}", entry.name()));
        };
        if entry.is_dir() || merged_by_hand.contains(&entry.name()) { continue; }
        let dest = dir.join(safe_rel(&rel));
        if dest.exists() { continue; }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&dest, buf).map_err(|e| e.to_string())?;
        files += 1;
    }

    // Des vraies données sont arrivées : l'état démo ne s'applique plus.
    let _ = std::fs::remove_file(dir.join("demo.flag"));
    Ok(MergeReport { files, local_extra })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: &str, label: &str, ts: u64) -> crate::models::BrainNode {
        serde_json::from_value(serde_json::json!({
            "id": id, "label": label, "kind": "leaf", "weight": 1, "updated_at": ts
        })).unwrap()
    }

    fn g(nodes: Vec<crate::models::BrainNode>) -> BrainGraph {
        BrainGraph { nodes, edges: vec![], markdown: String::new(), report: String::new(), generated_at: "t".into() }
    }

    #[test]
    fn merge_prend_le_plus_recent_et_ne_perd_rien() {
        let remote = std::env::temp_dir().join("brainlink_test_merge_remote");
        let local = std::env::temp_dir().join("brainlink_test_merge_local");
        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); std::fs::create_dir_all(d).unwrap(); }

        // Distant : A modifié récemment, C nouveau, tombstone sur D.
        std::fs::write(remote.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A-distant", 20), n("C", "C", 5), n("D", "D", 1)])).unwrap()).unwrap();
        std::fs::write(remote.join("deleted_nodes.json"), r#"[]"#).unwrap();
        let zip = export_in(&remote).unwrap();

        // Local : A ancien, B local-only, D supprimé ici (tombstone locale).
        std::fs::write(local.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A-local", 10), n("B", "B", 100)])).unwrap()).unwrap();
        std::fs::write(local.join("deleted_nodes.json"), r#"["D"]"#).unwrap();

        let report = merge_in(&local, &zip).unwrap();
        assert!(report.local_extra, "B est local-only → il faut repousser");

        let merged: BrainGraph = serde_json::from_str(
            &std::fs::read_to_string(local.join("brain.json")).unwrap()).unwrap();
        let label = |id: &str| merged.nodes.iter().find(|x| x.id == id).map(|x| x.label.clone());
        assert_eq!(label("A").as_deref(), Some("A-distant"), "le plus récent gagne");
        assert!(label("B").is_some(), "le nœud local-only survit");
        assert!(label("C").is_some(), "le nœud distant arrive");
        assert!(label("D").is_none(), "la suppression locale tient malgré le distant");
        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); }
    }

    /// Phase 8 — même scénario que `merge_prend_le_plus_recent_et_ne_perd_rien`,
    /// mais les deux côtés sont déjà migrés SQLite : `export_in` doit zipper
    /// `brain.db` (pas `brain.json`, absent), et `merge_in` doit le lire/écrire
    /// via SQLite sans perdre la sémantique LWW existante.
    #[test]
    fn merge_prend_le_plus_recent_et_ne_perd_rien_sqlite() {
        let remote = std::env::temp_dir().join("brainlink_test_merge_remote_sqlite");
        let local = std::env::temp_dir().join("brainlink_test_merge_local_sqlite");
        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); std::fs::create_dir_all(d).unwrap(); }

        // Distant : A modifié récemment, C nouveau, tombstone sur D. Compte migré SQLite.
        std::fs::write(remote.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A-distant", 20), n("C", "C", 5), n("D", "D", 1)])).unwrap()).unwrap();
        std::fs::write(remote.join("deleted_nodes.json"), r#"[]"#).unwrap();
        crate::storage::migrate_json_to_sqlite(&remote).unwrap();
        assert!(remote.join("brain.db").exists());
        let zip = export_in(&remote).unwrap();

        // Local : A ancien, B local-only, D supprimé ici (tombstone locale). Migré aussi.
        std::fs::write(local.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A-local", 10), n("B", "B", 100)])).unwrap()).unwrap();
        std::fs::write(local.join("deleted_nodes.json"), r#"["D"]"#).unwrap();
        crate::storage::migrate_json_to_sqlite(&local).unwrap();
        assert!(local.join("brain.db").exists());

        let report = merge_in(&local, &zip).unwrap();
        assert!(report.local_extra, "B est local-only → il faut repousser");

        let merged = crate::storage::load_brain_graph(&local).unwrap(); // hors cache, lecture directe
        let label = |id: &str| merged.nodes.iter().find(|x| x.id == id).map(|x| x.label.clone());
        assert_eq!(label("A").as_deref(), Some("A-distant"), "le plus récent gagne");
        assert!(label("B").is_some(), "le nœud local-only survit");
        assert!(label("C").is_some(), "le nœud distant arrive");
        assert!(label("D").is_none(), "la suppression locale tient malgré le distant");

        let a = merged.nodes.iter().find(|x| x.id == "A").unwrap();
        assert_eq!(a.updated_at, Some(20), "l'estampille fusionnée ne doit pas être recalculée à l'écriture");

        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); }
    }

    #[test]
    fn write_brain_estampille_les_noeuds_modifies() {
        let dir = std::env::temp_dir().join("brainlink_test_stamp");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A", 10), n("B", "B", 10)])).unwrap()).unwrap();
        // B change de label, A inchangé.
        let mut graph = g(vec![n("A", "A", 0), n("B", "B2", 0)]);
        write_brain(&dir, &mut graph).unwrap();
        let a = graph.nodes.iter().find(|x| x.id == "A").unwrap();
        let b = graph.nodes.iter().find(|x| x.id == "B").unwrap();
        assert_eq!(a.updated_at, Some(10), "nœud inchangé → garde son estampille");
        assert!(b.updated_at.unwrap() > 10, "nœud modifié → ré-estampillé maintenant");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_brain_sqlite_ne_touche_que_les_noeuds_reellement_modifies() {
        let dir = std::env::temp_dir().join("brainlink_test_stamp_sqlite");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A", 10), n("B", "B", 10), n("C", "C", 10)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();
        assert!(dir.join("brain.db").exists());

        // B change de label (modifié), A inchangé, C disparaît (supprimé), D arrive (nouveau).
        let mut graph = g(vec![n("A", "A", 0), n("B", "B2", 0), n("D", "D", 0)]);
        write_brain(&dir, &mut graph).unwrap();

        let a = graph.nodes.iter().find(|x| x.id == "A").unwrap();
        let b = graph.nodes.iter().find(|x| x.id == "B").unwrap();
        let d = graph.nodes.iter().find(|x| x.id == "D").unwrap();
        assert_eq!(a.updated_at, Some(10), "nœud inchangé → garde son estampille");
        assert!(b.updated_at.unwrap() > 10, "nœud modifié → ré-estampillé maintenant");
        assert!(d.updated_at.unwrap() > 10, "nouveau nœud → estampillé");

        // Relecture directe de brain.db (hors cache mémoire) pour vérifier ce qui a
        // vraiment été persisté : A/B/D présents, C supprimé, B porte le nouveau label.
        let conn = crate::storage::open_or_init(&dir).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 3, "A, B, D restent ; C a été supprimé");
        let c_gone: i64 = conn.query_row("SELECT COUNT(*) FROM nodes WHERE id='C'", [], |r| r.get(0)).unwrap();
        assert_eq!(c_gone, 0);
        let b_label: String = conn.query_row("SELECT label FROM nodes WHERE id='B'", [], |r| r.get(0)).unwrap();
        assert_eq!(b_label, "B2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Phase 5 — vérification empirique AVANT de retirer `GEN_LOCK`, comme le
    /// demande le plan. Verdict : le verrou reste nécessaire, on ne le retire
    /// PAS. Sans lui (constaté en supprimant temporairement les deux
    /// `crate::GEN_LOCK.lock()` ci-dessous pendant l'investigation), ce test
    /// échoue de façon reproductible : le nœud MCP créé pendant la fenêtre de
    /// la régénération disparaît, effacé par l'écriture de la régénération —
    /// exactement le bug du 2026-07-21 ("le canvas s'est vidé"). Les
    /// transactions SQLite ne protègent PAS contre ça : chaque écriture est
    /// atomique, mais la régénération diffe contre un INSTANTANÉ pris au début
    /// de son analyse (plusieurs secondes/minutes avec un LLM) — un nœud créé
    /// entre cet instantané et l'écriture finale n'existe pas dans son idée du
    /// monde, donc son diff le classe "supprimé". C'est une perte de mise à
    /// jour (lost update) applicative, pas un problème de concurrence au
    /// niveau fichier/base — WAL n'a aucune prise dessus.
    #[test]
    fn gen_lock_protege_lacceptation_mcp_pendant_une_regeneration() {
        let dir = std::env::temp_dir().join("brainlink_test_concurrence_gen_lock");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"),
            serde_json::to_string(&g(vec![n("root", "Cerveau", 1)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();

        let dir_regen = dir.clone();
        let regen = std::thread::spawn(move || {
            let _gen = crate::GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            // Instantané pris AVANT que le thread MCP n'ajoute son nœud —
            // représente l'analyse (LLM, plusieurs secondes en vrai) qui tourne
            // sur un état déjà périmé au moment où elle écrit.
            let mut snapshot = load_brain_cached(&dir_regen).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(80));
            // "Nouveau" graphe issu de la régénération : ne contient QUE ce que
            // l'analyse connaissait à son démarrage (le nœud MCP n'y est pas).
            write_brain(&dir_regen, &mut snapshot).unwrap();
        });

        let dir_mcp = dir.clone();
        let mcp = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20)); // tente de démarrer pendant la fenêtre de la régénération
            let _gen = crate::GEN_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let mut graph = load_brain_cached(&dir_mcp).unwrap();
            graph.nodes.push(n("mcp-note", "Note MCP", 0));
            graph.edges.push(crate::models::BrainEdge {
                source: "root".into(), target: "mcp-note".into(), kind: "contains".into(), relation: "contains".into(),
            });
            write_brain_touching(&dir_mcp, &mut graph, &["mcp-note".to_string()], &[]).unwrap();
        });

        mcp.join().unwrap();
        regen.join().unwrap();

        let conn = crate::storage::open_or_init(&dir).unwrap();
        let survives: i64 = conn.query_row("SELECT COUNT(*) FROM nodes WHERE id = 'mcp-note'", [], |r| r.get(0)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(survives, 1, "le verrou doit garantir que les deux opérations ne se chevauchent jamais");
    }

    #[test]
    fn write_brain_touching_ne_touche_que_les_ids_annonces() {
        let dir = std::env::temp_dir().join("brainlink_test_touching");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"),
            serde_json::to_string(&g(vec![n("A", "A", 10), n("B", "B", 10), n("C", "C", 10)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();

        // B est modifié (nouveau label), D est créé — seuls ceux-là sont annoncés
        // comme touchés. A et C ne changent pas et ne figurent PAS dans la liste :
        // write_brain_touching ne doit ni les lire ni les comparer.
        let mut graph = g(vec![n("A", "A", 10), n("B", "B2", 10), n("C", "C", 10), n("D", "D", 0)]);
        write_brain_touching(&dir, &mut graph, &["B".to_string(), "D".to_string()], &[]).unwrap();

        let conn = crate::storage::open_or_init(&dir).unwrap();
        let ts = |id: &str| -> i64 { conn.query_row("SELECT updated_at FROM nodes WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
        assert_eq!(ts("A"), 10, "A n'était pas annoncé comme touché → estampille inchangée");
        assert_eq!(ts("C"), 10, "C n'était pas annoncé comme touché → estampille inchangée");
        assert!(ts("B") > 10, "B annoncé comme touché → ré-estampillé");
        assert!(ts("D") > 10, "D annoncé comme touché (nouveau) → estampillé");
        let b_label: String = conn.query_row("SELECT label FROM nodes WHERE id='B'", [], |r| r.get(0)).unwrap();
        assert_eq!(b_label, "B2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Régression du bug remonté par Liam le 2026-07-23 : un lot qui crée un
    /// PARENT et y déplace des enfants EN MÊME TEMPS (ex. l'Archiviste : crée
    /// un dossier de thème puis y range ses pages, tout dans une seule
    /// résolution) échouait en "FOREIGN KEY constraint failed" dès que l'ordre
    /// d'itération de `graph.nodes` traitait un enfant avant son parent tout
    /// juste créé — silencieusement en pratique : les propositions étaient déjà
    /// marquées résolues (fichiers supprimés, `mcp_resolved.json` à jour) avant
    /// que l'écriture finale échoue, donnant l'illusion que tout avait marché
    /// alors que RIEN n'était persisté dans brain.db.
    #[test]
    fn write_brain_touching_accepte_un_enfant_avant_son_parent_dans_le_meme_lot() {
        let dir = std::env::temp_dir().join("brainlink_test_fk_order");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"),
            serde_json::to_string(&g(vec![n("root", "Root", 0)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();

        // "child" (parent_id = "new-parent") apparaît AVANT "new-parent" lui-même
        // dans le Vec — exactement l'ordre qui faisait échouer la FK avant le fix.
        let child: crate::models::BrainNode = serde_json::from_value(serde_json::json!({
            "id": "child", "label": "Enfant", "kind": "note", "weight": 1,
            "parent_id": "new-parent", "updated_at": 0
        })).unwrap();
        let new_parent: crate::models::BrainNode = serde_json::from_value(serde_json::json!({
            "id": "new-parent", "label": "Nouveau thème", "kind": "note", "weight": 1,
            "parent_id": "root", "updated_at": 0
        })).unwrap();
        let mut graph = g(vec![n("root", "Root", 0), child, new_parent]);

        write_brain_touching(&dir, &mut graph, &["child".to_string(), "new-parent".to_string()], &[])
            .expect("l'ordre enfant-avant-parent dans le même lot ne doit pas faire échouer l'écriture");

        let conn = crate::storage::open_or_init(&dir).unwrap();
        let parent_of: String = conn.query_row("SELECT parent_id FROM nodes WHERE id='child'", [], |r| r.get(0)).unwrap();
        assert_eq!(parent_of, "new-parent");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Reproduit précisément le cas remonté par Liam le 2026-07-22 : un délai
    /// perceptible à la création d'UNE SEULE note sur son vrai cerveau (415
    /// nœuds). Utilise le chemin RÉEL (`write_brain_touching`, ce que
    /// `insert_note_node_in` appelle vraiment) — pas de lecture ni comparaison
    /// des 415 nœuds existants, juste l'écriture de la note créée.
    #[test]
    fn creation_dune_seule_note_reste_rapide() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/brain.liam.json");
        if !fixture.exists() {
            eprintln!("fixture réelle absente — test sauté (normal hors machine de Liam)");
            return;
        }
        let dir = std::env::temp_dir().join("brainlink_test_une_seule_note");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&fixture, dir.join("brain.json")).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();

        // Reproduit exactement le chemin réel : load_brain_cached (peuple le
        // cache) puis une seule mutation avant write_brain_touching, comme le
        // fait insert_note_node_in derrière la commande Tauri `create_note_node`.
        let mut graph = load_brain_cached(&dir).unwrap();
        graph.nodes.push(n("note-bench", "Ma note", 0));
        graph.edges.push(crate::models::BrainEdge {
            source: "root".into(), target: "note-bench".into(), kind: "contains".into(), relation: "contains".into(),
        });

        let start = std::time::Instant::now();
        write_brain_touching(&dir, &mut graph, &["note-bench".to_string()], &[]).unwrap();
        let elapsed = start.elapsed();

        eprintln!("Phase 4 — création d'une seule note sur brain réel (415 nœuds) : {elapsed:?}");
        assert!(elapsed < std::time::Duration::from_millis(50), "délai perceptible pour une seule note : {elapsed:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_brain_cached_lit_json_si_pas_de_db() {
        let dir = std::env::temp_dir().join("brainlink_test_load_json_only");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&g(vec![n("A", "A-json", 1)])).unwrap()).unwrap();

        let graph = load_brain_cached(&dir).unwrap();
        assert_eq!(graph.nodes[0].label, "A-json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_brain_cached_prefere_sqlite_quand_brain_db_existe() {
        // Phase 3a : une fois brain.db présent (post-migration), la lecture doit
        // passer par storage::load_brain_graph plutôt que par brain.json.
        let dir = std::env::temp_dir().join("brainlink_test_load_prefere_sqlite");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&g(vec![n("A", "A-sqlite", 1)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();
        assert!(!dir.join("brain.json").exists(), "la migration doit avoir renommé le json");

        let graph = load_brain_cached(&dir).unwrap();
        assert_eq!(graph.nodes[0].label, "A-sqlite", "doit être lu depuis brain.db, pas brain.json (absent)");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_spaces_convergents_et_arbre_reconstruit() {
        let remote = std::env::temp_dir().join("brainlink_test_sp_remote");
        let local = std::env::temp_dir().join("brainlink_test_sp_local");
        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); std::fs::create_dir_all(d).unwrap(); }

        let node = |id: &str, kind: &str, parent: Option<&str>| -> crate::models::BrainNode {
            serde_json::from_value(serde_json::json!({
                "id": id, "label": id, "kind": kind, "weight": 1,
                "parent_id": parent, "updated_at": 5
            })).unwrap()
        };
        let edge = |s: &str, t: &str| BrainEdge {
            source: s.into(), target: t.into(), kind: "contains".into(), relation: "contains".into(),
        };

        // Distant : arbre avec une arête bidon en plus (ex-fusion d'arêtes), space s1
        // renommé récemment, tombstone sur s3.
        let mut rg = g(vec![node("brain", "root", None), node("n1", "leaf", Some("brain"))]);
        rg.edges = vec![edge("brain", "n1"), edge("fantome", "n1")];
        std::fs::write(remote.join("brain.json"), serde_json::to_string(&rg).unwrap()).unwrap();
        std::fs::write(remote.join("spaces.json"), r#"[{"id":"s1","name":"Renommé","updated_at":20}]"#).unwrap();
        std::fs::write(remote.join("deleted_spaces.json"), r#"["s3"]"#).unwrap();
        let zip = export_in(&remote).unwrap();

        // Local : un nœud orphelin (parent disparu), s1 avec un vieux nom, s2 à lui, s3 encore là.
        let mut lg = g(vec![node("brain", "root", None), node("n1", "leaf", Some("brain")), node("n2", "leaf", Some("fantome"))]);
        lg.edges = vec![edge("brain", "n1")];
        std::fs::write(local.join("brain.json"), serde_json::to_string(&lg).unwrap()).unwrap();
        std::fs::write(local.join("spaces.json"),
            r#"[{"id":"s1","name":"Vieux","updated_at":10},{"id":"s2","name":"Local","updated_at":5},{"id":"s3","name":"Mort","updated_at":1}]"#).unwrap();

        merge_in(&local, &zip).unwrap();

        let spaces: Vec<serde_json::Value> = serde_json::from_str(
            &std::fs::read_to_string(local.join("spaces.json")).unwrap()).unwrap();
        let name = |id: &str| spaces.iter().find(|s| s["id"] == id).map(|s| s["name"].as_str().unwrap().to_string());
        assert_eq!(name("s1").as_deref(), Some("Renommé"), "le renommage distant (plus récent) se propage");
        assert!(name("s2").is_some(), "l'espace local-only survit");
        assert!(name("s3").is_none(), "la tombstone d'espace tient");

        let merged: BrainGraph = serde_json::from_str(
            &std::fs::read_to_string(local.join("brain.json")).unwrap()).unwrap();
        let contains: Vec<_> = merged.edges.iter().filter(|e| e.kind == "contains").collect();
        assert_eq!(contains.len(), 2, "arbre reconstruit : une arête par parent_id, pas d'union");
        assert!(contains.iter().all(|e| e.source == "brain"), "l'orphelin est rattaché à la racine");
        let n2 = merged.nodes.iter().find(|n| n.id == "n2").unwrap();
        assert_eq!(n2.parent_id.as_deref(), Some("brain"));
        for d in [&remote, &local] { let _ = std::fs::remove_dir_all(d); }
    }

    #[test]
    fn noms_windows_safe_encodage_et_migration() {
        assert_eq!(safe_component("leaf:conv-1"), "leaf%3Aconv-1");
        // Idempotent : ré-encoder ne change rien (sinon double-encodage au 2e passage).
        assert_eq!(safe_component(&safe_component("leaf:conv-1")), "leaf%3Aconv-1");
        assert_eq!(safe_component("note-1"), "note-1");

        // Un zip contenant un chemin hérité avec `:` est écrit sous le nom encodé.
        let src = std::env::temp_dir().join("brainlink_test_safe_src");
        let dst = std::env::temp_dir().join("brainlink_test_safe_dst");
        for d in [&src, &dst] { let _ = std::fs::remove_dir_all(d); }
        std::fs::create_dir_all(src.join("node_history/leaf:conv-1")).unwrap();
        std::fs::write(src.join("node_history/leaf:conv-1/v1.md"), "historique").unwrap();
        std::fs::write(src.join("brain.json"), "{\"nodes\":[],\"edges\":[],\"markdown\":\"\",\"generated_at\":\"t\"}").unwrap();
        // migrate_unsafe_names (via export_in) renomme le dossier hérité…
        let zip = export_in(&src).unwrap();
        assert!(src.join("node_history/leaf%3Aconv-1").exists(), "migration du dossier hérité");
        // …et la fusion côté destinataire n'écrit que des chemins sûrs.
        std::fs::create_dir_all(&dst).unwrap();
        merge_in(&dst, &zip).unwrap();
        assert!(dst.join("node_history/leaf%3Aconv-1/v1.md").exists());
        for d in [&src, &dst] { let _ = std::fs::remove_dir_all(d); }
    }

    #[test]
    fn export_puis_import_round_trip() {
        let src = std::env::temp_dir().join("brainlink_test_backup_src");
        let dst = std::env::temp_dir().join("brainlink_test_backup_dst");
        for d in [&src, &dst] { let _ = std::fs::remove_dir_all(d); }
        std::fs::create_dir_all(src.join("node_history/note-1")).unwrap();
        std::fs::create_dir_all(src.join("assets")).unwrap();
        std::fs::write(src.join("brain.json"), r#"{"nodes":[]}"#).unwrap();
        std::fs::write(src.join("brain.md"), "# Cerveau").unwrap();
        std::fs::write(src.join("node_history/note-1/v1.md"), "ancien contenu").unwrap();
        std::fs::write(src.join("assets/img-1.png"), [137u8, 80, 78, 71]).unwrap();

        let zip = export_in(&src).unwrap();
        assert!(zip.len() > 100);

        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("brain.json"), r#"{"nodes":["ancien"]}"#).unwrap();
        let n = import_in(&dst, &zip).unwrap();
        assert_eq!(n, 4);
        assert_eq!(std::fs::read_to_string(dst.join("brain.json")).unwrap(), r#"{"nodes":[]}"#);
        assert_eq!(std::fs::read_to_string(dst.join("node_history/note-1/v1.md")).unwrap(), "ancien contenu");
        // Le brain.json écrasé a été gardé de côté
        assert!(dst.join("brain.json.avant-restauration").exists());
        for d in [&src, &dst] { let _ = std::fs::remove_dir_all(d); }
    }

    /// Phase 8 — point d'attention explicite du plan : en mode WAL, un commit
    /// récent peut n'exister que dans `brain.db-wal`. Sans checkpoint avant de
    /// zipper, l'export capturerait un `brain.db` incohérent (sans les
    /// dernières écritures) — pire qu'un simple fichier texte.
    #[test]
    fn export_in_checkpoint_le_wal_avant_de_zipper() {
        let dir = std::env::temp_dir().join("brainlink_test_export_wal_checkpoint");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&g(vec![n("A", "A", 1)])).unwrap()).unwrap();
        crate::storage::migrate_json_to_sqlite(&dir).unwrap();

        // Connexion maintenue ouverte : SQLite checkpointe automatiquement à la
        // fermeture de la DERNIÈRE connexion — sans ce garde, l'insert ci-dessous
        // se retrouverait déjà checkpointé avant même d'appeler export_in, et le
        // test ne prouverait rien.
        let _guard = crate::storage::open_or_init(&dir).unwrap();
        {
            let conn = crate::storage::open_or_init(&dir).unwrap();
            conn.execute("INSERT INTO nodes (id, label, kind, weight) VALUES ('B', 'B', 'leaf', 1)", []).unwrap();
        }
        assert!(dir.join("brain.db-wal").exists(), "le commit doit atterrir dans le WAL, pas encore checkpointé");

        let zip = export_in(&dir).unwrap();

        // Extrait le brain.db du zip vers un fichier indépendant et vérifie
        // que B (écrit seulement via le WAL) y est bien présent.
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip.as_slice())).unwrap();
        let mut entry = archive.by_name("brain.db").unwrap();
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf).unwrap();
        drop(entry);
        let extracted = std::env::temp_dir().join("brainlink_test_export_wal_extracted.db");
        std::fs::write(&extracted, &buf).unwrap();
        let conn = rusqlite::Connection::open(&extracted).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM nodes WHERE id = 'B'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "B (écrit via le WAL) doit être présent dans le brain.db exporté");

        drop(_guard);
        let _ = std::fs::remove_file(&extracted);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fingerprint_nul_sans_donnees_puis_positif_apres_ecriture() {
        let dir = std::env::temp_dir().join("brainlink_test_fingerprint");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(fingerprint_in(&dir), 0);
        std::fs::write(dir.join("brain.json"), "{}").unwrap();
        assert!(fingerprint_in(&dir) > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_refuse_les_chemins_traversants() {
        let dst = std::env::temp_dir().join("brainlink_test_backup_evil");
        let _ = std::fs::remove_dir_all(&dst);
        std::fs::create_dir_all(&dst).unwrap();
        // Archive forgée avec un chemin ../
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file("../evil.txt", opts).unwrap();
            zip.write_all(b"pwn").unwrap();
            zip.finish().unwrap();
        }
        assert!(import_in(&dst, &buf.into_inner()).is_err());
        let _ = std::fs::remove_dir_all(&dst);
    }
}
