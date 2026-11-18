//! แบ่งไฟล์ของ PR ใหญ่ให้ reviewer หลายคนแบบภาระใกล้เคียงกัน
//!
//! อัลกอริทึม: เรียงไฟล์จากใหญ่ไปเล็ก แล้วใส่ให้คนที่ภาระต่ำสุดตอนนั้น (LPT)
//! ถ้าไฟล์เดียวใหญ่กว่าส่วนแบ่งเฉลี่ย จะตัดเป็นช่วงบรรทัดแยกให้หลายคน
#![cfg_attr(not(test), allow(dead_code))]

/// งานหนึ่งชิ้นที่มอบให้ reviewer: ทั้งไฟล์ หรือช่วงบรรทัด `[from, to]` ของไฟล์
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub file: String,
    pub lines: u32,
    pub range: Option<(u32, u32)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub reviewer: String,
    pub pieces: Vec<Piece>,
}

impl Assignment {
    pub fn load(&self) -> u32 {
        self.pieces.iter().map(|p| p.lines).sum()
    }
}

/// `files` = (ชื่อไฟล์, จำนวนบรรทัดที่เปลี่ยน) ; `reviewers` เรียงตามลำดับความสำคัญ (คนแรกได้ก่อนเมื่อภาระเท่ากัน)
/// `split_large` = ตัดไฟล์ที่ใหญ่กว่าส่วนแบ่งเฉลี่ยเป็นช่วงบรรทัด
pub fn assign(files: &[(String, u32)], reviewers: &[String], split_large: bool) -> Vec<Assignment> {
    let mut out: Vec<Assignment> =
        reviewers.iter().map(|r| Assignment { reviewer: r.clone(), pieces: Vec::new() }).collect();
    if out.is_empty() {
        return out;
    }
    let total: u32 = files.iter().map(|(_, n)| *n).sum();
    let share = total.div_ceil(out.len() as u32).max(1);

    let mut pieces: Vec<Piece> = Vec::new();
    for (file, lines) in files {
        if *lines == 0 {
            continue;
        }
        if split_large && *lines > share && out.len() > 1 {
            pieces.extend(split_file(file, *lines, share));
        } else {
            pieces.push(Piece { file: file.clone(), lines: *lines, range: None });
        }
    }
    // ใหญ่ก่อน; เท่ากันเรียงตามชื่อ/ช่วงเพื่อให้ผลคงที่ทุกครั้ง
    pieces.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.file.cmp(&b.file)).then_with(|| a.range.cmp(&b.range)));

    for p in pieces {
        let idx = (0..out.len()).min_by_key(|&i| (out[i].load(), i)).unwrap_or(0);
        out[idx].pieces.push(p);
    }
    for a in &mut out {
        a.pieces.sort_by(|x, y| x.file.cmp(&y.file).then_with(|| x.range.cmp(&y.range)));
    }
    out
}

/// ตัดไฟล์ `lines` บรรทัดเป็นช่วงละไม่เกิน `chunk`
pub fn split_file(file: &str, lines: u32, chunk: u32) -> Vec<Piece> {
    let chunk = chunk.max(1);
    let mut out = Vec::new();
    let mut start = 1u32;
    while start <= lines {
        let end = (start + chunk - 1).min(lines);
        out.push(Piece { file: file.to_string(), lines: end - start + 1, range: Some((start, end)) });
        start = end + 1;
    }
    out
}

/// ภาระต่างกันมากสุดระหว่างคนที่ได้มากสุดกับน้อยสุด (ใช้วัดความสมดุล)
pub fn spread(assignments: &[Assignment]) -> u32 {
    let loads: Vec<u32> = assignments.iter().map(Assignment::load).collect();
    match (loads.iter().max(), loads.iter().min()) {
        (Some(hi), Some(lo)) => hi - lo,
        _ => 0,
    }
}

/// ข้อความ markdown สั้น ๆ ต่อคน เช่น `- @a (120): src/x.rs, src/big.rs L1-300`
pub fn render(assignments: &[Assignment]) -> String {
    assignments
        .iter()
        .map(|a| {
            let items: Vec<String> = a
                .pieces
                .iter()
                .map(|p| match p.range {
                    Some((s, e)) => format!("{} L{s}-{e}", p.file),
                    None => p.file.clone(),
                })
                .collect();
            let body = if items.is_empty() { "-".to_string() } else { items.join(", ") };
            format!("- @{} ({}): {}", a.reviewer, a.load(), body)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(v: &[(&str, u32)]) -> Vec<(String, u32)> {
        v.iter().map(|(f, n)| (f.to_string(), *n)).collect()
    }

    fn people(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn balances_by_largest_first() {
        let f = files(&[("a", 300), ("b", 200), ("c", 200), ("d", 100)]);
        let out = assign(&f, &people(&["x", "y"]), false);
        assert_eq!(out[0].load() + out[1].load(), 800);
        assert_eq!(spread(&out), 0);
        assert_eq!(out[0].reviewer, "x");
        assert!(out[0].pieces.iter().any(|p| p.file == "a"));
    }

    #[test]
    fn splits_single_huge_file_into_ranges() {
        let f = files(&[("big.rs", 700)]);
        let out = assign(&f, &people(&["x", "y"]), true);
        assert_eq!(out[0].pieces, vec![Piece { file: "big.rs".into(), lines: 350, range: Some((1, 350)) }]);
        assert_eq!(out[1].pieces, vec![Piece { file: "big.rs".into(), lines: 350, range: Some((351, 700)) }]);
        assert_eq!(render(&out), "- @x (350): big.rs L1-350\n- @y (350): big.rs L351-700");
    }

    #[test]
    fn without_split_whole_file_goes_to_one_person() {
        let f = files(&[("big.rs", 700), ("s.rs", 10)]);
        let out = assign(&f, &people(&["x", "y"]), false);
        assert_eq!(out[0].load(), 700);
        assert_eq!(out[1].load(), 10);
        assert_eq!(spread(&out), 690);
    }

    #[test]
    fn single_reviewer_never_splits() {
        let f = files(&[("big.rs", 900)]);
        let out = assign(&f, &people(&["solo"]), true);
        assert_eq!(out[0].pieces.len(), 1);
        assert_eq!(out[0].pieces[0].range, None);
    }

    #[test]
    fn empty_inputs() {
        assert!(assign(&files(&[("a", 5)]), &[], true).is_empty());
        let out = assign(&[], &people(&["x", "y"]), true);
        assert_eq!(spread(&out), 0);
        assert_eq!(render(&out), "- @x (0): -\n- @y (0): -");
        let zero = assign(&files(&[("empty", 0)]), &people(&["x"]), true);
        assert!(zero[0].pieces.is_empty());
    }

    #[test]
    fn split_file_edges() {
        assert_eq!(split_file("f", 5, 2).iter().map(|p| p.range).collect::<Vec<_>>(), vec![Some((1, 2)), Some((3, 4)), Some((5, 5))]);
        assert_eq!(split_file("f", 3, 0).len(), 3, "chunk 0 ต้องไม่วนไม่จบ");
        assert!(split_file("f", 0, 10).is_empty());
    }

    #[test]
    fn deterministic_on_ties() {
        let f = files(&[("b", 50), ("a", 50), ("c", 50), ("d", 50)]);
        let one = assign(&f, &people(&["x", "y"]), true);
        let two = assign(&f, &people(&["x", "y"]), true);
        assert_eq!(one, two);
        assert_eq!(one[0].pieces.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
    }
}
