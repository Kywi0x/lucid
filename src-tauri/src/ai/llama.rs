//! Moteur d'inférence local : pilote le binaire `llama-completion` (llama.cpp).
//!
//! Les modèles disponibles sont fetchés depuis le catalogue GPT4All (GitHub) au
//! premier lancement, puis mis en cache localement. Les URLs viennent du catalogue
//! officiel → jamais de 404/401 par URL hardcodée.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const APP_DIR: &str = "com.lucidflow.lucid";
/// Ancien identifiant (avant le 2026-07-20) — un `rename` best-effort au premier
/// démarrage post-update rapatrie toutes les données (comptes, modèle IA ~2,3 Go)
/// sans qu'un user ait à réinstaller. Silencieux si absent ou déjà migré.
const LEGACY_APP_DIR: &str = "fr.ideeri.brainlink";
/// Taille de la fenêtre de contexte passée à llama (`-c`). Exposée à l'UI via
/// `ai_info` pour afficher la capacité et avertir si un prompt la dépasse.
/// 16384 (et non 8192) : l'Archiviste envoie des prompts de tri qui grandissent
/// avec le « Non triable » — 65 pages enrichies d'un extrait de contenu font
/// déjà ~8300 tokens (ids/CSV très denses, ~2,4 char/token mesuré), ce qui
/// dépassait 8192 → llama-server répondait 400 et le tri ne se faisait jamais
/// (bug remonté par Liam le 2026-07-28). 16k laisse voir tout le lot d'un coup
/// (meilleur clustering) ; Gemma 3/4 supporte bien plus, coût KV cache modéste.
pub const CONTEXT_TOKENS: u32 = 16384;
/// Température de l'assistant conversationnel. Les extractions structurées de
/// l'Archiviste passent par `complete_json` (température 0) — cf. son doc.
const CHAT_TEMPERATURE: f32 = 0.2;
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

/// Racine machine : assets partagés entre tous les comptes (llama.cpp, modèles,
/// catalogue). Ne jamais y écrire de données utilisateur.
pub fn shared_data_dir() -> Option<PathBuf> {
    let base = dirs::data_dir()?;
    migrate_legacy_dir_in(&base);
    Some(base.join(APP_DIR))
}

/// Rapatrie `LEGACY_APP_DIR` vers `APP_DIR` si ce dernier n'existe pas encore
/// (renommage de l'identifiant, 2026-07-20) — `rename` est quasi instantané
/// (même volume), contrairement à une copie du dossier (llama.cpp + modèles,
/// plusieurs Go). Best-effort : une erreur (permissions, cross-device) laisse
/// simplement l'app repartir à neuf plutôt que de planter.
fn migrate_legacy_dir_in(base: &std::path::Path) {
    let dir = base.join(APP_DIR);
    let legacy = base.join(LEGACY_APP_DIR);
    if !dir.exists() && legacy.exists() {
        let _ = std::fs::rename(&legacy, &dir);
    }
}

/// Dossier de données de l'utilisateur actif : `users/<uuid>/` si un compte est
/// connecté (fichier `active_user` écrit par la commande `set_active_user`),
/// racine sinon (pré-login / install legacy). Lu à chaque appel : l'app, le
/// binaire MCP et les examples CLI restent cohérents sans état partagé.
pub fn app_data_dir() -> Option<PathBuf> {
    let root = shared_data_dir()?;
    match std::fs::read_to_string(root.join("active_user")) {
        Ok(id) if !id.trim().is_empty() => Some(root.join("users").join(id.trim())),
        _ => Some(root),
    }
}

fn catalog_cache_path() -> Option<PathBuf> {
    Some(shared_data_dir()?.join("model_catalog.json"))
}

fn config_path() -> Option<PathBuf> {
    Some(shared_data_dir()?.join("model_config.json"))
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
        .user_agent("lucid/0.1")
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
    if let Some(models_dir) = shared_data_dir().map(|d| d.join("models")) {
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

/// `Command` sans fenêtre console sur Windows — sans ça, chaque spawn
/// (llama, pdftoppm, tesseract, powershell…) fait flasher un CMD à l'écran.
pub fn quiet_command(bin: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(bin);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

/// Comme `quiet_command`, mais en plus à priorité CPU réduite pour LE PROCESS
/// D'INFÉRENCE spécifiquement — un passage de l'Archiviste peut enchaîner des
/// dizaines d'appels Gemma d'affilée (arbitrage de doublons + clustering
/// sémantique), plusieurs minutes en tout ; sans ça, ce calcul intensif affame
/// les autres threads de l'app — dont le rendu du canvas, qui se met à ramer/
/// geler pendant que "L'archiviste travaille…" (retour de Liam le 2026-07-23).
/// `nice` (POSIX, toujours présent) sur Mac/Linux. Sur Windows, l'équivalent
/// zéro-dépendance : un *priority class flag* dans `creation_flags` (le même
/// canal que `CREATE_NO_WINDOW`) — `BELOW_NORMAL_PRIORITY_CLASS` fait que
/// l'ordonnanceur cède le CPU aux apps de premier plan de l'user pendant qu'une
/// passe de l'Archiviste tourne (retour Liam 2026-07-30 : sur Windows la machine
/// ralentissait ses autres tâches — la branche Windows était un no-op, elle ne
/// baissait rien). Parité tenue : plus jamais "pire qu'avant".
fn low_priority_command(bin: impl AsRef<std::ffi::OsStr>) -> Command {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
        let mut cmd = Command::new(bin);
        cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        cmd
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut cmd = Command::new("nice");
        cmd.arg("-n").arg("15").arg(bin);
        cmd
    }
}

/// Nombre de threads passés à `llama-completion` (`-t`) : tous les cœurs sauf 2,
/// jamais moins de 1 — sans ça le binaire prend par défaut TOUS les cœurs et
/// n'en laisse aucun de libre pour le rendu du canvas (BrainMap) pendant qu'une
/// passe de l'Archiviste enchaîne des dizaines d'appels Gemma d'affilée. Vient
/// compléter `low_priority_command` (nice) : nice abaisse la priorité, ça ne
/// réduit pas le nombre de cœurs occupés.
fn worker_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).saturating_sub(2).max(1)
}

fn total_ram_gb() -> f32 {
    // La RAM ne change pas en cours de route : une seule lecture (le spawn
    // powershell sous Windows coûte ~1-2 s, inacceptable à chaque list_models).
    static RAM: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *RAM.get_or_init(|| {
        #[cfg(target_os = "macos")]
        let output = quiet_command("sysctl").args(["-n", "hw.memsize"]).output().ok();
        #[cfg(windows)]
        let output = quiet_command("powershell")
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
    })
}

/// Réserve RAM avant de choisir le modèle de génération : le serveur de
/// génération ET le serveur d'embedding (BGE-M3) tournent EN MÊME TEMPS, plus
/// l'app/WebView2. Sans cette marge, sur une machine 8 Go la RAM saturait à 98 %
/// (test Windows de Liam, 2026-07-30). ~2,5 Go = BGE-M3 chargé (~1 Go de working
/// set) + overhead app.
const COMPANION_RESERVE_GB: f32 = 2.5;

/// Modèles de génération à tenter, du plus performant (le plus gros qui tient en
/// RAM après réserve) au moins performant — chaîne de secours du bootstrap : si
/// un téléchargement échoue, on descend au suivant. Toujours au moins un élément
/// (le plus petit du catalogue) même si rien ne tient confortablement — mieux
/// vaut tenter le plus léger que ne rien proposer.
pub fn recommended_chain(catalog: &[ModelDef]) -> Vec<String> {
    let budget = total_ram_gb() - COMPANION_RESERVE_GB;
    let mut fit: Vec<&ModelDef> = catalog.iter().filter(|m| m.min_ram_gb <= budget).collect();
    // Plus gros d'abord (proxy de qualité), départage stable par id.
    fit.sort_by(|a, b| b.size_gb.partial_cmp(&a.size_gb).unwrap_or(std::cmp::Ordering::Equal).then(a.id.cmp(&b.id)));
    let mut ids: Vec<String> = fit.iter().map(|m| m.id.clone()).collect();
    if ids.is_empty() {
        if let Some(m) = catalog.first() { ids.push(m.id.clone()); } // rien ne tient → le plus petit
    }
    ids
}

/// ID du modèle recommandé : la tête de la chaîne (le plus performant qui tient).
pub fn recommended_id(catalog: &[ModelDef]) -> Option<String> {
    recommended_chain(catalog).into_iter().next()
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
    let candidate = shared_data_dir()?
        .join("llama.cpp").join("build").join("bin").join("llama-completion");
    candidate.is_file().then_some(candidate)
}

fn resolve_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_MODEL") {
        let p = PathBuf::from(p);
        if p.is_file() { return Some(p); }
    }
    let m = active_model_stored()?;
    let candidate = shared_data_dir()?.join("models").join(&m.file);
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

/// Télécharge une URL vers `dest` en émettant "download-progress" (idempotent :
/// no-op si le fichier est déjà là). Core partagé par le modèle de génération et
/// le modèle d'embedding — même boucle, même barre de progression côté UI.
fn download_file(app: &tauri::AppHandle, url: &str, dest: &Path) -> Result<(), String> {
    if dest.is_file() { return Ok(()); }
    std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| e.to_string())?;

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .user_agent("lucid/0.1")
        .build()
        .map_err(|e| e.to_string())?;

    let mut resp = client.get(url)
        .send()
        .map_err(|e| format!("Téléchargement échoué : {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Erreur {} pour {url}", resp.status()));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut downloaded = 0u64;
    let tmp = dest.with_extension("part");
    let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1_048_576];
    // Émettre un event IPC par chunk (~37 000 pour 2,3 Go) étranglait la boucle,
    // surtout sur Windows/WebView2 : on n'émet qu'au changement de pourcent.
    let mut last_percent: u8 = u8::MAX;

    loop {
        use std::io::{Read, Write};
        let n = resp.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        downloaded += n as u64;
        let percent = if total > 0 { (downloaded * 100 / total).min(100) as u8 } else { 0 };
        if percent != last_percent {
            last_percent = percent;
            let (dl_mb, tot_mb) = (downloaded as f32 / 1_048_576.0, total as f32 / 1_048_576.0);
            let _ = tauri::Emitter::emit(app, "download-progress", DownloadProgress {
                downloaded_mb: dl_mb, total_mb: tot_mb, percent,
            });
        }
    }
    drop(file);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

/// Télécharge le modèle actif (URL depuis le catalogue en cache, CDN GPT4All).
pub fn download_model(app: &tauri::AppHandle) -> Result<(), String> {
    let m = active_model_stored()
        .ok_or("Aucun modèle sélectionné. Choisis un modèle d'abord.")?;
    let dest = shared_data_dir().ok_or("Dossier de données introuvable.")?.join("models").join(&m.file);
    download_file(app, &m.url, &dest)
}

/// Télécharge le modèle d'embedding BGE-M3 (HuggingFace). Jamais proposé au
/// choix : un seul modèle d'embedding, tiré automatiquement au bootstrap.
pub fn download_embed_model(app: &tauri::AppHandle) -> Result<(), String> {
    let dest = shared_data_dir().ok_or("Dossier de données introuvable.")?.join("models").join(EMBED_MODEL_FILE);
    download_file(app, EMBED_MODEL_URL, &dest)
}

// ── Installation depuis un fichier local ──────────────────────────────────────

/// Copie un .gguf local vers `<data>/models/`. Fallback si téléchargement impossible.
pub fn install_from_path(app: &tauri::AppHandle, src: &std::path::Path) -> Result<(), String> {
    let m = active_model_stored().ok_or("Aucun modèle sélectionné.")?;
    let dest = shared_data_dir().ok_or("Dossier de données introuvable.")?.join("models").join(&m.file);
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

// ── Serveur d'inférence persistant ────────────────────────────────────────────
//
// Chaque appel `complete()` lançait jusqu'ici un process `llama-completion`
// neuf, qui recharge le modèle entier (~2-4 Go, upload GPU Metal compris) à
// chaque fois. Sans conséquence pour un appel isolé (chat), mais l'Archiviste
// enchaîne des dizaines/centaines d'appels d'affilée : autant de rechargements
// complets redondants, à l'origine du rythme "pause/freeze" observé côté
// canvas ET de la surchauffe remontée par Liam le 2026-07-24 (M4 Pro : `-ngl
// 99` décharge sur le GPU Metal, partagé avec le rendu du canvas — `nice`/`-t`
// ne changent rien à CETTE contention-là). `llama-server` (mode HTTP du même
// llama.cpp) charge le modèle UNE fois et le garde en mémoire : on le démarre
// paresseusement au premier appel, on le réutilise pour tous les suivants.
//
// Dégradation : si `llama-server` n'est pas présent à côté de
// `llama-completion` (pas encore bundlé/buildé) ou s'il échoue à démarrer,
// `complete()` retombe silencieusement sur l'ancien mode one-shot — jamais un
// échec dur, cf. [[feedback_parite_windows]] (jamais d'échec silencieux total,
// juste moins performant qu'avec le serveur).
const SERVER_PORT: u16 = 8721; // ponytail: port fixe, un seul serveur pour toute l'app — pas de découverte dynamique tant qu'un seul modèle actif à la fois suffit.
const SERVER_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
/// Fichiers stderr des serveurs (dossier machine) — un crash au démarrage
/// (SIGABRT, flag/modèle incompatible, Metal…) partait dans /dev/null et était
/// TOTALEMENT invisible ; on le capture pour le Diagnostic (retour bêta 2026-07-30).
pub const GEN_SERVER_LOG: &str = "llama-server.log";
pub const EMBED_SERVER_LOG: &str = "llama-embed.log";

/// stderr d'un llama-server → `<data>/<name>` (tronqué à chaque démarrage), au
/// lieu de /dev/null. Retombe sur null si le fichier n'est pas créable.
fn server_stderr(name: &str) -> std::process::Stdio {
    shared_data_dir()
        .and_then(|d| std::fs::File::create(d.join(name)).ok())
        .map(std::process::Stdio::from)
        .unwrap_or_else(std::process::Stdio::null)
}

struct ServerProc {
    child: std::process::Child,
    model: PathBuf,
    token: String,
}

/// Clé d'API aléatoire, une par lancement de serveur — sans ça, `llama-server`
/// tourne CORS ouvert ("allow all origins") sans authentification (avertissement
/// constaté au smoke test manuel du 2026-07-24) : n'importe quelle page web
/// ouverte dans le navigateur pourrait alors requêter le port loopback pendant
/// qu'il tourne. Le port reste local (`--host 127.0.0.1`), mais la clé ferme ce
/// cran d'attaque restant.
fn random_token() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("RNG OS indisponible");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Drop for ServerProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static SERVER: OnceLock<Mutex<Option<ServerProc>>> = OnceLock::new();

fn server_slot() -> &'static Mutex<Option<ServerProc>> {
    SERVER.get_or_init(|| Mutex::new(None))
}

/// `llama-server` à côté du binaire `llama-completion` déjà résolu (mêmes 3
/// emplacements possibles : override env, sidecar packagé, build dev) — pas
/// besoin de dupliquer `resolve_binary`, juste chercher son voisin.
fn resolve_server_binary() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("SECOND_BRAIN_LLAMA_SERVER_BIN") {
        let p = PathBuf::from(p);
        if p.is_file() { return Some(p); }
    }
    // Sidecar à côté de `llama-completion` (même dossier). Bundlé sur les DEUX
    // plateformes : `tauri.macos.conf.json` / `tauri.windows.conf.json` le
    // déclarent en `externalBin`, et `scripts/bundle-sidecars.{sh,ps1}` le tirent
    // de la release officielle llama.cpp. Tauri copiant les sidecars dans
    // `target/<profil>/`, ce chemin est aussi celui utilisé en dev.
    if let Some(completion) = resolve_binary() {
        let candidate = completion.with_file_name(format!("llama-server{}", std::env::consts::EXE_SUFFIX));
        if candidate.is_file() { return Some(candidate); }
    }
    // Dev : release officielle isolée dans son propre dossier (jamais mélangée
    // à `build/`/`build-static/`, où vivent les binaires custom déjà utilisés
    // par `llama-completion` — deux versions de dylibs de même nom dans un
    // seul dossier écraseraient/casseraient l'existant).
    let candidate = shared_data_dir()?.join("llama.cpp").join("server-release").join("llama-server");
    candidate.is_file().then_some(candidate)
}

fn server_health(token: &str) -> bool {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{SERVER_PORT}/health"))
        .bearer_auth(token)
        .timeout(Duration::from_millis(500))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Libère le port fixe avant de lancer NOTRE serveur — bug réel trouvé le
/// 2026-07-25 : `/health` répond 200 avec n'importe quel token (même faux),
/// donc un `llama-server` orphelin d'une session précédente (dev tuée de
/// force, cf. le geste `lsof -ti:1420` déjà documenté pour le port front) reste
/// vivant sur ce port et passe pour "sain" — l'app envoie alors ses vraies
/// requêtes `/completion` avec SON token à elle, rejetées en 401 par
/// l'orphelin qui n'accepte que le sien : fallback one-shot silencieux à
/// CHAQUE appel, donc plus aucun bénéfice du serveur persistant (le vrai
/// freeze observé, pas un problème machine). Un seul process légitime sur ce
/// port par design ("un seul serveur pour toute l'app") : on peut le tuer sans
/// risque avant de prendre sa place. Best-effort, jamais bloquant.
#[cfg(not(windows))]
fn free_port(port: u16) {
    let Ok(out) = std::process::Command::new("lsof").args(["-ti", &format!(":{port}")]).output() else { return };
    for pid in String::from_utf8_lossy(&out.stdout).split_whitespace() {
        let _ = std::process::Command::new("kill").args(["-9", pid]).status();
    }
}

/// Équivalent Windows (parité ADR-0015 — la version `lsof`/`kill` ne libérait
/// rien du tout sur Windows, où le problème est POURTANT pire : les process
/// enfants ne meurent pas avec leur parent, donc un `llama-server` orphelin
/// squatte le port à coup sûr après une fermeture brutale, et notre serveur
/// n'arrive plus à s'y lier → repli one-shot silencieux à chaque appel).
///
/// `netstat -ano` plutôt que `taskkill /IM llama-server.exe` : les deux serveurs
/// (génération et embedding) portent le MÊME nom d'image, tuer par nom
/// descendrait celui qu'on vient de démarrer. On cible donc par port.
#[cfg(windows)]
fn free_port(port: u16) {
    let me = std::process::id().to_string();
    let Ok(out) = std::process::Command::new("netstat").args(["-ano", "-p", "tcp"]).output() else { return };
    let needle = format!(":{port}");
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 5 { continue; }
        // cols[1] = adresse LOCALE. Nos propres connexions client vers ce port ont
        // une adresse locale éphémère : elles ne matchent pas, on ne se suicide
        // donc jamais (le garde `pid == me` reste par sécurité). L'état
        // (« LISTENING ») n'est pas testé : sa traduction dépend de la locale.
        if !cols[1].ends_with(&needle) { continue; }
        let pid = cols[cols.len() - 1];
        if pid == me || pid.parse::<u32>().is_err() { continue; }
        let _ = std::process::Command::new("taskkill").args(["/F", "/PID", pid]).status();
    }
}

/// Démarre (ou réutilise) le serveur pour CE modèle précis. `None` si le
/// binaire est absent ou si le démarrage échoue — l'appelant retombe alors sur
/// le mode one-shot. Renvoie la clé d'API à utiliser pour les requêtes.
fn ensure_server(binary: &Path, model: &Path) -> Option<String> {
    let mut slot = server_slot().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = slot.as_ref() {
        if s.model == model && server_health(&s.token) {
            return Some(s.token.clone());
        }
        // Modèle changé (bascule utilisateur) ou process mort : on repart propre.
        // ponytail: log minimal pour diagnostiquer un cycle "pause/reprend" pendant
        // une passe Archiviste — distingue un vrai crash serveur (respawn coûteux,
        // ~3-4s de rechargement modèle) d'une contention GPU/canvas sans redémarrage.
        if s.model != model {
            crate::elog!("ℹ️ llama-server : changement de modèle, redémarrage.");
        } else {
            crate::elog!("⚠️ llama-server : health check en échec, process relancé (contention GPU/mémoire ou crash).");
        }
        *slot = None;
    }
    free_port(SERVER_PORT);
    let token = random_token();
    let mut cmd = low_priority_command(binary);
    cmd.arg("-m").arg(model)
        .args(["-ngl", "99"])
        .args(["-t", &worker_threads().to_string()])
        .args(["-c", &CONTEXT_TOKENS.to_string()])
        .args(["--host", "127.0.0.1"]) // jamais accessible depuis le réseau, explicite plutôt que compter sur le défaut
        .args(["--port", &SERVER_PORT.to_string()])
        .args(["--api-key", &token]) // sans ça : CORS ouvert + pas d'auth (avertissement llama-server)
        .stdout(std::process::Stdio::null())
        .stderr(server_stderr(GEN_SERVER_LOG));
    let child = cmd.spawn().ok()?;
    let mut proc = ServerProc { child, model: model.to_path_buf(), token: token.clone() };
    let started = Instant::now();
    while started.elapsed() < SERVER_STARTUP_TIMEOUT {
        if server_health(&token) {
            *slot = Some(proc);
            crate::elog!("✅ llama-server prêt en {:.1}s.", started.elapsed().as_secs_f32());
            return Some(token);
        }
        if let Ok(Some(status)) = proc.child.try_wait() {
            crate::elog!("⚠️ llama-server mort avant de répondre ({status}).");
            return None; // mort avant même de répondre (modèle/flags invalides)
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    crate::elog!("⚠️ llama-server : timeout de démarrage ({:?}).", SERVER_STARTUP_TIMEOUT);
    None // `proc` droppé ici → Drop tue le process qui n'a jamais répondu
}

/// Arrête le serveur persistant s'il tourne — appelé à la fermeture de l'app
/// (sinon un process GPU-actif traînerait après avoir quitté Lucid).
pub fn shutdown_server() {
    *server_slot().lock().unwrap_or_else(|p| p.into_inner()) = None;
    *embed_server_slot().lock().unwrap_or_else(|p| p.into_inner()) = None;
}

// ── Embeddings (rangement déterministe, ADR-0019) ────────────────────────────
// Un 2ᵉ `llama-server` persistant, en mode `--embedding`, avec un modèle DÉDIÉ
// (BGE-M3) sur un port distinct du serveur de génération (Gemma). Il ne génère
// pas de texte : il rend un VECTEUR par document (position sur la « carte du
// sens »). Déterministe → pas d'hallucination. Sert le clustering de l'Archiviste.

/// URL du modèle d'embedding par défaut (BGE-M3, multilingue FR/EN, ctx 8192).
/// Quantifié Q8 (~600 Mo) — priorité qualité (ADR-0019). Overridable par env.
pub const EMBED_MODEL_URL: &str =
    "https://huggingface.co/gpustack/bge-m3-GGUF/resolve/main/bge-m3-Q8_0.gguf";
pub const EMBED_MODEL_FILE: &str = "bge-m3-Q8_0.gguf";
const EMBED_PORT: u16 = 8722;
/// Un embedding ne « voit » que sa fenêtre — on tronque le texte (un CSV peut
/// faire des Mo) SOUS les 8192 tokens du batch. À ~2,4 char/token, 8192 tokens
/// ≈ 19 600 chars ; on prend 15 000 (marge de sécurité) — largement assez pour
/// capter le sujet d'un document.
const EMBED_MAX_CHARS: usize = 15_000;

static EMBED_SERVER: OnceLock<Mutex<Option<ServerProc>>> = OnceLock::new();
fn embed_server_slot() -> &'static Mutex<Option<ServerProc>> {
    EMBED_SERVER.get_or_init(|| Mutex::new(None))
}

/// Health check générique (le serveur de génération utilise `server_health`,
/// figé sur son port — celui-ci prend le port en paramètre pour le 2ᵉ serveur).
fn health_on(port: u16, token: &str) -> bool {
    reqwest::blocking::Client::new()
        .get(format!("http://127.0.0.1:{port}/health"))
        .bearer_auth(token)
        .timeout(Duration::from_millis(500))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Chemin du modèle d'embedding : override env `LUCID_EMBED_MODEL`, sinon le
/// fichier attendu dans `models/`. `None` si absent (→ à télécharger).
pub fn resolve_embed_model() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LUCID_EMBED_MODEL") {
        let p = PathBuf::from(p);
        if p.is_file() { return Some(p); }
    }
    let candidate = shared_data_dir()?.join("models").join(EMBED_MODEL_FILE);
    candidate.is_file().then_some(candidate)
}

pub fn embed_model_available() -> bool { resolve_embed_model().is_some() }

// ── Statut de la stack (pour le panneau Diagnostic, D) ──────────────────────
/// Binaire de génération (`llama-completion`) résolu ?
pub fn completion_binary_available() -> bool { resolve_binary().is_some() }
/// Binaire serveur (`llama-server`) résolu ? Requis pour serveur persistant + embeddings.
pub fn server_binary_available() -> bool { resolve_server_binary().is_some() }
/// Modèle de génération actif présent sur le disque ?
pub fn generation_model_available() -> bool { resolve_model().is_some() }
/// RAM totale détectée (Go) — exposée pour le diagnostic.
pub fn detected_ram_gb() -> f32 { total_ram_gb() }

/// Démarre (ou réutilise) le serveur d'embedding. `None` si binaire ou modèle
/// absent, ou démarrage échoué. Renvoie la clé d'API.
fn ensure_embed_server() -> Option<String> {
    let binary = resolve_server_binary()?;
    let model = resolve_embed_model()?;
    let mut slot = embed_server_slot().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = slot.as_ref() {
        if s.model == model && health_on(EMBED_PORT, &s.token) {
            return Some(s.token.clone());
        }
        *slot = None;
    }
    free_port(EMBED_PORT);
    let token = random_token();
    let mut cmd = low_priority_command(&binary);
    cmd.arg("-m").arg(&model)
        .args(["-ngl", "99"])
        .args(["-t", &worker_threads().to_string()])
        .args(["-c", "8192"])
        // En mode embedding, tout l'input est traité en UN batch : sans relever
        // `-b`/`-ub` (défaut 512), tout document > 512 tokens échoue en 500
        // ("input too large to process", trouvé le 2026-07-29). On aligne le
        // batch sur le contexte pour accepter des docs jusqu'à 8192 tokens.
        .args(["-b", "8192"])
        .args(["-ub", "8192"])
        .arg("--embedding")
        .args(["--pooling", "mean"])
        .args(["--embd-normalize", "2"]) // L2 → cosinus = simple produit scalaire
        .args(["--host", "127.0.0.1"])
        .args(["--port", &EMBED_PORT.to_string()])
        .args(["--api-key", &token])
        .stdout(std::process::Stdio::null())
        .stderr(server_stderr(EMBED_SERVER_LOG));
    let mut proc = cmd.spawn().ok().map(|child| ServerProc { child, model: model.clone(), token: token.clone() })?;
    let started = Instant::now();
    while started.elapsed() < SERVER_STARTUP_TIMEOUT {
        if health_on(EMBED_PORT, &token) {
            *slot = Some(proc);
            crate::elog!("✅ llama-server (embedding) prêt en {:.1}s.", started.elapsed().as_secs_f32());
            return Some(token);
        }
        if let Ok(Some(status)) = proc.child.try_wait() {
            crate::elog!("⚠️ llama-server (embedding) mort avant de répondre ({status}).");
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    crate::elog!("⚠️ llama-server (embedding) : timeout de démarrage.");
    None
}

/// Vecteur d'embedding d'UN texte via l'endpoint OpenAI-compat `/v1/embeddings`
/// (format de réponse stable entre versions de llama.cpp). Texte tronqué à la
/// fenêtre du modèle.
fn embed_one(token: &str, text: &str) -> Result<Vec<f32>, String> {
    // Retry adaptatif : le nombre de tokens par caractère varie énormément (un
    // CSV dense fait ~1 token/char, un texte FR ~0,4). Impossible de deviner une
    // troncature sûre en chars. On part de EMBED_MAX_CHARS et, si le serveur
    // répond « input too large », on retronque de moitié jusqu'à passer — un
    // embedding n'a de toute façon besoin que du début pour capter le sujet.
    let client = reqwest::blocking::Client::new();
    let mut limit = EMBED_MAX_CHARS;
    loop {
        let truncated: String = text.chars().take(limit).collect();
        let body = serde_json::json!({ "input": truncated });
        let resp = client
            .post(format!("http://127.0.0.1:{EMBED_PORT}/v1/embeddings"))
            .bearer_auth(token)
            .json(&body)
            .timeout(Duration::from_secs(120))
            .send()
            .map_err(|e| format!("serveur d'embedding injoignable : {e}"))?;
        let status = resp.status();
        if status.is_success() {
            let json: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
            let arr = json.get("data")
                .and_then(|d| d.as_array())
                .and_then(|d| d.first())
                .and_then(|e| e.get("embedding"))
                .and_then(|v| v.as_array())
                .ok_or("réponse d'embedding inattendue")?;
            return Ok(arr.iter().filter_map(|v| v.as_f64().map(|f| f as f32)).collect());
        }
        // Input trop grand pour le batch → retronque et réessaie.
        if status.as_u16() == 500 && limit > 1000 {
            limit /= 2;
            continue;
        }
        return Err(format!("serveur d'embedding a répondu {status}"));
    }
}

/// Requêtes d'embedding en vol simultanément. `llama-server` sert plusieurs
/// slots en parallèle (4 visibles dans `llama-embed.log`) : les envoyer une par
/// une en attendant chaque réponse laissait les autres slots inoccupés pendant
/// tout l'indexage initial (231 documents en série, ~3 min — le run à froid que
/// Liam rejoue à chaque test, 2026-07-31). On ne change NI le modèle NI le
/// texte envoyé : les vecteurs sont identiques, seul l'ordonnancement change.
const EMBED_PARALLEL: usize = 4;

/// Embeddings d'une liste de textes, par vagues de `EMBED_PARALLEL` requêtes
/// concurrentes. L'ordre de sortie suit l'ordre d'entrée (l'appelant associe par
/// index). Renvoie une erreur claire si le moteur n'est pas prêt.
pub fn embed_texts(texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() { return Ok(Vec::new()); }
    let token = ensure_embed_server()
        .ok_or("Moteur d'embedding indisponible (modèle BGE-M3 absent ou serveur non démarré).")?;
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(EMBED_PARALLEL) {
        let results: Vec<Result<Vec<f32>, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk.iter()
                .map(|t| s.spawn(|| embed_one(&token, t)))
                .collect();
            handles.into_iter()
                .map(|h| h.join().unwrap_or_else(|_| Err("thread d'embedding interrompu".into())))
                .collect()
        });
        // `?` après la vague, pas pendant : les threads sont déjà joints, on ne
        // laisse jamais une requête en vol derrière soi.
        for r in results { out.push(r?); }
    }
    Ok(out)
}

/// Similarité cosinus entre deux vecteurs (∈ [-1, 1] ; ~1 = même sens). Les
/// vecteurs sont déjà L2-normalisés par le serveur (`--embd-normalize 2`), donc
/// c'est un simple produit scalaire — mais on renormalise par sécurité.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() { return 0.0; }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 { return 0.0; }
    dot / (na.sqrt() * nb.sqrt())
}

fn complete_via_server(token: &str, prompt: &str, max_tokens: u32, temperature: f32) -> Result<String, String> {
    let body = serde_json::json!({
        "prompt": prompt,
        "n_predict": max_tokens,
        "temperature": temperature,
        "top_p": 0.9,
        "cache_prompt": true,
    });
    let resp = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{SERVER_PORT}/completion"))
        .bearer_auth(token)
        .json(&body)
        .timeout(Duration::from_secs(180))
        .send()
        .map_err(|e| format!("llama-server injoignable : {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("llama-server a répondu {}", resp.status()));
    }
    let json: serde_json::Value = resp.json().map_err(|e| e.to_string())?;
    let content = json.get("content").and_then(|v| v.as_str()).unwrap_or("");
    Ok(clean_output(content))
}

// ── Moteur d'inférence ────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct LlamaEngine {
    binary: PathBuf,
    model: PathBuf,
}

/// Appels de génération soldés par une erreur — serveur ET repli one-shot en
/// échec, donc un vrai appel perdu (un échec du seul serveur, rattrapé par le
/// one-shot, ne compte pas). Compteur de process, jamais remis à zéro : les
/// lecteurs comparent deux relevés (cf. le rapport de l'Archiviste). Sans lui,
/// un échec passager ne laissait qu'une ligne de log invisible alors qu'il
/// creuse des trous dans le classement.
static FAILED_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Appels de génération TOTAUX (réussis + échoués) — le coût réel d'une passe, la
/// génération étant le goulot d'étranglement. Même usage que `FAILED_CALLS` :
/// cumulatif, les lecteurs comparent deux relevés.
static TOTAL_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Nombre cumulé d'appels de génération en échec depuis le lancement.
pub fn failed_calls() -> usize {
    FAILED_CALLS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Nombre cumulé d'appels de génération depuis le lancement.
pub fn total_calls() -> usize {
    TOTAL_CALLS.load(std::sync::atomic::Ordering::Relaxed)
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

    /// Génération conversationnelle (assistant) — un peu d'échantillonnage rend
    /// les réponses moins mécaniques.
    pub fn complete(&self, system: Option<&str>, user: &str, max_tokens: u32) -> Result<String, String> {
        self.complete_at(system, user, max_tokens, CHAT_TEMPERATURE)
    }

    /// Extraction STRUCTURÉE (JSON) — température 0, donc même entrée, même sortie.
    ///
    /// À 0,2, deux passes à froid sur les MÊMES fichiers donnaient des étiquettes
    /// de domaine différentes : 19 documents avaient changé de domaine entre deux
    /// runs, « Études & Cours » passant de 19 à 3 (mesuré le 2026-08-03). Or le
    /// domaine est la clé de regroupement du clustering : un paquet qui gagne 8
    /// documents produit d'autres clusters. Conséquence, aucun changement de
    /// réglage n'était mesurable — l'écart entre deux runs identiques dépassait
    /// l'effet cherché. Même cause pour les noms de dossiers qui changeaient d'une
    /// passe à l'autre.
    ///
    /// Ne rend pas la passe bit-à-bit reproductible : le serveur traite plusieurs
    /// embeddings dans un même lot et les sommes flottantes dépendent de sa
    /// composition. Mais ça supprime la source dominante.
    pub fn complete_json(&self, system: Option<&str>, user: &str, max_tokens: u32) -> Result<String, String> {
        self.complete_at(system, user, max_tokens, 0.0)
    }

    /// Point de passage unique de TOUS les appels de génération de l'app — d'où
    /// le comptage des échecs ici plutôt que sur chaque site appelant.
    fn complete_at(&self, system: Option<&str>, user: &str, max_tokens: u32, temperature: f32) -> Result<String, String> {
        TOTAL_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let out = self.complete_inner(system, user, max_tokens, temperature);
        if out.is_err() {
            FAILED_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        out
    }

    fn complete_inner(&self, system: Option<&str>, user: &str, max_tokens: u32, temperature: f32) -> Result<String, String> {
        // Embed the chat template directly dans le prompt selon la famille de modèle.
        // Sans ça, les modèles instruction-tuned (Llama 3, Mistral…) se comportent
        // comme des modèles de complétion brute et hallucinent librement.
        let filename = self.model.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let formatted = format_prompt(&filename, system, user);

        if let Some(server_bin) = resolve_server_binary() {
            if let Some(token) = ensure_server(&server_bin, &self.model) {
                match complete_via_server(&token, &formatted, max_tokens, temperature) {
                    Ok(out) => return Ok(out),
                    Err(e) => crate::elog!("⚠️ llama-server en échec ({e}), retour au mode one-shot."),
                }
            }
        }

        let mut cmd = low_priority_command(&self.binary);
        cmd.arg("-m").arg(&self.model)
            .args(["-ngl", "99"])
            .args(["-t", &worker_threads().to_string()])
            .args(["-c", &CONTEXT_TOKENS.to_string()])
            .args(["-n", &max_tokens.to_string()])
            .args(["--temp", &temperature.to_string()])
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
    use super::{clean_output, format_prompt, migrate_legacy_dir_in, APP_DIR, LEGACY_APP_DIR};

    #[test]
    fn migre_le_dossier_legacy_si_le_nouveau_est_absent() {
        let base = std::env::temp_dir().join("brainlink_test_migrate_legacy");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(LEGACY_APP_DIR)).unwrap();
        std::fs::write(base.join(LEGACY_APP_DIR).join("marker.txt"), "présent").unwrap();

        migrate_legacy_dir_in(&base);

        assert!(!base.join(LEGACY_APP_DIR).exists(), "l'ancien dossier a été renommé");
        assert_eq!(std::fs::read_to_string(base.join(APP_DIR).join("marker.txt")).unwrap(), "présent");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ne_touche_pas_a_un_nouveau_dossier_deja_present() {
        let base = std::env::temp_dir().join("brainlink_test_migrate_noop");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join(LEGACY_APP_DIR)).unwrap();
        std::fs::write(base.join(LEGACY_APP_DIR).join("marker.txt"), "legacy").unwrap();
        std::fs::create_dir_all(base.join(APP_DIR)).unwrap();
        std::fs::write(base.join(APP_DIR).join("marker.txt"), "déjà migré").unwrap();

        migrate_legacy_dir_in(&base);

        assert!(base.join(LEGACY_APP_DIR).exists(), "le legacy n'est pas touché s'il existe déjà côté nouveau");
        assert_eq!(std::fs::read_to_string(base.join(APP_DIR).join("marker.txt")).unwrap(), "déjà migré");
        let _ = std::fs::remove_dir_all(&base);
    }

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
