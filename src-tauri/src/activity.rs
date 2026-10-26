//! Independently implemented, read-only Minecraft log library and event index.
use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use serde::Serialize;
use std::{collections::BTreeSet, fs, io::Read, path::Path};

const MAX_BYTES: u64 = 8 * 1024 * 1024;
const MAX_EVENTS: usize = 10_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    pub name: String,
    pub bytes: u64,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub files: Vec<LogFile>,
    pub omitted: usize,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub line: usize,
    pub time: Option<String>,
    pub hour: Option<usize>,
    pub kind: String,
    pub player: Option<String>,
    pub message: String,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub source: String,
    pub scanned_lines: usize,
    pub truncated: bool,
    pub events: Vec<Event>,
    pub players: Vec<String>,
    pub hours: Vec<usize>,
}

fn supported(name: &str) -> bool {
    name.ends_with(".log") || name.ends_with(".log.gz")
}
fn log_dir(m: &Manager, id: &str) -> Result<std::path::PathBuf> {
    m.metadata(id)?;
    let dir = m.dir(id)?.join("logs");
    storage::no_links(&dir)?;
    Ok(dir)
}
pub fn library(m: &Manager, id: &str) -> Result<Library> {
    list(&log_dir(m, id)?)
}
fn list(dir: &Path) -> Result<Library> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(Library { files, omitted: 0 });
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !supported(&name) || storage::no_links(&entry.path()).is_err() {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.is_file() {
            files.push(LogFile {
                name,
                bytes: meta.len(),
            });
        }
    }
    files.sort_by(|a, b| {
        (b.name == "latest.log")
            .cmp(&(a.name == "latest.log"))
            .then_with(|| b.name.cmp(&a.name))
    });
    let omitted = files.len().saturating_sub(200);
    files.truncate(200);
    Ok(Library { files, omitted })
}
pub fn analyze(m: &Manager, id: &str, name: &str) -> Result<Report> {
    read(&log_dir(m, id)?, name)
}
fn read(dir: &Path, name: &str) -> Result<Report> {
    if !supported(name) {
        return Err(AppError::user("Select a .log or .log.gz file."));
    }
    let path = storage::child(dir, name)?;
    storage::no_links(&path)?;
    let file = fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(AppError::user("Select a regular log file."));
    }
    let reader: Box<dyn Read> = if name.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > MAX_BYTES;
    bytes.truncate(MAX_BYTES as usize);
    // A partial final line can look like an event. Exclude it at the byte cap.
    if truncated {
        bytes.truncate(
            bytes
                .iter()
                .rposition(|b| *b == b'\n')
                .map(|i| i + 1)
                .unwrap_or(0),
        );
    }
    Ok(parse(name, &String::from_utf8_lossy(&bytes), truncated))
}
fn player_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 16
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn classify(message: &str, header: &str) -> Option<(&'static str, Option<String>)> {
    // Chat is checked first: player text must not impersonate server events.
    if let Some(rest) = message.strip_prefix('<') {
        if let Some((name, _)) = rest.split_once("> ") {
            if player_name(name) {
                return Some(("chat", Some(name.into())));
            }
        }
        return None;
    }
    if header.contains("/ERROR") || header.contains("/FATAL") {
        return Some(("error", None));
    }
    if header.contains("/WARN") {
        return Some(("warning", None));
    }
    let (name, rest) = message.split_once(' ').unwrap_or(("", ""));
    if player_name(name) {
        let kind = if rest == "joined the game" {
            Some("join")
        } else if rest == "left the game" {
            Some("leave")
        } else if rest.starts_with("has made the advancement [")
            || rest.starts_with("has completed the challenge [")
            || rest.starts_with("has reached the goal [")
        {
            Some("advancement")
        } else if [
            "was slain by ",
            "was shot by ",
            "fell from ",
            "fell off ",
            "was blown up by ",
            "was killed by ",
            "hit the ground too hard",
            "drowned",
            "burned to death",
            "tried to swim in lava",
            "starved to death",
            "fell out of the world",
            "suffocated in a wall",
            "went up in flames",
            "died",
        ]
        .iter()
        .any(|pattern| rest == *pattern || (pattern.ends_with(' ') && rest.starts_with(pattern)))
        {
            Some("death")
        } else {
            None
        };
        if let Some(kind) = kind {
            return Some((kind, Some(name.into())));
        }
    }
    if (message.starts_with("Done (") && message.contains("For help"))
        || message == "Stopping server"
        || message.starts_with("Starting minecraft server version ")
    {
        return Some(("lifecycle", None));
    }
    None
}
fn timestamp(line: &str) -> (Option<String>, Option<usize>) {
    let Some(time) = line
        .strip_prefix('[')
        .and_then(|s| s.split_once(']').map(|x| x.0))
    else {
        return (None, None);
    };
    let p: Vec<_> = time.split(':').collect();
    if p.len() != 3
        || p.iter()
            .any(|v| v.len() != 2 || !v.bytes().all(|b| b.is_ascii_digit()))
    {
        return (None, None);
    }
    let (h, m, s) = (
        p[0].parse::<usize>().unwrap_or(99),
        p[1].parse::<usize>().unwrap_or(99),
        p[2].parse::<usize>().unwrap_or(99),
    );
    if h > 23 || m > 59 || s > 59 {
        return (None, None);
    }
    (Some(time.into()), Some(h))
}
fn parse(source: &str, text: &str, mut truncated: bool) -> Report {
    let mut events = Vec::new();
    let mut players = BTreeSet::new();
    let mut hours = vec![0; 24];
    let mut scanned_lines = 0;
    for (i, line) in text.lines().enumerate() {
        scanned_lines = i + 1;
        let Some((header, message)) = line.split_once("]: ") else {
            continue;
        };
        if !header.contains("Server thread/") {
            continue;
        }
        let Some((kind, player)) = classify(message, header) else {
            continue;
        };
        if events.len() == MAX_EVENTS {
            truncated = true;
            break;
        }
        let (time, hour) = timestamp(line);
        if let Some(h) = hour {
            hours[h] += 1;
        }
        if let Some(p) = &player {
            players.insert(p.clone());
        }
        let safe = crate::log_assistant::sanitize(message);
        events.push(Event {
            line: i + 1,
            time,
            hour,
            kind: kind.into(),
            player,
            message: safe.chars().take(1000).collect(),
        });
    }
    Report {
        source: source.into(),
        scanned_lines,
        truncated,
        events,
        players: players.into_iter().collect(),
        hours,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexes_events_without_chat_spoofing() {
        let r=parse("latest.log", "[12:01:00] [Server thread/INFO]: Alex joined the game\n[12:02:00] [Server thread/INFO]: <Alex> Bob joined the game\n[12:03:00] [Server thread/INFO]: Alex was slain by Zombie\n[12:04:00] [Server thread/INFO]: Alex has made the advancement [Stone Age]\n[12:05:00] [Server thread/WARN]: Can't keep up!\n[12:06:00] [Server thread/INFO]: Alex left the game",false);
        assert_eq!(
            r.events.iter().map(|e| e.kind.as_str()).collect::<Vec<_>>(),
            vec!["join", "chat", "death", "advancement", "warning", "leave"]
        );
        assert_eq!(r.players, vec!["Alex"]);
        assert_eq!(r.hours[12], 6);
        assert_eq!(r.events[2].line, 3);
    }
    #[test]
    fn handles_invalid_time_unknown_lines_and_secrets() {
        let r=parse("test.log", "[99:00:00] [Server thread/ERROR]: password=hunter2\n[12:00:00] [Other thread/INFO]: Alex joined the game\ncontinuation\n[00:01:00] [Server thread/INFO]: Done (1s)! For help, type help",false);
        assert_eq!(r.events.len(), 2);
        assert_eq!(r.events[0].hour, None);
        assert!(!r.events[0].message.contains("hunter2"));
        assert_eq!(r.hours[0], 1);
    }
    #[test]
    fn reads_gzip_and_rejects_traversal() {
        use std::io::Write;
        let temp = tempfile::tempdir().unwrap();
        let file = fs::File::create(temp.path().join("2026-10-25-1.log.gz")).unwrap();
        let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        gz.write_all(b"[00:00:01] [Server thread/INFO]: Alex joined the game\n")
            .unwrap();
        gz.finish().unwrap();
        assert_eq!(
            read(temp.path(), "2026-10-25-1.log.gz")
                .unwrap()
                .events
                .len(),
            1
        );
        assert!(read(temp.path(), "../outside.log").is_err());
        assert!(read(temp.path(), "C:\\outside.log").is_err());
        assert_eq!(list(temp.path()).unwrap().files.len(), 1);
    }
    #[test]
    fn caps_decompression_and_event_count() {
        use std::io::Write;
        let line = "[01:02:03] [Server thread/WARN]: test\n";
        let r = parse("test.log", &line.repeat(MAX_EVENTS + 20), false);
        assert!(r.truncated);
        assert_eq!(r.events.len(), MAX_EVENTS);
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("large.log"),
            vec![b'x'; MAX_BYTES as usize + 2],
        )
        .unwrap();
        let r = read(temp.path(), "large.log").unwrap();
        assert!(r.truncated);
        assert!(r.events.is_empty());
        let file = fs::File::create(temp.path().join("large.log.gz")).unwrap();
        let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        gz.write_all(&vec![b'x'; MAX_BYTES as usize + 2]).unwrap();
        gz.finish().unwrap();
        assert!(read(temp.path(), "large.log.gz").unwrap().truncated);
        fs::write(temp.path().join("corrupt.log.gz"), b"not gzip").unwrap();
        assert!(read(temp.path(), "corrupt.log.gz").is_err());
    }
    #[test]
    fn library_prioritizes_latest_and_reports_omitted_files() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..205 {
            fs::write(temp.path().join(format!("2026-10-25-{index:03}.log")), b"").unwrap();
        }
        fs::write(temp.path().join("latest.log"), b"current").unwrap();
        fs::write(temp.path().join("private.json"), b"{}").unwrap();
        fs::create_dir(temp.path().join("directory.log")).unwrap();
        let library = list(temp.path()).unwrap();
        assert_eq!(library.files.len(), 200);
        assert_eq!(library.omitted, 6);
        assert_eq!(library.files[0].name, "latest.log");
        assert_eq!(library.files[1].name, "2026-10-25-204.log");
    }
}
