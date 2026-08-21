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
    /// Raison pour laquelle Google refuse DÉFINITIVEMENT de rafraîchir (accès
    /// révoqué, refresh token expiré). Tant que c'est rempli, le connecteur se
    /// déclare déconnecté : sans ça, `is_connected()` restait vrai à vie sur la
    /// simple présence du fichier, chaque synchro échouait, et l'UI continuait
    /// d'afficher « connecté » — l'échec silencieux interdit par l'ADR-0015.
    #[serde(default)]
    needs_reconnect: Option<String>,
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
    load_tokens().is_some_and(|t| t.needs_reconnect.is_none())
}

/// Pourquoi Drive demande une reconnexion, `None` si tout va bien. Remonté à
/// l'UI pour distinguer « jamais connecté » de « accès expiré ».
pub fn reconnect_reason() -> Option<String> {
    load_tokens().and_then(|t| t.needs_reconnect)
}

/// Marque l'accès comme mort. On garde le fichier (et le refresh token) : c'est
/// la reconnexion qui l'écrasera, et l'effacer ferait perdre la trace du motif.
fn mark_needs_reconnect(reason: &str) {
    let Some(mut t) = load_tokens() else { return };
    if t.needs_reconnect.as_deref() == Some(reason) { return; }
    crate::elog!("⚠️ Drive : accès expiré, reconnexion nécessaire — {reason}");
    t.needs_reconnect = Some(reason.to_string());
    let _ = save_tokens(&t);
}

// ─── Sélection de dossiers ───────────────────────────────────────────────────
// L'utilisateur choisit ce que Lucid lit. Le filtre est appliqué EN LOCAL, pas
// côté API : l'API Drive n'a pas de requête récursive (`'<id>' in parents` ne
// rend que les enfants directs), donc un sous-arbre coûterait N requêtes,
// parfois plus que la liste complète. Et le coût n'est pas là — énumérer des
// métadonnées prend une minute, les télécharger et les OCRiser prend des heures.

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Selection {
    /// IDs des dossiers cochés. **Vide = tout le Drive.** C'est aussi l'état des
    /// comptes connectés avant cette feature (aucun fichier de config) : sans ce
    /// défaut, la mise à jour ferait disparaître leur Drive au sync suivant.
    #[serde(default)]
    pub folders: Vec<String>,
    /// Fichiers sans dossier cochable (racine du Drive, partage non indexé).
    /// Case dédiée : sans elle ils disparaissent sans un mot (ADR-0015).
    #[serde(default)]
    pub include_orphans: bool,
}

fn selection_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("google_drive_selection.json"))
}

pub fn selection() -> Selection {
    selection_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|r| serde_json::from_str(&r).ok())
        .unwrap_or_default()
}

pub fn set_selection(folders: Vec<String>, include_orphans: bool) -> Result<(), String> {
    let path = selection_path().ok_or("Dossier de données introuvable.")?;
    let sel = Selection { folders, include_orphans };
    std::fs::write(path, serde_json::to_string(&sel).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// Un fichier est retenu si l'un de ses ancêtres est coché. `folder_parents` et
/// `known_folders` sont construits sur les seuls dossiers de la liste — les deux
/// tables que `sync_docs` calcule déjà pour l'arborescence, donc le prédicat est
/// quasi gratuit.
fn is_selected(
    parents: &[String],
    folder_parents: &std::collections::HashMap<String, String>,
    known_folders: &std::collections::HashSet<String>,
    sel: &Selection,
) -> bool {
    if sel.folders.is_empty() {
        return true; // aucune sélection = tout le Drive
    }
    let Some(start) = parents.first() else {
        return sel.include_orphans; // aucun parent du tout
    };
    let mut current = start.clone();
    let mut seen_known = false;
    let mut visited = std::collections::HashSet::new();
    while visited.insert(current.clone()) {
        if sel.folders.contains(&current) {
            return true;
        }
        if known_folders.contains(&current) {
            seen_known = true;
        }
        match folder_parents.get(&current) {
            Some(parent) => current = parent.clone(),
            None => break,
        }
    }
    // Chaîne sans aucun dossier connu → le fichier est à la racine ou dans un
    // partage non indexé : il n'a aucune case à cocher, d'où la case dédiée.
    if seen_known { false } else { sel.include_orphans }
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
    let client = http();
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
        let msg = format!("Rafraîchissement refusé : {err} — {desc}");
        // Une réponse d'erreur du serveur OAuth est un refus DÉFINITIF (accès
        // révoqué, token expiré) : réessayer ne changera rien, seule une
        // reconnexion débloque. Une panne réseau, elle, sort par le `?` de
        // `.send()` plus haut et ne marque rien — on ne déconnecte pas
        // l'utilisateur parce que son wifi a coupé.
        mark_needs_reconnect(&msg);
        return Err(msg);
    }
    let at = resp["access_token"].as_str().ok_or("Pas d'access_token.")?.to_string();
    let ei = resp["expires_in"].as_i64().unwrap_or(3600);
    save_tokens(&Tokens {
        access_token: at.clone(),
        refresh_token: Some(rt),
        expires_at: unix_now() + ei,
        needs_reconnect: None, // ça remarche : on efface la marque
    })?;
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

    let client = http();
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
    save_tokens(&Tokens { access_token: at, refresh_token: rt, expires_at: unix_now() + ei, needs_reconnect: None })
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
    // `default` : les appels qui ne demandent pas tous les champs (empreinte,
    // liste des dossiers) réutilisent la même struct.
    #[serde(default)]
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

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

/// Pagination Drive : accumule toutes les pages d'une requête `files.list`.
/// `fields` ne contient que les champs du sous-objet `files(...)`.
/// Client HTTP du connecteur : **avec délais bornés**. `Client::new()` n'en pose
/// aucun — une requête qui cale attend pour toujours, et comme la synchro tourne
/// sous `GEN_LOCK`, tout ce qui attend ce verrou gèle avec elle (fenêtre figée
/// observée le 18/08/2026). Le délai global est large : il couvre le
/// téléchargement d'un PDF de 25 Mo, il ne borne que les vrais blocages.
fn http() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn fetch_files(
    client: &reqwest::blocking::Client,
    access_token: &str,
    q: &str,
    fields: &str,
) -> Result<Vec<DriveFile>, String> {
    let fields = format!("nextPageToken,files({fields})");
    let mut out: Vec<DriveFile> = Vec::new();
    let mut page_token: Option<String> = None;
    loop {
        let mut req = client
            .get("https://www.googleapis.com/drive/v3/files")
            .query(&[
                ("q", q),
                ("fields", fields.as_str()),
                ("pageSize", "1000"),
                // orderBy retiré : incompatible avec corpora=allDrives (Drive API renvoie 400)
                ("corpora", "allDrives"),
                ("includeItemsFromAllDrives", "true"),
                ("supportsAllDrives", "true"),
            ])
            .bearer_auth(access_token);
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
        let page: FileList = serde_json::from_str(&body)
            .map_err(|e| format!("Réponse Drive invalide : {e}"))?;
        let has_more = page.next_page_token.is_some();
        page_token = page.next_page_token;
        out.extend(page.files);
        if !has_more {
            break;
        }
    }
    Ok(out)
}

/// Un dossier tel que servi à l'écran de sélection.
#[derive(Serialize)]
pub struct DriveFolder {
    pub id: String,
    pub name: String,
    /// Parent direct — `None` si racine du Drive ou hors périmètre.
    pub parent: Option<String>,
    pub shared: bool,
}

/// Racines du sélecteur : dossiers de premier niveau de Mon Drive, unités
/// partagées, puis dossiers partagés avec moi.
///
/// **Chargement paresseux, mesuré et non spéculatif.** La version précédente
/// listait tous les dossiers d'un coup, en pariant qu'ils sont « une fraction
/// minuscule » du Drive. Sur un compte professionnel (16 unités partagées) ce
/// pari est faux : 24 694 dossiers en 42 s, pour un arbre de 24 694 lignes que
/// personne ne peut parcourir (mesuré le 18/08/2026). Ici : 153 entrées en 2 s,
/// et une requête par dépliage.
pub fn list_roots() -> Result<Vec<DriveFolder>, String> {
    let access_token = valid_access_token()?;
    let client = http();
    let mut out = Vec::new();

    // Premier niveau de Mon Drive.
    for f in fetch_files(
        &client,
        &access_token,
        &format!("'root' in parents and mimeType='{FOLDER_MIME}' and trashed=false"),
        "id,name",
    )? {
        out.push(DriveFolder { id: f.id, name: f.name, parent: None, shared: false });
    }

    // Unités partagées : leur id est aussi celui de leur dossier racine, donc
    // elles se déplient comme un dossier ordinaire.
    let resp = client
        .get("https://www.googleapis.com/drive/v3/drives")
        .query(&[("pageSize", "100")])
        .bearer_auth(&access_token)
        .send()
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        #[derive(Deserialize)]
        struct Drive { id: String, name: String }
        #[derive(Deserialize)]
        struct DriveList { #[serde(default)] drives: Vec<Drive> }
        if let Ok(list) = resp.json::<DriveList>() {
            for d in list.drives {
                out.push(DriveFolder { id: d.id, name: d.name, parent: None, shared: true });
            }
        }
    }

    // Dossiers partagés avec moi. `sharedWithMe` est refusé dans `fields` mais
    // valide dans `q` — c'est bien deux choses différentes côté API.
    for f in fetch_files(
        &client,
        &access_token,
        &format!("sharedWithMe=true and mimeType='{FOLDER_MIME}' and trashed=false"),
        "id,name",
    )? {
        out.push(DriveFolder { id: f.id, name: f.name, parent: None, shared: true });
    }

    Ok(out)
}

/// Noms des dossiers cochés. La sélection ne stocke que des ids : sans ça, on ne
/// peut pas dire à l'utilisateur **ce qui est synchronisé** quand les dossiers
/// concernés ne sont pas chargés dans l'arbre (demande de Liam, 18/08/2026).
/// Les noms ne sont pas persistés exprès — un dossier renommé dans Drive doit
/// s'afficher sous son nom actuel.
pub fn folder_labels(ids: &[String]) -> Vec<DriveFolder> {
    let Ok(access_token) = valid_access_token() else { return vec![] };
    let client = http();
    ids.iter()
        .filter_map(|id| {
            let f = fetch_meta(&client, &access_token, id, "id,name,parents")?;
            Some(DriveFolder { id: f.id, name: f.name, parent: f.parents.first().cloned(), shared: false })
        })
        .collect()
}

/// Dossiers dont le nom contient `needle`. Sert la barre de recherche du
/// sélecteur : sur 24 694 dossiers, déplier à la main ne suffit pas.
pub fn search_folders(needle: &str) -> Result<Vec<DriveFolder>, String> {
    let needle = needle.trim();
    if needle.len() < 2 {
        return Ok(vec![]);
    }
    let access_token = valid_access_token()?;
    let safe = needle.replace('\\', "\\\\").replace('\'', "\\'");
    Ok(fetch_files(
        &http(),
        &access_token,
        &format!("name contains '{safe}' and mimeType='{FOLDER_MIME}' and trashed=false"),
        "id,name,parents",
    )?
    .into_iter()
    .take(200)
    .map(|f| DriveFolder { id: f.id, name: f.name, parent: f.parents.first().cloned(), shared: false })
    .collect())
}

/// Sous-dossiers directs d'un dossier (ou d'une unité partagée). Une requête,
/// quelques dizaines de lignes.
pub fn list_children(parent: &str) -> Result<Vec<DriveFolder>, String> {
    let access_token = valid_access_token()?;
    let client = http();
    // `parent` vient d'un nœud déjà servi par l'API : on l'échappe quand même,
    // une apostrophe dans un id casserait la requête (et c'est une injection).
    let safe = parent.replace('\\', "\\\\").replace('\'', "\\'");
    Ok(fetch_files(
        &client,
        &access_token,
        &format!("'{safe}' in parents and mimeType='{FOLDER_MIME}' and trashed=false"),
        "id,name",
    )?
    .into_iter()
    .map(|f| DriveFolder {
        id: f.id,
        name: f.name,
        parent: Some(parent.to_string()),
        shared: false,
    })
    .collect())
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

/// Empreinte bon marché (nombre de fichiers + date de modif la plus récente) —
/// watch auto : Drive n'a pas d'équivalent `notify` (rien sur le disque local
/// à surveiller, l'API ne pousse rien sans webhook joignable depuis internet,
/// hors de portée d'une app desktop locale). Un sondage périodique reste donc
/// nécessaire, mais celui-ci ne télécharge que `modifiedTime` par fichier —
/// même pagination que `sync_docs`, sans le contenu ni les champs superflus —
/// jamais un `sync_docs()` complet juste pour savoir si quelque chose a changé.
///
/// L'empreinte **respecte la sélection de dossiers** : sans ça, un changement
/// dans un dossier exclu déclencherait une resynchro qui ne produirait rien.
/// C'est ce qui impose de demander `parents` et `mimeType` en plus de
/// `modifiedTime` — même nombre de requêtes, payload à peine plus gros.
pub fn changed_fingerprint() -> Option<String> {
    let access_token = valid_access_token().ok()?;
    let client = http();
    const FP_FIELDS: &str = "id,mimeType,parents,modifiedTime";
    let sel = selection();
    let scoped = !sel.folders.is_empty();
    let files = if scoped {
        fetch_in_selection(&client, &access_token, &sel, FP_FIELDS).ok()?
    } else {
        fetch_files(&client, &access_token, "trashed=false", FP_FIELDS).ok()?
    };

    let folder_parents = folder_parents_of(&files);
    let known_folders = known_folders_of(&files);

    let mut count = 0usize;
    let mut max_modified = String::new();
    for f in &files {
        if !scoped && !is_selected(&f.parents, &folder_parents, &known_folders, &sel) {
            continue;
        }
        count += 1;
        if let Some(m) = &f.modified_time {
            if m.as_str() > max_modified.as_str() {
                max_modified = m.clone();
            }
        }
    }
    Some(format!("{count}:{max_modified}"))
}

/// id → parent id, sur les seuls dossiers (traversée de la hiérarchie).
fn folder_parents_of(files: &[DriveFile]) -> std::collections::HashMap<String, String> {
    files
        .iter()
        .filter(|f| f.mime_type == FOLDER_MIME)
        .filter_map(|f| f.parents.first().map(|p| (f.id.clone(), p.clone())))
        .collect()
}

fn known_folders_of(files: &[DriveFile]) -> std::collections::HashSet<String> {
    files
        .iter()
        .filter(|f| f.mime_type == FOLDER_MIME)
        .map(|f| f.id.clone())
        .collect()
}

/// Métadonnées d'un fichier par son id. La liste ne rend que les *enfants* d'un
/// dossier, jamais le dossier lui-même : sans ça, un document rangé directement
/// sous un dossier coché arrive avec un `container_path` vide, donc sans bulle
/// dans le cerveau (régression du 18/08/2026).
fn fetch_meta(
    client: &reqwest::blocking::Client,
    access_token: &str,
    id: &str,
    fields: &str,
) -> Option<DriveFile> {
    client
        .get(format!("https://www.googleapis.com/drive/v3/files/{id}"))
        .query(&[("fields", fields), ("supportsAllDrives", "true")])
        .bearer_auth(access_token)
        .send()
        .ok()?
        .json::<DriveFile>()
        .ok()
}

/// Fichiers du **périmètre coché**, par descente de l'arbre : une requête par
/// dossier visité, au lieu d'énumérer tout le Drive.
///
/// Le coût suit la sélection, pas la taille du compte. Avant, cocher un dossier
/// de cinq PDF déclenchait quand même l'énumération complète — 24 694 dossiers +
/// 42 638 PDF sur un compte professionnel, une à deux minutes de réseau à chaque
/// synchro **et** à chaque sondage de 5 minutes (mesuré le 18/08/2026).
///
/// Sélection vide = tout le Drive : l'appelant garde l'énumération complète, qui
/// reste le seul moyen de tout voir.
fn fetch_in_selection(
    client: &reqwest::blocking::Client,
    access_token: &str,
    sel: &Selection,
    fields: &str,
) -> Result<Vec<DriveFile>, String> {
    let mut out: Vec<DriveFile> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut queue: Vec<String> = sel.folders.clone();
    // Les fichiers de la racine n'ont pas de dossier à cocher : leur case dédiée
    // les fait entrer ici (ADR-0015 — sinon ils disparaissent sans un mot).
    if sel.include_orphans {
        queue.push("root".to_string());
    }
    // Les dossiers cochés et leurs ancêtres : ils ne sont enfants de personne
    // dans ce parcours, donc personne ne rendrait leur nom — et c'est
    // précisément le nom que l'utilisateur cherche dans son cerveau.
    for id in &sel.folders {
        let mut cur = id.clone();
        // Remonte jusqu'à la racine (ou une chaîne déjà connue) : chemin complet,
        // pas seulement le dossier choisi.
        for _ in 0..32 {
            if !seen.insert(cur.clone()) {
                break;
            }
            let Some(meta) = fetch_meta(client, access_token, &cur, "id,name,mimeType,parents") else {
                break;
            };
            let parent = meta.parents.first().cloned();
            out.push(meta);
            match parent {
                Some(p) => cur = p,
                None => break,
            }
        }
        // Le dossier coché doit être visité comme conteneur, pas seulement décrit.
        seen.remove(id);
        queue.push(id.clone());
    }

    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue; // raccourci d'un cycle de parents, ou dossier coché deux fois
        }
        let safe = id.replace('\\', "\\\\").replace('\'', "\\'");
        let files = fetch_files(client, access_token, &format!("'{safe}' in parents and trashed=false"), fields)?;
        for f in files {
            if f.mime_type == FOLDER_MIME {
                queue.push(f.id.clone());
            }
            out.push(f);
        }
    }
    Ok(out)
}

/// Tous les fichiers Drive (tous formats, drives partagés inclus).
/// Renvoie (count_ingested, count_total).
pub fn sync_docs() -> Result<(usize, usize), String> {
    // Une seule synchro à la fois : la commande manuelle et la génération
    // déclenchée par le watcher ont tourné en parallèle le 18/08/2026 — deux
    // énumérations complètes du Drive et deux écritures du même fichier.
    static SYNC: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Ok(_busy) = SYNC.try_lock() else {
        return Err("Une synchronisation Google Drive est déjà en cours.".to_string());
    };
    // Témoin dans la barre d'outils. Ici et pas dans la commande : la synchro
    // automatique (watcher → refresh_connector_caches) passe par la même porte,
    // et c'est celle qu'on ne voyait jamais.
    let _badge = crate::SyncBadge::new("google-drive");
    let access_token = valid_access_token()?;
    let client = http();

    // Collecte toutes les pages avant de traiter (nécessaire pour construire folder_names/parents).
    const SYNC_FIELDS: &str = "id,name,createdTime,modifiedTime,mimeType,parents";
    let sel = selection();
    // Sélection vide = tout le Drive (défaut historique, cf. `Selection`).
    let scoped = !sel.folders.is_empty();
    let all_files = if scoped {
        fetch_in_selection(&client, &access_token, &sel, SYNC_FIELDS)?
    } else {
        fetch_files(&client, &access_token, "trashed=false", SYNC_FIELDS)?
    };

    // id → nom (pour les dossiers uniquement).
    let folder_names: std::collections::HashMap<String, String> = all_files.iter()
        .filter(|f| f.mime_type == FOLDER_MIME)
        .map(|f| (f.id.clone(), f.name.clone()))
        .collect();

    // id → parent_id (pour la traversée de la hiérarchie).
    let folder_parents = folder_parents_of(&all_files);
    let known_folders = known_folders_of(&all_files);

    // Cache local : évite de re-télécharger les fichiers inchangés.
    let existing: std::collections::HashMap<String, Conversation> = load_conversations()
        .into_iter()
        .map(|c| (c.summary.id.clone(), c))
        .collect();

    // — DEBUG temporaire : affiche tous les types MIME présents —
    {
        let mut types: Vec<String> = all_files.iter().map(|f| f.mime_type.clone()).collect();
        types.sort(); types.dedup();
        crate::elog!("🗂 Drive total={} | types: {}", all_files.len(), types.join(", "));
        let pdf_count = all_files.iter().filter(|f| f.mime_type == "application/pdf" || f.name.to_lowercase().ends_with(".pdf")).count();
        crate::elog!("📄 PDFs trouvés : {pdf_count}");
        crate::elog!("🔧 pdftotext PATH: {:?}", which_bin("pdftotext"));
    }

    let mut convs: Vec<Conversation> = Vec::new();
    let mut new_count = 0usize;

    let mut skipped = 0usize;
    let mut ignored: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    // Format supporté mais extraction sans résultat — distinct de `ignored`
    // (format non lisible par nature) et de `skipped` (hors sélection).
    let mut unreadable = 0usize;

    for f in all_files {
        // Ce qui n'est ni un document lisible ni un dossier (images, vidéos, zip…)
        // est ignoré — mais compté par type, et dit à la fin. Un fichier qui
        // disparaît sans un mot est l'anti-pattern interdit par l'ADR-0015.
        let Some(kind) = drive_kind(&f) else {
            if f.mime_type != FOLDER_MIME {
                *ignored.entry(f.mime_type.clone()).or_insert(0usize) += 1;
            }
            continue;
        };

        // Filtre de sélection — inutile quand le parcours est déjà ciblé : tout
        // ce qui remonte est dans le périmètre. Il ne sert qu'au mode « tout le
        // Drive », où il écarte le fichier AVANT la partie chère (download + OCR).
        if !scoped && !is_selected(&f.parents, &folder_parents, &known_folders, &sel) {
            skipped += 1;
            continue;
        }

        crate::elog!("📋 {kind} détecté : {} ({})", f.name, f.mime_type);

        let id = f.id.clone();
        let modified = f.modified_time.clone();

        // Hiérarchie complète des dossiers parents (racine → feuille immédiate).
        let container_path = build_container_path(&f.parents, &folder_names, &folder_parents);
        crate::elog!("  📁 parents_ids={:?} → path={:?}", f.parents, container_path);
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

        // A2 — même mémoire d'échec que les dossiers locaux : sur Drive la note est
        // encore plus salée, chaque essai repaie le TÉLÉCHARGEMENT avant l'OCR.
        let key = format!("google-drive::{id}");
        let stamp = modified.as_deref().unwrap_or("");
        let name = f.name.clone();
        if let Some(reason) = crate::connectors::known_extract_failure(&key, stamp) {
            unreadable += 1;
            crate::elog!("⏭️ Drive : {name} déjà illisible ({reason}), non rejoué.");
            continue;
        }
        let started = std::time::Instant::now();
        if let Some(conv) = ingest_file(&client, &access_token, f, kind, &project, &project_slug, container_path) {
            convs.push(conv);
            new_count += 1;
        } else {
            unreadable += 1;
            crate::connectors::remember_extract_failure(
                &key, stamp, "aucun texte exploitable", started.elapsed());
        }
    }

    crate::elog!(
        "📂 Google Drive : {new_count} nouveaux / {} total{}",
        convs.len(),
        if skipped > 0 { format!(" ({skipped} hors sélection)") } else { String::new() }
    );
    if unreadable > 0 {
        crate::elog!("   ↳ {unreadable} fichier(s) au format supporté mais sans texte exploitable.");
    }
    if !ignored.is_empty() {
        let mut par_type: Vec<(String, usize)> = ignored.into_iter().collect();
        par_type.sort_by(|a, b| b.1.cmp(&a.1));
        let total: usize = par_type.iter().map(|(_, n)| n).sum();
        let detail: Vec<String> = par_type
            .iter()
            .take(6)
            .map(|(m, n)| format!("{n}× {}", m.rsplit(['.', '/']).next().unwrap_or(m)))
            .collect();
        crate::elog!("   ↳ {total} fichier(s) non lisibles ignorés : {}", detail.join(", "));
    }

    let path = crate::ai::llama::app_data_dir()
        .ok_or("Dossier de données introuvable.")?
        .join("google_drive_conversations.json");
    std::fs::write(path, serde_json::to_string(&convs).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    Ok((new_count, convs.len()))
}

const MIME_SLIDES: &str = "application/vnd.google-apps.presentation";
const MIME_GDOC: &str = "application/vnd.google-apps.document";
const MIME_GSHEET: &str = "application/vnd.google-apps.spreadsheet";
const MIME_PPTX: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// Type de fichier Drive supporté → clé de dispatch, None = ignoré.
/// Types que Lucid sait lire sur Drive. Le mot rendu est soit un traitement
/// spécial (`pdf`, `slides`, `gdoc`, `gsheet`), soit une extension : dans ce cas
/// le fichier est téléchargé et confié à `file_to_source_text`, le même
/// extracteur que les dossiers locaux.
///
/// Avant le 18/08/2026, seuls PDF/Slides/pptx passaient : un dossier de 7 `.docx`
/// donnait zéro document, sans un mot (`docs/known-gaps.md`). Drive était plus
/// pauvre que le connecteur « dossiers locaux » alors que l'extracteur est commun.
fn drive_kind(f: &DriveFile) -> Option<&'static str> {
    let name = f.name.to_lowercase();
    if f.mime_type == "application/pdf" || name.ends_with(".pdf") { return Some("pdf"); }
    if f.mime_type == MIME_SLIDES { return Some("slides"); }
    if f.mime_type == MIME_PPTX || name.ends_with(".pptx") { return Some("pptx"); }
    // Formats Google natifs : pas de binaire à télécharger, on demande un export.
    if f.mime_type == MIME_GDOC { return Some("gdoc"); }
    if f.mime_type == MIME_GSHEET { return Some("gsheet"); }
    // Fichiers déposés tels quels : l'extension décide, comme en local.
    for ext in ["docx", "xlsx", "csv", "txt", "md", "rtf", "doc"] {
        if name.ends_with(&format!(".{ext}")) { return Some(ext); }
    }
    None
}

/// Ingère un fichier Drive supporté (PDF, Slides, pptx) → Conversation.
fn ingest_file(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: DriveFile,
    kind: &str,
    project: &str,
    project_slug: &str,
    container_path: Vec<String>,
) -> Option<Conversation> {
    let text = match kind {
        "slides" => export_text(client, access_token, &file, "text/plain"),
        "gdoc" => export_text(client, access_token, &file, "text/plain"),
        // Un tableur exporté en CSV : `file_to_source_text` en fait déjà un
        // tableau markdown pour les fichiers locaux, on garde le texte brut ici.
        "gsheet" => export_text(client, access_token, &file, "text/csv"),
        "pptx" => download_then(client, access_token, &file, "pptx", |p| pptx_to_markdown(p)),
        "pdf" => extract_pdf(client, access_token, &file),
        // docx, xlsx, csv, txt, md, rtf, doc — même extracteur que les dossiers locaux.
        ext => download_then(client, access_token, &file, ext, |p| {
            crate::file_to_source_text(p)
                .map_err(|e| crate::elog!("⚠️ Drive : {} illisible — {e}", p.display()))
                .ok()
        }),
    }?;
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

// ─── Extraction fichiers → Markdown ──────────────────────────────────────────

/// Télécharge un fichier binaire Drive dans un tmp, applique `convert`, nettoie.
fn download_then(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: &DriveFile,
    ext: &str,
    convert: impl FnOnce(&std::path::Path) -> Option<String>,
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
        crate::elog!("⚠️ {} trop volumineux (>25 Mo), skipped.", file.name);
        return None;
    }
    let safe_id: String = file.id.chars().take(8).filter(|c| c.is_alphanumeric()).collect();
    let tmp = std::env::temp_dir().join(format!("brainlink_{safe_id}.{ext}"));
    std::fs::write(&tmp, &bytes).ok()?;
    let result = convert(&tmp);
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Télécharge le PDF depuis Drive et extrait le texte en markdown.
fn extract_pdf(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: &DriveFile,
) -> Option<String> {
    download_then(client, access_token, file, "pdf", |p| pdf_to_markdown(p, &file.name))
}

/// Exporte le texte d'un Google Slides via l'API Drive (`files.export`).
/// ponytail: text/plain ne marque pas les diapos — suffisant pour brain.md ;
/// si la structure par diapo devient nécessaire, exporter en pptx et convertir.
fn export_text(
    client: &reqwest::blocking::Client,
    access_token: &str,
    file: &DriveFile,
    mime: &str,
) -> Option<String> {
    let resp = client
        .get(format!("https://www.googleapis.com/drive/v3/files/{}/export", file.id))
        .query(&[("mimeType", mime)])
        .bearer_auth(access_token)
        .send()
        .ok()?;
    if !resp.status().is_success() {
        crate::elog!("⚠️ export Drive {} : HTTP {}", file.name, resp.status());
        return None;
    }
    let text = resp.text().ok()?.trim().to_string();
    if text.len() < 20 { None } else { Some(text) }
}

// ─── PowerPoint .pptx → Markdown (pur Rust, parité Mac/Windows — ADR-0015) ────

/// Texte brut d'un fragment XML : `</a:p>`/`</w:p>` → sauts de ligne, tags
/// strippés, entités de base décodées. Partagé avec docx_to_text (lib.rs).
pub(crate) fn xml_text(xml: &str) -> String {
    let xml = xml
        .replace("</a:p>", "\n").replace("</w:p>", "\n")
        .replace("<a:tab/>", "\t").replace("<w:tab/>", "\t");
    let mut out = String::with_capacity(xml.len() / 4);
    let mut in_tag = false;
    for c in xml.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">")
        .replace("&quot;", "\"").replace("&apos;", "'")
}

/// Un .pptx est un zip : le texte de chaque diapo vit dans ppt/slides/slideN.xml.
/// Sortie : `## Diapo N` + texte, dans l'ordre des diapos.
pub fn pptx_to_markdown(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut zip = zip::ZipArchive::new(f).ok()?;

    // Indices des diapos, triés numériquement (slide2 avant slide10).
    let mut slides: Vec<(u32, String)> = (0..zip.len())
        .filter_map(|i| {
            let name = zip.by_index(i).ok()?.name().to_string();
            let n: u32 = name.strip_prefix("ppt/slides/slide")?
                .strip_suffix(".xml")?.parse().ok()?;
            Some((n, name))
        })
        .collect();
    slides.sort_by_key(|(n, _)| *n);

    let mut out = String::new();
    for (n, name) in slides {
        let mut xml = String::new();
        if zip.by_name(&name).ok()?.read_to_string(&mut xml).is_err() { continue; }
        let text = xml_text(&xml);
        let text = text.trim();
        if text.is_empty() { continue; }
        out.push_str(&format!("## Diapo {n}\n\n{text}\n\n"));
    }
    let out = out.trim().to_string();
    if out.is_empty() { None } else { Some(out) }
}

/// Résout un binaire externe : sidecar du bundle d'abord (app packagée,
/// binaire à côté de l'exécutable), puis Homebrew, puis le PATH.
pub(super) fn which_bin(name: &str) -> Option<String> {
    // Sidecar embarqué, en dev COMME en release : `build.rs` pose désormais
    // `target/Resources` → `src-tauri/resources`, donc les dylibs cherchées en
    // `@executable_path/../Resources/libs/` se résolvent aussi hors bundle. Avant
    // ça, ce bloc était sauté en debug et on retombait sur Homebrew : le poppler
    // testé en dev n'était pas celui livré aux utilisateurs (demande Liam,
    // 2026-08-03 : « ma version locale doit être = aux packages »).
    //
    // Conséquence assumée : un sidecar cassé ne se rattrape plus par Homebrew en
    // dev — c'est le but, l'échec devient visible sur la machine de dev plutôt que
    // chez l'utilisateur.
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
    crate::elog!("🔍 Extraction PDF : {name}");
    if let Some(text) = run_pdftotext(path) {
        crate::elog!("✅ pdftotext OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    // Fallback pur Rust (pdf-extract) : indispensable sur Windows où poppler
    // n'est pas embarqué — qualité moindre que pdftotext -layout mais universel.
    if let Some(text) = extract_pdf_rust(path) {
        crate::elog!("✅ pdf-extract OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    crate::elog!("⚠️ extraction texte vide — tentative OCR…");
    if let Some(text) = ocr_pdf(path) {
        crate::elog!("✅ OCR OK ({} chars)", text.len());
        return Some(post_process(text));
    }
    crate::elog!("❌ Extraction impossible : {name}");
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
    let out = crate::ai::llama::quiet_command(&bin)
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
    let _ = crate::ai::llama::quiet_command(&pdftoppm)
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
        let mut cmd = crate::ai::llama::quiet_command(&tesseract);
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
    if let Some(p) = selection_path() { let _ = std::fs::remove_file(p); }
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
mod selection_tests {
    use super::*;

    /// Arbre : racine(inconnue) → "travail" → "papiris" ; "perso" à côté.
    fn tables() -> (
        std::collections::HashMap<String, String>,
        std::collections::HashSet<String>,
    ) {
        let parents = [("travail", "root"), ("papiris", "travail"), ("perso", "root")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let known = ["travail", "papiris", "perso"]
            .into_iter()
            .map(str::to_string)
            .collect();
        (parents, known)
    }

    fn sel(folders: &[&str], orphans: bool) -> Selection {
        Selection {
            folders: folders.iter().map(|s| s.to_string()).collect(),
            include_orphans: orphans,
        }
    }

    #[test]
    fn selection_vide_prend_tout() {
        let (p, k) = tables();
        // Défaut des comptes déjà connectés : leur Drive ne doit pas disparaître.
        assert!(is_selected(&["perso".into()], &p, &k, &Selection::default()));
        assert!(is_selected(&[], &p, &k, &Selection::default()));
    }

    #[test]
    fn un_ancetre_coche_suffit() {
        let (p, k) = tables();
        let s = sel(&["travail"], false);
        assert!(is_selected(&["papiris".into()], &p, &k, &s), "sous-dossier hérité");
        assert!(is_selected(&["travail".into()], &p, &k, &s));
        assert!(!is_selected(&["perso".into()], &p, &k, &s));
    }

    #[test]
    fn les_orphelins_ont_leur_propre_case() {
        let (p, k) = tables();
        // Fichier à la racine du Drive : parent inconnu, aucune case à cocher.
        assert!(!is_selected(&["root".into()], &p, &k, &sel(&["travail"], false)));
        assert!(is_selected(&["root".into()], &p, &k, &sel(&["travail"], true)));
        // La case orphelins ne doit PAS repêcher un dossier connu non coché.
        assert!(!is_selected(&["perso".into()], &p, &k, &sel(&["travail"], true)));
    }

    #[test]
    fn cycle_de_parents_ne_boucle_pas() {
        let p = [("a", "b"), ("b", "a")]
            .into_iter()
            .map(|(x, y)| (x.to_string(), y.to_string()))
            .collect();
        let k = ["a", "b"].into_iter().map(str::to_string).collect();
        assert!(!is_selected(&["a".into()], &p, &k, &sel(&["autre"], false)));
    }
}

#[cfg(test)]
mod pptx_tests {
    use super::*;

    #[test]
    fn extrait_les_diapos_dans_lordre() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("lucid_test_pptx");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.pptx");
        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        // slide10 déclaré AVANT slide2 : le tri numérique doit remettre l'ordre.
        zw.start_file("ppt/slides/slide10.xml", opts).unwrap();
        zw.write_all(b"<p:sld><a:p><a:r><a:t>Conclusion</a:t></a:r></a:p></p:sld>").unwrap();
        zw.start_file("ppt/slides/slide2.xml", opts).unwrap();
        zw.write_all(b"<p:sld><a:p><a:r><a:t>Introduction &amp; plan</a:t></a:r></a:p></p:sld>").unwrap();
        zw.finish().unwrap();

        let md = pptx_to_markdown(&path).unwrap();
        assert!(md.contains("## Diapo 2"), "{md}");
        assert!(md.contains("Introduction & plan"), "entités décodées : {md}");
        let i2 = md.find("## Diapo 2").unwrap();
        let i10 = md.find("## Diapo 10").unwrap();
        assert!(i2 < i10, "diapo 2 avant diapo 10 : {md}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn refuse_un_pptx_sans_texte() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("lucid_test_pptx");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vide.pptx");
        let f = std::fs::File::create(&path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        zw.start_file("ppt/presentation.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zw.write_all(b"<p:presentation/>").unwrap();
        zw.finish().unwrap();

        assert!(pptx_to_markdown(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }
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
