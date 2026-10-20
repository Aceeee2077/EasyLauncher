//! Read-only inspection of Minecraft world snapshots.
//!
//! The module parses Anvil region files and NBT without loading Minecraft. It
//! powers the backup-page map, player positions, entity statistics, chunk
//! corruption checks and region-level snapshot comparison. Nothing here writes
//! to a world or a backup; the only file this module writes is the optional
//! online position track in the server folder.

use crate::{
    backup,
    backup_store::{self, Manifest},
    error::{AppError, Result},
    models::{Backup, BackupFormat},
    rcon,
    server::manager::Manager,
    storage,
};
use flate2::read::{GzDecoder, ZlibDecoder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const REGION_HEADER: usize = 8192;
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_NBT_BYTES: usize = 64 * 1024 * 1024;
const MAX_STRING_BYTES: usize = 1024 * 1024;
const MAX_LIST_ENTRIES: usize = 4_000_000;
const MAX_DEPTH: u8 = 64;
const MAX_SCAN_REGIONS: usize = 256;
const MAX_SCAN_CHUNKS: usize = 50_000;
const MAX_CORRUPTIONS: usize = 200;
const MAX_PLAYERS: usize = 500;
const MAX_DIFF_CHANGES: usize = 500;
const MAX_TRACK_PLAYERS: usize = 64;
const MAX_TRACK_POINTS: usize = 600;
const TRACK_DAYS: i64 = 7;
const TRACK_FILE: &str = "world-tracks.json";
const TRACK_VERSION: u32 = 1;
const MAX_MAP_SCALE: u32 = 8;

fn corrupt(message: impl Into<String>) -> AppError {
    AppError::new("This world data is damaged or incomplete.", message.into())
        .with_code("world.corrupt")
}

// ---------------------------------------------------------------------------
// Minimal NBT reader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Nbt {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<u8>),
    String(String),
    List(Vec<Nbt>),
    Compound(BTreeMap<String, Nbt>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

impl Nbt {
    fn get(&self, key: &str) -> Option<&Nbt> {
        match self {
            Nbt::Compound(map) => map.get(key),
            _ => None,
        }
    }
    fn as_compound(&self) -> Option<&BTreeMap<String, Nbt>> {
        match self {
            Nbt::Compound(map) => Some(map),
            _ => None,
        }
    }
    fn as_list(&self) -> Option<&[Nbt]> {
        match self {
            Nbt::List(list) => Some(list),
            _ => None,
        }
    }
    fn as_string(&self) -> Option<&str> {
        match self {
            Nbt::String(value) => Some(value),
            _ => None,
        }
    }
    fn as_i64(&self) -> Option<i64> {
        match self {
            Nbt::Byte(v) => Some(*v as i64),
            Nbt::Short(v) => Some(*v as i64),
            Nbt::Int(v) => Some(*v as i64),
            Nbt::Long(v) => Some(*v),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Nbt::Byte(v) => Some(*v as f64),
            Nbt::Short(v) => Some(*v as f64),
            Nbt::Int(v) => Some(*v as f64),
            Nbt::Long(v) => Some(*v as f64),
            Nbt::Float(v) => Some(*v as f64),
            Nbt::Double(v) => Some(*v),
            _ => None,
        }
    }
    fn as_longs(&self) -> Option<&[i64]> {
        match self {
            Nbt::LongArray(values) => Some(values),
            _ => None,
        }
    }
}

type ParseResult<T> = std::result::Result<T, String>;

struct NbtReader<'a> {
    data: &'a [u8],
    pos: usize,
}
impl<'a> NbtReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, len: usize) -> ParseResult<&'a [u8]> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or_else(|| "NBT length overflow.".to_string())?;
        if end > self.data.len() {
            return Err("NBT is truncated.".into());
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
    fn u8(&mut self) -> ParseResult<u8> {
        Ok(self.take(1)?[0])
    }
    fn i16(&mut self) -> ParseResult<i16> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }
    fn i32(&mut self) -> ParseResult<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }
    fn u16(&mut self) -> ParseResult<u16> {
        Ok(self.i16()? as u16)
    }
    fn i64(&mut self) -> ParseResult<i64> {
        let bytes = self.take(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }
    fn string(&mut self) -> ParseResult<String> {
        let len = self.u16()? as usize;
        if len > MAX_STRING_BYTES {
            return Err("NBT string is too long.".into());
        }
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "NBT string is not UTF-8.".to_string())
    }
    fn tag_name(&mut self) -> ParseResult<String> {
        self.string()
    }
    fn root(&mut self) -> ParseResult<Nbt> {
        let kind = self.u8()?;
        if kind != 10 {
            return Err("The NBT root is not a compound.".into());
        }
        let _name = self.tag_name()?;
        self.payload(10, 0)
    }
    fn compound(&mut self, depth: u8) -> ParseResult<Nbt> {
        if depth > MAX_DEPTH {
            return Err("NBT is nested too deeply.".into());
        }
        let mut map = BTreeMap::new();
        loop {
            let kind = self.u8()?;
            if kind == 0 {
                break;
            }
            let name = self.tag_name()?;
            let value = self.payload(kind, depth + 1)?;
            map.insert(name, value);
        }
        Ok(Nbt::Compound(map))
    }
    fn payload(&mut self, kind: u8, depth: u8) -> ParseResult<Nbt> {
        if depth > MAX_DEPTH {
            return Err("NBT is nested too deeply.".into());
        }
        Ok(match kind {
            1 => Nbt::Byte(self.u8()? as i8),
            2 => Nbt::Short(self.i16()?),
            3 => Nbt::Int(self.i32()?),
            4 => Nbt::Long(self.i64()?),
            5 => Nbt::Float(f32::from_bits(self.i32()? as u32)),
            6 => Nbt::Double(f64::from_bits(self.i64()? as u64)),
            7 => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_NBT_BYTES {
                    return Err("NBT byte array is too large.".into());
                }
                Nbt::ByteArray(self.take(len as usize)?.to_vec())
            }
            8 => Nbt::String(self.string()?),
            9 => {
                let element = self.u8()?;
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_LIST_ENTRIES {
                    return Err("NBT list is too large.".into());
                }
                let mut values = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    values.push(self.payload(element, depth + 1)?);
                }
                Nbt::List(values)
            }
            10 => self.compound(depth)?,
            11 => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_NBT_BYTES / 4 {
                    return Err("NBT int array is too large.".into());
                }
                let mut values = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    values.push(self.i32()?);
                }
                Nbt::IntArray(values)
            }
            12 => {
                let len = self.i32()?;
                if len < 0 || len as usize > MAX_NBT_BYTES / 8 {
                    return Err("NBT long array is too large.".into());
                }
                let mut values = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    values.push(self.i64()?);
                }
                Nbt::LongArray(values)
            }
            _ => return Err(format!("Unknown NBT tag type {kind}.")),
        })
    }
}

fn parse_nbt(bytes: &[u8]) -> ParseResult<Nbt> {
    if bytes.len() > MAX_NBT_BYTES {
        return Err("NBT is too large to inspect.".into());
    }
    NbtReader::new(bytes).root()
}
fn read_decoded(mut reader: impl Read, label: &str) -> ParseResult<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(MAX_NBT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{label} could not be decompressed: {e}"))?;
    if bytes.len() > MAX_NBT_BYTES {
        return Err(format!("{label} is too large after decompression."));
    }
    Ok(bytes)
}
fn decompress_gzip(bytes: &[u8]) -> ParseResult<Vec<u8>> {
    read_decoded(GzDecoder::new(bytes), "Gzip data")
}
fn decompress_zlib(bytes: &[u8]) -> ParseResult<Vec<u8>> {
    read_decoded(ZlibDecoder::new(bytes), "Zlib data")
}
fn decompress_playerdata(bytes: &[u8]) -> ParseResult<Vec<u8>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        decompress_gzip(bytes)
    } else if bytes.first().map(|b| b & 0x0f) == Some(8) {
        decompress_zlib(bytes)
    } else {
        Ok(bytes.to_vec())
    }
}

// ---------------------------------------------------------------------------
// Snapshot access
// ---------------------------------------------------------------------------

struct Snapshot {
    root: PathBuf,
    dir: PathBuf,
    backup: Backup,
    manifest: Option<Manifest>,
}

impl Snapshot {
    fn open(m: &Manager, id: &str, backup_id: &str) -> Result<Self> {
        let root = backup::backup_root(m, id)?;
        let dir = storage::managed(&root, backup_id)?;
        let backup = backup::read(&dir)?;
        let manifest = if backup.format == BackupFormat::Chunked {
            Some(backup_store::load_manifest(&dir)?)
        } else {
            None
        };
        Ok(Self {
            root,
            dir,
            backup,
            manifest,
        })
    }
    fn validate_path(&self, relative: &str) -> Result<()> {
        if relative.is_empty() || relative.len() > 1024 || relative.contains('\\') {
            return Err(corrupt("The backup contains an unsafe file path."));
        }
        let mut parts = relative.split('/');
        let world = parts.next().unwrap_or("");
        if !self.backup.worlds.iter().any(|name| name == world) {
            return Err(corrupt("The file is outside the backed-up worlds."));
        }
        for part in parts {
            if !safe_component(part) {
                return Err(corrupt("The backup contains an unsafe file name."));
            }
        }
        Ok(())
    }
    fn files(&self) -> Result<Vec<String>> {
        if let Some(manifest) = &self.manifest {
            return Ok(manifest.files.keys().cloned().collect());
        }
        let mut files = Vec::new();
        for world in &self.backup.worlds {
            let dir = storage::child(&self.dir, world)?;
            if dir.is_dir() {
                walk_files(&dir, world, 0, &mut files)?;
            }
        }
        files.sort();
        Ok(files)
    }
    fn read(&self, relative: &str) -> Result<Vec<u8>> {
        self.validate_path(relative)?;
        if let Some(manifest) = &self.manifest {
            if !manifest.files.contains_key(relative) {
                return Err(corrupt(format!("The backup does not contain {relative}.")));
            }
            return backup_store::read_file(&self.root, &self.dir, relative);
        }
        let mut path = self.dir.clone();
        for part in relative.split('/') {
            path = storage::child(&path, part)?;
        }
        storage::no_links(&path)?;
        let meta = fs::metadata(&path).map_err(|e| corrupt(format!("{}: {e}", path.display())))?;
        if !meta.is_file() {
            return Err(corrupt("The requested world file is missing."));
        }
        if meta.len() > MAX_FILE_BYTES {
            return Err(AppError::user("That world file is too large to inspect."));
        }
        Ok(fs::read(&path)?)
    }
}
fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['\\', '/', '\0'])
        && !name.chars().any(char::is_control)
}
fn walk_files(root: &Path, relative: &str, depth: u16, files: &mut Vec<String>) -> Result<()> {
    if depth > 64 || files.len() >= 250_000 {
        return Err(AppError::user("The world folder is too large to inspect."));
    }
    storage::no_links(root)?;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        storage::no_links(&path)?;
        let name = entry
            .file_name()
            .to_str()
            .ok_or_else(|| AppError::user("A world file name is not valid Unicode."))?
            .to_string();
        let child = format!("{relative}/{name}");
        if entry.file_type()?.is_dir() {
            walk_files(&path, &child, depth + 1, files)?;
        } else if entry.file_type()?.is_file() {
            files.push(child);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Region index
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionKind {
    Region,
    Entities,
}
#[derive(Debug, Clone)]
struct RegionFile {
    world: String,
    folder: String,
    label: String,
    x: i32,
    z: i32,
    relative: String,
    bytes: u64,
    sha256: Option<String>,
    kind: RegionKind,
}

fn parse_region_path(path: &str, kind: RegionKind) -> Option<(String, String, i32, i32)> {
    let marker = match kind {
        RegionKind::Region => "/region/",
        RegionKind::Entities => "/entities/",
    };
    let at = path.rfind(marker)?;
    let file = &path[at + marker.len()..];
    if file.contains('/') {
        return None;
    }
    let stem = file.strip_suffix(".mca")?;
    let mut parts = stem.split('.');
    if parts.next()? != "r" {
        return None;
    }
    let x = parts.next()?.parse().ok()?;
    let z = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    let prefix = path[..at].trim_end_matches('/');
    let mut split = prefix.splitn(2, '/');
    let world = split.next()?.to_string();
    let folder = split.next().unwrap_or("").to_string();
    Some((world, folder, x, z))
}
fn dimension_label(world: &str, folder: &str) -> String {
    match folder {
        "" => {
            if world.ends_with("_nether") {
                "the_nether".into()
            } else if world.ends_with("_the_end") {
                "the_end".into()
            } else {
                "overworld".into()
            }
        }
        "DIM-1" => "the_nether".into(),
        "DIM1" => "the_end".into(),
        other => other
            .strip_prefix("dimensions/")
            .unwrap_or(other)
            .to_string(),
    }
}
fn region_files(snapshot: &Snapshot) -> Result<Vec<RegionFile>> {
    let mut result = Vec::new();
    for relative in snapshot.files()? {
        let region = match parse_region_path(&relative, RegionKind::Region) {
            Some(value) => (RegionKind::Region, value),
            None => match parse_region_path(&relative, RegionKind::Entities) {
                Some(value) => (RegionKind::Entities, value),
                None => continue,
            },
        };
        let (kind, (world, folder, x, z)) = region;
        if !snapshot.backup.worlds.iter().any(|name| name == &world) {
            continue;
        }
        let (bytes, sha256) = if let Some(manifest) = &snapshot.manifest {
            match manifest.files.get(&relative) {
                Some(file) => (file.bytes, Some(file.sha256.clone())),
                None => continue,
            }
        } else {
            let mut path = snapshot.dir.clone();
            let mut valid = true;
            for part in relative.split('/') {
                match storage::child(&path, part) {
                    Ok(next) => path = next,
                    Err(_) => {
                        valid = false;
                        break;
                    }
                }
            }
            if !valid {
                continue;
            }
            let bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            (bytes, None)
        };
        result.push(RegionFile {
            label: dimension_label(&world, &folder),
            world,
            folder,
            x,
            z,
            relative,
            bytes,
            sha256,
            kind,
        });
    }
    result.sort_by(|a, b| {
        (
            &a.world,
            &a.folder,
            a.x,
            a.z,
            a.kind == RegionKind::Entities,
        )
            .cmp(&(
                &b.world,
                &b.folder,
                b.x,
                b.z,
                b.kind == RegionKind::Entities,
            ))
    });
    Ok(result)
}
fn chunk_coords(index: usize) -> (i32, i32) {
    ((index % 32) as i32, (index / 32) as i32)
}

// ---------------------------------------------------------------------------
// Region parsing and statistics
// ---------------------------------------------------------------------------

struct HeaderEntry {
    offset: usize,
    sectors: u8,
}
fn header_entries(bytes: &[u8]) -> ParseResult<Vec<Option<HeaderEntry>>> {
    if bytes.len() < REGION_HEADER {
        return Err("The region file is shorter than its header.".into());
    }
    let mut entries = Vec::with_capacity(1024);
    let mut intervals = Vec::new();
    for index in 0..1024 {
        let at = index * 4;
        let offset =
            ((bytes[at] as usize) << 16 | (bytes[at + 1] as usize) << 8 | bytes[at + 2] as usize)
                * 4096;
        let sectors = bytes[at + 3];
        if offset == 0 && sectors == 0 {
            entries.push(None);
            continue;
        }
        if sectors == 0 {
            entries.push(Some(HeaderEntry { offset, sectors: 0 }));
            continue;
        }
        let end = offset.saturating_add(sectors as usize * 4096);
        if offset < REGION_HEADER || end > bytes.len() {
            entries.push(Some(HeaderEntry { offset, sectors: 0 }));
            continue;
        }
        intervals.push((offset, end, index));
        entries.push(Some(HeaderEntry { offset, sectors }));
    }
    intervals.sort_by_key(|entry| entry.0);
    for pair in intervals.windows(2) {
        if pair[1].0 < pair[0].1 {
            for index in [pair[0].2, pair[1].2] {
                if let Some(Some(entry)) = entries.get_mut(index) {
                    entry.sectors = 0;
                }
            }
        }
    }
    Ok(entries)
}
fn region_chunk_nbt(bytes: &[u8], entry: &HeaderEntry) -> ParseResult<Nbt> {
    if entry.sectors == 0 || entry.offset < REGION_HEADER {
        return Err("The region header points at an invalid chunk.".into());
    }
    if entry.offset + 5 > bytes.len() {
        return Err("The chunk points past the end of the region file.".into());
    }
    let length = u32::from_be_bytes([
        bytes[entry.offset],
        bytes[entry.offset + 1],
        bytes[entry.offset + 2],
        bytes[entry.offset + 3],
    ]) as usize;
    if length < 1 {
        return Err("The chunk has an empty payload.".into());
    }
    let end = entry.offset + 4 + length;
    if end > bytes.len() || end > entry.offset + entry.sectors as usize * 4096 {
        return Err("The chunk payload exceeds its reserved sectors.".into());
    }
    let compression = bytes[entry.offset + 4];
    let payload = &bytes[entry.offset + 5..end];
    let nbt = match compression {
        1 => decompress_gzip(payload)?,
        2 => decompress_zlib(payload)?,
        3 => payload.to_vec(),
        4 => return Err("LZ4-compressed chunks are not supported yet.".into()),
        other => return Err(format!("Unknown chunk compression type {other}.")),
    };
    parse_nbt(&nbt)
}

struct Section {
    y: i32,
    palette: Vec<String>,
    data: Vec<i64>,
    compact: bool,
}
struct ChunkSummary {
    x: Option<i32>,
    z: Option<i32>,
    block_entities: Vec<String>,
    entities: Vec<String>,
    sections: Vec<Section>,
}
fn tag_names(value: Option<&Nbt>) -> Vec<String> {
    value
        .and_then(Nbt::as_list)
        .map(|list| {
            list.iter()
                .filter_map(|entry| {
                    entry
                        .get("id")
                        .and_then(Nbt::as_string)
                        .or_else(|| entry.get("Id").and_then(Nbt::as_string))
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}
fn read_sections(level: &Nbt, data_version: i32) -> ParseResult<Vec<Section>> {
    let list = level
        .get("sections")
        .and_then(Nbt::as_list)
        .or_else(|| level.get("Sections").and_then(Nbt::as_list));
    let Some(list) = list else {
        return Ok(Vec::new());
    };
    let mut sections = Vec::new();
    for (index, section) in list.iter().enumerate() {
        let Some(section) = section.as_compound() else {
            continue;
        };
        let y = section
            .get("Y")
            .and_then(Nbt::as_i64)
            .map(|value| value as i32)
            .unwrap_or(index as i32);
        let states = section.get("block_states").and_then(Nbt::as_compound);
        let (palette_value, data_value, compact) = if let Some(states) = states {
            (states.get("palette"), states.get("data"), true)
        } else {
            (
                section.get("Palette"),
                section.get("BlockStates"),
                data_version >= 2529,
            )
        };
        let Some(palette_value) = palette_value else {
            continue;
        };
        let Some(palette_list) = palette_value.as_list() else {
            return Err("A section palette is not a list.".into());
        };
        let mut palette = Vec::with_capacity(palette_list.len());
        for entry in palette_list {
            let name = entry
                .get("Name")
                .and_then(Nbt::as_string)
                .or_else(|| entry.as_string())
                .ok_or_else(|| "A section palette entry has no block name.".to_string())?;
            palette.push(name.to_string());
        }
        if palette.is_empty() || palette.len() > 4096 {
            return Err("A section has an invalid palette size.".into());
        }
        let data = data_value.and_then(Nbt::as_longs).unwrap_or(&[]).to_vec();
        sections.push(Section {
            y,
            palette,
            data,
            compact,
        });
    }
    Ok(sections)
}
fn inspect_chunk(nbt: &Nbt, with_sections: bool) -> ParseResult<ChunkSummary> {
    let root = nbt
        .as_compound()
        .ok_or_else(|| "The chunk root is not a compound.".to_string())?;
    let level = match root.get("Level") {
        Some(level) if level.as_compound().is_some() => level,
        _ => nbt,
    };
    let level_map = level
        .as_compound()
        .ok_or_else(|| "The chunk level is not a compound.".to_string())?;
    let x = level_map
        .get("xPos")
        .or_else(|| root.get("xPos"))
        .and_then(Nbt::as_i64)
        .map(|v| v as i32);
    let z = level_map
        .get("zPos")
        .or_else(|| root.get("zPos"))
        .and_then(Nbt::as_i64)
        .map(|v| v as i32);
    let block_entities = tag_names(
        level_map
            .get("block_entities")
            .or_else(|| level_map.get("TileEntities")),
    );
    let entities = tag_names(level_map.get("Entities"));
    let data_version = root
        .get("DataVersion")
        .or_else(|| level_map.get("DataVersion"))
        .and_then(Nbt::as_i64)
        .unwrap_or(0) as i32;
    let sections = if with_sections {
        read_sections(level, data_version)?
    } else {
        Vec::new()
    };
    Ok(ChunkSummary {
        x,
        z,
        block_entities,
        entities,
        sections,
    })
}

#[derive(Default)]
struct RegionScan {
    chunks: u32,
    corrupt: u32,
    block_entities: u32,
    entities: u32,
    block_types: BTreeMap<String, u32>,
    entity_types: BTreeMap<String, u32>,
    corruptions: Vec<(Option<(i32, i32)>, String)>,
    truncated: bool,
}
fn add_types(map: &mut BTreeMap<String, u32>, values: &[String]) {
    for value in values {
        *map.entry(value.clone()).or_insert(0) += 1;
    }
}
fn scan_region(bytes: &[u8], rx: i32, rz: i32, kind: RegionKind, budget: &mut usize) -> RegionScan {
    let mut scan = RegionScan::default();
    let entries = match header_entries(bytes) {
        Ok(entries) => entries,
        Err(error) => {
            scan.corrupt = 1;
            scan.corruptions.push((None, error));
            return scan;
        }
    };
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        if entry.sectors == 0 {
            scan.corrupt += 1;
            scan.corruptions.push((
                Some((
                    rx * 32 + chunk_coords(index).0,
                    rz * 32 + chunk_coords(index).1,
                )),
                "The region header points at an invalid or overlapping chunk.".into(),
            ));
            continue;
        }
        if *budget == 0 {
            scan.truncated = true;
            break;
        }
        *budget -= 1;
        let expected = (
            rx * 32 + chunk_coords(index).0,
            rz * 32 + chunk_coords(index).1,
        );
        let parsed = region_chunk_nbt(bytes, entry).and_then(|nbt| inspect_chunk(&nbt, false));
        match parsed {
            Ok(chunk) => {
                if chunk.x.is_some_and(|x| x != expected.0)
                    || chunk.z.is_some_and(|z| z != expected.1)
                {
                    scan.corrupt += 1;
                    scan.corruptions.push((
                        Some(expected),
                        "The chunk coordinates do not match its region slot.".into(),
                    ));
                    continue;
                }
                scan.chunks += 1;
                if kind == RegionKind::Region {
                    scan.block_entities += chunk.block_entities.len() as u32;
                    add_types(&mut scan.block_types, &chunk.block_entities);
                }
                scan.entities += chunk.entities.len() as u32;
                add_types(&mut scan.entity_types, &chunk.entities);
            }
            Err(error) => {
                scan.corrupt += 1;
                scan.corruptions.push((Some(expected), error));
            }
        }
    }
    scan
}

// ---------------------------------------------------------------------------
// Colours and map rendering
// ---------------------------------------------------------------------------

fn colour_word(name: &str) -> Option<(u8, u8, u8)> {
    let words = [
        ("light_blue", (92, 168, 204)),
        ("light_gray", (154, 154, 154)),
        ("magenta", (189, 82, 172)),
        ("orange", (207, 124, 55)),
        ("yellow", (201, 181, 74)),
        ("lime", (112, 185, 57)),
        ("pink", (220, 148, 172)),
        ("purple", (126, 74, 188)),
        ("brown", (109, 78, 50)),
        ("green", (78, 132, 56)),
        ("cyan", (63, 143, 147)),
        ("blue", (61, 92, 168)),
        ("red", (164, 57, 51)),
        ("black", (45, 45, 48)),
        ("gray", (110, 110, 110)),
        ("white", (220, 220, 215)),
    ];
    words
        .iter()
        .find(|(word, _)| name.contains(word))
        .map(|(_, rgb)| *rgb)
}
fn block_colour(name: &str) -> (u8, u8, u8) {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let name = name.to_ascii_lowercase();
    if name.contains("water") {
        return (52, 105, 160);
    }
    if name.contains("lava") || name.contains("magma") {
        return (200, 91, 32);
    }
    if name.contains("grass") || name.contains("moss") || name.contains("mycelium") {
        return (92, 142, 63);
    }
    if name.contains("leaves") {
        return (63, 113, 53);
    }
    if name.contains("sand") {
        return (208, 190, 131);
    }
    if name.contains("snow") || name.contains("powder_snow") {
        return (226, 233, 236);
    }
    if name.contains("ice") {
        return (145, 184, 205);
    }
    if name.contains("log") || name.contains("wood") || name.contains("plank") {
        return (128, 99, 61);
    }
    if name.contains("dirt") || name.contains("mud") || name.contains("farmland") {
        return (111, 82, 56);
    }
    if name.contains("gravel") || name.contains("clay") {
        return (142, 136, 128);
    }
    if name.contains("deepslate") || name.contains("stone") || name.contains("cobble") {
        return (120, 120, 120);
    }
    if name.contains("terracotta") || name.contains("concrete") || name.contains("wool") {
        if let Some(colour) = colour_word(&name) {
            return colour;
        }
        return (145, 106, 88);
    }
    if name.contains("netherrack") {
        return (109, 48, 48);
    }
    if name.contains("end_stone") || name.contains("purpur") {
        return (209, 204, 159);
    }
    if name.contains("obsidian") || name.contains("basalt") || name.contains("blackstone") {
        return (47, 43, 55);
    }
    if name == "air" || name == "cave_air" || name == "void_air" {
        return (0, 0, 0);
    }
    let hash = name.bytes().fold(0u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u32)
    });
    let palette = [
        (108, 122, 94),
        (126, 116, 96),
        (100, 116, 132),
        (138, 128, 112),
        (118, 134, 108),
        (132, 112, 118),
    ];
    palette[(hash as usize) % palette.len()]
}
fn is_air(name: &str) -> bool {
    matches!(
        name.strip_prefix("minecraft:").unwrap_or(name),
        "air" | "cave_air" | "void_air"
    )
}
fn shade(rgb: (u8, u8, u8), y: i32, x: i32, z: i32) -> [u8; 4] {
    let height = ((y as f32 + 64.0) / 400.0).clamp(0.0, 1.0);
    let texture = {
        let hash = (x as u32)
            .wrapping_mul(73_856_093)
            .wrapping_add((z as u32).wrapping_mul(19_349_663));
        ((hash >> 8) % 9) as f32 * 0.012 - 0.048
    };
    let factor = (0.64 + 0.42 * height + texture).clamp(0.45, 1.25);
    [
        (rgb.0 as f32 * factor).clamp(0.0, 255.0) as u8,
        (rgb.1 as f32 * factor).clamp(0.0, 255.0) as u8,
        (rgb.2 as f32 * factor).clamp(0.0, 255.0) as u8,
        255,
    ]
}
fn bits_for(palette: usize) -> u32 {
    if palette <= 1 {
        return 4;
    }
    let mut bits = 32 - (palette as u32 - 1).leading_zeros();
    bits = bits.max(4);
    bits
}
fn section_index(section: &Section, i: usize) -> Option<usize> {
    let bits = bits_for(section.palette.len()) as usize;
    let mask = (1u64 << bits) - 1;
    if section.compact {
        let per_long = 64 / bits;
        let long = *section.data.get(i / per_long)? as u64;
        Some(((long >> ((i % per_long) * bits)) & mask) as usize)
    } else {
        let bit = i * bits;
        let long_index = bit / 64;
        let offset = bit % 64;
        let mut value = *section.data.get(long_index)? as u64 >> offset;
        if offset + bits > 64 {
            value |= (*section.data.get(long_index + 1)? as u64) << (64 - offset);
        }
        Some((value & mask) as usize)
    }
}
fn fill_tops(section: &Section, tops: &mut [Option<(String, i32)>]) -> ParseResult<()> {
    let base_y = section.y * 16;
    if section.palette.len() == 1 {
        let name = &section.palette[0];
        if !is_air(name) {
            for top in tops.iter_mut() {
                if top.is_none() {
                    *top = Some((name.clone(), base_y + 15));
                }
            }
        }
        return Ok(());
    }
    if section.data.is_empty() {
        return Err("A multi-block section has no block data.".into());
    }
    for i in 0..4096 {
        let x = i % 16;
        let z = (i / 16) % 16;
        let y = i / 256;
        let top_index = z * 16 + x;
        if tops[top_index].is_some() {
            continue;
        }
        let Some(palette_index) = section_index(section, i) else {
            return Err("A section's block data is truncated.".into());
        };
        let Some(name) = section.palette.get(palette_index) else {
            return Err("A section refers to a missing palette entry.".into());
        };
        if !is_air(name) {
            tops[top_index] = Some((name.clone(), base_y + y as i32));
        }
    }
    Ok(())
}
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let value = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((value >> 18) & 63) as usize] as char);
        out.push(TABLE[((value >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

struct RenderResult {
    rgba: Vec<u8>,
    chunks: u32,
    corrupt: u32,
    empty: u32,
    warnings: Vec<String>,
}
fn render_region(bytes: &[u8], rx: i32, rz: i32, scale: u32) -> RenderResult {
    let width = 512 / scale;
    let mut rgba = vec![0u8; (width * width * 4) as usize];
    let mut result = RenderResult {
        rgba: Vec::new(),
        chunks: 0,
        corrupt: 0,
        empty: 0,
        warnings: Vec::new(),
    };
    let entries = match header_entries(bytes) {
        Ok(entries) => entries,
        Err(error) => {
            result.corrupt = 1;
            result.empty = 1024;
            result.warnings.push(error);
            result.rgba = rgba;
            return result;
        }
    };
    let mut present = 0;
    for (index, entry) in entries.iter().enumerate() {
        let Some(entry) = entry else {
            continue;
        };
        present += 1;
        if entry.sectors == 0 {
            result.corrupt += 1;
            continue;
        }
        let expected = (
            rx * 32 + chunk_coords(index).0,
            rz * 32 + chunk_coords(index).1,
        );
        let parsed = region_chunk_nbt(bytes, entry).and_then(|nbt| inspect_chunk(&nbt, true));
        let summary = match parsed {
            Ok(summary) => summary,
            Err(error) => {
                result.corrupt += 1;
                if result.warnings.len() < 12 {
                    result
                        .warnings
                        .push(format!("Chunk {},{}: {error}", expected.0, expected.1));
                }
                continue;
            }
        };
        if summary.x.is_some_and(|x| x != expected.0) || summary.z.is_some_and(|z| z != expected.1)
        {
            result.corrupt += 1;
            continue;
        }
        let mut tops: Vec<Option<(String, i32)>> = vec![None; 256];
        let mut sections = summary.sections;
        sections.sort_by_key(|section| std::cmp::Reverse(section.y));
        for section in &sections {
            if tops.iter().all(Option::is_some) {
                break;
            }
            if let Err(error) = fill_tops(section, &mut tops) {
                result.corrupt += 1;
                if result.warnings.len() < 12 {
                    result
                        .warnings
                        .push(format!("Chunk {},{}: {error}", expected.0, expected.1));
                }
                break;
            }
        }
        let block_x = chunk_coords(index).0 * 16;
        let block_z = chunk_coords(index).1 * 16;
        for (local_z, row) in tops.chunks(16).enumerate() {
            for (local_x, top) in row.iter().enumerate() {
                let Some((name, y)) = top else {
                    continue;
                };
                let x = block_x + local_x as i32;
                let z = block_z + local_z as i32;
                let px = (x as u32) / scale;
                let pz = (z as u32) / scale;
                if px >= width || pz >= width {
                    continue;
                }
                let colour = shade(block_colour(name), *y, rx * 512 + x, rz * 512 + z);
                let at = ((pz * width + px) * 4) as usize;
                rgba[at..at + 4].copy_from_slice(&colour);
            }
        }
        result.chunks += 1;
    }
    result.empty = 1024u32.saturating_sub(present);
    result.rgba = rgba;
    result
}

// ---------------------------------------------------------------------------
// Player positions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPlayer {
    pub uuid: String,
    pub name: String,
    pub world: String,
    pub dimension: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub health: f64,
    pub food: i32,
    pub xp: i32,
    pub observed_at: String,
}
#[derive(Deserialize)]
struct UserCacheEntry {
    name: String,
    uuid: String,
}
fn uuid_key(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '-')
        .collect::<String>()
        .to_ascii_lowercase()
}
fn player_names(m: &Manager, id: &str) -> BTreeMap<String, String> {
    let Ok(dir) = m.dir(id) else {
        return BTreeMap::new();
    };
    let Ok(path) = storage::child(&dir, "usercache.json") else {
        return BTreeMap::new();
    };
    if storage::no_links(&path).is_err() {
        return BTreeMap::new();
    }
    let Ok(bytes) = fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(entries) = serde_json::from_slice::<Vec<UserCacheEntry>>(&bytes) else {
        return BTreeMap::new();
    };
    entries
        .into_iter()
        .map(|entry| (uuid_key(&entry.uuid), entry.name))
        .collect()
}
fn normalise_dimension(value: &str) -> String {
    if let Some(value) = value.strip_prefix("minecraft:") {
        return value.to_string();
    }
    value.replacen(':', "/", 1)
}
fn fallback_dimension(world: &str) -> String {
    if world.ends_with("_nether") {
        "the_nether".into()
    } else if world.ends_with("_the_end") {
        "the_end".into()
    } else {
        "overworld".into()
    }
}
fn load_players(snapshot: &Snapshot, names: &BTreeMap<String, String>) -> Result<Vec<WorldPlayer>> {
    let mut players = Vec::new();
    for relative in snapshot.files()? {
        if players.len() >= MAX_PLAYERS {
            break;
        }
        let parts: Vec<&str> = relative.split('/').collect();
        if parts.len() != 3 || parts[1] != "playerdata" {
            continue;
        }
        let Some(filename) = parts[2].strip_suffix(".dat") else {
            continue;
        };
        let Ok(bytes) = snapshot.read(&relative) else {
            continue;
        };
        let parsed = decompress_playerdata(&bytes)
            .and_then(|nbt| parse_nbt(&nbt))
            .and_then(|nbt| {
                let root = nbt
                    .as_compound()
                    .ok_or_else(|| "The player data root is not a compound.".to_string())?;
                let pos = root
                    .get("Pos")
                    .and_then(Nbt::as_list)
                    .ok_or_else(|| "The player data has no position.".to_string())?;
                if pos.len() < 3 {
                    return Err("The player position is incomplete.".into());
                }
                let x = pos[0]
                    .as_f64()
                    .filter(|value| value.is_finite() && value.abs() < 30_000_000.0)
                    .ok_or_else(|| "The player X coordinate is invalid.".to_string())?;
                let y = pos[1]
                    .as_f64()
                    .filter(|value| value.is_finite() && value.abs() < 30_000_000.0)
                    .ok_or_else(|| "The player Y coordinate is invalid.".to_string())?;
                let z = pos[2]
                    .as_f64()
                    .filter(|value| value.is_finite() && value.abs() < 30_000_000.0)
                    .ok_or_else(|| "The player Z coordinate is invalid.".to_string())?;
                let dimension = root
                    .get("Dimension")
                    .and_then(Nbt::as_string)
                    .map(normalise_dimension)
                    .unwrap_or_else(|| fallback_dimension(parts[0]));
                let health = root
                    .get("Health")
                    .and_then(Nbt::as_f64)
                    .filter(|value| value.is_finite())
                    .unwrap_or(20.0)
                    .clamp(0.0, 20.0);
                let food = root
                    .get("foodLevel")
                    .and_then(Nbt::as_i64)
                    .unwrap_or(20)
                    .clamp(0, 20) as i32;
                let xp = root
                    .get("XpLevel")
                    .and_then(Nbt::as_i64)
                    .unwrap_or(0)
                    .clamp(0, 100_000) as i32;
                Ok(WorldPlayer {
                    uuid: filename.to_string(),
                    name: names
                        .get(&uuid_key(filename))
                        .cloned()
                        .unwrap_or_else(|| filename.to_string()),
                    world: parts[0].to_string(),
                    dimension,
                    x,
                    y,
                    z,
                    health,
                    food,
                    xp,
                    observed_at: snapshot.backup.created_at.clone(),
                })
            });
        if let Ok(player) = parsed {
            players.push(player);
        }
    }
    players.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(players)
}

// ---------------------------------------------------------------------------
// Preview report
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldCount {
    pub id: String,
    pub count: u32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldRegion {
    pub x: i32,
    pub z: i32,
    pub bytes: u64,
    pub chunks: u32,
    pub corrupt_chunks: u32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDimension {
    pub world: String,
    pub dimension: String,
    pub label: String,
    pub regions: Vec<WorldRegion>,
    pub chunks: u32,
    pub corrupt_chunks: u32,
    pub block_entities: u32,
    pub entities: u32,
    pub block_entity_types: Vec<WorldCount>,
    pub entity_types: Vec<WorldCount>,
    pub entity_regions: u32,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldCorruption {
    pub world: String,
    pub dimension: String,
    pub label: String,
    pub region_x: i32,
    pub region_z: i32,
    pub chunk_x: Option<i32>,
    pub chunk_z: Option<i32>,
    pub reason: String,
}
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldTotals {
    pub dimensions: u32,
    pub regions: u32,
    pub chunks: u32,
    pub corrupt_chunks: u32,
    pub block_entities: u32,
    pub entities: u32,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldPreview {
    pub backup_id: String,
    pub backup_created_at: String,
    pub dimensions: Vec<WorldDimension>,
    pub players: Vec<WorldPlayer>,
    pub tracks: Vec<WorldTrack>,
    pub totals: WorldTotals,
    pub corruptions: Vec<WorldCorruption>,
    pub omitted_corruptions: u32,
    pub warnings: Vec<String>,
    pub generated_at: String,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldMap {
    pub backup_id: String,
    pub world: String,
    pub dimension: String,
    pub label: String,
    pub region_x: i32,
    pub region_z: i32,
    pub origin_x: i32,
    pub origin_z: i32,
    pub scale: u32,
    pub width: u32,
    pub height: u32,
    pub pixels: String,
    pub chunks: u32,
    pub corrupt_chunks: u32,
    pub empty_chunks: u32,
    pub players: Vec<WorldPlayer>,
    pub tracks: Vec<WorldTrack>,
    pub warnings: Vec<String>,
    pub generated_at: String,
}

fn top_types(map: &BTreeMap<String, u32>) -> Vec<WorldCount> {
    let mut values: Vec<_> = map
        .iter()
        .map(|(id, count)| WorldCount {
            id: id.clone(),
            count: *count,
        })
        .collect();
    values.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.id.cmp(&b.id)));
    values.truncate(16);
    values
}
fn push_corruption(list: &mut Vec<WorldCorruption>, omitted: &mut u32, item: WorldCorruption) {
    if list.len() < MAX_CORRUPTIONS {
        list.push(item);
    } else {
        *omitted = omitted.saturating_add(1);
    }
}
fn build_preview(
    snapshot: &Snapshot,
    names: &BTreeMap<String, String>,
    tracks: Vec<WorldTrack>,
) -> Result<WorldPreview> {
    let files = region_files(snapshot)?;
    let mut dimensions: BTreeMap<(String, String), WorldDimension> = BTreeMap::new();
    for file in &files {
        if file.kind == RegionKind::Region {
            dimensions
                .entry((file.world.clone(), file.folder.clone()))
                .or_insert_with(|| WorldDimension {
                    world: file.world.clone(),
                    dimension: file.folder.clone(),
                    label: file.label.clone(),
                    regions: Vec::new(),
                    chunks: 0,
                    corrupt_chunks: 0,
                    block_entities: 0,
                    entities: 0,
                    block_entity_types: Vec::new(),
                    entity_types: Vec::new(),
                    entity_regions: 0,
                    truncated: false,
                });
        } else {
            dimensions
                .entry((file.world.clone(), file.folder.clone()))
                .and_modify(|dimension| dimension.entity_regions += 1)
                .or_insert_with(|| WorldDimension {
                    world: file.world.clone(),
                    dimension: file.folder.clone(),
                    label: file.label.clone(),
                    regions: Vec::new(),
                    chunks: 0,
                    corrupt_chunks: 0,
                    block_entities: 0,
                    entities: 0,
                    block_entity_types: Vec::new(),
                    entity_types: Vec::new(),
                    entity_regions: 1,
                    truncated: false,
                });
        }
    }
    let mut region_budget = MAX_SCAN_REGIONS;
    let mut chunk_budget = MAX_SCAN_CHUNKS;
    let mut corruptions = Vec::new();
    let mut omitted_corruptions = 0u32;
    let mut block_types: HashMap<(String, String), BTreeMap<String, u32>> = HashMap::new();
    let mut entity_types: HashMap<(String, String), BTreeMap<String, u32>> = HashMap::new();
    let mut warnings = Vec::new();
    for file in files.iter().filter(|file| file.kind == RegionKind::Region) {
        let key = (file.world.clone(), file.folder.clone());
        let Some(dimension) = dimensions.get_mut(&key) else {
            continue;
        };
        if region_budget == 0 {
            dimension.truncated = true;
            continue;
        }
        region_budget -= 1;
        let bytes = match snapshot.read(&file.relative) {
            Ok(bytes) => bytes,
            Err(error) => {
                dimension.corrupt_chunks += 1;
                push_corruption(
                    &mut corruptions,
                    &mut omitted_corruptions,
                    WorldCorruption {
                        world: file.world.clone(),
                        dimension: file.folder.clone(),
                        label: file.label.clone(),
                        region_x: file.x,
                        region_z: file.z,
                        chunk_x: None,
                        chunk_z: None,
                        reason: error.message.clone(),
                    },
                );
                continue;
            }
        };
        let scan = scan_region(
            &bytes,
            file.x,
            file.z,
            RegionKind::Region,
            &mut chunk_budget,
        );
        if scan.truncated {
            dimension.truncated = true;
        }
        dimension.chunks += scan.chunks;
        dimension.corrupt_chunks += scan.corrupt;
        dimension.block_entities += scan.block_entities;
        dimension.entities += scan.entities;
        let entry = block_types.entry(key.clone()).or_default();
        for (id, count) in scan.block_types {
            *entry.entry(id).or_insert(0) += count;
        }
        let entry = entity_types.entry(key.clone()).or_default();
        for (id, count) in scan.entity_types {
            *entry.entry(id).or_insert(0) += count;
        }
        for (coords, reason) in scan.corruptions {
            push_corruption(
                &mut corruptions,
                &mut omitted_corruptions,
                WorldCorruption {
                    world: file.world.clone(),
                    dimension: file.folder.clone(),
                    label: file.label.clone(),
                    region_x: file.x,
                    region_z: file.z,
                    chunk_x: coords.map(|value| value.0),
                    chunk_z: coords.map(|value| value.1),
                    reason,
                },
            );
        }
        dimension.regions.push(WorldRegion {
            x: file.x,
            z: file.z,
            bytes: file.bytes,
            chunks: scan.chunks,
            corrupt_chunks: scan.corrupt,
        });
    }
    let mut entity_budget = MAX_SCAN_REGIONS;
    for file in files
        .iter()
        .filter(|file| file.kind == RegionKind::Entities)
    {
        let key = (file.world.clone(), file.folder.clone());
        let Some(dimension) = dimensions.get_mut(&key) else {
            continue;
        };
        if entity_budget == 0 {
            dimension.truncated = true;
            continue;
        }
        entity_budget -= 1;
        let bytes = match snapshot.read(&file.relative) {
            Ok(bytes) => bytes,
            Err(error) => {
                dimension.corrupt_chunks += 1;
                push_corruption(
                    &mut corruptions,
                    &mut omitted_corruptions,
                    WorldCorruption {
                        world: file.world.clone(),
                        dimension: file.folder.clone(),
                        label: file.label.clone(),
                        region_x: file.x,
                        region_z: file.z,
                        chunk_x: None,
                        chunk_z: None,
                        reason: error.message.clone(),
                    },
                );
                continue;
            }
        };
        let scan = scan_region(
            &bytes,
            file.x,
            file.z,
            RegionKind::Entities,
            &mut chunk_budget,
        );
        if scan.truncated {
            dimension.truncated = true;
        }
        dimension.chunks += scan.chunks;
        dimension.corrupt_chunks += scan.corrupt;
        dimension.entities += scan.entities;
        let entry = entity_types.entry(key.clone()).or_default();
        for (id, count) in scan.entity_types {
            *entry.entry(id).or_insert(0) += count;
        }
        for (coords, reason) in scan.corruptions {
            push_corruption(
                &mut corruptions,
                &mut omitted_corruptions,
                WorldCorruption {
                    world: file.world.clone(),
                    dimension: file.folder.clone(),
                    label: file.label.clone(),
                    region_x: file.x,
                    region_z: file.z,
                    chunk_x: coords.map(|value| value.0),
                    chunk_z: coords.map(|value| value.1),
                    reason,
                },
            );
        }
    }
    for ((world, folder), dimension) in dimensions.iter_mut() {
        dimension.block_entity_types = block_types
            .remove(&(world.clone(), folder.clone()))
            .map(|map| top_types(&map))
            .unwrap_or_default();
        dimension.entity_types = entity_types
            .remove(&(world.clone(), folder.clone()))
            .map(|map| top_types(&map))
            .unwrap_or_default();
    }
    let mut dimensions: Vec<_> = dimensions.into_values().collect();
    dimensions.sort_by(|a, b| {
        (&a.world, &a.label, &a.dimension).cmp(&(&b.world, &b.label, &b.dimension))
    });
    let players = load_players(snapshot, names)?;
    let totals = WorldTotals {
        dimensions: dimensions.len() as u32,
        regions: dimensions.iter().map(|d| d.regions.len() as u32).sum(),
        chunks: dimensions.iter().map(|d| d.chunks).sum(),
        corrupt_chunks: dimensions.iter().map(|d| d.corrupt_chunks).sum(),
        block_entities: dimensions.iter().map(|d| d.block_entities).sum(),
        entities: dimensions.iter().map(|d| d.entities).sum(),
    };
    if dimensions.is_empty() {
        warnings.push("No Anvil region files were found in this backup.".into());
    }
    if region_budget == 0 || chunk_budget == 0 || entity_budget == 0 {
        warnings.push(
            "The scan stopped at its safety limit; some regions or chunks were not inspected."
                .into(),
        );
    }
    if !corruptions.is_empty() || omitted_corruptions > 0 {
        warnings.push(format!(
            "{} corrupt or unreadable chunks were reported.",
            totals.corrupt_chunks
        ));
    }
    Ok(WorldPreview {
        backup_id: snapshot.backup.id.clone(),
        backup_created_at: snapshot.backup.created_at.clone(),
        dimensions,
        players,
        tracks,
        totals,
        corruptions,
        omitted_corruptions,
        warnings,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub fn preview(m: &Manager, id: &str, backup_id: &str) -> Result<WorldPreview> {
    let snapshot = Snapshot::open(m, id, backup_id)?;
    let names = player_names(m, id);
    let tracks = load_tracks(m, id).unwrap_or_default();
    build_preview(&snapshot, &names, tracks)
}

// ---------------------------------------------------------------------------
// Map rendering
// ---------------------------------------------------------------------------

pub fn map(
    m: &Manager,
    id: &str,
    backup_id: &str,
    world: &str,
    dimension: &str,
    region_x: i32,
    region_z: i32,
    scale: u32,
) -> Result<WorldMap> {
    if scale == 0 || scale > MAX_MAP_SCALE || !scale.is_power_of_two() {
        return Err(AppError::user("Choose a map scale of 1, 2, 4 or 8."));
    }
    if region_x.abs() > 1_000_000 || region_z.abs() > 1_000_000 {
        return Err(AppError::user("The requested region is outside the world."));
    }
    let snapshot = Snapshot::open(m, id, backup_id)?;
    let files = region_files(&snapshot)?;
    let Some(file) = files.iter().find(|file| {
        file.kind == RegionKind::Region
            && file.world == world
            && file.folder == dimension
            && file.x == region_x
            && file.z == region_z
    }) else {
        return Err(AppError::user("That region is not present in this backup."));
    };
    let bytes = snapshot.read(&file.relative)?;
    let rendered = render_region(&bytes, region_x, region_z, scale);
    let players = load_players(&snapshot, &player_names(m, id))?
        .into_iter()
        .filter(|player| player.world == file.world && player.dimension == file.label)
        .collect();
    let width = 512 / scale;
    Ok(WorldMap {
        backup_id: snapshot.backup.id.clone(),
        world: file.world.clone(),
        dimension: file.folder.clone(),
        label: file.label.clone(),
        region_x,
        region_z,
        origin_x: region_x.saturating_mul(512),
        origin_z: region_z.saturating_mul(512),
        scale,
        width,
        height: width,
        pixels: base64(&rendered.rgba),
        chunks: rendered.chunks,
        corrupt_chunks: rendered.corrupt,
        empty_chunks: rendered.empty,
        players,
        tracks: load_tracks(m, id).unwrap_or_default(),
        warnings: rendered.warnings,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

// ---------------------------------------------------------------------------
// Region-level snapshot comparison
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDiffChange {
    pub world: String,
    pub dimension: String,
    pub label: String,
    pub region_x: i32,
    pub region_z: i32,
    pub status: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldDiff {
    pub before_id: String,
    pub after_id: String,
    pub added: u32,
    pub removed: u32,
    pub changed: u32,
    pub unchanged: u32,
    pub changes: Vec<WorldDiffChange>,
    pub truncated: bool,
    pub generated_at: String,
}
fn digest(snapshot: &Snapshot, file: &RegionFile) -> Result<(u64, String)> {
    if let Some(hash) = &file.sha256 {
        return Ok((file.bytes, hash.clone()));
    }
    let bytes = snapshot.read(&file.relative)?;
    Ok((bytes.len() as u64, hex_sha256(&bytes)))
}
fn hex_sha256(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}
fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> Result<WorldDiff> {
    let before_files = region_files(before)?;
    let after_files = region_files(after)?;
    let mut left = BTreeMap::new();
    for file in before_files
        .iter()
        .filter(|file| file.kind == RegionKind::Region)
    {
        left.insert(
            (file.world.clone(), file.folder.clone(), file.x, file.z),
            file,
        );
    }
    let mut right = BTreeMap::new();
    for file in after_files
        .iter()
        .filter(|file| file.kind == RegionKind::Region)
    {
        right.insert(
            (file.world.clone(), file.folder.clone(), file.x, file.z),
            file,
        );
    }
    let mut keys: Vec<_> = left.keys().chain(right.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    let mut changes = Vec::new();
    let (mut added, mut removed, mut changed, mut unchanged) = (0u32, 0u32, 0u32, 0u32);
    for key in keys {
        let before_file = left.get(&key);
        let after_file = right.get(&key);
        let (status, before_bytes, after_bytes) = match (before_file, after_file) {
            (None, Some(file)) => {
                added += 1;
                ("added", 0, file.bytes)
            }
            (Some(file), None) => {
                removed += 1;
                ("removed", file.bytes, 0)
            }
            (Some(before_file), Some(after_file)) => {
                let (before_bytes, before_hash) = digest(before, before_file)?;
                let (after_bytes, after_hash) = digest(after, after_file)?;
                if before_hash == after_hash {
                    unchanged += 1;
                    ("unchanged", before_bytes, after_bytes)
                } else {
                    changed += 1;
                    ("changed", before_bytes, after_bytes)
                }
            }
            (None, None) => continue,
        };
        if status != "unchanged" && changes.len() < MAX_DIFF_CHANGES {
            let file = after_file.or(before_file).expect("a region exists");
            changes.push(WorldDiffChange {
                world: file.world.clone(),
                dimension: file.folder.clone(),
                label: file.label.clone(),
                region_x: file.x,
                region_z: file.z,
                status: status.into(),
                before_bytes,
                after_bytes,
            });
        }
    }
    Ok(WorldDiff {
        before_id: before.backup.id.clone(),
        after_id: after.backup.id.clone(),
        added,
        removed,
        changed,
        unchanged,
        truncated: changes.len() == MAX_DIFF_CHANGES
            && (added + removed + changed) > MAX_DIFF_CHANGES as u32,
        changes,
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}
pub fn diff(m: &Manager, id: &str, before_id: &str, after_id: &str) -> Result<WorldDiff> {
    let before = Snapshot::open(m, id, before_id)?;
    let after = Snapshot::open(m, id, after_id)?;
    diff_snapshots(&before, &after)
}

// ---------------------------------------------------------------------------
// Optional online player tracking (read-only RCON queries, local history)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldTrackPoint {
    pub at: String,
    pub dimension: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldTrack {
    pub player: String,
    pub points: Vec<WorldTrackPoint>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackFile {
    version: u32,
    tracks: BTreeMap<String, TrackRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackRecord {
    points: Vec<WorldTrackPoint>,
}
impl Default for TrackFile {
    fn default() -> Self {
        Self {
            version: TRACK_VERSION,
            tracks: BTreeMap::new(),
        }
    }
}
fn track_path(m: &Manager, id: &str) -> Result<PathBuf> {
    storage::child(&m.dir(id)?, TRACK_FILE)
}
fn read_track_file(m: &Manager, id: &str) -> TrackFile {
    let Ok(path) = track_path(m, id) else {
        return TrackFile::default();
    };
    let Ok(bytes) = fs::read(path) else {
        return TrackFile::default();
    };
    serde_json::from_slice(&bytes)
        .ok()
        .filter(|file: &TrackFile| file.version == TRACK_VERSION)
        .unwrap_or_default()
}
pub fn load_tracks(m: &Manager, id: &str) -> Result<Vec<WorldTrack>> {
    let file = read_track_file(m, id);
    Ok(file
        .tracks
        .into_iter()
        .filter(|(_, record)| !record.points.is_empty())
        .map(|(player, record)| WorldTrack {
            player,
            points: record.points,
        })
        .collect())
}
fn parse_pos(output: &str) -> Option<(f64, f64, f64)> {
    let open = output.find('[')?;
    let close = output[open..].find(']')? + open;
    let values: Vec<f64> = output[open + 1..close]
        .split(',')
        .map(|value| {
            value
                .trim()
                .trim_end_matches(['d', 'D', 'f', 'F'])
                .parse::<f64>()
                .ok()
        })
        .collect::<Option<Vec<_>>>()?;
    if values.len() != 3
        || values
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 30_000_000.0)
    {
        return None;
    }
    Some((values[0], values[1], values[2]))
}
fn parse_dimension(output: &str) -> Option<String> {
    let value = if let Some(start) = output.find('"') {
        let rest = &output[start + 1..];
        let end = rest.find('"')? + start + 1;
        &output[start + 1..end]
    } else {
        output
            .split_whitespace()
            .find(|part| part.contains(':'))?
            .trim_matches(['[', ']', ',', '"'])
    };
    if value.is_empty() || value.len() > 128 || value.contains(['\n', '\r', '\0']) {
        return None;
    }
    Some(normalise_dimension(value))
}
fn prune_tracks(file: &mut TrackFile, now: chrono::DateTime<chrono::Utc>) {
    let cutoff = now - chrono::Duration::days(TRACK_DAYS);
    file.tracks.retain(|_, record| {
        record.points.retain(|point| {
            chrono::DateTime::parse_from_rfc3339(&point.at)
                .map(|at| at.with_timezone(&chrono::Utc) >= cutoff)
                .unwrap_or(true)
        });
        if record.points.len() > MAX_TRACK_POINTS {
            let excess = record.points.len() - MAX_TRACK_POINTS;
            record.points.drain(0..excess);
        }
        !record.points.is_empty()
    });
    if file.tracks.len() > MAX_TRACK_PLAYERS {
        while file.tracks.len() > MAX_TRACK_PLAYERS {
            if let Some(key) = file.tracks.keys().next().cloned() {
                file.tracks.remove(&key);
            }
        }
    }
}
pub async fn sample_tracks(m: Arc<Manager>, id: String) -> Result<Vec<WorldTrack>> {
    let names = rcon::players(&m, &id).await?;
    let mut file = read_track_file(&m, &id);
    let now = chrono::Utc::now();
    for player in names.into_iter().take(MAX_TRACK_PLAYERS) {
        if !crate::server::process::valid_player(&player) {
            continue;
        }
        let pos = match rcon::query(&m, &id, &format!("data get entity {player} Pos")).await {
            Ok(output) => parse_pos(&output),
            Err(_) => None,
        };
        let dimension =
            match rcon::query(&m, &id, &format!("data get entity {player} Dimension")).await {
                Ok(output) => parse_dimension(&output),
                Err(_) => None,
            };
        let (Some((x, y, z)), Some(dimension)) = (pos, dimension) else {
            continue;
        };
        let record = file
            .tracks
            .entry(player)
            .or_insert_with(|| TrackRecord { points: Vec::new() });
        let duplicate = record.points.last().is_some_and(|last| {
            last.dimension == dimension
                && ((last.x - x).powi(2) + (last.y - y).powi(2) + (last.z - z).powi(2)) < 0.25
                && chrono::DateTime::parse_from_rfc3339(&last.at)
                    .map(|at| now.signed_duration_since(at).num_seconds() < 30)
                    .unwrap_or(false)
        });
        if !duplicate {
            record.points.push(WorldTrackPoint {
                at: now.to_rfc3339(),
                dimension,
                x,
                y,
                z,
            });
        }
    }
    prune_tracks(&mut file, now);
    let path = track_path(&m, &id)?;
    storage::no_links(&path)?;
    storage::atomic_write(&path, &serde_json::to_vec(&file)?)?;
    load_tracks(&m, &id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::ZlibEncoder, Compression};
    use std::io::Write;

    fn be_i32(value: i32) -> Vec<u8> {
        value.to_be_bytes().to_vec()
    }
    fn nbt_string(value: &str) -> Vec<u8> {
        let mut out = (value.len() as u16).to_be_bytes().to_vec();
        out.extend_from_slice(value.as_bytes());
        out
    }
    fn named(kind: u8, name: &str, payload: Vec<u8>) -> Vec<u8> {
        let mut out = vec![kind];
        out.extend_from_slice(&nbt_string(name));
        out.extend_from_slice(&payload);
        out
    }
    fn compound(fields: Vec<Vec<u8>>) -> Vec<u8> {
        let mut out = Vec::new();
        for field in fields {
            out.extend_from_slice(&field);
        }
        out.push(0);
        out
    }
    fn root(fields: Vec<Vec<u8>>) -> Vec<u8> {
        named(10, "", compound(fields))
    }
    fn int(name: &str, value: i32) -> Vec<u8> {
        named(3, name, be_i32(value))
    }
    fn byte(name: &str, value: i8) -> Vec<u8> {
        named(1, name, vec![value as u8])
    }
    fn string(name: &str, value: &str) -> Vec<u8> {
        named(8, name, nbt_string(value))
    }
    fn list(name: &str, kind: u8, items: Vec<Vec<u8>>) -> Vec<u8> {
        let mut payload = vec![kind];
        payload.extend_from_slice(&be_i32(items.len() as i32));
        for item in items {
            payload.extend_from_slice(&item);
        }
        named(9, name, payload)
    }
    fn chunk_nbt(cx: i32, cz: i32) -> Vec<u8> {
        let palette = vec![compound(vec![string("Name", "minecraft:stone")])];
        let block_states = compound(vec![list("palette", 10, palette)]);
        let section = compound(vec![byte("Y", 4), named(10, "block_states", block_states)]);
        let chest = compound(vec![string("id", "minecraft:chest")]);
        let zombie = compound(vec![string("id", "minecraft:zombie")]);
        root(vec![
            int("DataVersion", 3465),
            int("xPos", cx),
            int("zPos", cz),
            list("sections", 10, vec![section]),
            list("block_entities", 10, vec![chest]),
            list("Entities", 10, vec![zombie]),
        ])
    }
    fn region_file(chunks: &[(i32, i32, Vec<u8>, bool)]) -> Vec<u8> {
        let mut body = Vec::new();
        let mut entries = vec![(0u32, 0u8); 1024];
        let mut cursor = 2usize;
        for (cx, cz, nbt, corrupt) in chunks {
            let compressed = if *corrupt {
                vec![0xff, 0xff]
            } else {
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                encoder.write_all(nbt).unwrap();
                encoder.finish().unwrap()
            };
            let mut payload = (compressed.len() as u32 + 1).to_be_bytes().to_vec();
            payload.push(2);
            payload.extend_from_slice(&compressed);
            let sectors = payload.len().div_ceil(4096).max(1) as u8;
            entries[(*cz as usize % 32) * 32 + (*cx as usize % 32)] = (cursor as u32, sectors);
            body.extend_from_slice(&payload);
            let padding = sectors as usize * 4096 - payload.len();
            body.resize(body.len() + padding, 0);
            cursor += sectors as usize;
        }
        let mut header = vec![0u8; 8192];
        for (index, (offset, sectors)) in entries.iter().enumerate() {
            header[index * 4] = (*offset >> 16) as u8;
            header[index * 4 + 1] = (*offset >> 8) as u8;
            header[index * 4 + 2] = *offset as u8;
            header[index * 4 + 3] = *sectors;
        }
        let mut out = header;
        out.extend_from_slice(&body);
        out
    }
    fn player_nbt() -> Vec<u8> {
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
        let pos = named(9, "Pos", {
            let mut value = vec![6];
            value.extend_from_slice(&be_i32(3));
            for number in [8.0f64, 65.0, -4.0] {
                value.extend_from_slice(&number.to_be_bytes());
            }
            value
        });
        let fields = vec![
            pos,
            string("Dimension", "minecraft:overworld"),
            named(5, "Health", 20.0f32.to_bits().to_be_bytes().to_vec()),
            int("foodLevel", 18),
            int("XpLevel", 7),
        ];
        gzip.write_all(&root(fields)).unwrap();
        gzip.finish().unwrap()
    }
    fn snapshot_from(dir: &Path) -> Snapshot {
        Snapshot {
            root: dir.to_path_buf(),
            dir: dir.to_path_buf(),
            backup: Backup {
                id: "12345678-1234-4123-8123-123456789abc".into(),
                created_at: "2026-10-20T12:00:00Z".into(),
                size_bytes: 0,
                worlds: vec!["world".into()],
                safety: false,
                reason: crate::models::BackupReason::Manual,
                label: None,
                minecraft_version: None,
                server_type: None,
                format: BackupFormat::Directory,
                stored_bytes: 0,
                chunk_count: 0,
                file_count: 0,
            },
            manifest: None,
        }
    }
    fn write_world(root: &Path, chunks: &[(i32, i32, Vec<u8>, bool)]) {
        fs::create_dir_all(root.join("world/region")).unwrap();
        fs::write(root.join("world/region/r.0.0.mca"), region_file(chunks)).unwrap();
        fs::create_dir_all(root.join("world/playerdata")).unwrap();
        fs::write(
            root.join("world/playerdata/12345678-1234-4123-8123-123456789abc.dat"),
            player_nbt(),
        )
        .unwrap();
    }

    #[test]
    fn nbt_reader_handles_compounds_lists_and_arrays() {
        let data = root(vec![
            int("Version", 7),
            string("Name", "world"),
            list("Values", 3, vec![be_i32(1), be_i32(2)]),
            named(12, "Longs", {
                let mut value = be_i32(2);
                value.extend_from_slice(&5i64.to_be_bytes());
                value.extend_from_slice(&(-3i64).to_be_bytes());
                value
            }),
        ]);
        let nbt = parse_nbt(&data).unwrap();
        assert_eq!(nbt.get("Version").and_then(Nbt::as_i64), Some(7));
        assert_eq!(nbt.get("Name").and_then(Nbt::as_string), Some("world"));
        assert_eq!(nbt.get("Longs").and_then(Nbt::as_longs), Some(&[5, -3][..]));
    }

    #[test]
    fn region_scan_counts_entities_and_detects_corrupt_chunks() {
        let nbt = chunk_nbt(0, 0);
        let bytes = region_file(&[(0, 0, nbt, false), (1, 0, vec![0], true)]);
        let mut budget = 10_000;
        let scan = scan_region(&bytes, 0, 0, RegionKind::Region, &mut budget);
        assert_eq!(scan.chunks, 1);
        assert_eq!(scan.corrupt, 1);
        assert_eq!(scan.block_entities, 1);
        assert_eq!(scan.entities, 1);
        assert_eq!(scan.block_types["minecraft:chest"], 1);
        assert_eq!(scan.entity_types["minecraft:zombie"], 1);
    }

    #[test]
    fn region_map_is_opaque_for_a_solid_chunk() {
        let bytes = region_file(&[(0, 0, chunk_nbt(0, 0), false)]);
        let rendered = render_region(&bytes, 0, 0, 2);
        assert_eq!(rendered.chunks, 1);
        assert_eq!(rendered.corrupt, 0);
        assert!(rendered.rgba.chunks(4).any(|pixel| pixel[3] == 255));
        assert!(rendered.warnings.is_empty());
    }

    #[test]
    fn preview_reads_players_and_dimension_stats() {
        let temp = tempfile::tempdir().unwrap();
        write_world(temp.path(), &[(0, 0, chunk_nbt(0, 0), false)]);
        let snapshot = snapshot_from(temp.path());
        let preview = build_preview(&snapshot, &BTreeMap::new(), Vec::new()).unwrap();
        assert_eq!(preview.totals.regions, 1);
        assert_eq!(preview.totals.chunks, 1);
        assert_eq!(preview.totals.block_entities, 1);
        assert_eq!(preview.totals.entities, 1);
        assert_eq!(preview.dimensions[0].label, "overworld");
        assert_eq!(preview.players.len(), 1);
        assert_eq!(preview.players[0].x, 8.0);
        assert_eq!(preview.players[0].dimension, "overworld");
    }

    #[test]
    fn preview_reads_a_chunked_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("backups");
        let server = temp.path().join("server");
        fs::create_dir_all(root.join(backup_store::CHUNK_DIR)).unwrap();
        write_world(&server, &[(0, 0, chunk_nbt(0, 0), false)]);
        let id = uuid::Uuid::new_v4().to_string();
        let dir = root.join(&id);
        fs::create_dir(&dir).unwrap();
        let stats = backup_store::create(&root, &dir, &server, &["world".into()]).unwrap();
        let backup = Backup {
            id,
            created_at: "2026-10-20T12:00:00Z".into(),
            size_bytes: stats.logical_bytes,
            worlds: vec!["world".into()],
            safety: false,
            reason: crate::models::BackupReason::Manual,
            label: None,
            minecraft_version: None,
            server_type: None,
            format: BackupFormat::Chunked,
            stored_bytes: stats.stored_bytes,
            chunk_count: stats.chunk_count,
            file_count: stats.file_count,
        };
        let manifest = backup_store::load_manifest(&dir).unwrap();
        let snapshot = Snapshot {
            root: root.clone(),
            dir: dir.clone(),
            backup,
            manifest: Some(manifest),
        };
        let preview = build_preview(&snapshot, &BTreeMap::new(), Vec::new()).unwrap();
        assert_eq!(preview.totals.regions, 1);
        assert_eq!(preview.totals.chunks, 1);
        assert_eq!(preview.players.len(), 1);
        let region = &preview.dimensions[0].regions[0];
        let bytes = snapshot
            .read(&format!("world/region/r.{}.{}.mca", region.x, region.z))
            .unwrap();
        assert_eq!(bytes.len() as u64, region.bytes);
    }

    #[test]
    fn region_diff_reports_added_and_changed_files() {
        let first = tempfile::tempdir().unwrap();
        write_world(first.path(), &[(0, 0, chunk_nbt(0, 0), false)]);
        let before = snapshot_from(first.path());
        let second = tempfile::tempdir().unwrap();
        write_world(
            second.path(),
            &[
                (0, 0, chunk_nbt(0, 0), false),
                (1, 0, chunk_nbt(1, 0), false),
            ],
        );
        let after = snapshot_from(second.path());
        let diff = diff_snapshots(&before, &after).unwrap();
        assert_eq!(diff.added, 0);
        assert_eq!(diff.changed, 1);
        assert_eq!(diff.changes.len(), 1);
        assert_eq!(diff.changes[0].status, "changed");
    }

    #[test]
    fn player_position_and_dimension_parsers() {
        assert_eq!(
            parse_pos("Steve has the following entity data: [1.5d, 64.0d, -2.0d]"),
            Some((1.5, 64.0, -2.0))
        );
        assert_eq!(
            parse_dimension("Steve has the following entity data: \"minecraft:the_nether\""),
            Some("the_nether".into())
        );
        assert!(parse_pos("no list").is_none());
    }

    #[test]
    fn region_paths_split_worlds_and_dimensions() {
        assert_eq!(
            parse_region_path("world/region/r.0.-1.mca", RegionKind::Region),
            Some(("world".into(), "".into(), 0, -1))
        );
        assert_eq!(
            parse_region_path("world/DIM-1/region/r.2.3.mca", RegionKind::Region),
            Some(("world".into(), "DIM-1".into(), 2, 3))
        );
        assert_eq!(
            dimension_label("world_nether", ""),
            "the_nether".to_string()
        );
    }
}
