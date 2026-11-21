use chrono::NaiveDate;

pub const DAY: i64 = 86_400_000;
pub const WEEK: i64 = 7 * DAY;
pub const VISIT_GAP: i64 = 30 * 60_000;

pub fn number(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() {
        return None;
    }
    bytes.iter().try_fold(0i64, |n, &b| {
        if !b.is_ascii_digit() {
            return None;
        }
        n.checked_mul(10)?.checked_add(i64::from(b - b'0'))
    })
}

pub fn timestamp(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.get(4) != Some(&b'-') {
        return number(b);
    }
    if !matches!(b.len(), 20 | 25)
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let offset = match &b[19..] {
        b"Z" => 0,
        [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
            let h = number(&[*h1, *h2])?;
            let m = number(&[*m1, *m2])?;
            if h >= 24 || m >= 60 {
                return None;
            }
            (h * 60 + m) * 60_000 * if *sign == b'+' { 1 } else { -1 }
        }
        _ => return None,
    };
    let date = NaiveDate::from_ymd_opt(
        number(&b[0..4])? as i32,
        number(&b[5..7])? as u32,
        number(&b[8..10])? as u32,
    )?;
    let datetime = date.and_hms_opt(
        number(&b[11..13])? as u32,
        number(&b[14..16])? as u32,
        number(&b[17..19])? as u32,
    )?;
    Some(datetime.and_utc().timestamp_millis() - offset)
}
