//! CSV ขนาดเล็กตาม RFC 4180 สำหรับ export ตารางภาระงานจากแดชบอร์ด
//!
//! - เขียน: ใส่ `"` ครอบเมื่อมี `,` `"` CR/LF หรือช่องว่างหัว/ท้าย และ escape `"` เป็น `""`
//! - อ่าน: รองรับ field ที่มี quote ข้ามบรรทัด, CRLF/LF, และบรรทัดสุดท้ายไม่มี newline
//! - กัน CSV injection: field ที่ขึ้นต้นด้วย `= + - @` (เมื่อเปิดใน spreadsheet) ถูกเติม `'` นำหน้าถ้าเปิด `sanitize`

use std::fmt;

/// ข้อผิดพลาดตอนอ่าน CSV (บรรทัดนับจาก 1)
#[derive(Debug, PartialEq, Eq)]
pub enum CsvError {
    UnterminatedQuote { line: usize },
    UnexpectedQuote { line: usize },
    RaggedRow { line: usize, expected: usize, found: usize },
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsvError::UnterminatedQuote { line } => write!(f, "line {line}: quote is not closed"),
            CsvError::UnexpectedQuote { line } => write!(f, "line {line}: quote inside unquoted field"),
            CsvError::RaggedRow { line, expected, found } => {
                write!(f, "line {line}: expected {expected} fields, found {found}")
            }
        }
    }
}

/// ตัวเขียน CSV สะสมผลลงใน `String`
#[derive(Debug, Default)]
pub struct Writer {
    out: String,
    sanitize: bool,
    width: Option<usize>,
}

fn needs_quotes(field: &str) -> bool {
    field.contains([',', '"', '\n', '\r'])
        || field.starts_with(char::is_whitespace)
        || field.ends_with(char::is_whitespace)
}

fn is_formula(field: &str) -> bool {
    matches!(field.chars().next(), Some('=' | '+' | '-' | '@' | '\t' | '\r'))
        && field.parse::<f64>().is_err()
}

/// escape field เดียว (ไม่ sanitize)
pub fn escape_field(field: &str) -> String {
    if needs_quotes(field) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    /// เปิดการกัน formula injection (แนะนำเมื่อไฟล์จะถูกเปิดใน Excel/Sheets)
    pub fn sanitized() -> Self {
        Self { sanitize: true, ..Self::default() }
    }

    /// เขียนหนึ่งแถว; ทุกแถวต้องมีจำนวนคอลัมน์เท่าแถวแรก
    pub fn row<I, S>(&mut self, fields: I) -> Result<(), CsvError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let cells: Vec<String> = fields
            .into_iter()
            .map(|f| {
                let f = f.as_ref();
                if self.sanitize && is_formula(f) {
                    escape_field(&format!("'{f}"))
                } else {
                    escape_field(f)
                }
            })
            .collect();
        let line = self.out.matches("\r\n").count() + 1;
        match self.width {
            Some(w) if w != cells.len() => {
                return Err(CsvError::RaggedRow { line, expected: w, found: cells.len() })
            }
            None => self.width = Some(cells.len()),
            _ => {}
        }
        self.out.push_str(&cells.join(","));
        self.out.push_str("\r\n");
        Ok(())
    }

    pub fn finish(self) -> String {
        self.out
    }
}

/// อ่าน CSV ทั้งก้อนเป็นแถว; บังคับให้ทุกแถวกว้างเท่ากัน
pub fn parse(input: &str) -> Result<Vec<Vec<String>>, CsvError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let mut quoted = false;
    let mut line = 1usize;
    let mut row_line = 1usize;
    let mut chars = input.chars().peekable();

    let finish_row = |rows: &mut Vec<Vec<String>>, row: Vec<String>, at: usize| -> Result<(), CsvError> {
        if let Some(first) = rows.first() {
            if first.len() != row.len() {
                return Err(CsvError::RaggedRow { line: at, expected: first.len(), found: row.len() });
            }
        }
        rows.push(row);
        Ok(())
    };

    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    field.push('"');
                }
                '"' => in_quotes = false,
                '\n' => {
                    line += 1;
                    field.push(c);
                }
                _ => field.push(c),
            }
            continue;
        }
        match c {
            '"' if field.is_empty() && !quoted => {
                in_quotes = true;
                quoted = true;
            }
            '"' => return Err(CsvError::UnexpectedQuote { line }),
            ',' => {
                row.push(std::mem::take(&mut field));
                quoted = false;
            }
            '\r' if chars.peek() == Some(&'\n') => {}
            '\n' | '\r' => {
                row.push(std::mem::take(&mut field));
                quoted = false;
                finish_row(&mut rows, std::mem::take(&mut row), row_line)?;
                line += 1;
                row_line = line;
            }
            _ if quoted => return Err(CsvError::UnexpectedQuote { line }),
            _ => field.push(c),
        }
    }
    if in_quotes {
        return Err(CsvError::UnterminatedQuote { line: row_line });
    }
    if !field.is_empty() || quoted || !row.is_empty() {
        row.push(field);
        finish_row(&mut rows, row, row_line)?;
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_only_when_needed() {
        assert_eq!(escape_field("plain"), "plain");
        assert_eq!(escape_field("a,b"), "\"a,b\"");
        assert_eq!(escape_field("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(escape_field("two\nlines"), "\"two\nlines\"");
        assert_eq!(escape_field(" pad"), "\" pad\"");
        assert_eq!(escape_field(""), "");
    }

    #[test]
    fn writer_uses_crlf_and_checks_width() {
        let mut w = Writer::new();
        w.row(["login", "lines"]).unwrap();
        w.row(["alice", "120"]).unwrap();
        assert_eq!(
            w.row(["bob"]),
            Err(CsvError::RaggedRow { line: 3, expected: 2, found: 1 })
        );
        assert_eq!(w.finish(), "login,lines\r\nalice,120\r\n");
    }

    #[test]
    fn sanitized_writer_blocks_formulas_but_keeps_numbers() {
        let mut w = Writer::sanitized();
        w.row(["=HYPERLINK(\"x\")", "-12", "@me", "+1.5"]).unwrap();
        assert_eq!(w.finish(), "\"'=HYPERLINK(\"\"x\"\")\",-12,'@me,+1.5\r\n");
    }

    #[test]
    fn parses_quotes_newlines_and_crlf() {
        let rows = parse("a,b\r\n\"x,1\",\"he said \"\"ok\"\"\"\n\"multi\nline\",\n").unwrap();
        assert_eq!(
            rows,
            vec![
                vec!["a".to_string(), "b".to_string()],
                vec!["x,1".to_string(), "he said \"ok\"".to_string()],
                vec!["multi\nline".to_string(), String::new()],
            ]
        );
    }

    #[test]
    fn parses_last_line_without_newline() {
        assert_eq!(parse("k,v\n1,2").unwrap().len(), 2);
        assert_eq!(parse("\"\"").unwrap(), vec![vec![String::new()]]);
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        assert_eq!(parse("a,b\n\"open"), Err(CsvError::UnterminatedQuote { line: 2 }));
        assert_eq!(parse("a,b\nx\"y,z\n"), Err(CsvError::UnexpectedQuote { line: 2 }));
        assert_eq!(parse("\"q\"x,1\n"), Err(CsvError::UnexpectedQuote { line: 1 }));
        assert_eq!(
            parse("a,b\n1,2,3\n"),
            Err(CsvError::RaggedRow { line: 2, expected: 2, found: 3 })
        );
        assert_eq!(
            CsvError::RaggedRow { line: 2, expected: 2, found: 3 }.to_string(),
            "line 2: expected 2 fields, found 3"
        );
    }

    #[test]
    fn round_trip() {
        let data = [["name", "note"], ["a,b", "x\"y"], [" s ", "line\r\nbreak"]];
        let mut w = Writer::new();
        for r in data {
            w.row(r).unwrap();
        }
        let back = parse(&w.finish()).unwrap();
        let expect: Vec<Vec<String>> =
            data.iter().map(|r| r.iter().map(|s| s.to_string()).collect()).collect();
        assert_eq!(back, expect);
    }
}
