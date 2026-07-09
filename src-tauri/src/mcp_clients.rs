//! Connexion one-click du serveur MCP Lucid aux clients IA installés
//! (Claude Desktop, Claude Code, Cursor) : on écrit l'entrée `mcpServers.lucid`
//! dans la config de chaque client, avec backup préalable.

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Serialize, Clone)]
pub struct AiClientStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub connected: bool,
    /// false = le client n'accepte pas de serveur MCP local (ex. ChatGPT :
    /// OpenAI n'autorise que des serveurs distants HTTPS). L'UI l'explique.
    pub supported: bool,
}

/// Binaire lucid_mcp : à côté de l'exécutable courant
/// (dev : target/debug/ ; app packagée : Contents/MacOS/ en sidecar).
pub fn mcp_binary_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let p = exe.parent().ok_or("exécutable sans dossier")?
        .join(format!("lucid_mcp{}", std::env::consts::EXE_SUFFIX));
    if p.exists() {
        return Ok(p);
    }
    Err("Binaire lucid_mcp introuvable à côté de l'app (cargo build --bin lucid_mcp).".into())
}

// ─── Descripteurs de clients ────────────────────────────────────────────────

struct ClientDesc {
    id: &'static str,
    name: &'static str,
    /// Message affiché après connexion (ou raison si `supported: false`).
    hint: &'static str,
    /// false = pas de connexion MCP locale possible (voir AiClientStatus).
    supported: bool,
}

const CLIENTS: &[ClientDesc] = &[
    ClientDesc { id: "claude-desktop", name: "Claude Desktop", hint: "Redémarre Claude Desktop (quitte complètement puis relance).", supported: true },
    ClientDesc { id: "claude-code",    name: "Claude Code",    hint: "Ouvre une nouvelle session `claude` dans un terminal.", supported: true },
    ClientDesc { id: "chatgpt",        name: "ChatGPT",        hint: "ChatGPT n'accepte que des serveurs MCP distants (HTTPS) — impossible de brancher un serveur local sans envoyer tes données en ligne. En attendant : importe ton export ZIP ChatGPT dans Sources.", supported: false },
    ClientDesc { id: "cursor",         name: "Cursor",         hint: "Redémarre Cursor.", supported: true },
    ClientDesc { id: "codex",          name: "Codex (OpenAI)", hint: "Redémarre Codex (app ou CLI).", supported: true },
];

fn config_path(id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    match id {
        // config_dir = ~/Library/Application Support (Mac) / %APPDATA% Roaming (Windows)
        "claude-desktop" => Some(dirs::config_dir()?.join("Claude").join("claude_desktop_config.json")),
        "claude-code"    => Some(home.join(".claude.json")),
        "cursor"         => Some(home.join(".cursor/mcp.json")),
        "codex"          => Some(home.join(".codex/config.toml")),
        _ => None,
    }
}

fn is_installed(id: &str) -> bool {
    let app = |p: &str| Path::new(p).exists();
    // data_local_dir = %LOCALAPPDATA% sur Windows (dossier d'install des apps Electron)
    let local = |sub: &str| dirs::data_local_dir().is_some_and(|d| d.join(sub).exists());
    match id {
        "claude-desktop" => app("/Applications/Claude.app")
            || local("AnthropicClaude")
            || config_path(id).is_some_and(|p| p.exists()),
        "claude-code"    => config_path(id).is_some_and(|p| p.exists()),
        "chatgpt"        => app("/Applications/ChatGPT.app") || local("Programs/ChatGPT"),
        "cursor"         => app("/Applications/Cursor.app")
            || local("Programs/cursor")
            || dirs::home_dir().is_some_and(|h| h.join(".cursor").exists()),
        "codex"          => dirs::home_dir().is_some_and(|h| h.join(".codex").exists()),
        _ => false,
    }
}

/// L'entrée MCP à écrire, par client (formats légèrement différents).
fn entry_for(id: &str, bin: &Path) -> Value {
    let cmd = bin.to_string_lossy();
    match id {
        "claude-code" => json!({ "type": "stdio", "command": cmd, "args": [], "env": {} }),
        _ => json!({ "command": cmd }),
    }
}

// ─── Lecture / écriture de config (injectable pour les tests) ──────────────

fn has_lucid(cfg: &Path) -> bool {
    std::fs::read_to_string(cfg).ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .map(|v| v.pointer("/mcpServers/lucid").is_some())
        .unwrap_or(false)
}

/// Insère (ou remplace) `mcpServers.lucid` dans la config, backup préalable,
/// en préservant tout le reste du fichier.
fn merge_mcp_entry(cfg: &Path, entry: Value) -> Result<(), String> {
    let mut root: Value = if cfg.exists() {
        serde_json::from_str(&std::fs::read_to_string(cfg).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Config illisible ({}) : {e}", cfg.display()))?
    } else {
        json!({})
    };
    let obj = root.as_object_mut().ok_or("Config inattendue (pas un objet JSON).")?;

    if cfg.exists() {
        std::fs::copy(cfg, cfg.with_extension("json.bak-lucid")).map_err(|e| e.to_string())?;
    } else if let Some(parent) = cfg.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    obj.entry("mcpServers")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or("`mcpServers` n'est pas un objet.")?
        .insert("lucid".into(), entry);

    std::fs::write(cfg, serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn remove_mcp_entry(cfg: &Path) -> Result<(), String> {
    if !cfg.exists() { return Ok(()); }
    let mut root: Value = serde_json::from_str(&std::fs::read_to_string(cfg).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if let Some(servers) = root.pointer_mut("/mcpServers").and_then(Value::as_object_mut) {
        if servers.remove("lucid").is_some() {
            std::fs::copy(cfg, cfg.with_extension("json.bak-lucid")).map_err(|e| e.to_string())?;
            std::fs::write(cfg, serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

// ─── Config TOML (Codex) ────────────────────────────────────────────────────
// ponytail: manipulation ligne à ligne, pas de crate toml — suffisant pour
// ajouter/retirer NOTRE section ; on ne touche à rien d'autre.

fn toml_has_lucid(cfg: &Path) -> bool {
    std::fs::read_to_string(cfg).map(|s| s.contains("[mcp_servers.lucid]")).unwrap_or(false)
}

/// Retire la section `[mcp_servers.lucid]` (et ses sous-sections) du TOML.
fn toml_strip_lucid(content: &str) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in content.lines() {
        let t = line.trim();
        if t == "[mcp_servers.lucid]" || t.starts_with("[mcp_servers.lucid.") {
            skipping = true;
            continue;
        }
        if skipping && t.starts_with('[') { skipping = false; }
        if !skipping { out.push(line); }
    }
    let mut s = out.join("\n");
    if !s.ends_with('\n') { s.push('\n'); }
    s
}

fn toml_add_lucid(cfg: &Path, bin: &Path) -> Result<(), String> {
    let mut content = if cfg.exists() {
        std::fs::copy(cfg, cfg.with_extension("toml.bak-lucid")).map_err(|e| e.to_string())?;
        toml_strip_lucid(&std::fs::read_to_string(cfg).map_err(|e| e.to_string())?)
    } else {
        if let Some(p) = cfg.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        String::new()
    };
    content.push_str(&format!("\n[mcp_servers.lucid]\ncommand = \"{}\"\nargs = []\n", bin.display()));
    std::fs::write(cfg, content).map_err(|e| e.to_string())
}

fn toml_remove_lucid(cfg: &Path) -> Result<(), String> {
    if !cfg.exists() || !toml_has_lucid(cfg) { return Ok(()); }
    std::fs::copy(cfg, cfg.with_extension("toml.bak-lucid")).map_err(|e| e.to_string())?;
    let stripped = toml_strip_lucid(&std::fs::read_to_string(cfg).map_err(|e| e.to_string())?);
    std::fs::write(cfg, stripped).map_err(|e| e.to_string())
}

// ─── API appelée par les commandes Tauri ────────────────────────────────────

pub fn status() -> Vec<AiClientStatus> {
    CLIENTS.iter().map(|c| AiClientStatus {
        id: c.id.into(),
        name: c.name.into(),
        installed: is_installed(c.id),
        connected: config_path(c.id).is_some_and(|p| {
            if c.id == "codex" { toml_has_lucid(&p) } else { has_lucid(&p) }
        }),
        supported: c.supported,
    }).collect()
}

pub fn connect(id: &str) -> Result<String, String> {
    let desc = CLIENTS.iter().find(|c| c.id == id).ok_or_else(|| format!("Client inconnu : {id}"))?;
    if !desc.supported {
        return Err(desc.hint.into());
    }
    let bin = mcp_binary_path()?;
    let cfg = config_path(id).ok_or("Dossier utilisateur introuvable.")?;
    if id == "codex" {
        toml_add_lucid(&cfg, &bin)?;
    } else {
        merge_mcp_entry(&cfg, entry_for(id, &bin))?;
    }
    Ok(format!("Connecté ✓ — {}", desc.hint))
}

pub fn disconnect(id: &str) -> Result<(), String> {
    let cfg = config_path(id).ok_or("Dossier utilisateur introuvable.")?;
    if id == "codex" { toml_remove_lucid(&cfg) } else { remove_mcp_entry(&cfg) }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("brainlink_test_mcpcli_{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("config.json")
    }

    #[test]
    fn merge_preserve_le_reste_de_la_config() {
        let cfg = tmp("merge");
        std::fs::write(&cfg, r#"{"preferences":{"theme":"dark"},"mcpServers":{"autre":{"command":"x"}}}"#).unwrap();
        merge_mcp_entry(&cfg, json!({"command": "/bin/lucid_mcp"})).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(v.pointer("/preferences/theme").unwrap(), "dark");
        assert_eq!(v.pointer("/mcpServers/autre/command").unwrap(), "x");
        assert_eq!(v.pointer("/mcpServers/lucid/command").unwrap(), "/bin/lucid_mcp");
        assert!(cfg.with_extension("json.bak-lucid").exists()); // backup créé
        assert!(has_lucid(&cfg));
    }

    #[test]
    fn toml_codex_ajoute_et_retire_sans_toucher_le_reste() {
        let cfg = tmp("toml").with_extension("toml");
        std::fs::write(&cfg, "notify = [\"x\"]\n\n[mcp_servers.figma]\nurl = \"https://mcp.figma.com/mcp\"\n\n[mcp_servers.node_repl]\ncommand = \"/bin/node_repl\"\n\n[mcp_servers.node_repl.env]\nCODEX_HOME = \"/tmp\"\n").unwrap();
        toml_add_lucid(&cfg, Path::new("/bin/lucid_mcp")).unwrap();
        let s = std::fs::read_to_string(&cfg).unwrap();
        assert!(s.contains("[mcp_servers.lucid]"));
        assert!(s.contains("command = \"/bin/lucid_mcp\""));
        assert!(s.contains("[mcp_servers.figma]") && s.contains("CODEX_HOME"));
        assert!(toml_has_lucid(&cfg));
        // Ré-ajout : pas de doublon (la section est remplacée)
        toml_add_lucid(&cfg, Path::new("/bin/lucid_mcp2")).unwrap();
        let s = std::fs::read_to_string(&cfg).unwrap();
        assert_eq!(s.matches("[mcp_servers.lucid]").count(), 1);
        assert!(s.contains("lucid_mcp2"));
        // Retrait : la section part, le reste demeure
        toml_remove_lucid(&cfg).unwrap();
        let s = std::fs::read_to_string(&cfg).unwrap();
        assert!(!s.contains("lucid"));
        assert!(s.contains("[mcp_servers.node_repl]") && s.contains("notify"));
    }

    #[test]
    fn merge_cree_le_fichier_si_absent_et_remove_nettoie() {
        let cfg = tmp("create");
        merge_mcp_entry(&cfg, json!({"command": "/bin/lucid_mcp"})).unwrap();
        assert!(has_lucid(&cfg));
        remove_mcp_entry(&cfg).unwrap();
        assert!(!has_lucid(&cfg));
    }
}
