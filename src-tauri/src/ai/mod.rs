//! IA embarquée : moteur llama.cpp local + pipeline d'analyse → brain.md.

pub mod llama;
pub mod pipeline;

pub use llama::LlamaEngine;
