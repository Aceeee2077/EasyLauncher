use crate::error::{AppError, Result};
use std::collections::BTreeMap;

// Java Properties: logical lines, escaped separators, comments and Unicode escapes.
#[derive(Debug, Clone)]
pub struct Properties {
    lines: Vec<(Option<String>, String)>,
    pub values: BTreeMap<String, String>,
}
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('f') => out.push('\u{c}'),
            Some('u') => {
                let h: String = chars.by_ref().take(4).collect();
                if let Ok(n) = u32::from_str_radix(&h, 16) {
                    let n = if (0xd800..=0xdbff).contains(&n) {
                        let mut lookahead = chars.clone();
                        if lookahead.next() == Some('\\') && lookahead.next() == Some('u') {
                            let low: String = lookahead.by_ref().take(4).collect();
                            if let Ok(low) = u32::from_str_radix(&low, 16) {
                                if (0xdc00..=0xdfff).contains(&low) {
                                    chars = lookahead;
                                    0x10000 + ((n - 0xd800) << 10) + (low - 0xdc00)
                                } else {
                                    n
                                }
                            } else {
                                n
                            }
                        } else {
                            n
                        }
                    } else {
                        n
                    };
                    if let Some(c) = char::from_u32(n) {
                        out.push(c);
                    }
                }
            }
            Some(c) => out.push(c),
            None => out.push('\\'),
        }
    }
    out
}
fn escape(s: &str, key: bool) -> String {
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '=' | ':' | '#' | '!' if key => {
                out.push('\\');
                out.push(c);
            }
            ' ' if key || i == 0 => out.push_str("\\ "),
            _ if !c.is_ascii() => {
                for u in c.encode_utf16(&mut [0; 2]) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
            _ => out.push(c),
        }
    }
    out
}
/// A single template-driven change, rendered in the UI before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyChange {
    pub key: String,
    pub before: Option<String>,
    pub after: String,
}
/// Built-in presets. They only touch gameplay and performance keys, never ports,
/// MOTDs or paths, so applying one cannot break connectivity or world layout.
pub fn template(name: &str) -> Result<Vec<(&'static str, &'static str)>> {
    Ok(match name {
        "survival" => vec![
            ("gamemode", "survival"),
            ("difficulty", "normal"),
            ("pvp", "true"),
            ("allow-flight", "false"),
            ("spawn-protection", "16"),
            ("max-players", "20"),
            ("view-distance", "10"),
            ("simulation-distance", "6"),
        ],
        "creative" => vec![
            ("gamemode", "creative"),
            ("difficulty", "peaceful"),
            ("pvp", "true"),
            ("allow-flight", "true"),
            ("spawn-protection", "0"),
            ("max-players", "20"),
            ("view-distance", "12"),
            ("simulation-distance", "8"),
        ],
        "performance" => vec![
            ("spawn-protection", "0"),
            ("max-players", "10"),
            ("view-distance", "6"),
            ("simulation-distance", "4"),
        ],
        _ => {
            return Err(AppError::user(
                "Choose a built-in configuration template: survival, creative or performance.",
            ))
        }
    })
}
/// List the actual differences a template would introduce, in template order.
pub fn preview(p: &Properties, pairs: &[(&'static str, &'static str)]) -> Vec<PropertyChange> {
    pairs
        .iter()
        .filter_map(|(key, after)| {
            let before = p.values.get(*key).cloned();
            if before.as_deref() == Some(*after) {
                return None;
            }
            Some(PropertyChange {
                key: key.to_string(),
                before,
                after: after.to_string(),
            })
        })
        .collect()
}
impl Properties {
    pub fn parse(text: &str) -> Self {
        let mut p = Self {
            lines: Vec::new(),
            values: BTreeMap::new(),
        };
        let mut logical = String::new();
        for line in text.lines() {
            if logical.is_empty() {
                logical.push_str(line);
            } else {
                logical.push_str(line.trim_start());
            }
            if logical.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1 {
                logical.pop();
                continue;
            }
            let t = logical.trim_start();
            if t.is_empty() || t.starts_with(['#', '!']) {
                p.lines.push((None, logical.clone()));
                logical.clear();
                continue;
            }
            let mut escaped = false;
            let mut split = t.len();
            for (i, c) in t.char_indices() {
                if !escaped && (c == '=' || c == ':' || c.is_ascii_whitespace()) {
                    split = i;
                    break;
                }
                if c == '\\' {
                    escaped = !escaped;
                } else {
                    escaped = false;
                }
            }
            let key = unescape(&t[..split]);
            let mut rest = t[split..].trim_start();
            if rest.starts_with(['=', ':']) {
                rest = rest[1..].trim_start();
            }
            p.values.insert(key.clone(), unescape(rest));
            p.lines.push((Some(key), logical.clone()));
            logical.clear();
        }
        if !logical.is_empty() {
            p.lines.push((None, logical));
        }
        p
    }
    pub fn set(&mut self, k: &str, v: &str) {
        self.values.insert(k.to_string(), v.to_string());
    }
    pub fn serialize(&self) -> String {
        let mut seen = std::collections::HashSet::new();
        let mut out = String::new();
        for (key, original) in &self.lines {
            if let Some(k) = key {
                if seen.insert(k.clone()) {
                    if let Some(v) = self.values.get(k) {
                        out.push_str(&format!("{}={}\n", escape(k, true), escape(v, false)));
                    }
                }
            } else {
                out.push_str(original);
                out.push('\n');
            }
        }
        for (k, v) in &self.values {
            if !seen.contains(k) {
                out.push_str(&format!("{}={}\n", escape(k, true), escape(v, false)));
            }
        }
        out
    }
    pub fn validate(&self) -> Result<()> {
        for (key, min, max) in [
            ("server-port", 1, 65535),
            ("max-players", 1, 1000),
            ("spawn-protection", 0, 10000),
            ("view-distance", 2, 32),
            ("simulation-distance", 2, 32),
        ] {
            if let Some(v) = self.values.get(key) {
                if !v
                    .parse::<u32>()
                    .map(|n| (min..=max).contains(&n))
                    .unwrap_or(false)
                {
                    return Err(AppError::user(format!(
                        "{key} must be between {min} and {max}."
                    )));
                }
            }
        }
        for key in ["pvp", "online-mode", "allow-flight", "white-list"] {
            if let Some(v) = self.values.get(key) {
                if !["true", "false"].contains(&v.as_str()) {
                    return Err(AppError::user(format!("{key} must be true or false.")));
                }
            }
        }
        for (k, allowed) in [
            (
                "gamemode",
                vec!["survival", "creative", "adventure", "spectator"],
            ),
            ("difficulty", vec!["peaceful", "easy", "normal", "hard"]),
        ] {
            if let Some(v) = self.values.get(k) {
                if !allowed.contains(&v.as_str()) {
                    return Err(AppError::user(format!("Choose a valid {k}.")));
                }
            }
        }
        if let Some(n) = self.values.get("level-name") {
            validate_world(n)?;
        }
        // Keep control channels local and avoid references to paths outside managed storage.
        for key in ["enable-rcon", "enable-query", "enable-management-server"] {
            if self.values.get(key).map(|s| s == "true").unwrap_or(false) {
                return Err(AppError::user(format!(
                    "{key} is not supported in V0.1. Use the local console."
                )));
            }
        }
        if self
            .values
            .get("server-ip")
            .map(|s| !s.is_empty())
            .unwrap_or(false)
        {
            return Err(AppError::user(
                "Leave server-ip empty so EasyLauncher can reliably check the listening port.",
            ));
        }
        Ok(())
    }
}
pub fn validate_world(name: &str) -> Result<()> {
    crate::storage::child(std::path::Path::new("."), name)?;
    let lower = name.to_ascii_lowercase();
    if [
        "backups",
        "logs",
        "mods",
        "plugins",
        "libraries",
        "versions",
        "cache",
        "config",
        "crash-reports",
    ]
    .contains(&lower.as_str())
        || lower.starts_with("restore-")
        || lower.starts_with("deleted-")
        || name.contains('.')
        || name.len() > 100
    {
        return Err(AppError::user("Choose a world name that does not conflict with managed folders. Use up to 100 letters, numbers, spaces, underscores or hyphens."));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_modify_preserve() {
        let mut p=Properties::parse("# Keep this\nserver-port=25565\ncustom\\:key : yes\nmotd=Hi\\nworld\ncontinued=one\\\n  two\n");
        assert_eq!(p.values["custom:key"], "yes");
        assert_eq!(p.values["motd"], "Hi\nworld");
        assert_eq!(p.values["continued"], "onetwo");
        p.set("server-port", "25566");
        let s = p.serialize();
        let p2 = Properties::parse(&s);
        assert_eq!(p2.values["server-port"], "25566");
        assert_eq!(p2.values["custom:key"], "yes");
        assert!(s.contains("# Keep this"));
    }
    #[test]
    fn unicode_and_validation() {
        let mut p = Properties::parse("motd=\\u4f60\\u597d\n");
        assert_eq!(p.values["motd"], "你好");
        assert_eq!(Properties::parse(&p.serialize()).values, p.values);
        p.set("server-port", "0");
        assert!(p.validate().is_err());
        p.set("server-port", "25565");
        p.set("level-name", "../bad");
        assert!(p.validate().is_err());
    }
    #[test]
    fn supplementary_unicode_and_reserved_worlds() {
        let mut p = Properties::parse("");
        p.set("motd", "你好 🌲");
        assert_eq!(Properties::parse(&p.serialize()).values["motd"], "你好 🌲");
        for world in ["backups", "logs", "CON", "world.", "restore-recovery"] {
            assert!(validate_world(world).is_err());
        }
    }
    #[test]
    fn duplicate_keys_last_value_wins() {
        let p = Properties::parse("x=1\nx=2");
        assert_eq!(p.serialize(), "x=2\n");
    }
    #[test]
    fn template_preview_skips_equal_values_and_keeps_comments() {
        let pairs = template("survival").unwrap();
        let mut p = Properties::parse("# keep me\nmotd=Stay\nview-distance=10\n");
        let changes = preview(&p, &pairs);
        assert!(!changes.iter().any(|c| c.key == "view-distance"));
        let view = changes.iter().find(|c| c.key == "gamemode").unwrap();
        assert_eq!(view.before, None);
        assert_eq!(view.after, "survival");
        for (key, value) in &pairs {
            p.set(key, value);
        }
        p.validate().unwrap();
        let text = p.serialize();
        assert!(text.contains("# keep me"));
        assert!(text.contains("motd=Stay"));
        let parsed = Properties::parse(&text);
        assert_eq!(parsed.values["gamemode"], "survival");
        assert_eq!(preview(&parsed, &pairs), Vec::new());
        assert!(template("unknown").is_err());
    }
}
