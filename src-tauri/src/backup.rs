//! Sauvegarde / restauration du cerveau : zip en mémoire des données utiles
//! (~2 Mo — brain.json, notes, historique, images…), à destination du cloud
//! (Supabase Storage, uploadé côté front). Les modèles IA et llama.cpp sont
//! exclus : retéléchargeables, et ~10 Go.

use std::io::{Read, Write};
use std::path::Path;

/// Fichiers / dossiers embarqués dans la sauvegarde (relatifs au dossier de données).
pub const FILES: &[&str] = &[
    "brain.json",
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

/// Charge brain.json — depuis le cache mémoire s'il correspond au même dossier
/// (compte actif inchangé), sinon relit le disque et peuple le cache.
pub fn load_brain_cached(dir: &Path) -> Result<crate::models::BrainGraph, String> {
    if let Some(g) = cached_for(dir) { return Ok(g); }
    let raw = std::fs::read_to_string(dir.join("brain.json")).map_err(|e| e.to_string())?;
    let graph: crate::models::BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
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
    files.chain(dirs).map(|p| mtime(&p)).max().unwrap_or(0)
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

    // Filet : brain.json actuel sauvegardé avant écrasement.
    let brain = dir.join("brain.json");
    if brain.exists() {
        let _ = std::fs::copy(&brain, dir.join("brain.json.avant-restauration"));
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

/// Sérialise un nœud SANS son champ `updated_at` — pour comparer deux versions
/// d'un nœud sans que l'estampille elle-même compte comme une différence.
fn node_sig(n: &crate::models::BrainNode) -> String {
    let mut v = serde_json::to_value(n).unwrap_or_default();
    if let Some(o) = v.as_object_mut() { o.remove("updated_at"); }
    v.to_string()
}

/// Écrit brain.json en estampillant `updated_at` sur chaque nœud nouveau ou
/// modifié par rapport à la version précédente du fichier. Point de passage
/// unique : tous les writers de brain.json doivent l'utiliser, sinon les
/// modifications ne se propagent pas dans la fusion de sync.
pub fn write_brain(dir: &Path, graph: &mut BrainGraph) -> Result<(), String> {
    // "old" vient du cache mémoire quand possible — évite une lecture complète
    // de plus du disque en plus de celle déjà faite par l'appelant pour
    // préparer `graph` (cf. `load_brain_cached`).
    let old: Option<BrainGraph> = cached_for(dir).or_else(|| {
        std::fs::read_to_string(dir.join("brain.json")).ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    });
    let old_by_id: std::collections::HashMap<String, (Option<u64>, String)> = old
        .map(|g| g.nodes.into_iter().map(|n| (n.id.clone(), (n.updated_at, node_sig(&n)))).collect())
        .unwrap_or_default();
    let now = now_secs();
    for n in &mut graph.nodes {
        n.updated_at = match old_by_id.get(&n.id) {
            Some((stamp, sig)) if *sig == node_sig(n) => stamp.or(Some(now)),
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

/// Fusionne une sauvegarde distante dans les données locales (sync cloud).
pub fn merge_in(dir: &Path, bytes: &[u8]) -> Result<MergeReport, String> {
    migrate_unsafe_names(dir);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Archive illisible : {e}"))?;

    // Filet : brain.json actuel gardé avant fusion.
    if dir.join("brain.json").exists() {
        let _ = std::fs::copy(dir.join("brain.json"), dir.join("brain.json.avant-restauration"));
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

    // brain.json : nœud par nœud, le plus récent gagne ; union des deux côtés,
    // moins les tombstones. Écrit tel quel (les estampilles fusionnées font foi).
    let local_brain: Option<BrainGraph> = std::fs::read_to_string(dir.join("brain.json"))
        .ok().and_then(|r| serde_json::from_str(&r).ok());
    let remote_brain: Option<BrainGraph> = zip_json(&mut archive, "brain.json");
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
                        if !remote_newer && node_sig(rn) != node_sig(&ln) { local_extra = true; }
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
            std::fs::write(
                dir.join("brain.json"),
                serde_json::to_string_pretty(&merged).map_err(|e| e.to_string())?,
            ).map_err(|e| e.to_string())?;
            // Seul autre écrivain de brain.json en dehors de `write_brain` (sync
            // cloud) — doit lui aussi tenir le cache mémoire à jour, sinon une
            // lecture juste après une fusion servirait une version périmée.
            set_cache(dir, &merged);
        }
        (None, Some(remote)) => {
            std::fs::write(
                dir.join("brain.json"),
                serde_json::to_string_pretty(&remote).map_err(|e| e.to_string())?,
            ).map_err(|e| e.to_string())?;
            set_cache(dir, &remote);
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
    let merged_by_hand = ["brain.json", "spaces.json", "deleted_nodes.json"];
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
