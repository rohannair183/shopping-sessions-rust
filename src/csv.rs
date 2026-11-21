use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CHUNK_BYTES: u64 = 4 << 20;

pub fn fields<const N: usize>(line: &str) -> Option<[&str; N]> {
    #[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
    if (short_row::WIDTH..u128::BITS as usize).contains(&line.len()) {
        return short_row::fields(line);
    }
    split_fields(line)
}

#[cfg(all(target_arch = "x86_64", target_feature = "sse2"))]
mod short_row {
    use std::arch::x86_64::{_mm_cmpeq_epi8, _mm_loadu_si128, _mm_movemask_epi8, _mm_set1_epi8};

    pub(super) const WIDTH: usize = 16;

    #[inline]
    fn delimiters(block: &[u8; WIDTH]) -> (u16, u16) {
        // SSE2 is enabled for this module, and the array contains a full unaligned load.
        unsafe {
            let bytes = _mm_loadu_si128(block.as_ptr().cast());
            let commas = _mm_cmpeq_epi8(bytes, _mm_set1_epi8(b',' as i8));
            let quotes = _mm_cmpeq_epi8(bytes, _mm_set1_epi8(b'"' as i8));
            (
                _mm_movemask_epi8(commas) as u16,
                _mm_movemask_epi8(quotes) as u16,
            )
        }
    }

    pub(super) fn fields<const N: usize>(line: &str) -> Option<[&str; N]> {
        let bytes = line.as_bytes();
        let last = bytes.len() - WIDTH;
        let mut commas = 1u128 << bytes.len(); // Treat the end as a field delimiter.
        let mut quotes = 0u128;
        // The last load may overlap the previous load. OR keeps each position once.
        for start in (0..last).step_by(WIDTH).chain(std::iter::once(last)) {
            let block = bytes[start..start + WIDTH].try_into().unwrap();
            let (c, q) = delimiters(block);
            commas |= u128::from(c) << start;
            quotes |= u128::from(q) << start;
        }

        let mut row = [""; N];
        let mut start = 0;
        for item in &mut row {
            if commas == 0 {
                return None;
            }
            let end = commas.trailing_zeros() as usize;
            commas &= commas - 1;
            let first = 1u128 << start;
            *item = if quotes & first == 0 {
                &line[start..end]
            } else {
                if end < start + 2 {
                    return None;
                }
                let last = 1u128 << (end - 1);
                if quotes & last == 0 {
                    return None;
                }
                quotes ^= first | last;
                &line[start + 1..end - 1]
            };
            start = end + 1;
        }
        (commas == 0 && quotes == 0).then_some(row)
    }
}

fn split_fields<const N: usize>(line: &str) -> Option<[&str; N]> {
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

#[cfg(test)]
mod tests {
    use super::{fields, split_fields};

    #[test]
    fn short_rows_match_the_general_parser_at_block_edges() {
        let suffixes = [
            "abc,", "é", " x ", "\r", "", "\"", "\"x", "x\"", "\"x\"", "\"\"", "\"x\"y\"",
        ];
        for padding in 0..=140 {
            for suffix in suffixes {
                let line = format!("é,{},{}", "a".repeat(padding), suffix);
                assert_eq!(fields::<3>(&line), split_fields::<3>(&line), "{line:?}");
            }
        }
    }

    #[test]
    fn short_rows_match_the_general_parser_for_non_ascii_and_malformed_fields() {
        let cases = [
            "", "x", "é日", "\"x\"", "\"\"", "x\"y", "\"x", "x\"", "\"", "0\r",
        ];
        for a in cases {
            for b in cases {
                for c in cases {
                    let line = format!("{a},{b},anchor0123456789,{c}");
                    assert_eq!(fields::<4>(&line), split_fields::<4>(&line), "{line:?}");
                }
            }
        }
    }
}
