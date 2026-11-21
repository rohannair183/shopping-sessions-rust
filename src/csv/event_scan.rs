use super::fields;

const WIDTH: usize = u64::BITS as usize;
const MAX_INLINE_ROW: usize = u128::BITS as usize - 1;

#[derive(Default)]
struct Masks {
    commas: u64,
    quotes: u64,
    newlines: u64,
}

#[cfg(not(target_arch = "x86_64"))]
fn classify(block: &[u8; WIDTH]) -> Masks {
    let mut masks = Masks::default();
    for (i, &byte) in block.iter().enumerate() {
        match byte {
            b',' => masks.commas |= 1 << i,
            b'"' => masks.quotes |= 1 << i,
            b'\n' => masks.newlines |= 1 << i,
            _ => {}
        }
    }
    masks
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use super::{Masks, WIDTH};
    use std::arch::x86_64::*;

    pub(super) fn sse2(block: &[u8; WIDTH]) -> Masks {
        let mut m = Masks::default();
        for offset in (0..WIDTH).step_by(16) {
            // SSE2 is part of x86-64. Each load stays within the 64-byte block.
            unsafe {
                let bytes = _mm_loadu_si128(block.as_ptr().add(offset).cast());
                let mask =
                    |byte| _mm_movemask_epi8(_mm_cmpeq_epi8(bytes, _mm_set1_epi8(byte))) as u64;
                m.commas |= mask(b',' as i8) << offset;
                m.quotes |= mask(b'"' as i8) << offset;
                m.newlines |= mask(b'\n' as i8) << offset;
            }
        }
        m
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn avx2(block: &[u8; WIDTH]) -> Masks {
        let mut m = Masks::default();
        for offset in [0, 32] {
            // The caller checks AVX2 support. Each load stays within the block.
            let bytes = unsafe { _mm256_loadu_si256(block.as_ptr().add(offset).cast()) };
            let mask = |byte| {
                _mm256_movemask_epi8(_mm256_cmpeq_epi8(bytes, _mm256_set1_epi8(byte))) as u32 as u64
            };
            m.commas |= mask(b',' as i8) << offset;
            m.quotes |= mask(b'"' as i8) << offset;
            m.newlines |= mask(b'\n' as i8) << offset;
        }
        m
    }
}

pub(super) fn visit(text: &str, visit_row: impl FnMut([&str; 7])) {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            scan(text, |block| unsafe { x86::avx2(block) }, visit_row);
        } else {
            scan(text, x86::sse2, visit_row);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    scan(text, classify, visit_row);
}

fn decode(line: &str, mut commas: u128, mut quotes: u128) -> Option<[&str; 7]> {
    commas |= 1 << line.len();
    let mut row = [""; 7];
    let mut start = 0;
    for field in &mut row {
        if commas == 0 {
            return None;
        }
        let end = commas.trailing_zeros() as usize;
        commas &= commas - 1;
        let left = 1 << start;
        *field = if quotes & left == 0 {
            &line[start..end]
        } else {
            if end < start + 2 {
                return None;
            }
            let right = 1 << (end - 1);
            if quotes & right == 0 {
                return None;
            }
            quotes ^= left | right;
            &line[start + 1..end - 1]
        };
        start = end + 1;
    }
    (commas == 0 && quotes == 0).then_some(row)
}

// Each input block is classified once. Masks for an unfinished short row
// carry into the next block. Long rows use the general parser on borrowed text.
fn scan(
    text: &str,
    classify: impl Fn(&[u8; WIDTH]) -> Masks,
    mut visit_row: impl FnMut([&str; 7]),
) {
    let mut row_start = 0;
    let mut prior_commas = 0;
    let mut prior_quotes = 0;
    for (index, block) in text.as_bytes().chunks(WIDTH).enumerate() {
        let base = index * WIDTH;
        let masks = if let Ok(full) = block.try_into() {
            classify(full)
        } else {
            let mut tail = [0; WIDTH];
            tail[..block.len()].copy_from_slice(block);
            classify(&tail)
        };
        let mut newlines = masks.newlines;
        let mut local_start = 0;
        while newlines != 0 {
            let end = newlines.trailing_zeros() as usize;
            newlines &= newlines - 1;
            let line = &text[row_start..base + end];
            let row = if line.len() <= MAX_INLINE_ROW {
                let before_end = (1u64 << end) - 1;
                let prior = base + local_start - row_start;
                let commas = (masks.commas & before_end) >> local_start;
                let quotes = (masks.quotes & before_end) >> local_start;
                decode(
                    line,
                    prior_commas | (u128::from(commas) << prior),
                    prior_quotes | (u128::from(quotes) << prior),
                )
            } else {
                fields(line)
            };
            if let Some(row) = row {
                visit_row(row);
            }
            local_start = end + 1;
            row_start = base + local_start;
            prior_commas = 0;
            prior_quotes = 0;
        }
        if local_start < block.len() && base + block.len() - row_start <= MAX_INLINE_ROW {
            let prior = base + local_start - row_start;
            prior_commas |= u128::from(masks.commas >> local_start) << prior;
            prior_quotes |= u128::from(masks.quotes >> local_start) << prior;
        }
    }
}
