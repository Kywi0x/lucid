//! Connecteurs : sources de données que Second Brain ingère.

pub mod claude_ai;
pub mod claude_code;
pub mod google_drive;
pub mod notion;
pub mod obsidian;

pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
