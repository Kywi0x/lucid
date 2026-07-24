//! Phase 1 de la migration SQLite (`brain.json` → `brain.db`) : schéma et
//! ouverture de connexion. Posé à côté du code existant — aucun appelant du
//! chemin JSON n'est modifié ici (cf. plan de build dans le vault).

use rusqlite::Connection;
use std::path::Path;

/// Ouvre `brain.db` dans `dir` (le crée avec son schéma si absent), active WAL.
pub fn open_or_init(dir: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(dir.join("brain.db"))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS nodes (
            id              TEXT PRIMARY KEY,
            label           TEXT NOT NULL,
            kind            TEXT NOT NULL,
            weight          INTEGER NOT NULL,
            summary         TEXT NOT NULL DEFAULT '',
            keywords        TEXT NOT NULL DEFAULT '[]',
            decisions       TEXT NOT NULL DEFAULT '[]',
            patterns        TEXT NOT NULL DEFAULT '[]',
            community       INTEGER NOT NULL DEFAULT 0,
            parent_id       TEXT REFERENCES nodes(id),
            synthesized_at  TEXT,
            date            TEXT,
            content         TEXT NOT NULL DEFAULT '',
            connector       TEXT,
            source_id       TEXT,
            source_project  TEXT,
            source_text     TEXT NOT NULL DEFAULT '',
            updated_at      INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_nodes_parent_id ON nodes(parent_id);

        CREATE TABLE IF NOT EXISTS edges (
            id       INTEGER PRIMARY KEY AUTOINCREMENT,
            source   TEXT NOT NULL,
            target   TEXT NOT NULL,
            kind     TEXT NOT NULL DEFAULT 'contains',
            relation TEXT NOT NULL DEFAULT 'contains'
        );

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )?;
    Ok(conn)
}

/// Valeur de `meta` (markdown/report/generated_at — pas encore recalculés depuis
/// nodes/edges, Phase 6 s'en charge ; en attendant on les porte tels quels).
fn get_meta(conn: &Connection, key: &str) -> String {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .unwrap_or_default()
}

pub(crate) fn set_meta(tx: &rusqlite::Transaction, key: &str, value: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Insère ou met à jour un nœud (Phase 4 : un UPDATE/INSERT ciblé par nœud
/// réellement modifié, jamais une réécriture totale). Réutilisé par la
/// migration one-shot (Phase 2, toujours des inserts sur base vide) et par
/// `backup::write_brain` côté SQLite (Phase 4, vrais upserts).
pub(crate) fn upsert_node(tx: &rusqlite::Transaction, n: &crate::models::BrainNode) -> rusqlite::Result<()> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO nodes (id, label, kind, weight, summary, keywords, decisions, patterns,
            community, parent_id, synthesized_at, date, content, connector, source_id,
            source_project, source_text, updated_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
         ON CONFLICT(id) DO UPDATE SET
            label=excluded.label, kind=excluded.kind, weight=excluded.weight, summary=excluded.summary,
            keywords=excluded.keywords, decisions=excluded.decisions, patterns=excluded.patterns,
            community=excluded.community, parent_id=excluded.parent_id, synthesized_at=excluded.synthesized_at,
            date=excluded.date, content=excluded.content, connector=excluded.connector, source_id=excluded.source_id,
            source_project=excluded.source_project, source_text=excluded.source_text, updated_at=excluded.updated_at",
    )?;
    stmt.execute(rusqlite::params![
            n.id,
            n.label,
            n.kind,
            n.weight as i64,
            n.summary,
            serde_json::to_string(&n.keywords).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&n.decisions).unwrap_or_else(|_| "[]".into()),
            serde_json::to_string(&n.patterns).unwrap_or_else(|_| "[]".into()),
            n.community as i64,
            n.parent_id,
            n.synthesized_at,
            n.date,
            n.content,
            n.connector,
            n.source_id,
            n.source_project,
            n.source_text,
            n.updated_at.map(|v| v as i64),
        ],
    )?;
    Ok(())
}

/// Reconstruit le `BrainGraph` complet depuis `brain.db` — équivalent SQL de
/// l'ancienne désérialisation de `brain.json`. Signature de retour identique :
/// zéro changement côté appelants (cf. Phase 3a du plan).
pub fn load_brain_graph(dir: &Path) -> Result<crate::models::BrainGraph, String> {
    let conn = open_or_init(dir).map_err(|e| e.to_string())?;
    graph_from_conn(&conn)
}

/// Comme `load_brain_graph`, mais sur un fichier `.db` arbitraire (Phase 7 :
/// un snapshot dans `snapshots/`) plutôt que le `brain.db` actif du compte —
/// pas de création de schéma, le fichier en a déjà un (copié par `VACUUM INTO`).
pub fn load_brain_graph_from_file(db_path: &Path) -> Result<crate::models::BrainGraph, String> {
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    graph_from_conn(&conn)
}

fn graph_from_conn(conn: &Connection) -> Result<crate::models::BrainGraph, String> {
    let nodes = conn
        .prepare(
            "SELECT id, label, kind, weight, summary, keywords, decisions, patterns, community,
                    parent_id, synthesized_at, date, content, connector, source_id, source_project,
                    source_text, updated_at
             FROM nodes",
        )
        .map_err(|e| e.to_string())?
        .query_map([], |r| {
            let keywords: String = r.get(5)?;
            let decisions: String = r.get(6)?;
            let patterns: String = r.get(7)?;
            let weight: i64 = r.get(3)?;
            let community: i64 = r.get(8)?;
            let updated_at: Option<i64> = r.get(17)?;
            Ok(crate::models::BrainNode {
                id: r.get(0)?,
                label: r.get(1)?,
                kind: r.get(2)?,
                weight: weight as usize,
                summary: r.get(4)?,
                keywords: serde_json::from_str(&keywords).unwrap_or_default(),
                decisions: serde_json::from_str(&decisions).unwrap_or_default(),
                patterns: serde_json::from_str(&patterns).unwrap_or_default(),
                community: community as u32,
                parent_id: r.get(9)?,
                synthesized_at: r.get(10)?,
                date: r.get(11)?,
                content: r.get(12)?,
                connector: r.get(13)?,
                source_id: r.get(14)?,
                source_project: r.get(15)?,
                source_text: r.get(16)?,
                updated_at: updated_at.map(|v| v as u64),
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let edges = edges_from(conn)?;

    Ok(crate::models::BrainGraph {
        nodes,
        edges,
        markdown: get_meta(conn, "markdown"),
        report: get_meta(conn, "report"),
        generated_at: get_meta(conn, "generated_at"),
    })
}

fn edges_from(conn: &Connection) -> Result<Vec<crate::models::BrainEdge>, String> {
    conn.prepare("SELECT source, target, kind, relation FROM edges")
        .map_err(|e| e.to_string())?
        .query_map([], |r| {
            Ok(crate::models::BrainEdge {
                source: r.get(0)?,
                target: r.get(1)?,
                kind: r.get(2)?,
                relation: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Lecture légère des arêtes seules (pas de reconstruction des 415 nœuds ni de
/// leur `source_text`) — pour les écritures ciblées qui savent déjà quels
/// nœuds elles ont touchés (cf. `backup::write_brain_touching`) et n'ont besoin
/// de comparer que les arêtes, pas cher (~1 ms), contre le graphe entier.
pub fn load_edges(dir: &Path) -> Result<Vec<crate::models::BrainEdge>, String> {
    let conn = open_or_init(dir).map_err(|e| e.to_string())?;
    edges_from(&conn)
}

/// Copie atomique et cohérente de `brain.db` vers `dest` (Phase 7) — `VACUUM
/// INTO` inclut le WAL en cours, contrairement à un simple `fs::copy` du
/// fichier principal qui laisserait des écritures récentes hors de la copie.
pub fn snapshot_to(dir: &Path, dest: &Path) -> Result<(), String> {
    let conn = open_or_init(dir).map_err(|e| e.to_string())?;
    conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().to_string()])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Remplace le contenu de `brain.db` par `graph` tel quel — AUCUN estampillage
/// (contrairement à `upsert_node` utilisé depuis `write_brain`/`write_brain_touching`,
/// qui laisse l'appelant décider, mais ici l'appelant est `backup::merge_in`,
/// Phase 8 : les `updated_at` viennent d'une fusion LWW déjà tranchée — les
/// réécrire à "maintenant" invaliderait la fusion au prochain cycle de sync).
pub fn replace_graph(dir: &Path, graph: &crate::models::BrainGraph) -> Result<(), String> {
    let mut conn = open_or_init(dir).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let new_ids: std::collections::HashSet<&str> = graph.nodes.iter().map(|n| n.id.as_str()).collect();
    let existing_ids: Vec<String> = tx
        .prepare("SELECT id FROM nodes")
        .map_err(|e| e.to_string())?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    for id in &existing_ids {
        if !new_ids.contains(id.as_str()) {
            tx.execute("DELETE FROM nodes WHERE id = ?1", [id]).map_err(|e| e.to_string())?;
        }
    }
    for n in &graph.nodes {
        upsert_node(&tx, n).map_err(|e| format!("upsert nœud {} : {e}", n.id))?;
    }
    tx.execute("DELETE FROM edges", []).map_err(|e| e.to_string())?;
    for e in &graph.edges {
        tx.execute(
            "INSERT INTO edges (source, target, kind, relation) VALUES (?1,?2,?3,?4)",
            rusqlite::params![e.source, e.target, e.kind, e.relation],
        )
        .map_err(|e| e.to_string())?;
    }
    set_meta(&tx, "markdown", &graph.markdown).map_err(|e| e.to_string())?;
    set_meta(&tx, "report", &graph.report).map_err(|e| e.to_string())?;
    set_meta(&tx, "generated_at", &graph.generated_at).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Migration one-shot : si `brain.db` n'existe pas encore et `brain.json` oui,
/// importe tout dans une seule transaction puis met `brain.json` de côté
/// (renommé, jamais supprimé — même filet de sécurité que
/// `brain.json.avant-restauration` dans `backup.rs`). No-op si `brain.db`
/// existe déjà (migration déjà faite) ou si `brain.json` est absent.
pub fn migrate_json_to_sqlite(dir: &Path) -> Result<(), String> {
    let json_path = dir.join("brain.json");
    let db_path = dir.join("brain.db");
    if db_path.exists() || !json_path.exists() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(&json_path).map_err(|e| e.to_string())?;
    let graph: crate::models::BrainGraph = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let mut conn = open_or_init(dir).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    for n in &graph.nodes {
        upsert_node(&tx, n).map_err(|e| format!("insertion nœud {} : {e}", n.id))?;
    }
    for e in &graph.edges {
        tx.execute(
            "INSERT INTO edges (source, target, kind, relation) VALUES (?1,?2,?3,?4)",
            rusqlite::params![e.source, e.target, e.kind, e.relation],
        )
        .map_err(|err| err.to_string())?;
    }
    set_meta(&tx, "markdown", &graph.markdown).map_err(|e| e.to_string())?;
    set_meta(&tx, "report", &graph.report).map_err(|e| e.to_string())?;
    set_meta(&tx, "generated_at", &graph.generated_at).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    std::fs::rename(&json_path, dir.join(format!("brain.json.migre-{ts}")))
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_to_capture_un_etat_coherent_y_compris_le_wal() {
        let dir = std::env::temp_dir().join("brainlink_test_snapshot_to");
        let dest_dir = std::env::temp_dir().join("brainlink_test_snapshot_to_dest");
        let _ = std::fs::remove_dir_all(&dir); std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dest_dir); std::fs::create_dir_all(&dest_dir).unwrap();

        let conn = open_or_init(&dir).unwrap();
        conn.execute("INSERT INTO nodes (id, label, kind, weight) VALUES ('A', 'A', 'leaf', 1)", []).unwrap();
        drop(conn);

        let dest = dest_dir.join("snap.db");
        snapshot_to(&dir, &dest).unwrap();

        let snap_conn = Connection::open(&dest).unwrap();
        let count: i64 = snap_conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 1, "le snapshot doit contenir les nœuds déjà écrits (y compris via le WAL)");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    #[test]
    fn open_or_init_cree_les_tables_et_active_wal() {
        let dir = std::env::temp_dir().join("brainlink_test_storage_init");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let conn = open_or_init(&dir).unwrap();

        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode.to_lowercase(), "wal");

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, vec!["edges".to_string(), "meta".to_string(), "nodes".to_string()]);

        // Idempotent : ré-ouvrir sur le même fichier ne doit pas échouer sur les CREATE TABLE.
        drop(conn);
        open_or_init(&dir).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn renamed_json_exists(dir: &Path) -> bool {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().starts_with("brain.json.migre-"))
    }

    #[test]
    fn migre_json_vers_sqlite_et_met_le_json_de_cote() {
        let dir = std::env::temp_dir().join("brainlink_test_migrate_synth");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let graph = crate::models::BrainGraph {
            nodes: vec![
                serde_json::from_value(serde_json::json!({
                    "id": "root", "label": "Cerveau", "kind": "root", "weight": 2
                }))
                .unwrap(),
                serde_json::from_value(serde_json::json!({
                    "id": "A", "label": "Projet A", "kind": "leaf", "weight": 1,
                    "parent_id": "root", "keywords": ["rust", "sqlite"], "updated_at": 42
                }))
                .unwrap(),
            ],
            edges: vec![crate::models::BrainEdge {
                source: "root".into(),
                target: "A".into(),
                kind: "contains".into(),
                relation: "contains".into(),
            }],
            markdown: "# Cerveau\n\n- Projet A".into(),
            report: "Cerveau > Projet A".into(),
            generated_at: "2026-07-22T10:00:00Z".into(),
        };
        std::fs::write(dir.join("brain.json"), serde_json::to_string(&graph).unwrap()).unwrap();

        migrate_json_to_sqlite(&dir).unwrap();

        assert!(!dir.join("brain.json").exists(), "l'original doit être renommé, pas laissé en place");
        assert!(renamed_json_exists(&dir), "le filet de sécurité brain.json.migre-<ts> doit exister");

        let conn = open_or_init(&dir).unwrap();
        let (label, keywords): (String, String) = conn
            .query_row("SELECT label, keywords FROM nodes WHERE id = 'A'", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(label, "Projet A");
        assert_eq!(keywords, r#"["rust","sqlite"]"#);

        let edge_count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(edge_count, 1);
        assert_eq!(get_meta(&conn, "markdown"), graph.markdown);
        assert_eq!(get_meta(&conn, "report"), graph.report);
        assert_eq!(get_meta(&conn, "generated_at"), graph.generated_at);
        drop(conn);

        // load_brain_graph doit reconstruire exactement la même chose (Phase 3a).
        let reloaded = load_brain_graph(&dir).unwrap();
        assert_eq!(reloaded.nodes.len(), graph.nodes.len());
        assert_eq!(reloaded.edges.len(), graph.edges.len());
        assert_eq!(reloaded.markdown, graph.markdown);
        assert_eq!(reloaded.report, graph.report);
        assert_eq!(reloaded.generated_at, graph.generated_at);

        // Idempotence : brain.db existe déjà → no-op (une vraie ré-insertion planterait
        // sur la clé primaire dupliquée, donc l'absence d'erreur ici prouve le no-op).
        migrate_json_to_sqlite(&dir).unwrap();

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Script de contrôle Phase 2 : migre la vraie fixture de Liam (415 nœuds) et
    /// compare nœud par nœud (pas juste "ça a tourné sans erreur"). Sautée si la
    /// fixture n'est pas présente (fichier gitignored, données réelles locales
    /// uniquement — cf. .gitignore).
    #[test]
    fn migre_le_brain_json_reel_de_liam_sans_perte() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/brain.liam.json");
        if !fixture.exists() {
            eprintln!("fixture réelle absente — test sauté (normal hors machine de Liam)");
            return;
        }

        let original: crate::models::BrainGraph =
            serde_json::from_str(&std::fs::read_to_string(&fixture).unwrap()).unwrap();

        let dir = std::env::temp_dir().join("brainlink_test_migrate_reelle");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&fixture, dir.join("brain.json")).unwrap();

        migrate_json_to_sqlite(&dir).unwrap();

        assert!(!dir.join("brain.json").exists());
        assert!(renamed_json_exists(&dir));

        let conn = open_or_init(&dir).unwrap();
        let node_count: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0)).unwrap();
        let edge_count: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0)).unwrap();
        assert_eq!(node_count as usize, original.nodes.len());
        assert_eq!(edge_count as usize, original.edges.len());

        for n in &original.nodes {
            let (label, kind, weight, summary, keywords, decisions, patterns, community, parent_id, updated_at): (
                String, String, i64, String, String, String, String, i64, Option<String>, Option<i64>,
            ) = conn
                .query_row(
                    "SELECT label, kind, weight, summary, keywords, decisions, patterns, community, parent_id, updated_at
                     FROM nodes WHERE id = ?1",
                    [&n.id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
                )
                .unwrap_or_else(|e| panic!("nœud {} absent de brain.db : {e}", n.id));

            assert_eq!(label, n.label, "label divergent pour {}", n.id);
            assert_eq!(kind, n.kind, "kind divergent pour {}", n.id);
            assert_eq!(weight as usize, n.weight, "weight divergent pour {}", n.id);
            assert_eq!(summary, n.summary, "summary divergent pour {}", n.id);
            assert_eq!(community as u32, n.community, "community divergente pour {}", n.id);
            assert_eq!(parent_id, n.parent_id, "parent_id divergent pour {}", n.id);
            assert_eq!(updated_at.map(|v| v as u64), n.updated_at, "updated_at divergent pour {}", n.id);

            let keywords: Vec<String> = serde_json::from_str(&keywords).unwrap();
            let decisions: Vec<String> = serde_json::from_str(&decisions).unwrap();
            let patterns: Vec<String> = serde_json::from_str(&patterns).unwrap();
            assert_eq!(keywords, n.keywords, "keywords divergents pour {}", n.id);
            assert_eq!(decisions, n.decisions, "decisions divergentes pour {}", n.id);
            assert_eq!(patterns, n.patterns, "patterns divergents pour {}", n.id);
        }

        let mut db_edges: Vec<(String, String, String, String)> = conn
            .prepare("SELECT source, target, kind, relation FROM edges")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut orig_edges: Vec<(String, String, String, String)> = original
            .edges
            .iter()
            .map(|e| (e.source.clone(), e.target.clone(), e.kind.clone(), e.relation.clone()))
            .collect();
        db_edges.sort();
        orig_edges.sort();
        assert_eq!(db_edges, orig_edges, "les arêtes doivent correspondre exactement (multi-ensemble)");

        assert_eq!(get_meta(&conn, "markdown"), original.markdown, "markdown divergent");
        assert_eq!(get_meta(&conn, "report"), original.report, "report divergent");
        assert_eq!(get_meta(&conn, "generated_at"), original.generated_at, "generated_at divergent");
        drop(conn);

        // Phase 3a : load_brain_graph doit reconstruire le même graphe que l'original.
        let reloaded = load_brain_graph(&dir).unwrap();
        assert_eq!(reloaded.nodes.len(), original.nodes.len());
        assert_eq!(reloaded.edges.len(), original.edges.len());
        assert_eq!(reloaded.markdown, original.markdown);
        assert_eq!(reloaded.report, original.report);
        assert_eq!(reloaded.generated_at, original.generated_at);
        let reloaded_label = |id: &str| reloaded.nodes.iter().find(|n| n.id == id).map(|n| n.label.clone());
        for n in original.nodes.iter().step_by(97) {
            assert_eq!(reloaded_label(&n.id), Some(n.label.clone()), "nœud {} divergent via load_brain_graph", n.id);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
