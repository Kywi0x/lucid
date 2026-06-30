//! Connecteur Notion.
//! Auth : Integration Token (secret_xxx) — pas d'OAuth nécessaire.
//! Cache : notion_cache.json dans app_data_dir.

use crate::models::{Conversation, ConversationSummary, Message};
use serde::{Deserialize, Serialize};

// ─── Token ───────────────────────────────────────────────────────────────────

fn token_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("notion_token.json"))
}

fn cache_path() -> Option<std::path::PathBuf> {
    crate::ai::llama::app_data_dir().map(|d| d.join("notion_cache.json"))
}

#[derive(Serialize, Deserialize)]
struct TokenFile {
    token: String,
}

pub fn save_token(token: &str) -> Result<(), String> {
    let path = token_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(
        path,
        serde_json::to_string(&TokenFile { token: token.to_string() }).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// Valide le token en appelant l'API et renvoie le nom de l'integration.
pub fn validate_token(token: &str) -> Result<String, String> {
    let c = client(token);
    let v: serde_json::Value = c
        .get("https://api.notion.com/v1/users/me")
        .send()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())?;

    if v.get("status").and_then(|s| s.as_u64()).is_some() {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("Token invalide");
        return Err(format!("Notion : {msg}"));
    }

    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("integration").to_string();
    Ok(name)
}

fn load_token() -> Option<String> {
    let raw = std::fs::read_to_string(token_path()?).ok()?;
    serde_json::from_str::<TokenFile>(&raw).ok().map(|f| f.token)
}

pub fn is_connected() -> bool {
    load_token().is_some()
}

pub fn disconnect() {
    if let Some(p) = token_path() { let _ = std::fs::remove_file(p); }
    if let Some(p) = cache_path() { let _ = std::fs::remove_file(p); }
}

// ─── API types ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchResp {
    results: Vec<PageObj>,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct PageObj {
    id: String,
    created_time: String,
    last_edited_time: String,
    parent: ParentObj,
    properties: serde_json::Value,
}

#[derive(Deserialize)]
struct ParentObj {
    #[serde(rename = "type")]
    kind: String,
    page_id: Option<String>,
    database_id: Option<String>,
}

#[derive(Deserialize)]
struct BlocksResp {
    results: Vec<BlockObj>,
    has_more: bool,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct BlockObj {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(flatten)]
    data: serde_json::Value,
}

// ─── Client helper ────────────────────────────────────────────────────────────

fn client(token: &str) -> reqwest::blocking::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        "Notion-Version",
        reqwest::header::HeaderValue::from_static("2022-06-28"),
    );
    headers.insert(
        reqwest::header::AUTHORIZATION,
        reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
    );
    reqwest::blocking::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap()
}

// ─── Text extraction ──────────────────────────────────────────────────────────

fn rich_text(v: &serde_json::Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t.get("plain_text")?.as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn block_to_text(b: &BlockObj) -> Option<String> {
    let d = &b.data;
    match b.kind.as_str() {
        "paragraph" => {
            let t = rich_text(d.get("paragraph")?.get("rich_text")?);
            if t.is_empty() { None } else { Some(t) }
        }
        "heading_1" => Some(format!("# {}", rich_text(d.get("heading_1")?.get("rich_text")?))),
        "heading_2" => Some(format!("## {}", rich_text(d.get("heading_2")?.get("rich_text")?))),
        "heading_3" => Some(format!("### {}", rich_text(d.get("heading_3")?.get("rich_text")?))),
        "bulleted_list_item" => Some(format!("• {}", rich_text(d.get("bulleted_list_item")?.get("rich_text")?))),
        "numbered_list_item" => Some(format!("1. {}", rich_text(d.get("numbered_list_item")?.get("rich_text")?))),
        "to_do" => {
            let done = d.get("to_do")?.get("checked")?.as_bool().unwrap_or(false);
            Some(format!("[{}] {}", if done { "x" } else { " " }, rich_text(d.get("to_do")?.get("rich_text")?)))
        }
        "quote"   => Some(format!("> {}", rich_text(d.get("quote")?.get("rich_text")?))),
        "callout" => Some(rich_text(d.get("callout")?.get("rich_text")?)),
        "toggle"  => Some(rich_text(d.get("toggle")?.get("rich_text")?)),
        "code" => {
            let lang = d.get("code")?.get("language")?.as_str().unwrap_or("");
            let t = rich_text(d.get("code")?.get("rich_text")?);
            Some(format!("```{lang}\n{t}\n```"))
        }
        "divider"    => Some("---".into()),
        "child_page" => Some(format!("📄 {}", d.get("child_page")?.get("title")?.as_str().unwrap_or(""))),
        "image"      => Some("[image]".into()),
        "video"      => Some("[vidéo]".into()),
        "embed"      => Some("[embed]".into()),
        "file"       => Some("[fichier]".into()),
        _ => None,
    }
}

fn prop_to_text(v: &serde_json::Value) -> String {
    let null = serde_json::Value::Null;
    match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
        "title"        => rich_text(v.get("title").unwrap_or(&null)),
        "rich_text"    => rich_text(v.get("rich_text").unwrap_or(&null)),
        "number"       => v.get("number").map(|n| n.to_string()).unwrap_or_default(),
        "select"       => v.get("select").and_then(|s| s.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string(),
        "multi_select" => v.get("multi_select").and_then(|a| a.as_array())
            .map(|a| a.iter().filter_map(|s| s.get("name")?.as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default(),
        "date"         => v.get("date").and_then(|d| d.get("start")).and_then(|s| s.as_str()).unwrap_or("").to_string(),
        "checkbox"     => v.get("checkbox").and_then(|b| b.as_bool()).map(|b| if b { "✓" } else { "✗" }).unwrap_or("").to_string(),
        "url"          => v.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string(),
        "email"        => v.get("email").and_then(|e| e.as_str()).unwrap_or("").to_string(),
        "phone_number" => v.get("phone_number").and_then(|p| p.as_str()).unwrap_or("").to_string(),
        "formula" => {
            let f = v.get("formula").unwrap_or(&null);
            match f.get("type").and_then(|t| t.as_str()) {
                Some("string")  => f.get("string").and_then(|s| s.as_str()).unwrap_or("").to_string(),
                Some("number")  => f.get("number").map(|n| n.to_string()).unwrap_or_default(),
                Some("boolean") => f.get("boolean").and_then(|b| b.as_bool()).map(|b| if b { "✓" } else { "✗" }).unwrap_or("").to_string(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

fn fetch_db_content(c: &reqwest::blocking::Client, db_id: &str, db_title: &str) -> String {
    let url = format!("https://api.notion.com/v1/databases/{db_id}/query");
    let v: serde_json::Value = match c.post(&url)
        .json(&serde_json::json!({"page_size": 50}))
        .send().ok().and_then(|r| r.json().ok())
    {
        Some(v) => v,
        None => return format!("🗄 {db_title}"),
    };

    let results = match v.get("results").and_then(|r| r.as_array()) {
        Some(r) if !r.is_empty() => r,
        _ => return format!("🗄 {db_title} (vide)"),
    };

    let first_props = match results[0].get("properties").and_then(|p| p.as_object()) {
        Some(p) => p,
        None => return format!("🗄 {db_title}"),
    };

    let columns: Vec<String> = first_props.keys().cloned().collect();
    let sep = columns.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");

    let mut table = vec![
        format!("**{}**", db_title),
        format!("| {} |", columns.join(" | ")),
        format!("| {} |", sep),
    ];

    for row in results.iter().take(20) {
        let row_id = row.get("id").and_then(|id| id.as_str()).unwrap_or_default();
        if let Some(props) = row.get("properties").and_then(|p| p.as_object()) {
            let null = serde_json::Value::Null;
            let cells: Vec<String> = columns.iter()
                .map(|col| {
                    let v = props.get(col).unwrap_or(&null);
                    let text = prop_to_text(v).replace('|', "\\|");
                    // Titre de la ligne → lien cliquable vers la page de l'entrée
                    let is_title = v.get("type").and_then(|t| t.as_str()) == Some("title");
                    if is_title && !text.is_empty() && !row_id.is_empty() {
                        format!("[{}](#notion-{})", text, row_id)
                    } else {
                        text
                    }
                })
                .collect();
            table.push(format!("| {} |", cells.join(" | ")));
        }
    }

    table.join("\n")
}

/// `depth` contrôle la récursion dans les sous-pages :
/// - 0 = contenu immédiat uniquement (child_page → titre seulement)
/// - 1 = fetch des child_page à 1 niveau de profondeur
/// Les child_database sont toujours fetchés (pas de récursion, juste les lignes).
fn fetch_blocks(c: &reqwest::blocking::Client, page_id: &str, depth: u8) -> String {
    let mut parts = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let url = format!("https://api.notion.com/v1/blocks/{page_id}/children?page_size=100");
        let mut req = c.get(&url);
        if let Some(ref cur) = cursor { req = req.query(&[("start_cursor", cur.as_str())]); }
        let resp: BlocksResp = match req.send().and_then(|r| r.json()) {
            Ok(r) => r, Err(_) => break,
        };
        for b in &resp.results {
            match b.kind.as_str() {
                "child_database" => {
                    let title = b.data.get("child_database")
                        .and_then(|d| d.get("title"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("Base de données")
                        .to_string();
                    parts.push(fetch_db_content(c, &b.id, &title));
                }
                // Table simple Notion (≠ child_database) : les lignes sont des blocs enfants
                "table" => {
                    let has_header = b.data.get("table")
                        .and_then(|t| t.get("has_column_header"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let rows_url = format!("https://api.notion.com/v1/blocks/{}/children?page_size=100", b.id);
                    if let Ok(rows) = c.get(&rows_url).send().and_then(|r| r.json::<BlocksResp>()) {
                        let mut lines: Vec<String> = vec![];
                        for (i, row) in rows.results.iter().enumerate() {
                            if row.kind != "table_row" { continue; }
                            let cells: Vec<String> = row.data.get("table_row")
                                .and_then(|tr| tr.get("cells"))
                                .and_then(|c| c.as_array())
                                .map(|arr| arr.iter()
                                    .map(|cell| rich_text(cell).replace('|', "\\|"))
                                    .collect())
                                .unwrap_or_default();
                            if cells.is_empty() { continue; }
                            lines.push(format!("| {} |", cells.join(" | ")));
                            if i == 0 && has_header {
                                let sep = cells.iter().map(|_| "---").collect::<Vec<_>>().join(" | ");
                                lines.push(format!("| {} |", sep));
                            }
                        }
                        if !lines.is_empty() { parts.push(lines.join("\n")); }
                    }
                }
                "child_page" => {
                    let title = b.data.get("child_page")
                        .and_then(|d| d.get("title"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("Page")
                        .to_string();
                    if depth > 0 {
                        let content = fetch_blocks(c, &b.id, depth - 1);
                        if content.trim().is_empty() {
                            parts.push(format!("📄 {title}"));
                        } else {
                            parts.push(format!("### {title}\n{content}"));
                        }
                    } else {
                        parts.push(format!("📄 {title}"));
                    }
                }
                _ => {
                    if let Some(t) = block_to_text(b) { parts.push(t); }
                }
            }
        }
        if resp.has_more { cursor = resp.next_cursor; } else { break; }
    }
    parts.join("\n")
}

fn extract_title(props: &serde_json::Value) -> String {
    props.as_object()
        .and_then(|obj| {
            obj.values().find_map(|v| {
                let arr = v.get("title")?.as_array()?;
                let t: String = arr.iter()
                    .filter_map(|t| t.get("plain_text")?.as_str())
                    .collect();
                if t.is_empty() { None } else { Some(t) }
            })
        })
        .unwrap_or_else(|| "Sans titre".into())
}

fn fetch_page_title(
    c: &reqwest::blocking::Client,
    id: &str,
    cache: &mut std::collections::HashMap<String, String>,
) -> String {
    if let Some(t) = cache.get(id) { return t.clone(); }
    let v: serde_json::Value = c
        .get(format!("https://api.notion.com/v1/pages/{id}"))
        .send().ok().and_then(|r| r.json().ok()).unwrap_or_default();
    let title = extract_title(&v["properties"]);
    cache.insert(id.to_string(), title.clone());
    title
}


/// Renvoie (parent_page_id, database_title) pour une base de données.
/// parent_page_id = None si la DB est au niveau workspace ou inaccessible.
fn fetch_db_info(
    c: &reqwest::blocking::Client,
    db_id: &str,
    cache: &mut std::collections::HashMap<String, (Option<String>, String)>,
) -> (Option<String>, String) {
    if let Some(r) = cache.get(db_id) { return r.clone(); }
    let v: serde_json::Value = c
        .get(format!("https://api.notion.com/v1/databases/{db_id}"))
        .send().ok().and_then(|r| r.json().ok()).unwrap_or_default();
    let page_id = v["parent"]["page_id"].as_str().map(|s| s.to_string());
    let title = v.get("title")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter()
            .filter_map(|t| t.get("plain_text")?.as_str())
            .collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Base de données".to_string());
    let result = (page_id, title);
    cache.insert(db_id.to_string(), result.clone());
    result
}

// ─── Sync ─────────────────────────────────────────────────────────────────────

pub fn sync() -> Result<(usize, usize), String> {
    let token = load_token().ok_or("Non connecté à Notion.")?;
    let c = client(&token);

    // Pages existantes en cache (pour compter les nouvelles)
    let existing: std::collections::HashSet<String> =
        load_conversations().iter().map(|cv| cv.summary.id.clone()).collect();

    // Pagination sur /search
    let mut pages: Vec<PageObj> = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut body = serde_json::json!({
            "filter": {"value": "page", "property": "object"},
            "page_size": 100,
        });
        if let Some(ref cur) = cursor { body["start_cursor"] = serde_json::Value::String(cur.clone()); }
        let raw: serde_json::Value = c
            .post("https://api.notion.com/v1/search")
            .json(&body)
            .send()
            .map_err(|e| e.to_string())?
            .json()
            .map_err(|e| e.to_string())?;

        // Détecte les erreurs API (ex. token révoqué)
        if raw.get("status").and_then(|s| s.as_u64()).is_some() {
            let msg = raw.get("message").and_then(|m| m.as_str()).unwrap_or("Erreur API");
            return Err(format!("Notion : {msg}"));
        }

        let resp: SearchResp = serde_json::from_value(raw).map_err(|e| e.to_string())?;
        pages.extend(resp.results);
        if resp.has_more { cursor = resp.next_cursor; } else { break; }
    }

    // Cap à 200 pages pour éviter les syncs infinies sur de gros workspaces
    pages.truncate(200);

    let total = pages.len();
    if total == 0 {
        return Ok((0, 0));
    }

    let mut conversations: Vec<Conversation> = Vec::new();
    let mut title_cache: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // db_id → (parent_page_id, db_title)
    let mut db_info_cache: std::collections::HashMap<String, (Option<String>, String)> = std::collections::HashMap::new();

    for page in pages {
        let title = extract_title(&page.properties);
        title_cache.insert(page.id.clone(), title.clone());

        let container_path = match page.parent.kind.as_str() {
            "workspace" => vec![title.clone()],
            "page_id" => page.parent.page_id.as_deref()
                .map(|id| {
                    let parent = fetch_page_title(&c, id, &mut title_cache);
                    vec![parent, title.clone()]
                })
                .unwrap_or_else(|| vec![title.clone()]),
            // Ligne de DB → feuille directe sous la page parente (pas de sous-conteneur par ligne).
            // Fallback sur le titre de la DB si la page parente est inaccessible ou au niveau workspace.
            "database_id" => {
                if let Some(db_id) = page.parent.database_id.as_deref() {
                    let (parent_page_id, db_title) = fetch_db_info(&c, db_id, &mut db_info_cache);
                    match parent_page_id {
                        Some(parent_id) => {
                            let grandparent = fetch_page_title(&c, &parent_id, &mut title_cache);
                            if grandparent.is_empty() || grandparent == "Sans titre" {
                                vec![db_title]
                            } else {
                                vec![grandparent]
                            }
                        }
                        None => vec![db_title],
                    }
                } else {
                    vec![]
                }
            }
            _ => vec![title.clone()],
        };

        let blocks = fetch_blocks(&c, &page.id, 1);

        // Toujours inclure le titre + chemin comme contexte minimum pour le LLM
        let context = if container_path.is_empty() {
            format!("# {title}")
        } else {
            format!("# {title}\nCatégorie : {}", container_path.join(" > "))
        };
        let text = if blocks.trim().is_empty() {
            context
        } else {
            format!("{context}\n\n{blocks}")
        };

        conversations.push(Conversation {
            summary: ConversationSummary {
                id: page.id.clone(),
                title,
                project: "notion".into(),
                project_slug: "notion".into(),
                source: "notion".into(),
                container_path,
                message_count: 1,
                first_timestamp: Some(page.created_time),
                last_timestamp:  Some(page.last_edited_time.clone()),
            },
            messages: vec![Message {
                role: "assistant".into(),
                text,
                timestamp: Some(page.last_edited_time),
            }],
        });
    }

    let new_count = conversations.iter().filter(|cv| !existing.contains(&cv.summary.id)).count();

    let cache = cache_path().ok_or("Dossier de données introuvable.")?;
    std::fs::write(cache, serde_json::to_string_pretty(&conversations).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;

    Ok((new_count, total))
}

/// Fetch on-demand le contenu d'une page par ID (pour les liens notion:ID dans l'UI).
pub fn load_page(id: &str) -> Result<String, String> {
    let token = load_token().ok_or("Non connecté à Notion.")?;
    let c = client(&token);
    Ok(fetch_blocks(&c, id, 1))
}

// ─── Cache read ───────────────────────────────────────────────────────────────

pub fn load_conversations() -> Vec<Conversation> {
    let path = match cache_path() { Some(p) => p, None => return vec![] };
    let raw  = match std::fs::read_to_string(path) { Ok(r) => r, Err(_) => return vec![] };
    serde_json::from_str::<Vec<Conversation>>(&raw).unwrap_or_default()
}

pub fn load_by_id(id: &str) -> Option<Conversation> {
    // Cache uniquement — les appels API sont réservés au sync.
    load_conversations().into_iter().find(|cv| cv.summary.id == id)
}
