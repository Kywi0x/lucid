fn main() {
    link_resources_for_dev();
    tauri_build::build()
}

/// Rend l'arborescence de dev IDENTIQUE à celle du bundle, pour les sidecars.
///
/// Tauri copie déjà les sidecars (`llama-completion`, `llama-server`, `pdftotext`,
/// `pdftoppm`, `tesseract`) dans `target/<profil>/`. Mais les binaires poppler et
/// tesseract cherchent leurs dylibs en `@executable_path/../Resources/libs/`,
/// c'est-à-dire `target/Resources/libs/` en dev — un dossier qui n'existait pas.
/// Résultat : le sidecar se lançait et ne sortait rien, et le code le sautait donc
/// en debug pour retomber sur Homebrew. En dev on testait alors un poppler
/// différent de celui livré aux utilisateurs — exactement le trou que la règle de
/// parité (ADR-0015) interdit.
///
/// On pose donc un lien `target/Resources` → `src-tauri/resources`, qui reproduit
/// `Contents/Resources/` du .app : `libs/` et `tessdata/` se résolvent alors en dev
/// comme dans le bundle. Recréé à chaque build (donc survit à `cargo clean`).
///
/// Best-effort et silencieux : sur un système sans lien symbolique, ou si la cible
/// existe déjà, on ne casse pas le build — le pire cas est l'ancien comportement.
fn link_resources_for_dev() {
    let Ok(out) = std::env::var("OUT_DIR") else { return };
    // OUT_DIR = target/<profil>/build/<crate>-<hash>/out → on remonte à `target/`.
    let target = std::path::Path::new(&out)
        .ancestors()
        .nth(4)
        .map(std::path::Path::to_path_buf);
    let Some(target) = target else { return };
    let link = target.join("Resources");
    if link.exists() {
        return;
    }
    let resources = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    if !resources.is_dir() {
        return;
    }
    #[cfg(unix)]
    let _ = std::os::unix::fs::symlink(&resources, &link);
    #[cfg(windows)]
    let _ = std::os::windows::fs::symlink_dir(&resources, &link);
}
