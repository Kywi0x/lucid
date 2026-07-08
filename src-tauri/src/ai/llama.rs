//! Moteur d'inférence local : pilote le binaire `llama-completion` (llama.cpp).
//!
//! Les modèles disponibles sont fetchés depuis le catalogue GPT4All (GitHub) au
//! premier lancement, puis mis en cache localement. Les URLs viennent du catalogue
//! officiel → jamais de 404/401 par URL hardcodée.

use std::path::PathBuf;
use std::process::Command;

const APP_DIR: &str = "fr.ideeri.brainlink";
/// Taille de la fenêtre de contexte passée à llama (`-c`). Exposée à l'UI via
/// `ai_info` pour afficher la capacité et avertir si un prompt la dépasse.
pub const CONTEXT_TOKENS: u32 = 8192;
/// Catalogue GPT4All officiel — URLs vérifiées et maintenues par l'équipe GPT4All.
const CATALOG_URL: &str =
    "https://raw.githubusercontent.com/nomic-ai/gpt4all/main/gpt4all-chat/metadata/models3.json";

// ── Modèle (owned strings, sérialisable) ─────────────────────────────────────

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ModelDef {
    pub id: String,
    pub name: String,
    pub file: String,
    pub url: String,
    pub size_gb: f32,
    pub min_ram_gb: f32,
}

// ── Dossier de données ────────────────────────────────────────────────────────

pub fn app_data_dir() -> Option<PathBuf> {
    Some(dirs::data_dir()?.join(APP_DIR))
}

fn catalog_cache_path() -> Option<PathBuf> {
    Some(app_data_dir()?.join("model_catalog.json"))
}

fn config_path() -> Option<PathBuf> {
    Some(app_data_dir()?.join("model_config.json"))
}

// ── Catalogue de modèles ──────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct RawEntry {
    name: Option<String>,
    filename: Option<String>,
    url: Option<String>,
    filesize: Option<serde_json::Value>,
    #[serde(rename = "ramrequired")]
    ram_required: Option<serde_json::Value>,
    #[serde(rename = "isEmbeddingModel")]
    is_embedding_model: Option<bool>,
}

/// Familles de modèles recommandées pour la synthèse de connaissances.
/// Tous sont instruction-tuned et produisent du JSON structuré de manière fiable.
const QUALITY_FAMILIES: &[&str] = &[
    "phi-3",        // Phi-3 / Phi-3.5 : petit mais excellent pour le suivi d'instructions
    "mistral",      // Mistral 7B : référence pour la synthèse structurée
    "llama-3",      // Llama 3 8B : meilleur raisonnement du catalogue GPT4All
    "llama3",       // variante de nommage
    "qwen2.5",      // Qwen 2.5 : très bon en JSON / extraction
    "qwen-2.5",
    "gemma-2",      // Gemma 2 (2B / 9B) : compact et précis
    "gemma2",
];

fn raw_to_model(e: RawEntry) -> Option<ModelDef> {
    let url = e.url?;
    let file = e.filename?;
    // Uniquement les modèles hébergés sur le CDN GPT4All (pas HuggingFace)
    if !url.starts_with("https://gpt4all.io/models/gguf/") { return None; }
    if e.is_embedding_model == Some(true) { return None; }

    // Familles curatées uniquement
    let file_lower = file.to_lowercase();
    if !QUALITY_FAMILIES.iter().any(|f| file_lower.contains(f)) { return None; }

    let size_bytes: u64 = match &e.filesize {
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        _ => 0,
    };
    let size_gb = size_bytes as f32 / 1_073_741_824.0;
    if size_gb > 9.0 || size_gb < 0.5 { return None; }

    let min_ram_gb: f32 = match &e.ram_required {
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(4.0),
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(4.0) as f32,
        _ => 4.0,
    };

    let name = e.name.unwrap_or_else(|| file.trim_end_matches(".gguf").to_string());
    let id = file.trim_end_matches(".gguf")
        .to_lowercase()
        .replace(['.', ' ', '_'], "-");

    Some(ModelDef { id, name, file, url, size_gb, min_ram_gb })
}

/// Fetch le catalogue depuis GitHub, cache localement, retourne les modèles filtrés.
fn fetch_catalog() -> Vec<ModelDef> {
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("brainlink/0.1")
        .build()
    {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let text = match client.get(CATALOG_URL).send() {
        Ok(r) if r.status().is_success() => match r.text() { Ok(t) => t, Err(_) => return vec![] },
        _ => return vec![],
    };

    let entries: Vec<RawEntry> = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let mut models: Vec<ModelDef> = entries.into_iter().filter_map(raw_to_model).collect();
    models.sort_by(|a, b| a.size_gb.partial_cmp(&b.size_gb).unwrap_or(std::cmp::Ordering::Equal));

    if let Some(path) = catalog_cache_path() {
        let _ = std::fs::create_dir_all(path.parent().unwrap());
        if let Ok(json) = serde_json::to_string(&serde_json::json!({
            "version": CATALOG_VERSION,
            "models": models,
        })) {
            let _ = std::fs::write(&path, json);
        }
    }

    models
}

const CATALOG_VERSION: &str = "v2-curated"; // bump pour invalider le cache si le filtre change

/// Modèles curés hébergés HORS du CDN GPT4All (HuggingFace) — donc invisibles du
/// fetch distant filtré (`raw_to_model` n'accepte que `gpt4all.io`). Téléchargeables
/// in-app via l'URL `resolve/main` (redirection 302 HF suivie par reqwest par défaut).
fn curated_models() -> Vec<ModelDef> {
    vec![
        // Gemma 4 E4B (Google, avr. 2026) — successeur de Gemma 3 4B. Repo officiel ggml-org.
        ModelDef {
            id: "gemma-4-e4b-it-q4-k-m".into(),
            name: "Gemma 4 E4B (Google)".into(),
            file: "gemma-4-E4B-it-Q4_K_M.gguf".into(),
            url: "https://huggingface.co/ggml-org/gemma-4-E4B-it-GGUF/resolve/main/gemma-4-E4B-it-Q4_K_M.gguf".into(),
            size_gb: 5.34,
            min_ram_gb: 8.0,
        },
    ]
}

/// Charge le catalogue (cache → fetch réseau) + modèles curés HF + GGUF présents localement.
pub fn load_catalog() -> Vec<ModelDef> {
    // 1. Catalogue distant (depuis cache ou réseau) + curés HF (Gemma 4…).
    let mut remote = load_remote_catalog();
    for c in curated_models() {
        if !remote.iter().any(|m| m.file == c.file) { remote.push(c); }
    }

    // 2. Modèles locaux déjà présents dans <data>/models/ (ex. Gemma téléchargé manuellement)
    let remote_files: std::collections::HashSet<&str> = remote.iter().map(|m| m.file.as_str()).collect();
    let mut local: Vec<ModelDef> = Vec::new();
    if let Some(models_dir) = app_data_dir().map(|d| d.join("models")) {
        if let Ok(entries) = std::fs::read_dir(&models_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("gguf") { continue; }
                // Ignorer les fichiers .part (téléchargements incomplets)
                if path.to_string_lossy().ends_with(".gguf.part") { continue; }
                let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if remote_files.contains(file.as_str()) { continue; } // déjà dans le catalogue
                let size_gb = path.metadata().map(|m| m.len() as f32 / 1_073_741_824.0).unwrap_or(4.0);
                let name = file.trim_end_matches(".gguf")
                    .replace(['-', '_', '.'], " ")
                    .split_whitespace()
                    .map(|w| { let mut c = w.chars(); c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default() })
                    .collect::<Vec<_>>().join(" ");
                let id = file.trim_end_matches(".gguf").to_lowercase().replace(['.', ' ', '_'], "-");
                local.push(ModelDef {
                    id, name, file,
                    url: String::new(), // local uniquement, pas de téléchargement
                    size_gb,
                    min_ram_gb: (size_gb * 1.4).ceil(),
                });
            }
        }
    }

    let mut all = remote;
    all.extend(local);
    all.sort_by(|a, b| a.size_gb.partial_cmp(&b.size_gb).unwrap_or(std::cmp::Ordering::Equal));
    all
}

fn load_remote_catalog() -> Vec<ModelDef> {
    if let Some(path) = catalog_cache_path() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                if v.get("version").and_then(|v| v.as_str()) == Some(CATALOG_VERSION) {
                    if let Ok(models) = serde_json::from_value::<Vec<ModelDef>>(v["models"].clone()) {
                        if !models.is_empty() { return models; }
                    }
                }
            }
        }
    }
    fetch_catalog()
}

// ── Sélection du modèle actif ─────────────────────────────────────────────────

fn total_ram_gb() -> f32 {
    #[cfg(target_os = "macos")]
    let output = Command::new("sysctl").args(["-n", "hw.memsize"]).output().ok();
    #[cfg(windows)]
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command",
               "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory"])
        .output().ok();
    #[cfg(not(any(target_os = "macos", windows)))]
    let output: Option<std::process::Output> = None; // Linux : fallback 8.0

    output
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|b| b as f32 / 1_073_741_824.0)
        .unwrap_or(8.0)
}

/// ID du modèle recommandé selon la RAM : le plus grand qui tient confortablement.
pub fn recommended_id(catalog: &[ModelDef]) -> Option<String> {
    let ram = total_ram_gb();
    catalog.iter()
        .filter(|m| m.min_ram_gb <= ram)
        .last()
        .map(|m| m.id.clone())
}

/// Lit le modèle actif depuis `model_config.json` (inclut l'URL complète).
pub fn active_model_stored() -> Option<ModelDef> {
    let raw = std::fs::read_to_string(config_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    serde_json::from_value(v["active_model"].clone()).ok()
}

/// Sélectionne un modèle par ID (cherche dans le catalogue, persiste l'URL).
pub fn select_model(id: &str) -> Result<(), String> {
    let catalog = load_catalog();
    let model = catalog.into_iter().find(|m| m.id == id)
        .ok_or_else(|| format!("Modèle « {id} » introuvable dans le catalogue."))?;
    let path = config_path().ok_or("Dossier de données introuvable.")?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::json!({ "active_model": model }).to_string())
        .map_err(|e| e.to_string())
}

// ── Résolution binaire / modèle ───────────────────────────────────────────────

fn resolve_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_LLAMA_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() { return Some(p); }
    }
    // App packagée : sidecar statique à côté de l'exécutable (Contents/MacOS/).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sidecar = dir.join(format!("llama-completion{}", std::env::consts::EXE_SUFFIX));
            if sidecar.is_file() { return Some(sidecar); }
        }
    }
    // Dev : checkout llama.cpp dans le dossier de données.
    let candidate = app_data_dir()?
        .join("llama.cpp").join("build").join("bin").join("llama-completion");
    candidate.is_file().then_some(candidate)
}

fn resolve_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_MODEL") {
        let p = PathBuf::from(p);
        if p.is_file() { return Some(p); }
    }
    let m = active_model_stored()?;
    let candidate = app_data_dir()?.join("models").join(&m.file);
    candidate.is_file().then_some(candidate)
}

pub fn model_needs_download() -> bool {
    resolve_model().is_none()
}

// ── Téléchargement depuis le CDN ──────────────────────────────────────────────

#[derive(serde::Serialize, Clone)]
pub struct DownloadProgress {
    pub downloaded_mb: f32,
    pub total_mb: f32,
    pub percent: u8,
}

/// Télécharge le modèle actif (URL depuis le catalogue en cache, CDN GPT4All).
pub fn download_model(app: &tauri::AppHandle) -> Result<(), String> {
    let m = active_model_stored()
        .ok_or("Aucun modèle sélectionné. Choisis un modèle d'abord.")?;
    let dest = app_data_dir().ok_or("Dossier de données introuvable.")?.join("models").join(&m.file);
    if dest.is_file() { return Ok(()); }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .user_agent("brainlink/0.1")
        .build()
        .map_err(|e| e.to_string())?;

    let mut resp = client.get(&m.url)
        .send()
        .map_err(|e| format!("Téléchargement échoué : {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Erreur {} pour {}", resp.status(), m.url));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded = 0u64;
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 65_536];

    loop {
        use std::io::{Read, Write};
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        let (dl_mb, tot_mb) = (downloaded as f32 / 1_048_576.0, total as f32 / 1_048_576.0);
        let percent = if total > 0 { (downloaded * 100 / total).min(100) as u8 } else { 0 };
        let _ = tauri::Emitter::emit(app, "download-progress", DownloadProgress {
            downloaded_mb: dl_mb, total_mb: tot_mb, percent,
        });
    }
    drop(file);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}

// ── Installation depuis un fichier local ──────────────────────────────────────

/// Copie un .gguf local vers `<data>/models/`. Fallback si téléchargement impossible.
pub fn install_from_path(app: &tauri::AppHandle, src: &std::path::Path) -> Result<(), String> {
    let m = active_model_stored().ok_or("Aucun modèle sélectionné.")?;
    let dest = app_data_dir().ok_or("Dossier de données introuvable.")?.join("models").join(&m.file);
    if dest.exists() { return Ok(()); }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;

    let total = src.metadata().map(|m| m.len()).unwrap_or(0);
    let mut reader = std::fs::File::open(src)
        .map_err(|e| format!("Impossible d'ouvrir le fichier : {e}"))?;
    let tmp = dest.with_extension("part");
    let mut writer = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut copied = 0u64;
    let mut buf = vec![0u8; 1_048_576];
    loop {
        use std::io::{Read, Write};
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        writer.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        copied += n as u64;
        let pct = if total > 0 { (copied * 100 / total).min(100) as u8 } else { 0 };
        let _ = tauri::Emitter::emit(app, "install-progress", pct);
    }
    drop(writer);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())
}

// ── Moteur d'inférence ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct LlamaEngine {
    binary: PathBuf,
    model: PathBuf,
}

impl LlamaEngine {
    pub fn detect() -> Result<Self, String> {
        let binary = resolve_binary()
            .ok_or("Binaire llama-completion introuvable. Lance le build de llama.cpp.")?;
        let model = resolve_model()
            .ok_or("Modèle introuvable. Télécharge-le depuis l'interface.")?;
        Ok(Self { binary, model })
    }

    pub fn is_ready() -> bool {
        resolve_binary().is_some() && resolve_model().is_some()
    }

    pub fn complete(&self, system: Option<&str>, user: &str, max_tokens: u32) -> Result<String, String> {
        // Embed the chat template directly dans le prompt selon la famille de modèle.
        // Sans ça, les modèles instruction-tuned (Llama 3, Mistral…) se comportent
        // comme des modèles de complétion brute et hallucinent librement.
        let filename = self.model.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let formatted = format_prompt(&filename, system, user);

        let mut cmd = Command::new(&self.binary);
        cmd.arg("-m").arg(&self.model)
            .args(["-ngl", "99"])
            .args(["-c", &CONTEXT_TOKENS.to_string()])
            .args(["-n", &max_tokens.to_string()])
            .args(["--temp", "0.2"])
            .args(["--top-p", "0.9"])
            .arg("-no-cnv")
            .arg("--no-display-prompt");
        cmd.arg("-p").arg(&formatted);

        let output = cmd.output()
            .map_err(|e| format!("Échec d'exécution de llama-completion : {e}"))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "llama-completion a échoué ({}) : {}",
                output.status,
                err.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
            ));
        }
        Ok(clean_output(&String::from_utf8_lossy(&output.stdout)))
    }
}

/// Applique le chat template correct selon la famille de modèle.
/// Chaque modèle instruction-tuned a ses tokens spéciaux — sans eux le modèle
/// se comporte en complétion brute et ignore les instructions système.
fn format_prompt(filename: &str, system: Option<&str>, user: &str) -> String {
    let sys = system.unwrap_or("");
    if filename.contains("llama-3") || filename.contains("llama3") {
        // Llama 3 / 3.1 / 3.2 (Meta)
        format!(
            "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\n\
{sys}<|eot_id|><|start_header_id|>user<|end_header_id|>\n\n\
{user}<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n"
        )
    } else if filename.contains("mistral") || filename.contains("mixtral") {
        // Mistral / Mixtral
        if sys.is_empty() {
            format!("[INST] {user} [/INST]")
        } else {
            format!("[INST] {sys}\n\n{user} [/INST]")
        }
    } else if filename.contains("gemma-4") || filename.contains("gemma4") {
        // Gemma 4 — tokens <|turn>/<turn|>, tour système natif. On OMET délibérément
        // le token <|think|> : sinon le modèle entre en mode raisonnement (chain-of-thought
        // dans un canal <|channel>thought), ce qui gaspille des tokens et pollue le JSON.
        // Pour l'extraction/synthèse on veut la réponse directe.
        format!("<|turn>system\n{sys}<turn|>\n<|turn>user\n{user}<turn|>\n<|turn>model\n")
    } else if filename.contains("gemma") {
        // Gemma 2 / 3
        format!(
            "<start_of_turn>user\n{sys}\n\n{user}<end_of_turn>\n<start_of_turn>model\n"
        )
    } else if filename.contains("phi-3") || filename.contains("phi3") {
        // Phi-3 / Phi-3.5
        format!("<|system|>\n{sys}<|end|>\n<|user|>\n{user}<|end|>\n<|assistant|>\n")
    } else if filename.contains("qwen") {
        // Qwen (ChatML)
        format!(
            "<|im_start|>system\n{sys}<|im_end|>\n\
<|im_start|>user\n{user}<|im_end|>\n\
<|im_start|>assistant\n"
        )
    } else {
        // Fallback : même comportement qu'avant (pas de template)
        if sys.is_empty() { user.to_string() } else { format!("{sys}\n\n{user}") }
    }
}

fn clean_output(s: &str) -> String {
    let mut out = s
        .replace("<end_of_turn>", "")
        .replace("<turn|>", "")   // Gemma 4 fin de tour
        .replace("<eos>", "")     // Gemma 4 fin de séquence
        .replace("<|im_end|>", "")
        .replace("<|end|>", "")
        .replace("[end of text]", "")
        .trim()
        .to_string();

    if let Some(rest) = out.strip_prefix("```") {
        let rest = rest.splitn(2, '\n').nth(1).unwrap_or(rest);
        out = rest.to_string();
        if let Some(idx) = out.rfind("```") { out.truncate(idx); }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::{clean_output, format_prompt};

    #[test]
    fn strips_json_fences() {
        assert_eq!(clean_output("```json\n{\"a\": 1}\n```\n [end of text]"), "{\"a\": 1}");
    }

    #[test]
    fn passes_plain_json() {
        assert_eq!(clean_output("  {\"a\":1}  "), "{\"a\":1}");
    }

    #[test]
    fn strips_gemma4_tokens() {
        assert_eq!(clean_output("réponse<turn|><eos>"), "réponse");
    }

    #[test]
    fn curated_has_downloadable_gemma4() {
        let c = super::curated_models();
        let g4 = c.iter().find(|m| m.file == "gemma-4-E4B-it-Q4_K_M.gguf").expect("Gemma 4 curé attendu");
        assert!(g4.url.starts_with("https://huggingface.co/") && g4.url.ends_with(".gguf"), "URL HF directe attendue");
        // id doit matcher l'id dérivé du nom de fichier (cohérence avec le scan local / select_model)
        let derived = g4.file.trim_end_matches(".gguf").to_lowercase().replace(['.', ' ', '_'], "-");
        assert_eq!(g4.id, derived, "id curé doit correspondre à l'id dérivé du fichier");
    }

    #[test]
    fn gemma4_uses_turn_template_not_gemma3() {
        // Un GGUF « gemma-4 » doit prendre le template <|turn>, PAS celui de Gemma 3.
        let p = format_prompt("gemma-4-e4b-it-q4_k_m.gguf", Some("sys"), "salut");
        assert!(p.contains("<|turn>system") && p.contains("<|turn>model"), "template Gemma 4 attendu");
        assert!(!p.contains("<start_of_turn>"), "ne doit pas utiliser le template Gemma 2/3");
        // Gemma 2/3 garde son template.
        let p3 = format_prompt("gemma-3-4b-it-q4_k_m.gguf", Some("sys"), "salut");
        assert!(p3.contains("<start_of_turn>user"), "Gemma 3 garde son template");
    }
}
