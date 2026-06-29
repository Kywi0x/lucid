use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::Command;

// ─── Credentials bundlées (pattern "installed app" Google — pas de secret côté user) ──

// ponytail: à remplir depuis Google Cloud Console → OAuth 2.0 → Desktop app.
const GOOGLE_CLIENT_ID: &str = "445187885318-ocn9o9u0akqpp0rs9h0ftvtpj441jl1r.apps.googleusercontent.com";
const GOOGLE_CLIENT_SECRET: &str = "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";

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
    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", GOOGLE_CLIENT_ID),
            ("client_secret", GOOGLE_CLIENT_SECRET),
            ("refresh_token", rt.as_str()),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(err) = resp.get("error") {
        return Err(format!("Rafraîchissement refusé : {err}"));
    }
    let at = resp["access_token"].as_str().ok_or("Pas d'access_token.")?.to_string();
    let ei = resp["expires_in"].as_i64().unwrap_or(3600);
    save_tokens(&Tokens { access_token: at.clone(), refresh_token: Some(rt), expires_at: unix_now() + ei })?;
    Ok(at)
}

// ─── OAuth loopback flow ──────────────────────────────────────────────────────

/// Prépare la session OAuth : bind un port local, construit l'URL d'autorisation.
/// Retourne (listener, auth_url, redirect_uri) — le caller ouvre auth_url dans le navigateur.
pub fn prepare_connect() -> Result<(TcpListener, String, String), String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect_uri = format!("http://localhost:{port}");

    let redirect_enc = format!("http%3A%2F%2Flocalhost%3A{port}");
    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/auth\
         ?client_id={GOOGLE_CLIENT_ID}\
         &redirect_uri={redirect_enc}\
         &response_type=code\
         &scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.readonly\
         &access_type=offline\
         &prompt=consent",
    );
    Ok((listener, auth_url, redirect_uri))
}

/// Attend la redirection OAuth, extrait le code et l'échange contre des tokens.
pub fn finish_connect(listener: TcpListener, redirect_uri: &str) -> Result<(), String> {
    let code = wait_for_code(listener)?;

    let client = reqwest::blocking::Client::new();
    let resp: serde_json::Value = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("code", code.as_str()),
            ("client_id", GOOGLE_CLIENT_ID),
            ("client_secret", GOOGLE_CLIENT_SECRET),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if let Some(err) = resp.get("error") {
        return Err(format!("Échange de code refusé : {err}"));
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
                ("orderBy", "modifiedTime desc"),
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

    let mut convs: Vec<Conversation> = Vec::new();
    let mut new_count = 0usize;

    for f in all_files {
        // Dossiers, audio et vidéo : pas de contenu textuel exploitable.
        if f.mime_type == "application/vnd.google-apps.folder"
            || f.mime_type.starts_with("audio/")
            || f.mime_type.starts_with("video/")
        {
            continue;
        }

        let id = f.id.clone();
        let modified = f.modified_time.clone();

        // Hiérarchie complète des dossiers parents (racine → feuille immédiate).
        let container_path = build_container_path(&f.parents, &folder_names, &folder_parents);
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

/// Ingère un fichier Drive. Stratégie par type :
/// - Google Workspace (Doc/Sheet/Slide) → export texte via API
/// - Autres (PDF, Word, image…) → stub avec nom + métadonnées (le brain sait que ça existe)
fn ingest_file(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: DriveFile,
    project: &str,
    project_slug: &str,
    container_path: Vec<String>,
) -> Option<Conversation> {
    let is_google_workspace = file.mime_type.starts_with("application/vnd.google-apps.")
        && !matches!(
            file.mime_type.as_str(),
            "application/vnd.google-apps.folder"
                | "application/vnd.google-apps.shortcut"
                | "application/vnd.google-apps.unknown"
        );

    let text = if is_google_workspace {
        let export_mime = if file.mime_type.contains("spreadsheet") {
            "text/csv"
        } else {
            "text/plain"
        };
        let resp = client
            .get(format!(
                "https://www.googleapis.com/drive/v3/files/{}/export",
                file.id
            ))
            .query(&[("mimeType", export_mime)])
            .bearer_auth(access_token)
            .send()
            .ok()?;
        if !resp.status().is_success() {
            eprintln!("⚠️ Export échoué ({}) : {}", file.name, resp.status());
            return None;
        }
        let t = resp.text().ok()?;
        if t.trim().is_empty() {
            return None;
        }
        t
    } else {
        try_extract_binary(client, access_token, &file).unwrap_or_else(|| {
            let ext = file.name.rsplit('.').next().unwrap_or("?").to_uppercase();
            format!("Fichier Drive : {}\nType : {ext}", file.name)
        })
    };

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

// ─── Extraction binaire (PDF / image) ────────────────────────────────────────

fn try_extract_binary(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: &DriveFile,
) -> Option<String> {
    let mime = file.mime_type.as_str();
    let is_pdf = mime == "application/pdf" || file.name.to_lowercase().ends_with(".pdf");
    let is_image = mime.starts_with("image/") && !mime.contains("svg");

    if !is_pdf && !is_image {
        return None; // Word/Excel/vidéo → stub (OCR/whisper phase 2)
    }

    let resp = client
        .get(format!("https://www.googleapis.com/drive/v3/files/{}", file.id))
        .query(&[("alt", "media")])
        .bearer_auth(access_token)
        .send()
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let bytes = resp.bytes().ok()?;
    if bytes.len() > 10 * 1024 * 1024 {
        eprintln!("⚠️ {} trop volumineux (>10 Mo), skipped.", file.name);
        return None;
    }

    let safe_id: String = file.id.chars().take(8).collect();
    let ext = if is_pdf {
        "pdf".to_string()
    } else {
        file.name.rsplit('.').next().unwrap_or("jpg").to_lowercase()
    };
    let tmp = std::env::temp_dir().join(format!("brainlink_{safe_id}.{ext}"));
    std::fs::write(&tmp, &bytes).ok()?;

    let result = if is_pdf {
        extract_pdf_text(&tmp)
    } else {
        extract_image_text(&tmp)
    };
    let _ = std::fs::remove_file(&tmp);
    result
}

/// pdftotext (poppler) — `brew install poppler` sur macOS.
fn extract_pdf_text(path: &std::path::Path) -> Option<String> {
    let out = Command::new("pdftotext").arg(path).arg("-").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// tesseract OCR — `brew install tesseract tesseract-lang` sur macOS.
fn extract_image_text(path: &std::path::Path) -> Option<String> {
    let out = Command::new("tesseract")
        .arg(path)
        .arg("stdout")
        .arg("-l").arg("fra+eng")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    // < 30 chars = probablement du bruit OCR
    if text.len() < 30 { None } else { Some(text) }
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
