use crate::models::ServerType;
pub fn parse_java(text: &str) -> Option<u32> {
    let line = text.lines().find(|s| {
        s.contains("version \"") || s.starts_with("openjdk ") || s.starts_with("java ")
    })?;
    let version = if let Some(s) = line.split('"').nth(1) {
        s
    } else {
        line.split_whitespace().nth(1)?
    };
    let mut parts = version.split(['.', '-', '+', '_']);
    let first = parts.next()?.parse::<u32>().ok()?;
    if first == 1 {
        parts.next()?.parse().ok()
    } else {
        Some(first)
    }
}
pub fn required_java(version: &str, software: ServerType, manifest: Option<u32>) -> u32 {
    let v: Vec<u32> = version.split('.').filter_map(|x| x.parse().ok()).collect();
    let base = if v.first().copied().unwrap_or(0) >= 26 {
        25
    } else if v.get(1).copied().unwrap_or(0) >= 21
        || (v.get(1) == Some(&20) && v.get(2).copied().unwrap_or(0) >= 5)
    {
        21
    } else if v.get(1).copied().unwrap_or(0) >= 18 {
        17
    } else if v.get(1) == Some(&17) {
        16
    } else {
        8
    };
    let paper = if software == ServerType::Paper {
        if v.get(1).copied().unwrap_or(0) >= 20 {
            21
        } else if v.get(1).copied().unwrap_or(0) >= 17 {
            17
        } else {
            8
        }
    } else {
        8
    };
    base.max(manifest.unwrap_or(8)).max(paper)
}
pub fn compatible(major: u32, required: u32) -> bool {
    // Use the corresponding supported major: legacy mod loaders often break on newer Java.
    major == required
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_outputs() {
        for (s, n) in [
            (
                "java version \"1.8.0_401\"\nJava(TM) SE Runtime Environment",
                8,
            ),
            ("openjdk version \"21.0.5\" 2024-10-15 LTS", 21),
            ("openjdk 25.0.1 2025-10-21\nOpenJDK Runtime", 25),
            ("java version \"17.0.12\"", 17),
            ("openjdk version \"26-ea\"", 26),
        ] {
            assert_eq!(parse_java(s), Some(n));
        }
        assert_eq!(parse_java("not installed"), None);
    }
    #[test]
    fn compatibility_boundaries() {
        for (v, n) in [
            ("1.16.5", 8),
            ("1.17.1", 16),
            ("1.18", 17),
            ("1.20.4", 17),
            ("1.20.5", 21),
            ("1.21.11", 21),
            ("26.1", 25),
        ] {
            assert_eq!(required_java(v, ServerType::Vanilla, None), n);
        }
        assert_eq!(required_java("1.20.4", ServerType::Paper, None), 21);
        assert!(!compatible(8, 21));
        assert!(!compatible(25, 8));
    }
}
