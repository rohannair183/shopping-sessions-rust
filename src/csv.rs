use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CHUNK_BYTES: u64 = 4 << 20;

pub fn fields<const N: usize>(line: &str) -> Option<[&str; N]> {
    let mut parts = line.split(',');
    let mut row = [""; N];
    for item in &mut row {
        let raw = parts.next()?;
        let value = if let Some(inner) = raw.strip_prefix('"') {
            inner.strip_suffix('"')?
        } else {
            raw
        };
        if value.contains('"') {
            return None;
        }
        *item = value;
    }
    parts.next().is_none().then_some(row)
}

pub struct Chunk {
    path: PathBuf,
    start: u64,
    end: u64,
}

pub fn chunks(folder: &Path) -> io::Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    for entry in fs::read_dir(folder)? {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "csv") {
            continue;
        }
        let len = path.metadata()?.len();
        for start in (0..len).step_by(CHUNK_BYTES as usize) {
            chunks.push(Chunk {
                path: path.clone(),
                start,
                end: (start + CHUNK_BYTES).min(len),
            });
        }
    }
    Ok(chunks)
}

impl Chunk {
    pub fn whole(path: PathBuf) -> io::Result<Self> {
        let end = path.metadata()?.len();
        Ok(Self {
            path,
            start: 0,
            end,
        })
    }

    pub fn read(&self, mut visit: impl FnMut(&str)) -> io::Result<()> {
        let mut reader = BufReader::new(File::open(&self.path)?);
        // Starting one byte early also handles chunks that begin exactly at a row boundary.
        let mut pos = self.start.saturating_sub(1);
        reader.seek(SeekFrom::Start(pos))?;
        pos += reader.skip_until(b'\n')? as u64;
        let mut line = String::new();
        while pos < self.end {
            line.clear();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            pos += n as u64;
            visit(line.strip_suffix('\n').unwrap_or(&line));
        }
        Ok(())
    }
}
