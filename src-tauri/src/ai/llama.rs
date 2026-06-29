//! Moteur d'inférence local : pilote le binaire `llama-cli` (llama.cpp) en
//! subprocess avec le modèle Gemma 3 4B. Aucune dépendance réseau.
//!
//! Résolution des chemins (binaire + modèle), par ordre de priorité :
//!   1. variables d'environnement `SECOND_BRAIN_LLAMA_BIN` / `SECOND_BRAIN_MODEL`
//!   2. dossier de données applicatif (`<data_dir>/fr.ideeri.brainlink/…`)

use std::path::PathBuf;
use std::process::Command;

const MODEL_FILE: &str = "gemma-3-4b-it-Q4_K_M.gguf";
const APP_DIR: &str = "fr.ideeri.brainlink";

#[derive(Clone, Debug)]
pub struct LlamaEngine {
    binary: PathBuf,
    model: PathBuf,
}

/// Dossier de données applicatif (`~/Library/Application Support/fr.ideeri.brainlink` sur macOS).
pub fn app_data_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP_DIR))
}

fn resolve_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_LLAMA_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidate = app_data_dir()?
        .join("llama.cpp")
        .join("build")
        .join("bin")
        .join("llama-completion");
    candidate.is_file().then_some(candidate)
}

fn resolve_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_MODEL") {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let candidate = app_data_dir()?.join("models").join(MODEL_FILE);
    candidate.is_file().then_some(candidate)
}

impl LlamaEngine {
    /// Détecte le binaire et le modèle. Renvoie une erreur lisible si absents.
    pub fn detect() -> Result<Self, String> {
        let binary = resolve_binary()
            .ok_or("Binaire llama-cli introuvable. Lance le build de llama.cpp.")?;
        let model = resolve_model()
            .ok_or("Modèle Gemma introuvable. Télécharge-le dans <data>/models/.")?;
        Ok(Self { binary, model })
    }

    /// `true` si le moteur est prêt (binaire + modèle présents).
    pub fn is_ready() -> bool {
        resolve_binary().is_some() && resolve_model().is_some()
    }

    /// Exécute une complétion one-shot et renvoie le texte généré (stdout).
    /// Le gabarit de chat Gemma 3 est appliqué automatiquement par llama-completion
    /// via les options `-sys` (message système) et `-p` (message utilisateur).
    pub fn complete(
        &self,
        system: Option<&str>,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, String> {
        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m")
            .arg(&self.model)
            .args(["-ngl", "99"]) // offload GPU (Metal)
            .args(["-c", "8192"]) // taille de contexte
            .args(["-n", &max_tokens.to_string()])
            .args(["--temp", "0.2"])
            .args(["--top-p", "0.9"])
            .arg("-no-cnv") // complétion one-shot, pas d'interactif
            .arg("--no-display-prompt");
        if let Some(s) = system {
            cmd.arg("-sys").arg(s);
        }
        cmd.arg("-p").arg(user);

        let output = cmd
            .output()
            .map_err(|e| format!("Échec d'exécution de llama-completion : {e}"))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "llama-completion a échoué ({}) : {}",
                output.status,
                err.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
            ));
        }

        let text = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(clean_output(&text))
    }
}

/// Nettoie la sortie : retire les marqueurs de fin de tour, `[end of text]` et
/// les éventuelles clôtures de bloc de code (```json … ```).
fn clean_output(s: &str) -> String {
    let mut out = s
        .replace("<end_of_turn>", "")
        .replace("[end of text]", "")
        .trim()
        .to_string();

    // Retire les fences Markdown si le modèle a emballé sa réponse.
    if let Some(rest) = out.strip_prefix("```") {
        // saute l'éventuel langage (ex. "json") sur la première ligne
        let rest = rest.splitn(2, '\n').nth(1).unwrap_or(rest);
        out = rest.to_string();
        if let Some(idx) = out.rfind("```") {
            out.truncate(idx);
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::clean_output;

    #[test]
    fn strips_json_fences() {
        let s = "```json\n{\"a\": 1}\n```\n [end of text]";
        assert_eq!(clean_output(s), "{\"a\": 1}");
    }

    #[test]
    fn passes_plain_json() {
        assert_eq!(clean_output("  {\"a\":1}  "), "{\"a\":1}");
    }
}
