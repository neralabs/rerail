//! Captured files: `rerail-engine capture-tapes` writes them (`T16CAP01`, then per file a path, a tape tag,
//! the source and the format-15 image).

pub struct File {
    pub path: String,
    pub tag: u8,
    pub source: Vec<u8>,
    pub image: Vec<u8>,
}

impl File {
    /// The language's name, from the tape tag (`language::tape_tag`).
    pub fn language(&self) -> &'static str {
        language(self.tag)
    }

    /// Whether the file is in the test half: dictionaries train on the other half only.
    pub fn is_test(&self) -> bool {
        let mut h: u64 = 0xcbf29ce484222325;
        for b in self.path.bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        h & 1 == 1
    }
}

pub fn language(tag: u8) -> &'static str {
    match tag {
        1 => "python",
        2 => "typescript",
        3 => "tsx",
        6 => "javascript",
        7 => "php",
        9 => "rust",
        10 => "sql",
        8 => "blade",
        12 => "vue",
        13 => "rr",
        14 => "json",
        15 => "percent-format",
        16 => "brace-format",
        17 => "eslint-directive",
        18 => "java",
        19 => "csharp",
        20 => "html",
        21 => "css",
        22 => "markdown",
        23 => "markdown-inline",
        24 => "kotlin",
        25 => "swift",
        26 => "scala",
        27 => "go",
        28 => "c",
        29 => "cpp",
        30 => "annotation",
        31 => "jsx",
        32 => "html-element",
        33 => "regex",
        34 => "python-pattern",
        250 => "test-html",
        _ => "other",
    }
}

/// A hash of the path with a seed: shards and subsamples that do not follow the test split.
pub fn path_hash(path: &str, seed: u64) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325 ^ seed.wrapping_mul(0x9e3779b97f4a7c15);
    for b in path.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h ^ (h >> 29)
}

/// Each file of a capture in turn, never all at once (a whole repository's capture is gigabytes).
pub fn stream(path: &std::path::Path, mut f: impl FnMut(File) -> Result<(), String>) -> Result<(), String> {
    use std::io::Read;
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut r = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).map_err(err)?);
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic).map_err(err)?;
    if &magic != b"T16CAP01" {
        return Err(format!("{}: not a capture", path.display()));
    }
    let mut take = |n: usize| -> Result<Vec<u8>, String> {
        let mut v = vec![0u8; n];
        r.read_exact(&mut v).map_err(err)?;
        Ok(v)
    };
    loop {
        let Ok(len) = take(2) else { return Ok(()) };
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        let path = String::from_utf8_lossy(&take(len)?).into_owned();
        let tag = take(1)?[0];
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let source = take(n)?;
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let image = take(n)?;
        f(File { path, tag, source, image })?;
    }
}

/// Every file of a capture, read as a stream (a capture can be hundreds of megabytes).
pub fn read(path: &std::path::Path) -> Result<Vec<File>, String> {
    use std::io::Read;
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut r = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(path).map_err(err)?);
    let mut magic = [0u8; 8];
    r.read_exact(&mut magic).map_err(err)?;
    if &magic != b"T16CAP01" {
        return Err(format!("{}: not a capture", path.display()));
    }
    let mut take = |n: usize| -> Result<Vec<u8>, String> {
        let mut v = vec![0u8; n];
        r.read_exact(&mut v).map_err(err)?;
        Ok(v)
    };
    let mut files = Vec::new();
    loop {
        let Ok(len) = take(2) else { break };
        let len = u16::from_le_bytes([len[0], len[1]]) as usize;
        let path = String::from_utf8_lossy(&take(len)?).into_owned();
        let tag = take(1)?[0];
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let source = take(n)?;
        let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let image = take(n)?;
        files.push(File { path, tag, source, image });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}
