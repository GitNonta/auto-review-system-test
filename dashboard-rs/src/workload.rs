//! รายงานภาระงานรีวิวต่อคน (Equal Review Workload) แบบ export ได้
//!
//! รับรายการงานรีวิว (`Assignment`) → รวมเป็นแถวต่อคน (`WorkloadRow`) ด้วยสูตรเดียวกับบอท
//! `Score = C + pending_weight * P` แล้วจัดอันดับ, เลือกผู้รีวิวคนถัดไป, สรุปสถิติ และ export CSV
//!
//! โมดูลนี้ไม่ยิง GitHub เอง: ผู้เรียกแปลง snapshot/marker เป็น `Assignment` ก่อน

use std::collections::BTreeMap;

use crate::csv;
use crate::stats;

const DAY_SECS: i64 = 86_400;

/// งานรีวิวหนึ่งชิ้นของ reviewer หนึ่งคนใน PR หนึ่งตัว
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub login: String,
    pub pr: i64,
    pub lines: i64,
    /// epoch วินาทีที่มอบหมาย
    pub assigned_at: i64,
    /// epoch วินาทีที่รีวิวเสร็จ (None = ยังค้าง)
    pub done_at: Option<i64>,
}

/// พารามิเตอร์ของสูตร (ค่าเริ่มต้นตรงกับ `.github/auto-review.yml`)
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    pub history_days: i64,
    pub pending_weight: i64,
    pub pending_cap: i64,
    pub tie_threshold: i64,
}

impl Default for Params {
    fn default() -> Self {
        Self { history_days: 30, pending_weight: 2, pending_cap: 800, tie_threshold: 300 }
    }
}

/// หนึ่งแถวของรายงาน
#[derive(Debug, Clone, PartialEq)]
pub struct WorkloadRow {
    pub login: String,
    /// C: บรรทัดที่รีวิวเสร็จในช่วง `history_days`
    pub completed: i64,
    /// P: บรรทัดที่ยังค้าง
    pub pending: i64,
    pub pending_prs: usize,
    pub score: i64,
    pub over_cap: bool,
    /// อายุงานค้างที่นานที่สุด (วินาที)
    pub oldest_pending: Option<i64>,
}

fn norm(login: &str) -> String {
    login.trim().to_ascii_lowercase()
}

/// รวมงานเป็นแถวต่อคน; คนใน `team` ที่ไม่มีงานเลยได้แถวศูนย์ด้วย
/// เรียงตาม score น้อย→มาก แล้วตาม login เพื่อให้ผลคงที่
pub fn build_rows(team: &[String], items: &[Assignment], params: &Params, now: i64) -> Vec<WorkloadRow> {
    let since = now - params.history_days * DAY_SECS;
    let mut map: BTreeMap<String, WorkloadRow> = BTreeMap::new();
    for login in team {
        let key = norm(login);
        map.entry(key.clone()).or_insert_with(|| empty_row(&key));
    }
    for a in items {
        let key = norm(&a.login);
        if key.is_empty() || a.lines < 0 {
            continue;
        }
        let row = map.entry(key.clone()).or_insert_with(|| empty_row(&key));
        match a.done_at {
            Some(done) if done >= since && done <= now => row.completed += a.lines,
            Some(_) => {}
            None => {
                row.pending += a.lines;
                row.pending_prs += 1;
                let age = (now - a.assigned_at).max(0);
                row.oldest_pending = Some(row.oldest_pending.map_or(age, |o| o.max(age)));
            }
        }
    }
    let mut rows: Vec<WorkloadRow> = map
        .into_values()
        .map(|mut r| {
            r.score = r.completed + params.pending_weight * r.pending;
            r.over_cap = r.pending > params.pending_cap;
            r
        })
        .collect();
    rows.sort_by(|a, b| a.score.cmp(&b.score).then_with(|| a.login.cmp(&b.login)));
    rows
}

fn empty_row(login: &str) -> WorkloadRow {
    WorkloadRow {
        login: login.to_string(),
        completed: 0,
        pending: 0,
        pending_prs: 0,
        score: 0,
        over_cap: false,
        oldest_pending: None,
    }
}

/// เลือก reviewer คนถัดไปแบบเดียวกับบอท:
/// 1. ตัดผู้เขียนและคนที่งานค้างเกินเพดาน (ถ้าเกินทุกคน ใช้ทุกคนแทน)
/// 2. คนที่ Score ห่างจากต่ำสุดไม่เกิน `tie_threshold` ถือว่าใกล้เคียง
/// 3. ในกลุ่มใกล้เคียง เลือกคนที่ไม่ได้รีวิวงานของผู้เขียนนี้นานที่สุด (`last_reviewed` ไม่มี = นานสุด)
/// 4. ยังเท่ากัน → Round-Robin ตามลำดับ login หลัง `last_pick`
pub fn pick_next(
    rows: &[WorkloadRow],
    author: &str,
    last_reviewed: &BTreeMap<String, i64>,
    last_pick: Option<&str>,
    params: &Params,
) -> Option<String> {
    let author = norm(author);
    let eligible: Vec<&WorkloadRow> = rows.iter().filter(|r| r.login != author).collect();
    let under_cap: Vec<&WorkloadRow> = eligible.iter().copied().filter(|r| !r.over_cap).collect();
    let pool = if under_cap.is_empty() { eligible } else { under_cap };
    let min = pool.iter().map(|r| r.score).min()?;
    let close: Vec<&WorkloadRow> = pool
        .into_iter()
        .filter(|r| r.score - min <= params.tie_threshold)
        .collect();
    let last_seen = |r: &WorkloadRow| last_reviewed.get(&r.login).copied().unwrap_or(i64::MIN);
    let oldest = close.iter().map(|r| last_seen(r)).min()?;
    let mut tied: Vec<&str> = close
        .iter()
        .filter(|r| last_seen(r) == oldest)
        .map(|r| r.login.as_str())
        .collect();
    tied.sort_unstable();
    let next = match last_pick.map(norm) {
        Some(prev) => tied.iter().find(|l| **l > prev.as_str()).or(tied.first()),
        None => tied.first(),
    };
    next.map(|s| s.to_string())
}

/// สรุปความเท่ากันของภาระงาน (ใช้ Score)
pub fn fairness(rows: &[WorkloadRow]) -> Option<stats::Summary> {
    let scores: Vec<f64> = rows.iter().map(|r| r.score as f64).collect();
    stats::summarize(&scores)
}

fn days(secs: Option<i64>) -> String {
    match secs {
        Some(s) => format!("{:.1}", s as f64 / DAY_SECS as f64),
        None => String::new(),
    }
}

/// export เป็น CSV (กัน formula injection เพราะ login มาจากภายนอก)
pub fn to_csv(rows: &[WorkloadRow]) -> String {
    let mut w = csv::Writer::sanitized();
    w.row(["login", "completed", "pending", "pending_prs", "score", "over_cap", "oldest_pending_days"])
        .expect("header defines width");
    for r in rows {
        w.row([
            r.login.clone(),
            r.completed.to_string(),
            r.pending.to_string(),
            r.pending_prs.to_string(),
            r.score.to_string(),
            r.over_cap.to_string(),
            days(r.oldest_pending),
        ])
        .expect("same width as header");
    }
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn team() -> Vec<String> {
        ["Alice", "bob", "Carol"].iter().map(|s| s.to_string()).collect()
    }

    fn done(login: &str, pr: i64, lines: i64, days_ago: i64) -> Assignment {
        Assignment {
            login: login.into(),
            pr,
            lines,
            assigned_at: NOW - (days_ago + 1) * DAY_SECS,
            done_at: Some(NOW - days_ago * DAY_SECS),
        }
    }

    fn pending(login: &str, pr: i64, lines: i64, days_ago: i64) -> Assignment {
        Assignment { login: login.into(), pr, lines, assigned_at: NOW - days_ago * DAY_SECS, done_at: None }
    }

    #[test]
    fn rows_apply_score_formula_and_window() {
        let items = vec![
            done("alice", 1, 100, 2),
            done("alice", 2, 999, 45), // นอกหน้าต่าง 30 วัน
            pending("bob", 3, 50, 1),
            pending("BOB", 4, 30, 3),
        ];
        let rows = build_rows(&team(), &items, &Params::default(), NOW);
        let get = |l: &str| rows.iter().find(|r| r.login == l).unwrap().clone();
        assert_eq!(get("alice").completed, 100);
        assert_eq!(get("alice").score, 100);
        assert_eq!(get("bob").pending, 80);
        assert_eq!(get("bob").pending_prs, 2);
        assert_eq!(get("bob").score, 160);
        assert_eq!(get("bob").oldest_pending, Some(3 * DAY_SECS));
        assert_eq!(get("carol").score, 0);
        let order: Vec<&str> = rows.iter().map(|r| r.login.as_str()).collect();
        assert_eq!(order, ["carol", "alice", "bob"]);
    }

    #[test]
    fn rows_include_non_team_reviewers_and_flag_cap() {
        let items = vec![pending("dave", 9, 900, 0)];
        let rows = build_rows(&team(), &items, &Params::default(), NOW);
        let dave = rows.iter().find(|r| r.login == "dave").unwrap();
        assert!(dave.over_cap);
        assert_eq!(dave.score, 1800);
    }

    #[test]
    fn pick_lowest_score_excluding_author() {
        let items = vec![done("bob", 1, 1000, 1), done("carol", 2, 2000, 1)];
        let rows = build_rows(&team(), &items, &Params::default(), NOW);
        let none = BTreeMap::new();
        assert_eq!(pick_next(&rows, "Bob", &none, None, &Params::default()).as_deref(), Some("alice"));
        assert_eq!(pick_next(&rows, "alice", &none, None, &Params::default()).as_deref(), Some("bob"));
    }

    #[test]
    fn pick_breaks_ties_by_exchange_then_round_robin() {
        let items = vec![done("alice", 1, 100, 1), done("bob", 2, 250, 1), done("carol", 3, 200, 1)];
        let rows = build_rows(&team(), &items, &Params::default(), NOW);
        // ทั้งสามคนห่างกันไม่เกิน 300 → ดูว่าใครไม่ได้รีวิวผู้เขียนนี้นานสุด
        let mut seen = BTreeMap::new();
        seen.insert("alice".to_string(), NOW - DAY_SECS);
        seen.insert("carol".to_string(), NOW - 10 * DAY_SECS);
        assert_eq!(pick_next(&rows, "zed", &seen, None, &Params::default()).as_deref(), Some("bob"));
        // ไม่มีประวัติเลย → Round-Robin ต่อจากคนล่าสุด
        let none = BTreeMap::new();
        assert_eq!(pick_next(&rows, "zed", &none, Some("alice"), &Params::default()).as_deref(), Some("bob"));
        assert_eq!(pick_next(&rows, "zed", &none, Some("carol"), &Params::default()).as_deref(), Some("alice"));
        assert_eq!(pick_next(&rows, "zed", &none, None, &Params::default()).as_deref(), Some("alice"));
    }

    #[test]
    fn pick_skips_over_cap_unless_everyone_is() {
        let p = Params::default();
        let items = vec![pending("alice", 1, 900, 1), done("carol", 2, 1500, 1)];
        let rows = build_rows(&team(), &items, &p, NOW);
        assert_eq!(pick_next(&rows, "bob", &BTreeMap::new(), None, &p).as_deref(), Some("carol"));
        let all = vec![pending("alice", 1, 1200, 1), pending("bob", 2, 1100, 1), pending("carol", 3, 850, 1)];
        let rows = build_rows(&team(), &all, &p, NOW);
        assert_eq!(pick_next(&rows, "x", &BTreeMap::new(), None, &p).as_deref(), Some("carol"));
        assert_eq!(pick_next(&[], "x", &BTreeMap::new(), None, &p), None);
    }

    #[test]
    fn fairness_reports_gap() {
        let items = vec![done("alice", 1, 900, 1), done("bob", 2, 1000, 1), done("carol", 3, 1100, 1)];
        let rows = build_rows(&team(), &items, &Params::default(), NOW);
        let s = fairness(&rows).unwrap();
        assert!((s.gap - 0.2).abs() < 1e-9);
        assert!((s.mean - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn csv_export_round_trips_and_is_sanitized() {
        let items = vec![pending("=evil()", 1, 10, 2), done("alice", 2, 5, 1)];
        let rows = build_rows(&[], &items, &Params::default(), NOW);
        let text = to_csv(&rows);
        let parsed = csv::parse(&text).unwrap();
        assert_eq!(parsed[0][0], "login");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[1], ["alice", "5", "0", "0", "5", "false", ""]);
        assert_eq!(parsed[2], ["'=evil()", "0", "10", "1", "20", "false", "2.0"]);
    }
}
