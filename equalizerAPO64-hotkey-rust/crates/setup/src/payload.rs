//! Self-contained installer payload.
//!
//! `PackPayload` appends a compressed archive to `Setup.exe`.  The running
//! installer finds it through a fixed-size footer at the end of its own image
//! and extracts it into a temporary directory, so a release can be a single
//! executable.  The directory layout (`Setup.exe` next to `files\`) stays
//! supported for development builds.
//!
//! Archive layout (all integers little endian):
//!
//! ```text
//! [exe image][entry data ...][index][footer]
//! footer: "EAPOPKG1" u64 index_offset u64 index_len
//! index : u32 count, then count * { u32 path_len, path, u64 offset,
//!                                  u64 comp_len, u64 raw_len }
//! ```
//!
//! Entry data is raw DEFLATE; offsets are absolute file offsets.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;

const MAGIC: &[u8; 8] = b"EAPOPKG1";
const FOOTER_LEN: u64 = 24;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub offset: u64,
    pub comp_len: u64,
    pub raw_len: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct PayloadStats {
    pub files: u32,
    pub payload_bytes: u64,
    pub index_bytes: u64,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// Reads the archive index from `exe`.  `Ok(None)` when the file has no payload
/// footer (a plain executable or the directory layout).
pub fn read_entries(exe: &Path) -> io::Result<Option<Vec<Entry>>> {
    let mut file = File::open(exe)?;
    let file_len = file.metadata()?.len();
    if file_len < FOOTER_LEN {
        return Ok(None);
    }
    file.seek(SeekFrom::End(-(FOOTER_LEN as i64)))?;
    let mut footer = [0u8; FOOTER_LEN as usize];
    file.read_exact(&mut footer)?;
    if &footer[..8] != MAGIC {
        return Ok(None);
    }
    let index_offset = u64::from_le_bytes(footer[8..16].try_into().unwrap());
    let index_len = u64::from_le_bytes(footer[16..24].try_into().unwrap());
    let data_end = file_len - FOOTER_LEN;
    if index_offset.checked_add(index_len).map_or(true, |end| end > data_end) {
        return Err(invalid("payload index lies outside the executable"));
    }
    file.seek(SeekFrom::Start(index_offset))?;
    let mut index = vec![0u8; index_len as usize];
    file.read_exact(&mut index)?;
    parse_index(&index).map(Some)
}

fn parse_index(index: &[u8]) -> io::Result<Vec<Entry>> {
    let mut cursor = 0usize;
    let mut take = |len: usize| -> io::Result<&[u8]> {
        let end = cursor
            .checked_add(len)
            .filter(|end| *end <= index.len())
            .ok_or_else(|| invalid("truncated payload index"))?;
        let slice = &index[cursor..end];
        cursor = end;
        Ok(slice)
    };

    let count = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let path_len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let path = String::from_utf8(take(path_len)?.to_vec())
            .map_err(|_| invalid("payload path is not UTF-8"))?;
        let offset = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let comp_len = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let raw_len = u64::from_le_bytes(take(8)?.try_into().unwrap());
        safe_join(Path::new(""), &path)?;
        entries.push(Entry { path, offset, comp_len, raw_len });
    }
    Ok(entries)
}

/// Resolves an archive path below `root`, refusing anything absolute or
/// containing a parent component.
fn safe_join(root: &Path, relative: &str) -> io::Result<PathBuf> {
    let normalized = relative.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || normalized.contains(':') {
        return Err(invalid("payload path is not relative"));
    }
    let mut path = root.to_path_buf();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(invalid("payload path escapes the target directory"));
        }
        path.push(part);
    }
    if path == root {
        return Err(invalid("payload entry has an empty path"));
    }
    Ok(path)
}

/// Extracts every entry of the payload in `exe` below `dest`.
///
/// Returns `Ok(None)` when the executable carries no payload.
pub fn extract(exe: &Path, dest: &Path) -> io::Result<Option<usize>> {
    let Some(entries) = read_entries(exe)? else {
        return Ok(None);
    };
    let mut file = File::open(exe)?;
    for entry in &entries {
        let target = safe_join(dest, &entry.path)?;
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        file.seek(SeekFrom::Start(entry.offset))?;
        let mut compressed = vec![0u8; entry.comp_len as usize];
        file.read_exact(&mut compressed)?;
        let mut decoder = DeflateDecoder::new(&compressed[..]);
        let mut contents = Vec::with_capacity(entry.raw_len as usize);
        decoder.read_to_end(&mut contents)?;
        if contents.len() as u64 != entry.raw_len {
            return Err(invalid("payload entry has an unexpected size"));
        }
        fs::write(&target, &contents)?;
    }
    Ok(Some(entries.len()))
}

/// Directory the embedded payload was extracted to, extracted once per process.
static STAGING: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Extracts the payload appended to the current executable, if any, and returns
/// the directory holding it.
pub fn embedded_staging() -> Option<PathBuf> {
    STAGING
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            extract_payload_next_to(&exe).ok().flatten()
        })
        .clone()
}

fn extract_payload_next_to(exe: &Path) -> io::Result<Option<PathBuf>> {
    if read_entries(exe)?.is_none() {
        return Ok(None);
    }
    let dest = std::env::temp_dir().join(format!("eapo-setup-{}", std::process::id()));
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    extract(exe, &dest)?;
    Ok(Some(dest))
}

/// Removes the temporary extraction directory, if one was created.
pub fn cleanup() {
    if let Some(Some(directory)) = STAGING.get() {
        let _ = fs::remove_dir_all(directory);
    }
}

/// Appends the contents of `files_dir` to `out` and returns what was written.
pub fn append_archive(out: &mut File, files_dir: &Path) -> io::Result<PayloadStats> {
    let base = out.seek(SeekFrom::End(0))?;
    let mut files = Vec::new();
    collect_files(files_dir, files_dir, &mut files)?;
    files.sort();

    let mut entries = Vec::with_capacity(files.len());
    let mut written = 0u64;
    for (relative, path) in files {
        let raw = fs::read(&path)?;
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(&raw)?;
        let compressed = encoder.finish()?;
        let offset = base + written;
        out.write_all(&compressed)?;
        written += compressed.len() as u64;
        entries.push(Entry {
            path: relative,
            offset,
            comp_len: compressed.len() as u64,
            raw_len: raw.len() as u64,
        });
    }

    let index_offset = base + written;
    let mut index = Vec::new();
    index.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for entry in &entries {
        index.extend_from_slice(&(entry.path.len() as u32).to_le_bytes());
        index.extend_from_slice(entry.path.as_bytes());
        index.extend_from_slice(&entry.offset.to_le_bytes());
        index.extend_from_slice(&entry.comp_len.to_le_bytes());
        index.extend_from_slice(&entry.raw_len.to_le_bytes());
    }
    out.write_all(&index)?;
    out.write_all(MAGIC)?;
    out.write_all(&index_offset.to_le_bytes())?;
    out.write_all(&(index.len() as u64).to_le_bytes())?;
    out.flush()?;

    Ok(PayloadStats {
        files: entries.len() as u32,
        payload_bytes: written,
        index_bytes: index.len() as u64,
    })
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    for item in fs::read_dir(directory)? {
        let item = item?;
        let path = item.path();
        let metadata = item.metadata()?;
        if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| invalid("payload file outside the payload directory"))?;
            let relative = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            files.push((relative, path));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("eapo-payload-{name}-{unique}"));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn round_trips_an_appended_archive() {
        let root = temp_dir("roundtrip");
        let payload = root.join("files");
        fs::create_dir_all(payload.join("config")).unwrap();
        fs::write(payload.join("EqualizerAPO.dll"), vec![0x4d, 0x5a, 0x90, 0x00, 0xff]).unwrap();
        fs::write(payload.join("config").join("config.txt"), b"Preamp: -6 dB\r\n").unwrap();

        let exe = root.join("Setup.exe");
        fs::write(&exe, b"MZ fake image").unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(&exe).unwrap();
        let stats = append_archive(&mut file, &payload).unwrap();
        drop(file);
        assert_eq!(stats.files, 2);

        let entries = read_entries(&exe).unwrap().expect("payload present");
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.path == "config/config.txt"));

        let dest = root.join("extracted");
        let count = extract(&exe, &dest).unwrap().expect("payload present");
        assert_eq!(count, 2);
        assert_eq!(
            fs::read(dest.join("config").join("config.txt")).unwrap(),
            b"Preamp: -6 dB\r\n"
        );
        assert_eq!(
            fs::read(dest.join("EqualizerAPO.dll")).unwrap(),
            vec![0x4d, 0x5a, 0x90, 0x00, 0xff]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plain_executable_has_no_payload() {
        let root = temp_dir("plain");
        let exe = root.join("Setup.exe");
        fs::write(&exe, b"MZ not a package").unwrap();
        assert!(read_entries(&exe).unwrap().is_none());
        assert!(extract(&exe, &root.join("out")).unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_paths_that_escape_the_target() {
        assert!(safe_join(Path::new("C:\\x"), "config/a.txt").is_ok());
        assert!(safe_join(Path::new("C:\\x"), "../evil.txt").is_err());
        assert!(safe_join(Path::new("C:\\x"), "config/../../evil.txt").is_err());
        assert!(safe_join(Path::new("C:\\x"), "C:/Windows/evil.txt").is_err());
        assert!(safe_join(Path::new("C:\\x"), "\\\\server\\share\\evil.txt").is_err());
        assert!(safe_join(Path::new("C:\\x"), "").is_err());
    }

    #[test]
    fn rejects_a_truncated_index() {
        let root = temp_dir("truncated");
        let exe = root.join("Setup.exe");
        let mut bytes = b"MZ image".to_vec();
        let index_offset = bytes.len() as u64;
        bytes.extend_from_slice(&7u32.to_le_bytes()); // count promises more entries
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&index_offset.to_le_bytes());
        bytes.extend_from_slice(&4u64.to_le_bytes());
        fs::write(&exe, &bytes).unwrap();
        assert!(read_entries(&exe).is_err());
        let _ = fs::remove_dir_all(&root);
    }
}
