//! อ่าน `patch` (unified diff) ที่ GitHub ส่งมากับ `GET /pulls/{n}/files`
//!
//! ใช้หาว่าบรรทัดไหนของไฟล์ใหม่ถูกเพิ่ม/แก้ เพื่อบอก reviewer ว่าต้องดูช่วงไหน
//! และนับ additions/deletions เองได้เมื่อ API ไม่ส่งตัวเลขมา (เช่น snapshot เก่า)
#![cfg_attr(not(test), allow(dead_code))]

/// หัว hunk `@@ -old_start,old_len +new_start,new_len @@ section`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    pub section: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    Context { old: u32, new: u32 },
    Added { new: u32 },
    Removed { old: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    BadHeader { line_no: usize, text: String },
    LineOutsideHunk { line_no: usize },
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatchError::BadHeader { line_no, text } => write!(f, "บรรทัด {line_no}: หัว hunk ไม่ถูกต้อง `{text}`"),
            PatchError::LineOutsideHunk { line_no } => write!(f, "บรรทัด {line_no}: เนื้อหาอยู่นอก hunk"),
        }
    }
}

/// สรุปตัวเลขของ patch หนึ่งไฟล์
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stat {
    pub additions: u32,
    pub deletions: u32,
    pub hunks: u32,
}

impl Stat {
    /// ขนาดที่ใช้คิด Score (เพิ่ม + ลบ)
    pub fn changes(&self) -> u32 {
        self.additions + self.deletions
    }
}

/// แยก `@@ -a,b +c,d @@ rest` → (a, b, c, d, rest) ; ถ้าไม่มี `,len` ถือว่า len = 1
pub fn parse_hunk_header(line: &str) -> Option<(u32, u32, u32, u32, String)> {
    let rest = line.strip_prefix("@@ ")?;
    let end = rest.find(" @@")?;
    let (ranges, tail) = rest.split_at(end);
    let section = tail.trim_start_matches(" @@").trim().to_string();
    let mut it = ranges.split_whitespace();
    let old = it.next()?.strip_prefix('-')?;
    let new = it.next()?.strip_prefix('+')?;
    if it.next().is_some() {
        return None;
    }
    let (os, ol) = parse_range(old)?;
    let (ns, nl) = parse_range(new)?;
    Some((os, ol, ns, nl, section))
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// parse patch ทั้งไฟล์ ข้ามบรรทัดหัว `diff --git`/`---`/`+++`/`index` ที่อาจติดมาก่อน hunk แรก
pub fn parse(patch: &str) -> Result<Vec<Hunk>, PatchError> {
    let mut hunks: Vec<Hunk> = Vec::new();
    let (mut old_no, mut new_no) = (0u32, 0u32);
    for (idx, raw) in patch.lines().enumerate() {
        let line_no = idx + 1;
        if raw.starts_with("@@") {
            let (os, ol, ns, nl, section) =
                parse_hunk_header(raw).ok_or_else(|| PatchError::BadHeader { line_no, text: raw.to_string() })?;
            old_no = os;
            new_no = ns;
            hunks.push(Hunk { old_start: os, old_len: ol, new_start: ns, new_len: nl, section, lines: Vec::new() });
            continue;
        }
        let Some(h) = hunks.last_mut() else {
            if is_file_header(raw) || raw.is_empty() {
                continue;
            }
            return Err(PatchError::LineOutsideHunk { line_no });
        };
        match raw.chars().next() {
            Some('+') => {
                h.lines.push(Line::Added { new: new_no });
                new_no += 1;
            }
            Some('-') => {
                h.lines.push(Line::Removed { old: old_no });
                old_no += 1;
            }
            // "\ No newline at end of file" ไม่ใช่บรรทัดจริง
            Some('\\') => {}
            _ => {
                h.lines.push(Line::Context { old: old_no, new: new_no });
                old_no += 1;
                new_no += 1;
            }
        }
    }
    Ok(hunks)
}

fn is_file_header(line: &str) -> bool {
    ["diff --git ", "index ", "--- ", "+++ ", "new file mode", "deleted file mode", "similarity index", "rename from", "rename to"]
        .iter()
        .any(|p| line.starts_with(p))
}

pub fn stat(hunks: &[Hunk]) -> Stat {
    let mut s = Stat { hunks: hunks.len() as u32, ..Stat::default() };
    for h in hunks {
        for l in &h.lines {
            match l {
                Line::Added { .. } => s.additions += 1,
                Line::Removed { .. } => s.deletions += 1,
                Line::Context { .. } => {}
            }
        }
    }
    s
}

/// ช่วงบรรทัดปิด `[start, end]` ในไฟล์ใหม่
pub type Range = (u32, u32);

/// บรรทัดของไฟล์ใหม่ที่ถูกเพิ่ม รวมเป็นช่วงต่อเนื่อง
/// `gap` = ยอมให้ช่วงห่างกันไม่เกินกี่บรรทัดแล้วยังรวมเป็นช่วงเดียว (0 = ต้องติดกันจริง)
pub fn added_ranges(hunks: &[Hunk], gap: u32) -> Vec<Range> {
    let lines = hunks.iter().flat_map(|h| h.lines.iter()).filter_map(|l| match l {
        Line::Added { new } => Some(*new),
        _ => None,
    });
    merge_lines(lines, gap)
}

/// รวมเลขบรรทัด (เรียงแล้วหรือไม่ก็ได้) ให้เป็นช่วง
pub fn merge_lines<I: IntoIterator<Item = u32>>(lines: I, gap: u32) -> Vec<Range> {
    let mut v: Vec<u32> = lines.into_iter().collect();
    v.sort_unstable();
    v.dedup();
    let mut out: Vec<Range> = Vec::new();
    for n in v {
        match out.last_mut() {
            Some((_, end)) if n <= end.saturating_add(gap).saturating_add(1) => *end = n,
            _ => out.push((n, n)),
        }
    }
    out
}

/// แสดงช่วงแบบที่คอมเมนต์ของบอทใช้ เช่น `L1-20, L35, L40-41`
pub fn format_ranges(ranges: &[Range]) -> String {
    ranges
        .iter()
        .map(|&(a, b)| if a == b { format!("L{a}") } else { format!("L{a}-{b}") })
        .collect::<Vec<_>>()
        .join(", ")
}

/// ตัดช่วงให้เหลือเฉพาะส่วนที่อยู่ใน `[from, to]`
pub fn clip_ranges(ranges: &[Range], from: u32, to: u32) -> Vec<Range> {
    ranges
        .iter()
        .filter_map(|&(a, b)| {
            let (s, e) = (a.max(from), b.min(to));
            (s <= e).then_some((s, e))
        })
        .collect()
}

/// จำนวนบรรทัดรวมของชุดช่วง
pub fn range_len(ranges: &[Range]) -> u32 {
    ranges.iter().map(|&(a, b)| b - a + 1).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 1111111..2222222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,4 +1,5 @@ fn main() {
 line1
-line2
+line2 changed
+line2b
 line3
 line4
@@ -10,3 +11,4 @@
 ten
+eleven
 twelve
 thirteen
\\ No newline at end of file";

    #[test]
    fn header_parsing() {
        assert_eq!(parse_hunk_header("@@ -1,4 +1,5 @@ fn x"), Some((1, 4, 1, 5, "fn x".to_string())));
        assert_eq!(parse_hunk_header("@@ -0,0 +1 @@"), Some((0, 0, 1, 1, String::new())));
        assert_eq!(parse_hunk_header("@@ -3 +3,2 @@"), Some((3, 1, 3, 2, String::new())));
        assert_eq!(parse_hunk_header("@@ -a,1 +1 @@"), None);
        assert_eq!(parse_hunk_header("@@ -1 +1"), None);
        assert_eq!(parse_hunk_header("@@ -1 +1 +2 @@"), None);
        assert_eq!(parse_hunk_header("nope"), None);
    }

    #[test]
    fn parse_sample_and_stat() {
        let hunks = parse(SAMPLE).unwrap();
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].section, "fn main() {");
        let s = stat(&hunks);
        assert_eq!(s, Stat { additions: 3, deletions: 1, hunks: 2 });
        assert_eq!(s.changes(), 4);
        assert_eq!(hunks[1].lines[1], Line::Added { new: 12 });
        assert_eq!(hunks[1].lines.len(), 4, "บรรทัด \\ No newline ต้องไม่ถูกนับ");
    }

    #[test]
    fn line_numbers_track_both_sides() {
        let hunks = parse(SAMPLE).unwrap();
        let h = &hunks[0];
        assert_eq!(h.lines[0], Line::Context { old: 1, new: 1 });
        assert_eq!(h.lines[1], Line::Removed { old: 2 });
        assert_eq!(h.lines[2], Line::Added { new: 2 });
        assert_eq!(h.lines[3], Line::Added { new: 3 });
        assert_eq!(h.lines[4], Line::Context { old: 3, new: 4 });
    }

    #[test]
    fn ranges_from_sample() {
        let hunks = parse(SAMPLE).unwrap();
        assert_eq!(added_ranges(&hunks, 0), vec![(2, 3), (12, 12)]);
        assert_eq!(added_ranges(&hunks, 10), vec![(2, 12)]);
        assert_eq!(format_ranges(&added_ranges(&hunks, 0)), "L2-3, L12");
    }

    #[test]
    fn new_file_patch() {
        let patch: String = std::iter::once("@@ -0,0 +1,3 @@".to_string())
            .chain((1..=3).map(|i| format!("+row {i}")))
            .collect::<Vec<_>>()
            .join("\n");
        let hunks = parse(&patch).unwrap();
        assert_eq!(stat(&hunks).additions, 3);
        assert_eq!(added_ranges(&hunks, 0), vec![(1, 3)]);
    }

    #[test]
    fn errors_are_reported_with_line_numbers() {
        assert_eq!(parse("garbage\n@@ -1 +1 @@"), Err(PatchError::LineOutsideHunk { line_no: 1 }));
        let e = parse("@@ -1 +1 @@\n x\n@@ broken @@").unwrap_err();
        assert!(matches!(e, PatchError::BadHeader { line_no: 3, .. }));
        assert!(e.to_string().contains("บรรทัด 3"));
        assert!(parse("").unwrap().is_empty());
    }

    #[test]
    fn merge_unsorted_with_duplicates() {
        assert_eq!(merge_lines([5, 1, 2, 2, 3, 9], 0), vec![(1, 3), (5, 5), (9, 9)]);
        assert_eq!(merge_lines([5, 1, 2, 3, 9], 1), vec![(1, 5), (9, 9)]);
        assert_eq!(merge_lines(std::iter::empty(), 0), vec![]);
        assert_eq!(merge_lines([u32::MAX - 1, u32::MAX], 0), vec![(u32::MAX - 1, u32::MAX)]);
    }

    #[test]
    fn clip_and_len() {
        let r = vec![(1, 10), (20, 30), (40, 40)];
        assert_eq!(clip_ranges(&r, 5, 25), vec![(5, 10), (20, 25)]);
        assert_eq!(clip_ranges(&r, 11, 19), vec![]);
        assert_eq!(range_len(&r), 10 + 11 + 1);
        assert_eq!(format_ranges(&[]), "");
    }
}
