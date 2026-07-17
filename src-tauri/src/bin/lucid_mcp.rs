//! Serveur MCP Lucid — expose le cerveau (brain.json) à n'importe quel client
//! MCP (Claude Desktop, Claude Code, …) via stdio (JSON-RPC 2.0, un message
//! par ligne). Tools : brain_overview, brain_search, brain_node.
//!
//! Lancement : `cargo run --bin lucid_mcp` (ou le binaire compilé).
//! Enregistrement côté Claude Code : `claude mcp add lucid <chemin du binaire>`.
//!
//! Binaire volontairement autonome : il relit brain.json à chaque appel
//! (données fraîches, fichier ~400 Ko) et définit son propre modèle tolérant
//! plutôt que d'importer la lib Tauri.

use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, Write};

const APP_DIR: &str = "fr.ideeri.brainlink"; // = ai::llama::APP_DIR

#[derive(Deserialize)]
struct Graph {
    #[serde(default)]
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
    #[serde(default)]
    content: String,
    /// Texte source embarqué par le pipeline (souvent le seul rempli : `content`
    /// n'existe que pour les notes éditées dans Lucid).
    #[serde(default)]
    source_text: String,
    #[serde(default)]
    parent_id: Option<String>,
}

/// Le corps lisible d'un nœud : la note éditée si elle existe, sinon le texte source.
fn body_of(n: &Node) -> &str {
    if n.content.is_empty() { &n.source_text } else { &n.content }
}

/// Même résolution que `ai::llama::app_data_dir` : `users/<uuid>/` si un compte
/// est connecté dans Lucid (fichier `active_user`), racine sinon (install legacy).
fn data_dir() -> Result<std::path::PathBuf, String> {
    let root = dirs::data_dir().ok_or("dossier de données introuvable")?.join(APP_DIR);
    Ok(match std::fs::read_to_string(root.join("active_user")) {
        Ok(id) if !id.trim().is_empty() => root.join("users").join(id.trim()),
        _ => root,
    })
}

fn load_graph() -> Result<Graph, String> {
    let path = data_dir()?.join("brain.json");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("brain.json illisible ({}) : {e}. Génère d'abord le cerveau dans Lucid.", path.display()))?;
    serde_json::from_str(&raw).map_err(|e| format!("brain.json invalide : {e}"))
}

/// Fil d'Ariane « Lucid › Projet › Page » d'un nœud.
fn path_of(n: &Node, graph: &Graph) -> String {
    let by_id: std::collections::HashMap<&str, &Node> =
        graph.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut parts = vec![];
    let mut cur = n.parent_id.as_deref();
    let mut guard = 0;
    while let Some(id) = cur {
        guard += 1;
        if guard > 12 { break; }
        match by_id.get(id) {
            Some(p) => { parts.push(p.label.clone()); cur = p.parent_id.as_deref(); }
            None => break,
        }
    }
    parts.reverse();
    parts.join(" › ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { return s.to_string(); }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}\n\n*[… tronqué]*")
}

// ── Tools ────────────────────────────────────────────────────────────────────

fn tool_overview() -> Result<String, String> {
    let g = load_graph()?;
    let root = g.nodes.iter().find(|n| n.kind == "root").ok_or("cerveau vide")?;
    let mut out = format!("# {} — vue d'ensemble du cerveau\n", root.label);
    for proj in g.nodes.iter().filter(|n| n.parent_id.as_deref() == Some(root.id.as_str())) {
        let child_count = g.nodes.iter().filter(|n| n.parent_id.as_deref() == Some(proj.id.as_str())).count();
        out.push_str(&format!("\n## {} (`{}`, {} sous-pages)\n", proj.label, proj.id, child_count));
        if !proj.summary.is_empty() {
            out.push_str(&format!("{}\n", proj.summary));
        }
        if !proj.keywords.is_empty() {
            out.push_str(&format!("Mots-clés : {}\n", proj.keywords.join(", ")));
        }
    }
    out.push_str("\nUtilise `brain_search` pour chercher un sujet, `brain_node` avec un id pour lire une page.\n");
    Ok(out)
}

fn tool_search(query: &str) -> Result<String, String> {
    let g = load_graph()?;
    let terms: Vec<String> = query.to_lowercase().split_whitespace().map(String::from).collect();
    if terms.is_empty() { return Err("query vide".into()); }

    let mut scored: Vec<(i64, &Node)> = g.nodes.iter().filter_map(|n| {
        let label = n.label.to_lowercase();
        let kw = n.keywords.join(" ").to_lowercase();
        let summary = n.summary.to_lowercase();
        let content = body_of(n).to_lowercase();
        let mut score = 0i64;
        for t in &terms {
            if label.contains(t) { score += 5; }
            if kw.contains(t) { score += 3; }
            if summary.contains(t) { score += 2; }
            if content.contains(t) { score += 1; }
        }
        (score > 0).then_some((score, n))
    }).collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));

    if scored.is_empty() {
        return Ok(format!("Aucun résultat pour « {query} »."));
    }
    let mut out = format!("Résultats pour « {query} » :\n");
    for (score, n) in scored.iter().take(8) {
        let path = path_of(n, &g);
        out.push_str(&format!(
            "\n- **{}** (`{}`, {}) — {}\n  {}\n",
            n.label, n.id, n.kind,
            if path.is_empty() { "racine".to_string() } else { path },
            truncate(if n.summary.is_empty() { body_of(n) } else { &n.summary }, 200).replace('\n', " "),
        ));
        let _ = score;
    }
    out.push_str("\nLis une page complète avec `brain_node`.\n");
    Ok(out)
}

fn tool_node(node_id: &str) -> Result<String, String> {
    let g = load_graph()?;
    let n = g.nodes.iter().find(|n| n.id == node_id)
        .ok_or_else(|| format!("nœud `{node_id}` introuvable (utilise brain_search pour trouver un id)"))?;
    let mut out = format!("# {}\n\n- id : `{}` · type : {}\n", n.label, n.id, n.kind);
    let path = path_of(n, &g);
    if !path.is_empty() { out.push_str(&format!("- chemin : {path}\n")); }
    if !n.keywords.is_empty() { out.push_str(&format!("- mots-clés : {}\n", n.keywords.join(", "))); }
    if !n.decisions.is_empty() {
        out.push_str("\n## Décisions\n");
        for d in &n.decisions { out.push_str(&format!("- {d}\n")); }
    }
    if !n.summary.is_empty() { out.push_str(&format!("\n## Résumé\n{}\n", n.summary)); }
    // 24 000 caractères ≈ 6 300 tokens : assez pour travailler, sans exploser le
    // contexte de l'IA cliente sur un gros PDF. ponytail: plafond fixe — passer
    // à un paramètre max_chars si un client a besoin du texte intégral.
    let body = body_of(n);
    if !body.is_empty() { out.push_str(&format!("\n## Contenu\n{}\n", truncate(body, 24_000))); }
    let children: Vec<&Node> = g.nodes.iter()
        .filter(|c| c.parent_id.as_deref() == Some(n.id.as_str())).collect();
    if !children.is_empty() {
        out.push_str("\n## Sous-pages\n");
        for c in children { out.push_str(&format!("- {} (`{}`)\n", c.label, c.id)); }
    }
    Ok(out)
}

/// Propose une note. N'écrit JAMAIS dans brain.json : dépose une proposition
/// dans `mcp_pending/` que l'utilisateur valide (ou refuse) dans Lucid.
/// L'id renvoyé peut servir de parent_id pour construire un arbre.
fn tool_add_note(parent_id: &str, label: &str, content: &str) -> Result<String, String> {
    if label.trim().is_empty() { return Err("label vide".into()); }
    let pending_dir = data_dir()?.join("mcp_pending");
    std::fs::create_dir_all(&pending_dir).map_err(|e| e.to_string())?;

    // Le parent doit exister dans le graphe… ou être une proposition en attente.
    let g = load_graph()?;
    let in_graph = g.nodes.iter().any(|n| n.id == parent_id);
    let in_pending = pending_dir.join(format!("{parent_id}.json")).exists();
    if !in_graph && !in_pending {
        return Err(format!(
            "parent `{parent_id}` introuvable — utilise brain_overview/brain_search pour trouver une page, ou l'id d'une proposition précédente"
        ));
    }

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let id = format!("mcp-{nanos}");
    let proposal = json!({
        "id": id,
        "parent_id": parent_id,
        "label": label.trim(),
        "content": content,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(pending_dir.join(format!("{id}.json")), proposal.to_string())
        .map_err(|e| e.to_string())?;
    Ok(format!(
        "Proposition `{id}` déposée — elle apparaît dans Lucid et attend la validation de l'utilisateur. \
         Pour créer une sous-page de celle-ci, rappelle brain_add_note avec parent_id=\"{id}\"."
    ))
}

// ── Protocole MCP (JSON-RPC 2.0 sur stdio) ──────────────────────────────────

fn tools_list() -> Value {
    json!([
        {
            "name": "brain_overview",
            "description": "Vue d'ensemble du second cerveau de l'utilisateur (projets, résumés, mots-clés). À appeler en premier pour découvrir ce que contient le cerveau.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            // readOnlyHint → Claude Desktop autorise « Toujours autoriser » (lecture seule, sans effet de bord).
            "annotations": { "title": "Vue d'ensemble", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "brain_search",
            "description": "Recherche dans le second cerveau de l'utilisateur (notes, projets, concepts, documents importés). Renvoie les pages les plus pertinentes avec leur id.",
            "inputSchema": {
                "type": "object",
                "properties": { "query": { "type": "string", "description": "Termes de recherche" } },
                "required": ["query"],
                "additionalProperties": false
            },
            "annotations": { "title": "Recherche", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "brain_node",
            "description": "Lit une page complète du cerveau (résumé, décisions, contenu markdown, sous-pages) à partir de son id.",
            "inputSchema": {
                "type": "object",
                "properties": { "node_id": { "type": "string", "description": "Id du nœud (ex. p:brainlink, note-123…)" } },
                "required": ["node_id"],
                "additionalProperties": false
            },
            "annotations": { "title": "Lire une page", "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "brain_add_note",
            "description": "Propose la création d'une page/note dans le second cerveau de l'utilisateur. La proposition apparaît dans Lucid où l'utilisateur doit la VALIDER avant qu'elle soit réellement créée. L'id renvoyé peut être utilisé comme parent_id pour proposer des sous-pages (arbre).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "parent_id": { "type": "string", "description": "Id de la page parente (via brain_search/brain_overview), ou id d'une proposition précédente pour une sous-page" },
                    "label": { "type": "string", "description": "Titre de la page" },
                    "content": { "type": "string", "description": "Contenu markdown de la page (optionnel)" }
                },
                "required": ["parent_id", "label"],
                "additionalProperties": false
            },
            // Écriture (proposition à valider dans Lucid) → PAS readOnly : Claude Desktop
            // continue de demander une confirmation, mais l'action n'est pas destructive.
            "annotations": { "title": "Proposer une page", "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        }
    ])
}

fn handle_tool_call(name: &str, args: &Value) -> Result<String, String> {
    match name {
        "brain_overview" => tool_overview(),
        "brain_search" => tool_search(args.get("query").and_then(Value::as_str).unwrap_or("")),
        "brain_node" => tool_node(args.get("node_id").and_then(Value::as_str).unwrap_or("")),
        "brain_add_note" => tool_add_note(
            args.get("parent_id").and_then(Value::as_str).unwrap_or(""),
            args.get("label").and_then(Value::as_str).unwrap_or(""),
            args.get("content").and_then(Value::as_str).unwrap_or(""),
        ),
        other => Err(format!("tool inconnu : {other}")),
    }
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    eprintln!("lucid-mcp : serveur démarré (stdio)");

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() { continue; }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            eprintln!("lucid-mcp : message illisible ignoré");
            continue;
        };
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");

        // Notifications (pas d'id) : rien à répondre.
        if id.is_none() || id == Some(Value::Null) { continue; }
        let id = id.unwrap();

        let response = match method {
            "initialize" => {
                let proto = msg.pointer("/params/protocolVersion")
                    .and_then(Value::as_str).unwrap_or("2024-11-05");
                json!({ "jsonrpc": "2.0", "id": id, "result": {
                    "protocolVersion": proto,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "lucid-brain", "version": env!("CARGO_PKG_VERSION") }
                }})
            }
            "ping" => json!({ "jsonrpc": "2.0", "id": id, "result": {} }),
            "tools/list" => json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": tools_list() } }),
            "tools/call" => {
                let name = msg.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
                let default_args = json!({});
                let args = msg.pointer("/params/arguments").unwrap_or(&default_args);
                match handle_tool_call(name, args) {
                    Ok(text) => json!({ "jsonrpc": "2.0", "id": id, "result": {
                        "content": [{ "type": "text", "text": text }]
                    }}),
                    Err(e) => json!({ "jsonrpc": "2.0", "id": id, "result": {
                        "content": [{ "type": "text", "text": format!("Erreur : {e}") }],
                        "isError": true
                    }}),
                }
            }
            other => json!({ "jsonrpc": "2.0", "id": id, "error": {
                "code": -32601, "message": format!("méthode non supportée : {other}")
            }}),
        };

        let mut out = stdout.lock();
        if writeln!(out, "{response}").and_then(|_| out.flush()).is_err() { break; }
    }
    eprintln!("lucid-mcp : arrêt");
}
