//! Installed fonts, for text alphas: the faces (family and style) of every TrueType or OpenType
//! file (and collection) in the system's font folders, read from each file's `name` table only,
//! so listing hundreds of fonts stays quick. Native only; the browser has no font folders.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One face of an installed font file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FontFace {
    pub family: String,
    pub style: String,
    pub path: PathBuf,
    /// Face index in a collection (`.ttc`), else 0.
    pub index: u32,
}

/// Files and faces visited at most (a font folder is never this large, a wrong one may be).
const MAX_FILES: usize = 20_000;
const MAX_FACES_PER_FILE: u32 = 64;
const MAX_NAME_TABLE: u32 = 1 << 20;

/// The installed fonts' faces, sorted by family then style.
pub fn system_fonts() -> Vec<FontFace> {
    let mut files = Vec::new();
    for dir in font_dirs() {
        collect(&dir, 0, &mut files);
    }
    let mut faces: Vec<FontFace> = files.iter().flat_map(|p| faces_of(p).unwrap_or_default()).collect();
    faces.sort();
    faces.dedup_by(|a, b| a.family == b.family && a.style == b.style);
    faces
}

fn font_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        let mut v = vec![PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts")];
        v.extend(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft\\Windows\\Fonts")));
        v
    } else if cfg!(target_os = "macos") {
        let mut v = vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/Library/Fonts")];
        v.extend(home.map(|h| h.join("Library/Fonts")));
        v
    } else {
        let mut v = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
        v.extend(home.iter().flat_map(|h| [h.join(".local/share/fonts"), h.join(".fonts")]));
        v
    }
}

/// Font files under `dir` (a few levels deep).
fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = e.path();
        match e.file_type() {
            Ok(t) if t.is_dir() && depth < 4 => collect(&path, depth + 1, out),
            Ok(t) if t.is_file() => {
                let ext = path.extension().and_then(|x| x.to_str()).unwrap_or_default().to_ascii_lowercase();
                if matches!(ext.as_str(), "ttf" | "otf" | "ttc" | "otc") {
                    out.push(path);
                }
            }
            _ => {}
        }
    }
}

/// The faces of the font file at `path` (each face of a collection).
pub fn faces_of(path: &Path) -> std::io::Result<Vec<FontFace>> {
    let mut file = std::fs::File::open(path)?;
    let mut read_at = |offset: u64, length: usize| -> std::io::Result<Vec<u8>> {
        file.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0; length];
        file.read_exact(&mut buf)?;
        Ok(buf)
    };

    let header = read_at(0, 12)?;
    let offsets: Vec<u64> = if header.get(..4) == Some(b"ttcf") {
        let face_count = u32_at(&header, 8).unwrap_or(0).min(MAX_FACES_PER_FILE);
        let table = read_at(12, 4 * face_count as usize)?;
        (0..face_count as usize).filter_map(|i| u32_at(&table, 4 * i)).map(u64::from).collect()
    } else {
        vec![0]
    };

    let mut faces = Vec::new();
    for (index, offset) in offsets.into_iter().enumerate() {
        let directory = read_at(offset, 12)?;
        let tables = read_at(offset + 12, 16 * usize::from(u16_at(&directory, 4).unwrap_or(0)))?;
        let Some(record) = tables.as_chunks::<16>().0.iter().find(|r| r.get(..4) == Some(b"name")) else { continue };
        let (Some(offset), Some(length)) = (u32_at(record, 8), u32_at(record, 12)) else { continue };
        let names = read_at(u64::from(offset), length.min(MAX_NAME_TABLE) as usize)?;
        let name = |ids: [u16; 2]| ids.iter().find_map(|id| name_string(&names, *id));
        if let Some(family) = name([16, 1]) {
            let style = name([17, 2]).unwrap_or_else(|| "Regular".into());
            faces.push(FontFace { family, style, path: path.to_path_buf(), index: index as u32 });
        }
    }

    Ok(faces)
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(o..o.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

/// Name `id` of a `name` table, in English when it has several languages: Windows and Unicode
/// records are UTF-16BE, Macintosh ones single-byte (read as Latin-1).
fn name_string(table: &[u8], id: u16) -> Option<String> {
    let (count, base) = (usize::from(u16_at(table, 2)?), usize::from(u16_at(table, 4)?));
    let mut best: Option<(u8, String)> = None;
    for record in (0..count).filter_map(|i| table.get(6 + 12 * i..18 + 12 * i)) {
        let (platform, language, name_id) = (u16_at(record, 0)?, u16_at(record, 4)?, u16_at(record, 6)?);
        let (length, offset) = (usize::from(u16_at(record, 8)?), usize::from(u16_at(record, 10)?));
        let Some(bytes) = table.get(base + offset..base + offset + length) else { continue };
        if name_id != id {
            continue;
        }

        let (rank, name) = match platform {
            3 | 0 => {
                let name = String::from_utf16_lossy(&bytes.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect::<Vec<_>>());
                (if language == 0x409 { 0 } else { 1 }, name)
            }
            1 => (if language == 0 { 2 } else { 3 }, bytes.iter().map(|b| char::from(*b)).collect()),
            _ => continue,
        };
        let name = name.trim().to_string();
        if !name.is_empty() && best.as_ref().is_none_or(|(best_rank, _)| rank < *best_rank) {
            best = Some((rank, name));
        }
    }

    best.map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `name` table with the given (platform, language, name id, bytes) records.
    fn name_table(records: &[(u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
        let base = 6 + 12 * records.len();
        let mut t: Vec<u8> = [0u16, records.len() as u16, base as u16].iter().flat_map(|v| v.to_be_bytes()).collect();
        let mut strings: Vec<u8> = Vec::new();
        for (platform, language, id, bytes) in records {
            for v in [*platform, 1, *language, *id, bytes.len() as u16, strings.len() as u16] {
                t.extend(v.to_be_bytes());
            }
            strings.extend(bytes);
        }
        t.extend(strings);
        t
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(u16::to_be_bytes).collect()
    }

    #[test]
    fn names_prefer_english_windows_records() {
        let t =
            name_table(&[(1, 0, 1, b"Mac Name".to_vec()), (3, 0x407, 1, utf16("Deutsch")), (3, 0x409, 1, utf16("Plain Sans")), (3, 0x409, 2, utf16("Bold"))]);
        assert_eq!(name_string(&t, 1).as_deref(), Some("Plain Sans"));
        assert_eq!(name_string(&t, 2).as_deref(), Some("Bold"));
        assert_eq!(name_string(&t, 16), None);
        // Truncated or hostile tables give nothing rather than panicking.
        for n in 0..t.len() {
            let _ = name_string(&t[..n], 1);
        }
        assert_eq!(name_string(&[0xff; 64], 1), None);
    }

    #[test]
    fn faces_of_a_file_and_garbage() {
        let dir = std::env::temp_dir().join(format!("tc-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("font.ttf");
        // A font file with only a name table: offset table, one table record, the table.
        let names = name_table(&[(3, 0x409, 1, utf16("Plain Sans")), (3, 0x409, 2, utf16("Italic"))]);
        let mut file: Vec<u8> = [0x0001_0000u32.to_be_bytes().to_vec(), 1u16.to_be_bytes().to_vec(), vec![0; 6]].concat();
        file.extend(b"name");
        file.extend([0u32, 28, names.len() as u32].iter().flat_map(|v| v.to_be_bytes()));
        file.extend(&names);
        std::fs::write(&path, &file).unwrap();
        let faces = faces_of(&path).unwrap();
        assert_eq!((faces[0].family.as_str(), faces[0].style.as_str(), faces[0].index), ("Plain Sans", "Italic", 0));
        std::fs::write(&path, b"garbage").unwrap();
        assert!(faces_of(&path).is_err() || faces_of(&path).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
