//! Parser brain.md (version 3) → BrainGraph.
//!
//! Lit le format structuré produit par `pipeline::assemble_markdown` et
//! reconstruit un `BrainGraph` complet — nodes + edges — sans passer par le LLM.
//! Utilisé par la logique cache au démarrage.

use crate::models::{BrainEdge, BrainGraph, BrainNode};
use std::collections::HashMap;

// ─── Structures internes ──────────────────────────────────────────────────────

#[derive(Default)]
struct Draft {
    id: String,
    label: String,
    kind: String,
    weight: usize,
    community: u32,
    parent_id: Option<String>,
    connector: Option<String>,
    source_id: Option<String>,
    source_project: Option<String>,
    summary_lines: Vec<String>,
    keywords: Vec<String>,
    decisions: Vec<String>,
    patterns: Vec<String>,
}

impl Draft {
    fn is_valid(&self) -> bool { !self.id.is_empty() && !self.kind.is_empty() }

    fn into_parts(self) -> (BrainNode, BrainEdge) {
        let parent = self.parent_id.clone().unwrap_or_else(|| "root".into());
        let edge = BrainEdge {
            source: parent,
            target: self.id.clone(),
            kind: "contains".into(),
            relation: "contains".into(),
        };
        let node = BrainNode {
            summary: self.summary_lines.join(" ").trim().to_string(),
            id: self.id,
            label: self.label,
            kind: self.kind,
            weight: self.weight.max(1),
            keywords: self.keywords,
            decisions: self.decisions,
            patterns: self.patterns,
            community: self.community,
            parent_id: self.parent_id,
            synthesized_at: None,
            date: None,
            content: String::new(),
            connector: self.connector,
            source_id: self.source_id,
            source_project: self.source_project,
            source_text: String::new(),
        };
        (node, edge)
    }
}

#[derive(PartialEq)]
enum Section { Summary, Decisions, Patterns }

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Parse `<!-- key: value | key2: value2 -->` → HashMap.
fn parse_meta(line: &str) -> HashMap<String, String> {
    line.trim_start_matches("<!--")
        .trim_end_matches("-->")
        .trim()
        .split('|')
        .filter_map(|part| {
            let mut it = part.splitn(2, ':');
            Some((it.next()?.trim().to_string(), it.next()?.trim().to_string()))
        })
        .collect()
}

// ─── Parseur principal ────────────────────────────────────────────────────────

/// Parse un brain.md v3 → BrainGraph prêt à l'emploi.
pub fn parse(content: &str) -> BrainGraph {
    let mut nodes: Vec<BrainNode> = vec![BrainNode {
        id: "root".into(),
        label: "Lucid".into(),
        kind: "root".into(),
        weight: 1,
        summary: String::new(),
        keywords: vec![],
        decisions: vec![],
        patterns: vec![],
        community: 0,
        parent_id: None,
        synthesized_at: None,
        date: None,
        content: String::new(),
        connector: None,
        source_id: None,
        source_project: None,
        source_text: String::new(),
    }];
    let mut edges: Vec<BrainEdge> = Vec::new();
    let mut generated_at = String::new();
    let mut draft: Option<Draft> = None;
    let mut section = Section::Summary;
    let mut in_content = false;

    for line in content.lines() {
        // ── Métadonnées racine ──
        if line.starts_with("<!-- generated:") {
            if let Some(v) = parse_meta(line).remove("generated") {
                generated_at = v;
            }
            continue;
        }

        // ── Heading → nouveau nœud ──
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if hashes >= 2 {
            let rest = line[hashes..].trim();
            let (kind, label) = if let Some(l) = rest.strip_prefix("Espace: ") {
                ("container", l.trim())
            } else if let Some(l) = rest.strip_prefix("Page: ") {
                ("leaf", l.trim())
            } else {
                continue;
            };

            // Flush nœud précédent
            if let Some(d) = draft.take() {
                if d.is_valid() {
                    let (n, e) = d.into_parts();
                    nodes.push(n);
                    edges.push(e);
                }
            }
            draft = Some(Draft { label: label.to_string(), kind: kind.to_string(), ..Default::default() });
            section = Section::Summary;
            in_content = false;
            continue;
        }

        // ── Métadonnées du nœud ──
        if line.starts_with("<!-- id:") {
            if let Some(ref mut d) = draft {
                let mut meta = parse_meta(line);
                if let Some(id) = meta.remove("id") { d.id = id; }
                if let Some(w) = meta.remove("weight").and_then(|s| s.parse().ok()) { d.weight = w; }
                if let Some(c) = meta.remove("community").and_then(|s| s.parse().ok()) { d.community = c; }
                if let Some(p) = meta.remove("parent") {
                    if p != "root" { d.parent_id = Some(p); }
                }
                if let Some(c) = meta.remove("connector") { d.connector = Some(c); }
                if let Some(s) = meta.remove("source_id") { d.source_id = Some(s); }
                if let Some(s) = meta.remove("source_project") { d.source_project = Some(s); }
                in_content = true;
            }
            continue;
        }

        if !in_content { continue; }

        // ── Marqueurs de section ──
        if line.starts_with("**Mots-clés**") {
            if let Some(ref mut d) = draft {
                let kw = line.splitn(2, ':').nth(1).unwrap_or("").trim();
                d.keywords = kw.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            }
            continue;
        }
        if line.starts_with("**Décisions**") { section = Section::Decisions; continue; }
        if line.starts_with("**Patterns**")  { section = Section::Patterns;  continue; }

        // ── Items de liste ──
        if line.trim_start().starts_with("- ") {
            if let Some(ref mut d) = draft {
                match section {
                    Section::Decisions => d.decisions.push(line.trim().trim_start_matches("- ").to_string()),
                    Section::Patterns  => d.patterns.push(line.trim().trim_start_matches("- ").to_string()),
                    Section::Summary   => {}
                }
            }
            continue;
        }

        // ── Résumé (texte libre avant les sections **) ──
        if section == Section::Summary && !line.trim().is_empty() && !line.starts_with("**") {
            if let Some(ref mut d) = draft { d.summary_lines.push(line.trim().to_string()); }
        }
    }

    // Flush dernier nœud
    if let Some(d) = draft {
        if d.is_valid() {
            let (n, e) = d.into_parts();
            nodes.push(n);
            edges.push(e);
        }
    }

    // Mise à jour du poids root
    let count = nodes.iter().filter(|n| n.kind != "root").count();
    if let Some(root) = nodes.iter_mut().find(|n| n.kind == "root") {
        root.weight = count;
    }

    BrainGraph {
        nodes,
        edges,
        markdown: content.to_string(),
        report: String::new(),
        generated_at,
    }
}
