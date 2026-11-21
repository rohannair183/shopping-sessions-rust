use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
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

    pub fn read_fields<const N: usize>(&self, visit: impl FnMut([&str; N])) -> io::Result<()> {
        let mut reader = BufReader::new(File::open(&self.path)?);
        let mut pos = self.start.saturating_sub(1);
        reader.seek(SeekFrom::Start(pos))?;
        pos += reader.skip_until(b'\n')? as u64;
        if pos >= self.end {
            return Ok(());
        }
        let limit = self.end - pos;
        let mut bytes = Vec::with_capacity(limit as usize + 128);
        reader.by_ref().take(limit).read_to_end(&mut bytes)?;
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            // The chunk owns the row that started before its end, including its tail.
            reader.read_until(b'\n', &mut bytes)?;
            if bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        chunk_scan::visit(text, visit);
        Ok(())
    }
}

mod chunk_scan;

#[cfg(test)]
mod tests {
    use super::{Chunk, chunk_scan, fields, split_fields};

    #[test]
    fn event_masks_match_general_fields_at_block_and_row_edges() {
        let mut input = String::new();
        for pad in 0..=132 {
            for tail in [
                "",
                "x",
                "\r",
                "é",
                "\"\"",
                "\"quoted\"",
                "\"bad",
                "bad\"",
                "\"a\"b\"",
                "a,b",
            ] {
                input.push_str(&format!("e,{},0,utc,\"view\",日,{tail}\n", "u".repeat(pad)));
            }
        }
        input.push_str("\nwrong\n,,,\n,,,\"comma,inside\",,,\n");
        let expected: Vec<_> = input
            .split_terminator('\n')
            .filter_map(split_fields::<7>)
            .map(|row| row.map(str::to_owned))
            .collect();
        let mut actual = Vec::new();
        chunk_scan::visit::<7>(&input, |row| actual.push(row.map(str::to_owned)));
        assert_eq!(expected, actual);
    }

    fn check_chunk_splits<const N: usize>(input: &str) -> std::io::Result<()> {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };
        let salt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("chunk-scan-{}-{N}-{salt}.csv", std::process::id()));
        fs::write(&path, input)?;
        let expected: Vec<_> = input
            .split('\n')
            .skip(1)
            .filter_map(split_fields::<N>)
            .map(|row| row.map(str::to_owned))
            .collect();
        for split in 0..=input.len() as u64 {
            let mut actual = Vec::new();
            for (start, end) in [(0, split), (split, input.len() as u64)] {
                Chunk {
                    path: path.clone(),
                    start,
                    end,
                }
                .read_fields::<N>(|row| actual.push(row.map(str::to_owned)))?;
            }
            assert_eq!(actual, expected, "split at byte {split}");
        }
        fs::remove_file(path)
    }

    #[test]
    fn event_chunks_preserve_header_ownership_and_unterminated_tail() -> std::io::Result<()> {
        let input = "h1,h2,h3,h4,h5,h6,h7\ne,u,1,2,view,p,\n\nq,\"日\",3,4,cart,\"p\",bad\r\ns,u,5,6,purchase,p,7\r";
        check_chunk_splits::<7>(input)
    }

    #[test]
    fn click_chunks_match_the_line_reader_at_every_split() -> std::io::Result<()> {
        let long_id = "a".repeat(150);
        let input = format!(
            "id,user,campaign,ts\n\"c1\",é,ad,t\n\nc2,u,b\"ad,t\nc3,u,too,many,fields\n{long_id},u,ad,t\nc4,u,ad,end\r"
        );
        check_chunk_splits::<4>(&input)
    }

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
