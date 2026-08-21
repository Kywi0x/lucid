//! Connecteurs : sources de données que Second Brain ingère.

pub mod apple_notes;
pub mod chatgpt;
pub mod claude_ai;
pub mod claude_code;
pub mod cowork;
pub mod google_drive;
pub mod local_folder;
pub mod obsidian;

pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
pub const SOURCE_COWORK: &str = "cowork";

// ─── Échecs d'extraction déjà payés (A2) ──────────────────────────────────────
//
// Constat du run Windows (679 documents) : 5 fichiers rejoués à CHAQUE passe,
// avec tentative OCR à chaque fois — l'un à 106 s puis 177 s. Plusieurs minutes
// perdues par scan pour zéro résultat, sur un premier scan déjà long.
//
// On mémorise donc l'échec avec la même clé que le cache d'extraction : tant que
// le fichier n'a pas changé (mtime local / `modifiedTime` Drive), on ne le
// retente pas — mais on continue de l'ANNONCER comme illisible, jamais de
// disparition silencieuse (ADR-0015).
//
// Le critère de mémorisation est le COÛT, pas le type d'erreur. Un échec
// instantané (format non supporté, binaire OCR absent) ne coûte rien à rejouer,
// et surtout il doit se débloquer tout seul le jour où l'utilisateur installe
// tesseract — le mémoriser serait un piège. Seul ce qui a réellement brûlé du
// temps mérite d'être retenu.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(serde::Serialize, serde::Deserialize, Clone, PartialEq, Debug)]
struct ExtractFailure {
    /// mtime (local) ou `modifiedTime` (Drive) au moment de l'échec.
    stamp: String,
    reason: String,
}

/// En dessous, rejouer ne coûte rien : on préfère retenter.
const REMEMBER_ABOVE: Duration = Duration::from_secs(5);

/// L'échec est daté du fichier ET de l'outillage disponible. Cas réel visé : un
/// PDF scanné où `pdftoppm` rend bien les 10 pages (donc quelques secondes
/// dépensées, au-dessus du seuil) mais où `tesseract` manque — l'échec serait
/// mémorisé alors qu'installer tesseract doit le débloquer. En faisant entrer
/// l'outillage dans la clé, toute installation invalide les échecs d'un coup.
/// Calculé UNE fois par lancement : un `which` par fichier sur 42 000 fichiers
/// coûterait plus cher que ce qu'on économise (donc redémarrage requis après une
/// installation — comportement normal et documenté).
fn toolchain_stamp() -> &'static str {
    static T: OnceLock<&'static str> = OnceLock::new();
    T.get_or_init(|| {
        let ocr = google_drive::which_bin("pdftoppm").is_some()
            && google_drive::which_bin("tesseract").is_some();
        if ocr { "ocr1" } else { "ocr0" }
    })
}

/// Clé de version : le fichier n'a pas changé ET l'outillage non plus.
fn versioned(stamp: &str) -> String {
    format!("{stamp}|{}", toolchain_stamp())
}

fn failures_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("extract_failures.json"))
}

// ponytail: pas de purge des entrées dont le fichier a disparu — quelques
// dizaines d'octets chacune, jamais relues. À faire si le fichier grossit.
fn failures() -> &'static Mutex<HashMap<String, ExtractFailure>> {
    static F: OnceLock<Mutex<HashMap<String, ExtractFailure>>> = OnceLock::new();
    F.get_or_init(|| {
        Mutex::new(
            failures_path()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .and_then(|r| serde_json::from_str(&r).ok())
                .unwrap_or_default(),
        )
    })
}

/// Cœur testable de la consultation : l'entrée ne vaut que pour LA version du
/// fichier qui a échoué. Fichier modifié → on retente.
fn lookup(map: &HashMap<String, ExtractFailure>, key: &str, stamp: &str) -> Option<String> {
    map.get(key).filter(|f| f.stamp == stamp).map(|f| f.reason.clone())
}

/// Cœur testable de l'enregistrement. Rend `true` si l'échec a été retenu.
fn record(
    map: &mut HashMap<String, ExtractFailure>,
    key: &str,
    stamp: &str,
    reason: &str,
    cost: Duration,
) -> bool {
    if cost < REMEMBER_ABOVE {
        return false;
    }
    map.insert(key.to_string(), ExtractFailure { stamp: stamp.into(), reason: reason.into() });
    true
}

/// Raison de l'échec précédent si CE fichier, dans CETTE version, a déjà échoué
/// cher. L'appelant doit quand même le compter comme illisible dans son rapport.
pub fn known_extract_failure(key: &str, stamp: &str) -> Option<String> {
    let map = failures().lock().unwrap_or_else(|p| p.into_inner());
    lookup(&map, key, &versioned(stamp))
}

/// Retient un échec s'il a coûté cher. Best-effort : ne jamais faire échouer un
/// scan parce qu'on n'a pas su écrire un cache.
pub fn remember_extract_failure(key: &str, stamp: &str, reason: &str, cost: Duration) {
    let mut map = failures().lock().unwrap_or_else(|p| p.into_inner());
    if !record(&mut map, key, &versioned(stamp), reason, cost) {
        return;
    }
    crate::elog!("🧠 échec d'extraction mémorisé ({:.0}s) : {key} — {reason}", cost.as_secs_f32());
    if let (Some(p), Ok(json)) = (failures_path(), serde_json::to_string(&*map)) {
        let _ = std::fs::write(p, json);
    }
}

#[cfg(test)]
mod extract_failure_tests {
    use super::*;

    #[test]
    fn ne_retient_que_les_echecs_qui_ont_coute_cher() {
        let mut map = HashMap::new();
        // Binaire OCR absent : échec instantané. Le mémoriser empêcherait le
        // fichier de repasser le jour où tesseract est installé.
        assert!(!record(&mut map, "k", "t1", "OCR requis", Duration::from_millis(40)));
        assert_eq!(lookup(&map, "k", "t1"), None);

        // OCR joué pour rien pendant 177 s : à ne plus jamais rejouer.
        assert!(record(&mut map, "k", "t1", "aucun texte", Duration::from_secs(177)));
        assert_eq!(lookup(&map, "k", "t1").as_deref(), Some("aucun texte"));

        // Fichier modifié depuis : l'échec ne vaut plus, on retente.
        assert_eq!(lookup(&map, "k", "t2"), None);
    }

    /// Le tampon d'outillage entre dans la clé : installer tesseract doit
    /// débloquer les fichiers abandonnés faute d'OCR, sans les toucher.
    #[test]
    fn installer_l_ocr_invalide_les_echecs_precedents() {
        let mut map = HashMap::new();
        record(&mut map, "k", "t1|ocr0", "aucun texte", Duration::from_secs(20));
        assert_eq!(lookup(&map, "k", "t1|ocr0").as_deref(), Some("aucun texte"));
        assert_eq!(lookup(&map, "k", "t1|ocr1"), None, "outillage changé → on retente");
    }
}
