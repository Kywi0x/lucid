use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

// ─── Credentials Google (stockées dans app data, jamais committées) ─────────

#[derive(Serialize, Deserialize)]
struct Credentials {
    client_id: String,
    client_secret: String,
}

fn credentials_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("google_credentials.json"))
}

fn load_credentials() -> Option<Credentials> {
    std::fs::read_to_string(credentials_path()?)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
}

pub fn save_creds(client_id: &str, client_secret: &str) -> Result<(), String> {
    let path = credentials_path().ok_or("Dossier de données introuvable.")?;
    let c = Credentials { client_id: client_id.to_string(), client_secret: client_secret.to_string() };
    std::fs::write(path, serde_json::to_string(&c).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

pub fn has_credentials() -> bool {
    load_credentials().is_some()
        || (!std::env::var("GOOGLE_CLIENT_ID").unwrap_or_default().is_empty()
            && !std::env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default().is_empty())
}

// Priorité : baké au build (option_env!) → env/.env runtime → fichier user (legacy).
// L'env passe AVANT le fichier pour qu'un `.env` frais l'emporte sur un
// google_credentials.json obsolète (ex. après un reset du secret côté Google).
fn google_client_id() -> String {
    if let Some(id) = option_env!("GOOGLE_CLIENT_ID") {
        if !id.is_empty() { return id.to_string(); }
    }
    std::env::var("GOOGLE_CLIENT_ID").ok().filter(|s| !s.is_empty())
        .or_else(|| load_credentials().map(|c| c.client_id))
        .unwrap_or_default()
}

fn google_client_secret() -> String {
    if let Some(s) = option_env!("GOOGLE_CLIENT_SECRET") {
        if !s.is_empty() { return s.to_string(); }
    }
    std::env::var("GOOGLE_CLIENT_SECRET").ok().filter(|s| !s.is_empty())
        .or_else(|| load_credentials().map(|c| c.client_secret))
        .unwrap_or_default()
}

// ─── PKCE (RFC 7636) ───────────────────────────────────────────────────────
// Un client desktop est PUBLIC : le client_secret est extractible du binaire.
// PKCE remplace le secret comme garde-fou de l'échange de code : un `verifier`
// aléatoire est généré à la connexion, seul son hash (challenge S256) part dans
// l'URL d'auth, et le verifier n'est révélé qu'à l'échange du code.
fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("RNG OS indisponible");
    let verifier = b64url(&bytes);
    let digest = { use sha2::Digest; sha2::Sha256::digest(verifier.as_bytes()) };
    (verifier, b64url(&digest))
}

/// base64url sans padding (RFC 4648 §5) — l'encodage exigé par PKCE.
fn b64url(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18 & 63) as usize] as char);
        out.push(T[(n >> 12 & 63) as usize] as char);
        if chunk.len() > 1 { out.push(T[(n >> 6 & 63) as usize] as char); }
        if chunk.len() > 2 { out.push(T[(n & 63) as usize] as char); }
    }
    out
}

// ─── Tokens ──────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    /// Unix timestamp d'expiration.
    expires_at: i64,
}

fn tokens_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("google_tokens.json"))
}

fn save_tokens(t: &Tokens) -> Result<(), String> {
    let path = tokens_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(path, serde_json::to_string(t).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn load_tokens() -> Option<Tokens> {
    std::fs::read_to_string(tokens_path()?)
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
}

pub fn is_connected() -> bool {
    load_tokens().is_some()
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Renvoie un access_token valide, en le rafraîchissant si nécessaire.
fn valid_access_token() -> Result<String, String> {
    let tokens = load_tokens().ok_or("Non connecté. Lance d'abord la connexion Google.")?;
    if unix_now() < tokens.expires_at - 60 {
        return Ok(tokens.access_token);
    }
    let rt = tokens
        .refresh_token
        .ok_or("Pas de refresh token — reconnecte-toi à Google.")?;
    let client = reqwest::blocking::Client::new();
    let cid = google_client_id();
    let csecret = google_client_secret();
    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", cid.as_str()),
            ("client_secret", csecret.as_str()),
            ("refresh_token", rt.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(err) = resp.get("error") {
        let desc = resp.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
        return Err(format!("Rafraîchissement refusé : {err} — {desc}"));
    }
    let at = resp["access_token"].as_str().ok_or("Pas d'access_token.")?.to_string();
    let ei = resp["expires_in"].as_i64().unwrap_or(3600);
    save_tokens(&Tokens { access_token: at.clone(), refresh_token: Some(rt), expires_at: unix_now() + ei })?;
    Ok(at)
}

// ─── OAuth loopback flow ──────────────────────────────────────────────────────

/// Prépare la session OAuth : bind un port local, construit l'URL d'autorisation.
/// Retourne (listener, auth_url, redirect_uri, code_verifier) — le caller ouvre
/// auth_url dans le navigateur puis passe le verifier à `finish_connect`.
pub fn prepare_connect() -> Result<(TcpListener, String, String, String), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}");

    let redirect_enc = format!("http%3A%2F%2Flocalhost%3A{port}");
    let client_id = google_client_id();
    let (verifier, challenge) = pkce_pair();
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/auth\
         ?client_id={client_id}\
         &redirect_uri={redirect_enc}\
         &response_type=code\
         &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.readonly\
         &access_type=offline\
         &prompt=consent\
         &code_challenge={challenge}\
         &code_challenge_method=S256",
    );
    Ok((listener, auth_url, redirect_uri, verifier))
}

/// Attend la redirection OAuth, extrait le code et l'échange contre des tokens.
/// `code_verifier` = celui généré par `prepare_connect` (PKCE).
pub fn finish_connect(listener: TcpListener, redirect_uri: &str, code_verifier: &str) -> Result<(), String> {
    let code = wait_for_code(listener)?;

    let client = reqwest::blocking::Client::new();
    let cid = google_client_id();
    let csecret = google_client_secret();
    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", cid.as_str()),
            ("client_secret", csecret.as_str()),
            ("code_verifier", code_verifier),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(err) = resp.get("error") {
        let desc = resp.get("error_description").and_then(|d| d.as_str()).unwrap_or("");
        return Err(format!("Échange de code refusé : {err} — {desc}"));
    }
    let at = resp["access_token"].as_str().ok_or("Pas d'access_token.")?.to_string();
    let rt = resp["refresh_token"].as_str().map(str::to_string);
    let ei = resp["expires_in"].as_i64().unwrap_or(3600);
    save_tokens(&Tokens { access_token: at, refresh_token: rt, expires_at: unix_now() + ei })
}

/// Lit le premier GET du navigateur sur le socket, extrait le `code` OAuth.
fn wait_for_code(listener: TcpListener) -> Result<String, String> {
    let (mut stream, _) = listener
        .accept()
        .map_err(|e| format!("Attente du redirect OAuth : {e}"))?;

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Envoie une page de succès au navigateur.
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
          <html><body style='font-family:sans-serif;padding:2rem'>\
          <h2>&#x2705; Connexion r\xc3\xa9ussie !</h2>\
          <p>Tu peux fermer cet onglet.</p></body></html>",
    );

    // Extrait le code depuis "GET /?code=XXXX&... HTTP/1.1"
    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split(' ').nth(1).unwrap_or("");

    if path.contains("error=") {
        return Err("Connexion Google refusée par l'utilisateur.".to_string());
    }

    path.split('?')
        .nth(1)
        .and_then(|q| q.split('&').find(|p| p.starts_with("code=")))
        .map(|p| p.trim_start_matches("code=").to_string())
        .ok_or("Code OAuth introuvable dans le redirect.".to_string())
}

// ─── Drive API sync ───────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    #[serde(rename = "createdTime")]
    created_time: Option<String>,
    #[serde(rename = "modifiedTime")]
    modified_time: Option<String>,
    #[serde(default)]
    parents: Vec<String>,
}

fn slugify(s: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true; // évite un tiret en tête
    for c in s.chars() {
        if c.is_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// Remonte la chaîne complète de dossiers parents (de la racine vers la feuille).
/// Utilise les IDs comme clés de traversée pour éviter les collisions de noms.
fn build_container_path(
    file_parents: &[String],
    folder_names: &std::collections::HashMap<String, String>,
    folder_parents: &std::collections::HashMap<String, String>,
) -> Vec<String> {
    let Some(start_id) = file_parents.first() else {
        return vec![];
    };
    let mut path = Vec::new();
    let mut current_id = start_id.as_str();
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current_id.to_string()) {
        match folder_names.get(current_id) {
            Some(name) => path.push(name.clone()),
            None => break, // racine Drive ou dossier partagé non indexé
        }
        match folder_parents.get(current_id) {
            Some(parent_id) => current_id = parent_id.as_str(),
            None => break,
        }
    }
    path.reverse(); // racine → feuille
    path
}

/// Tous les fichiers Drive (tous formats, drives partagés inclus).
/// Renvoie (count_ingested, count_total).
pub fn sync_docs() -> Result<(usize, usize), String> {
    let access_token = valid_access_token()?;
    let client = reqwest::blocking::Client::new();

    // Collecte toutes les pages avant de traiter (nécessaire pour construire folder_names/parents).
    let mut all_files: Vec<DriveFile> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get("https://www.googleapis.com/drive/v3/files")
            .query(&[
                ("q", "trashed=false"),
                ("fields", "nextPageToken,files(id,name,createdTime,modifiedTime,mimeType,parents)"),
                ("pageSize", "1000"),
                // orderBy retiré : incompatible avec corpora=allDrives (Drive API renvoie 400)
                ("corpora", "allDrives"),
                ("includeItemsFromAllDrives", "true"),
                ("supportsAllDrives", "true"),
            ])
            .bearer_auth(&access_token);
        if let Some(ref token) = page_token {
            req = req.query(&[("pageToken", token.as_str())]);
        }
        let resp = req.send().map_err(|e| e.to_string())?;
        let status = resp.status();
        let body = resp.text().map_err(|e| e.to_string())?;
        if !status.is_success() {
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                .unwrap_or_else(|| body.chars().take(300).collect());
            return Err(format!("Drive API {status} : {msg}"));
        }
        let file_list: FileList = serde_json::from_str(&body)
            .map_err(|e| format!("Réponse Drive invalide : {e}"))?;
        let has_more = file_list.next_page_token.is_some();
        page_token = file_list.next_page_token;
        all_files.extend(file_list.files);
        if !has_more { break; }
    }

    // id → nom (pour les dossiers uniquement).
    let folder_names: std::collections::HashMap<String, String> = all_files.iter()
        .filter(|f| f.mime_type == "application/vnd.google-apps.folder")
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();

    // id → parent_id (pour la traversée de la hiérarchie).
    let folder_parents: std::collections::HashMap<String, String> = all_files.iter()
        .filter(|f| f.mime_type == "application/vnd.google-apps.folder")
        .filter_map(|f| f.parents.first().map(|p| (f.id.clone(), p.clone())))
        .collect();

    // Cache local : évite de re-télécharger les fichiers inchangés.
    let existing: std::collections::HashMap<String, Conversation> = load_conversations()
        .into_iter()
        .map(|c| (c.summary.id.clone(), c))
        .collect();

    // — DEBUG temporaire : affiche tous les types MIME présents —
    {
        let mut types: Vec<String> = all_files.iter().map(|f| f.mime_type.clone()).collect();
        types.sort(); types.dedup();
        eprintln!("🗂 Drive total={} | types: {}", all_files.len(), types.join(", "));
        let pdf_count = all_files.iter().filter(|f| f.mime_type == "application/pdf" || f.name.to_lowercase().ends_with(".pdf")).count();
        eprintln!("📄 PDFs trouvés : {pdf_count}");
        eprintln!("🔧 pdftotext PATH: {:?}", which_bin("pdftotext"));
    }

    let mut convs: Vec<Conversation> = Vec::new();
    let mut new_count = 0usize;

    for f in all_files {
        // PDF uniquement — Google Docs, images, vidéos ignorés.
        if f.mime_type != "application/pdf" && !f.name.to_lowercase().ends_with(".pdf") {
            continue;
        }
        eprintln!("📋 PDF détecté : {} ({})", f.name, f.mime_type);

        let id = f.id.clone();
        let modified = f.modified_time.clone();

        // Hiérarchie complète des dossiers parents (racine → feuille immédiate).
        let container_path = build_container_path(&f.parents, &folder_names, &folder_parents);
        eprintln!("  📁 parents_ids={:?} → path={:?}", f.parents, container_path);
        let project = container_path.last().cloned().unwrap_or_else(|| "Google Drive".to_string());
        let project_slug = slugify(&project);

        // Réutilise le cache local si le fichier n'a pas changé.
        // Met à jour project/slug/container_path même sur cache hit (migration).
        if let Some(mut cached) = existing.get(&id).cloned() {
            if cached.summary.last_timestamp == modified {
                cached.summary.project = project;
                cached.summary.project_slug = project_slug;
                cached.summary.container_path = container_path;
                convs.push(cached);
                continue;
            }
        }

        if let Some(conv) = ingest_file(&client, &access_token, f, &project, &project_slug, container_path) {
            convs.push(conv);
            new_count += 1;
        }
    }

    eprintln!("📂 Google Drive : {new_count} nouveaux / {} total", convs.len());

    let path = crate::ai::llama::app_data_dir()
        .ok_or("Dossier de données introuvable.")?
        .join("google_drive_conversations.json");
    std::fs::write(path, serde_json::to_string(&convs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    Ok((new_count, convs.len()))
}

/// Ingère un PDF Drive → Conversation.
fn ingest_file(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: DriveFile,
    project: &str,
    project_slug: &str,
    container_path: Vec<String>,
) -> Option<Conversation> {
    let text = extract_pdf(client, access_token, &file)?;
    let ts = file.modified_time.clone();
    Some(Conversation {
        summary: ConversationSummary {
            id: file.id,
            title: file.name,
            project: project.to_string(),
            project_slug: project_slug.to_string(),
            source: "google-drive".to_string(),
            container_path,
            message_count: 1,
            first_timestamp: file.created_time,
            last_timestamp: file.modified_time,
        },
        messages: vec![Message {
            role: "assistant".to_string(),
            text,
            timestamp: ts,
        }],
    })
}

// ─── Extraction PDF → Markdown ───────────────────────────────────────────────

/// Télécharge le PDF depuis Drive et extrait le texte en markdown.
fn extract_pdf(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: &DriveFile,
) -> Option<String> {
    let resp = client
        .get(format!("https://www.googleapis.com/drive/v3/files/{}", file.id))
        .query(&[("alt", "media")])
        .bearer_auth(access_token)
        .send()
        .ok()?;
    if !resp.status().is_success() { return None; }
    let bytes = resp.bytes().ok()?;
    if bytes.len() > 25 * 1024 * 1024 {
        eprintln!("⚠️ {} trop volumineux (>25 Mo), skipped.", file.name);
        return None;
    }
    let safe_id: String = file.id.chars().take(8).filter(|c| c.is_alphanumeric()).collect();
    let tmp = std::env::temp_dir().join(format!("brainlink_{safe_id}.pdf"));
    std::fs::write(&tmp, &bytes).ok()?;
    let result = pdf_to_markdown(&tmp, &file.name);
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Résout un binaire externe : sidecar du bundle d'abord (app packagée,
/// binaire à côté de l'exécutable), puis Homebrew, puis le PATH.
fn which_bin(name: &str) -> Option<String> {
    // Sidecar embarqué : UNIQUEMENT en release (app packagée). En dev, Tauri
    // copie dans target/debug/ une version relocalisée pour le bundle (dylibs →
    // ../Resources/libs, absent hors bundle) → elle se lance mais sort du vide.
    // On la saute donc en debug pour utiliser Homebrew/PATH qui, lui, marche.
    #[cfg(not(debug_assertions))]
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
            // Sidecar direct (macOS externalBin), puis sous-dossiers Windows
            // `poppler/` et `tesseract/` (chaque toolchain isolée avec ses DLLs,
            // chargées depuis le dossier de l'exe — cf. tauri.windows.conf.json).
            for cand in [
                dir.join(&file),
                dir.join("poppler").join(&file),
                dir.join("tesseract").join(&file),
            ] {
                if cand.is_file() { return Some(cand.to_string_lossy().into_owned()); }
            }
        }
    }
    // Dev Windows : binaires posés par scripts/bundle-sidecars.ps1 (pas d'install
    // système type Homebrew) → on pioche directement dans resources/ du repo.
    #[cfg(all(windows, debug_assertions))]
    {
        let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
        for sub in ["win-poppler", "win-tesseract"] {
            let cand = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("resources").join(sub).join(&file);
            if cand.is_file() { return Some(cand.to_string_lossy().into_owned()); }
        }
    }
    // Fallbacks Unix (Homebrew + PATH).
    #[cfg(unix)]
    {
        for prefix in ["/opt/homebrew/bin", "/usr/local/bin"] {
            let p = std::path::Path::new(prefix).join(name);
            if p.is_file() { return Some(p.to_string_lossy().into_owned()); }
        }
        return Command::new("which").arg(name).output().ok()
            .and_then(|o| if o.status.success() { Some(String::from_utf8_lossy(&o.stdout).trim().to_string()) } else { None })
            .filter(|s| !s.is_empty());
    }
    #[cfg(not(unix))]
    None
}

/// Dossier tessdata embarqué (fra+eng) pour le tesseract sidecar.
/// macOS bundle : Contents/Resources/tessdata · Windows release : tessdata/ à côté
/// de l'exe · dev Windows : resources/win-tessdata du repo (bundle-sidecars.ps1).
/// None → tesseract système utilise son tessdata par défaut (Homebrew).
fn tessdata_prefix() -> Option<std::path::PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let win = dir.join("tessdata");
            if win.is_dir() { return Some(win); }
            if let Some(parent) = dir.parent() {
                let mac = parent.join("Resources").join("tessdata");
                if mac.is_dir() { return Some(mac); }
            }
        }
    }
    #[cfg(all(windows, debug_assertions))]
    {
        let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources").join("win-tessdata");
        if dev.is_dir() { return Some(dev); }
    }
    None
}

/// pdftotext (poppler) en premier ; fallback OCR (pdftoppm + tesseract) pour les PDFs scannés.
/// `brew install poppler tesseract tesseract-lang`
pub fn pdf_to_markdown(path: &std::path::Path, name: &str) -> Option<String> {
    eprintln!("🔍 Extraction PDF : {name}");
    if let Some(text) = run_pdftotext(path) {
        eprintln!("✅ pdftotext OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    // Fallback pur Rust (pdf-extract) : indispensable sur Windows où poppler
    // n'est pas embarqué — qualité moindre que pdftotext -layout mais universel.
    if let Some(text) = extract_pdf_rust(path) {
        eprintln!("✅ pdf-extract OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    eprintln!("⚠️ extraction texte vide — tentative OCR…");
    if let Some(text) = ocr_pdf(path) {
        eprintln!("✅ OCR OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    eprintln!("❌ Extraction impossible : {name}");
    None
}

/// Extraction texte sans binaire externe. pdf-extract panique sur certains PDFs
/// malformés → catch_unwind pour dégrader en None au lieu de crasher le sync.
fn extract_pdf_rust(path: &std::path::Path) -> Option<String> {
    let path = path.to_path_buf();
    let text = std::panic::catch_unwind(move || pdf_extract::extract_text(&path).ok())
        .ok()
        .flatten()?;
    let text = text.trim().to_string();
    if text.len() < 20 { None } else { Some(text) }
}

fn run_pdftotext(path: &std::path::Path) -> Option<String> {
    // Cherche dans Homebrew si absent du PATH par défaut
    let bin = which_bin("pdftotext")
        .unwrap_or_else(|| "/opt/homebrew/bin/pdftotext".to_string());
    let out = Command::new(&bin)
        .args(["-layout", "-nopgbrk"])
        .arg(path)
        .arg("-")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.len() < 20 { None } else { Some(text) }
}

/// OCR via pdftoppm (rend le PDF en images) + tesseract — pour les PDFs scannés.
fn ocr_pdf(path: &std::path::Path) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("pdf");
    let tmp_dir = std::env::temp_dir().join(format!("brainlink_ocr_{stem}"));
    std::fs::create_dir_all(&tmp_dir).ok()?;

    let pdftoppm = which_bin("pdftoppm")
        .unwrap_or_else(|| "/opt/homebrew/bin/pdftoppm".to_string());
    let tesseract = which_bin("tesseract")
        .unwrap_or_else(|| "/opt/homebrew/bin/tesseract".to_string());

    // 200 DPI, max 10 pages
    let _ = Command::new(&pdftoppm)
        .args(["-r", "200", "-png", "-l", "10"])
        .arg(path)
        .arg(tmp_dir.join("page"))
        .output();

    let mut entries: Vec<_> = std::fs::read_dir(&tmp_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |x| x == "png"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut pages = Vec::new();
    for entry in &entries {
        let mut cmd = Command::new(&tesseract);
        if let Some(td) = tessdata_prefix() { cmd.env("TESSDATA_PREFIX", td); }
        if let Ok(ocr) = cmd
            .arg(entry.path())
            .arg("stdout")
            .args(["-l", "fra+eng"])
            .output()
        {
            let t = String::from_utf8_lossy(&ocr.stdout).trim().to_string();
            if t.len() > 20 { pages.push(t); }
        }
        let _ = std::fs::remove_file(entry.path());
    }
    let _ = std::fs::remove_dir(&tmp_dir);

    if pages.is_empty() { None } else { Some(pages.join("\n\n---\n\n")) }
}

/// Transforme le texte brut pdftotext en Markdown structuré :
/// tableaux alignés → pipes, ALL CAPS courts → ## heading, numéros de page supprimés.
fn post_process(text: String) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < lines.len() {
        let raw  = lines[i];
        let trim = raw.trim();

        // Numéro de page isolé (seulement chiffres/tirets, < 8 chars)
        if !trim.is_empty()
            && trim.len() < 8
            && trim.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ' ')
        {
            i += 1; continue;
        }

        // Ligne vide
        if trim.is_empty() {
            if !out.ends_with("\n\n") { out.push('\n'); }
            i += 1; continue;
        }

        // Tableau : 2+ lignes consécutives avec 2+ colonnes séparées par 3+ espaces
        if let Some((table_md, next_i)) = try_table_block(&lines, i) {
            if !out.ends_with('\n') { out.push('\n'); }
            out.push('\n');
            out.push_str(&table_md);
            out.push('\n');
            i = next_i; continue;
        }

        // Heading : ALL CAPS OU ligne courte sans ponctuation, entourée de blancs
        let alpha: String = trim.chars().filter(|c| c.is_alphabetic()).collect();
        let wc = trim.split_whitespace().count();
        let is_all_caps = alpha.len() >= 3 && alpha.chars().all(|c| c.is_uppercase());
        let prev_blank = i == 0 || lines[i - 1].trim().is_empty();
        let next_blank = i + 1 >= lines.len() || lines[i + 1].trim().is_empty();
        let looks_like_heading = trim.len() <= 60
            && wc >= 1 && wc <= 8
            && !trim.ends_with('.')
            && !trim.contains(". ")
            && !trim.contains('|')
            && (is_all_caps || (prev_blank && next_blank));
        if looks_like_heading {
            if !out.ends_with("\n\n") { out.push('\n'); }
            let level = if is_all_caps || wc <= 3 { "##" } else { "###" };
            out.push_str(&format!("{level} {}\n\n", to_title_case(trim)));
            i += 1; continue;
        }

        out.push_str(trim);
        out.push('\n');
        i += 1;
    }

    // Collapse 3+ sauts de ligne consécutifs → 2
    let mut final_out = String::new();
    let mut nl_count = 0u8;
    for c in out.trim().chars() {
        if c == '\n' { nl_count += 1; if nl_count <= 2 { final_out.push(c); } }
        else { nl_count = 0; final_out.push(c); }
    }
    final_out
}

/// Découpe une ligne en colonnes en utilisant 2+ espaces consécutifs comme séparateur.
fn split_cols(line: &str) -> Vec<String> {
    let mut cols: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut spaces = 0usize;
    for c in line.chars() {
        if c == ' ' {
            spaces += 1;
        } else {
            if spaces >= 2 && !cell.trim().is_empty() {
                cols.push(cell.trim().to_string());
                cell = String::new();
            } else if spaces > 0 {
                cell.push(' ');
            }
            spaces = 0;
            cell.push(c);
        }
    }
    if !cell.trim().is_empty() { cols.push(cell.trim().to_string()); }
    cols
}

/// Tente de lire un bloc de tableau à partir de `start`.
/// Retourne (markdown_table, next_line_index) ou None.
fn try_table_block(lines: &[&str], start: usize) -> Option<(String, usize)> {
    let is_row = |l: &str| split_cols(l).len() >= 2;
    if !is_row(lines[start]) { return None; }

    // Collecte les lignes du bloc (tolère 1 ligne vide interne)
    let mut end = start + 1;
    let mut gap = 0u8;
    while end < lines.len() {
        if lines[end].trim().is_empty() { gap += 1; if gap > 1 { break; } end += 1; }
        else if is_row(lines[end]) { gap = 0; end += 1; }
        else { break; }
    }

    let rows: Vec<Vec<String>> = lines[start..end]
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| split_cols(l))
        .filter(|c| c.len() >= 2)
        .collect();

    if rows.len() < 2 { return None; }

    let ncols = rows.iter().map(|r| r.len()).max()?;
    let mut md = String::new();
    for (idx, row) in rows.iter().enumerate() {
        let mut cells = row.clone();
        while cells.len() < ncols { cells.push(String::new()); }
        let escaped: Vec<String> = cells.iter().map(|c| c.replace('|', "\\|")).collect();
        md.push_str(&format!("| {} |\n", escaped.join(" | ")));
        if idx == 0 {
            md.push_str(&format!("| {} |\n", (0..ncols).map(|_| "---").collect::<Vec<_>>().join(" | ")));
        }
    }
    Some((md, end))
}

fn to_title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let lower = w.to_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ─── Déconnexion ──────────────────────────────────────────────────────────────

/// Supprime les tokens et les conversations cachées → déconnexion propre.
pub fn disconnect() {
    if let Some(p) = tokens_path() { let _ = std::fs::remove_file(p); }
    if let Some(p) = crate::ai::llama::app_data_dir().map(|d| d.join("google_drive_conversations.json")) {
        let _ = std::fs::remove_file(p);
    }
}

// ─── Persistance locale ───────────────────────────────────────────────────────

pub fn load_conversations() -> Vec<Conversation> {
    let path = match crate::ai::llama::app_data_dir().map(|d| d.join("google_drive_conversations.json")) {
        Some(p) => p,
        None => return vec![],
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

pub fn load_by_id(id: &str) -> Option<Conversation> {
    load_conversations().into_iter().find(|c| c.summary.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_no_padding() {
        // Vecteurs base64url sans padding.
        assert_eq!(b64url(b"Man"), "TWFu");
        assert_eq!(b64url(b"Ma"), "TWE");
        assert_eq!(b64url(b"M"), "TQ");
        // Pas de caractères non-url (+ / =).
        assert!(b64url(&[0xff, 0xff, 0xff]).bytes().all(|c| c != b'+' && c != b'/' && c != b'='));
    }

    #[test]
    fn pkce_verifier_and_challenge_coherent() {
        let (v, c) = pkce_pair();
        // 32 octets → 43 chars base64url, verifier ET challenge (sha256=32 octets).
        assert_eq!(v.len(), 43);
        assert_eq!(c.len(), 43);
        // Le challenge DOIT être b64url(sha256(verifier)) (sinon Google refuse l'échange).
        let expected = { use sha2::Digest; b64url(&sha2::Sha256::digest(v.as_bytes())) };
        assert_eq!(c, expected);
        // Charset PKCE (unreserved) uniquement.
        assert!(v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        // Deux appels → verifiers différents (aléa réel).
        assert_ne!(pkce_pair().0, v);
    }
}
