//! แปลงช่วงเวลาแบบสั้น (เช่น `48h`, `2d`, `1d12h`, `90m`) เป็นวินาที และแปลงกลับเป็นข้อความอ่านง่าย
//!
//! ใช้กับค่าอย่าง `overdue_hours` หรือ query `?older_than=2d` ของแดชบอร์ด
//! ไม่มี dependency ภายนอก

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// ข้อผิดพลาดตอน parse
#[derive(Debug, PartialEq, Eq)]
pub enum DurationError {
    Empty,
    MissingNumber(char),
    MissingUnit(String),
    UnknownUnit(char),
    Overflow,
}

fn unit_seconds(unit: char) -> Option<i64> {
    match unit {
        's' => Some(1),
        'm' => Some(MINUTE),
        'h' => Some(HOUR),
        'd' => Some(DAY),
        _ => None,
    }
}

/// parse เช่น `"1d12h"` → 129600 (ช่องว่างระหว่างส่วนได้, ตัวพิมพ์ใหญ่ได้)
pub fn parse_duration(input: &str) -> Result<i64, DurationError> {
    let text: String = input.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if text.is_empty() {
        return Err(DurationError::Empty);
    }
    let mut total: i64 = 0;
    let mut number = String::new();
    for c in text.chars() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        let secs = unit_seconds(c).ok_or(DurationError::UnknownUnit(c))?;
        if number.is_empty() {
            return Err(DurationError::MissingNumber(c));
        }
        let n: i64 = number.parse().map_err(|_| DurationError::Overflow)?;
        let part = n.checked_mul(secs).ok_or(DurationError::Overflow)?;
        total = total.checked_add(part).ok_or(DurationError::Overflow)?;
        number.clear();
    }
    if !number.is_empty() {
        return Err(DurationError::MissingUnit(number));
    }
    Ok(total)
}

/// แปลงวินาทีเป็นข้อความ เช่น 129600 → `"1d 12h"` (แสดงไม่เกิน 2 หน่วยใหญ่สุด)
pub fn format_duration(seconds: i64) -> String {
    if seconds <= 0 {
        return "0s".to_string();
    }
    let mut rest = seconds;
    let mut parts = Vec::new();
    for (unit, size) in [("d", DAY), ("h", HOUR), ("m", MINUTE), ("s", 1)] {
        if rest >= size {
            parts.push(format!("{}{}", rest / size, unit));
            rest %= size;
        }
        if parts.len() == 2 {
            break;
        }
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_and_combined_units() {
        assert_eq!(parse_duration("48h"), Ok(48 * HOUR));
        assert_eq!(parse_duration("2d"), Ok(2 * DAY));
        assert_eq!(parse_duration("1d12h"), Ok(DAY + 12 * HOUR));
        assert_eq!(parse_duration(" 1D 30M "), Ok(DAY + 30 * MINUTE));
        assert_eq!(parse_duration("45s"), Ok(45));
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(parse_duration(""), Err(DurationError::Empty));
        assert_eq!(parse_duration("h"), Err(DurationError::MissingNumber('h')));
        assert_eq!(parse_duration("12"), Err(DurationError::MissingUnit("12".into())));
        assert_eq!(parse_duration("3x"), Err(DurationError::UnknownUnit('x')));
        assert_eq!(parse_duration("99999999999999999999d"), Err(DurationError::Overflow));
    }

    #[test]
    fn formats_two_largest_units() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(59), "59s");
        assert_eq!(format_duration(DAY + 12 * HOUR + 5), "1d 12h");
        assert_eq!(format_duration(2 * HOUR + 30 * MINUTE), "2h 30m");
        assert_eq!(format_duration(parse_duration("48h").unwrap()), "2d");
    }
}
