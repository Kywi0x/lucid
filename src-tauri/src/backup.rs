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
    "brain_cache.json",
    "notion_cache.json",
    "google_drive_conversations.json",
];
pub const DIRS: &[&str] = &["snapshots", "node_history", "assets", "mcp_pending"];

/// Zippe les données du cerveau. Renvoie les octets du zip.
pub fn export_in(dir: &Path) -> Result<Vec<u8>, String> {
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
        let dest = dir.join(rel);
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

#[cfg(test)]
mod tests {
    use super::*;

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
