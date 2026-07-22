//! Détection de secrets avant écriture dans brain.json — obligatoire, autonome
//! ET validation manuelle (plan Archiviste v0). Scan par chaînes, sans regex
//! (aucune dépendance ajoutée) : préfixes littéraux connus + mots-clés suivis
//! d'une valeur qui ressemble à un secret.

const MASK: &str = "[SECRET MASQUÉ]";

const LITERAL_PREFIXES: &[&str] = &[
    "sk-", "ghp_", "gho_", "github_pat_", "AKIA", "AIza", "xoxb-", "xoxp-", "xoxa-",
    "-----BEGIN", "eyJ",
];

const KEYWORD_HINTS: &[&str] = &[
    "api_key", "apikey", "api-key", "secret", "password", "passwd", "token",
    "access_key", "client_secret",
];

/// Une ligne contient-elle un préfixe littéral connu de secret ?
fn has_literal_prefix(line: &str) -> bool {
    LITERAL_PREFIXES.iter().any(|p| line.contains(p))
}

/// `clé: valeur` ou `clé=valeur` où `clé` évoque un secret et `valeur` a une
/// forme plausible (≥ 8 caractères, pas d'espace) → renvoie la valeur trouvée.
fn keyword_value(line: &str) -> Option<&str> {
    let lower = line.to_lowercase();
    let hint = KEYWORD_HINTS.iter().find(|h| lower.contains(**h))?;
    let after_hint = &line[lower.find(hint)? + hint.len()..];
    let sep = after_hint.find([':', '='])?;
    let value = after_hint[sep + 1..].trim_start();
    let value = value.split_whitespace().next().unwrap_or("");
    let value = value.trim_matches(['"', '\'', ',', ';']);
    (value.len() >= 8).then_some(value)
}

/// Masque tout secret détecté dans `text`. Renvoie le texte (éventuellement
/// masqué) et si un masquage a eu lieu.
pub fn mask_secrets(text: &str) -> (String, bool) {
    let mut masked = false;
    let out: Vec<String> = text
        .lines()
        .map(|line| {
            if has_literal_prefix(line) {
                masked = true;
                return MASK.to_string();
            }
            if let Some(value) = keyword_value(line) {
                masked = true;
                return line.replacen(value, MASK, 1);
            }
            line.to_string()
        })
        .collect();
    (out.join("\n"), masked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masque_une_cle_api_litterale() {
        let (out, masked) = mask_secrets("Voici ma clé : sk-abcdef1234567890");
        assert!(masked);
        assert_eq!(out, MASK);
    }

    #[test]
    fn masque_un_mot_de_passe_par_mot_cle() {
        let (out, masked) = mask_secrets("password: hunter22222");
        assert!(masked);
        assert!(out.contains(MASK), "{out}");
        assert!(!out.contains("hunter22222"));
    }

    #[test]
    fn laisse_un_texte_normal_intact() {
        let text = "Réunion demain 14h, décision : on garde Tauri.\nRAS.";
        let (out, masked) = mask_secrets(text);
        assert!(!masked);
        assert_eq!(out, text);
    }

    #[test]
    fn ignore_une_valeur_trop_courte() {
        let (out, masked) = mask_secrets("token: abc");
        assert!(!masked);
        assert_eq!(out, "token: abc");
    }
}
