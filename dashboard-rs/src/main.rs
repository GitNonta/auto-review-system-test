//! Dashboard (อ่านอย่างเดียว) สำหรับระบบ Auto-assign Reviewer — พอร์ตจาก `.github/scripts/dashboard.cjs`
//!
//! แสดงต่อ PR ที่เปิดอยู่: reviewer แต่ละคน, รีวิวแล้วหรือยัง, กี่บรรทัด,
//! ช่วงบรรทัด/ไฟล์ที่ต้องดู (planScope ตัวเดียวกับบอท), ป้ายสถานะ และภาระงานค้างต่อคน
//!
//! ใช้:
//!   dashboard-rs --repo owner/name --out dashboard.html
//!   dashboard-rs --repo owner/name --serve 7788 [--host 0.0.0.0]
//!   dashboard-rs --snapshot snap.json --out dashboard.html
//!
//! ไม่อ่านค่าจาก GitHub แบบเขียนกลับ: เรียกแค่ GET (pulls/files/reviews/comments)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{extract::State, Router};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::Mutex;

const MARK: &str = "<!-- auto-review-state:";
const MARK_END: &str = "-->";
const BOT_LOGIN: &str = "github-actions[bot]";
const API: &str = "https://api.github.com";
const DEFAULT_THRESHOLD: i64 = 600;

/* ------------------------------ helpers ------------------------------ */

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn escape_html(v: &str) -> String {
    let mut out = String::with_capacity(v.len() + 8);
    for c in v.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// แปลง ISO 8601 (แบบที่ GitHub ส่งมา) เป็น epoch วินาที
fn iso_epoch(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |s: &str| s.parse::<i64>().ok();
    let y = num(iso.get(0..4)?)?;
    let mo = num(iso.get(5..7)?)? as u32;
    let d = num(iso.get(8..10)?)? as u32;
    let h = num(iso.get(11..13)?)?;
    let mi = num(iso.get(14..16)?)?;
    let se = num(iso.get(17..19).unwrap_or("00"))?;
    let mut epoch = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se;
    let rest = &iso[19..];
    if let Some(pos) = rest.find(['+', '-']) {
        let tz = &rest[pos..];
        let sign: i64 = if tz.starts_with('-') { -1 } else { 1 };
        let hh: i64 = tz.get(1..3).and_then(|s| s.parse().ok()).unwrap_or(0);
        let mm: i64 = tz.get(4..6).and_then(|s| s.parse().ok()).unwrap_or(0);
        epoch -= sign * (hh * 3600 + mm * 60);
    }
    Some(epoch)
}

fn iso_now() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86400));
    let rem = secs.rem_euclid(86400);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn rel_time(iso: Option<&str>, now: i64) -> String {
    let Some(t) = iso.and_then(iso_epoch) else {
        return "—".to_string();
    };
    let diff = (now - t).max(0);
    let mins = diff as f64 / 60.0;
    if mins < 1.0 {
        return "เมื่อสักครู่".to_string();
    }
    if mins < 60.0 {
        return format!("{} นาทีที่แล้ว", mins.round() as i64);
    }
    let hours = diff as f64 / 3600.0;
    if hours < 48.0 {
        return format!("{} ชม.ที่แล้ว", hours.round() as i64);
    }
    let days = hours / 24.0;
    if days < 60.0 {
        return format!("{} วันที่แล้ว", days.round() as i64);
    }
    let (y, m, d) = civil_from_days(t.div_euclid(86400));
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn now_epoch() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// glob แบบเดียวกับ globToRegex: `*` = อะไรก็ได้ยกเว้น `/`, `**` = อะไรก็ได้รวม `/`, `?` = 1 ตัวอักษร
fn glob_match(pat: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        if p.is_empty() {
            return t.is_empty();
        }
        if p.len() >= 2 && p[0] == '*' && p[1] == '*' {
            let mut rest = &p[2..];
            if !rest.is_empty() && rest[0] == '/' {
                rest = &rest[1..];
            }
            for i in 0..=t.len() {
                if go(rest, &t[i..]) {
                    return true;
                }
            }
            return false;
        }
        match p[0] {
            '*' => {
                for i in 0..=t.len() {
                    if go(&p[1..], &t[i..]) {
                        return true;
                    }
                    if i < t.len() && t[i] == '/' {
                        break;
                    }
                }
                false
            }
            '?' => !t.is_empty() && t[0] != '/' && go(&p[1..], &t[1..]),
            c => !t.is_empty() && t[0] == c && go(&p[1..], &t[1..]),
        }
    }
    let p: Vec<char> = pat.chars().collect();
    let t: Vec<char> = text.chars().collect();
    go(&p, &t)
}

/* ------------------------------ config ------------------------------ */

#[derive(Clone, Debug)]
struct Cfg {
    large_pr_threshold: i64,
    exclude_patterns: Vec<String>,
}

impl Default for Cfg {
    fn default() -> Self {
        Cfg { large_pr_threshold: DEFAULT_THRESHOLD, exclude_patterns: Vec::new() }
    }
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    let t = t.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')).unwrap_or(t);
    let t = t.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(t);
    t.to_string()
}

/// อ่าน YAML แบบเบา: key แบน + list ใต้ key (ไม่ต้องพึ่ง parser เต็ม)
fn read_config_lite(text: &str) -> Cfg {
    let mut cfg = Cfg::default();
    let mut key: Option<String> = None;
    let mut list: Vec<String> = Vec::new();
    let flush = |cfg: &mut Cfg, key: &Option<String>, list: &mut Vec<String>| {
        if let Some(k) = key {
            if k == "exclude_patterns" {
                cfg.exclude_patterns = std::mem::take(list);
            }
        }
        list.clear();
    };
    for raw in text.lines() {
        let trimmed = raw.trim_start();
        if trimmed.starts_with('#') || trimmed.trim().is_empty() {
            continue;
        }
        if let Some(item) = trimmed.strip_prefix("- ") {
            if key.is_some() {
                list.push(unquote(item));
            }
            continue;
        }
        let bytes: Vec<char> = trimmed.chars().collect();
        let mut name_end = 0;
        for (i, c) in bytes.iter().enumerate() {
            if *c == ':' {
                name_end = i;
                break;
            }
        }
        if name_end == 0 {
            continue;
        }
        let name: String = bytes[..name_end].iter().collect();
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        flush(&mut cfg, &key, &mut list);
        let value: String = bytes[name_end + 1..].iter().collect();
        let value = value.split('#').next().unwrap_or("").trim().to_string();
        key = Some(name.clone());
        if value.is_empty() {
            continue;
        }
        match name.as_str() {
            "large_pr_threshold" => {
                if let Ok(n) = value.parse::<i64>() {
                    cfg.large_pr_threshold = n;
                }
                key = None;
            }
            "exclude_patterns" => {
                if value.trim() != "[]" {
                    cfg.exclude_patterns.push(unquote(&value));
                }
                key = None;
            }
            _ => key = None,
        }
    }
    flush(&mut cfg, &key, &mut list);
    cfg
}

fn load_config(path: Option<&str>) -> (Cfg, Option<String>) {
    let candidates: Vec<String> = match path {
        Some(p) => vec![p.to_string()],
        None => vec![
            ".github/auto-review.yml".to_string(),
            "../.github/auto-review.yml".to_string(),
            "../../.github/auto-review.yml".to_string(),
        ],
    };
    for c in &candidates {
        if let Ok(text) = std::fs::read_to_string(c) {
            return (read_config_lite(&text), None);
        }
    }
    (
        Cfg::default(),
        Some(format!("อ่าน config ไม่ได้ ({}) — ใช้ค่าเริ่มต้น", candidates.join(", "))),
    )
}

/* ------------------------------ pure helpers (จาก auto-review.cjs) ------------------------------ */

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
struct LineRange {
    start: i64,
    end: i64,
}

/// patch → ช่วงบรรทัดฝั่งใหม่ (เฉพาะ hunk ที่มีการเพิ่ม/แก้)
fn hunks_from_patch(patch: Option<&str>) -> Option<Vec<LineRange>> {
    let patch = patch?;
    if patch.is_empty() {
        return None;
    }
    let lines: Vec<&str> = patch.split('\n').collect();
    let mut ranges: Vec<LineRange> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !line.starts_with("@@ ") {
            continue;
        }
        let mut parts = line.split(' ');
        let _ = parts.next();
        let _old = parts.next().unwrap_or("");
        let new = parts.next().unwrap_or("");
        let new = new.strip_prefix('+').unwrap_or("");
        let mut it = new.split(',');
        let start: i64 = match it.next().and_then(|s| s.parse().ok()) {
            Some(v) => v,
            None => continue,
        };
        let count: i64 = match it.next() {
            Some(s) => s.parse().unwrap_or(1),
            None => 1,
        };
        let mut added = 0i64;
        for next in lines.iter().skip(i + 1) {
            if next.starts_with("@@ ") {
                break;
            }
            if next.starts_with('+') && !next.starts_with("+++") {
                added += 1;
            }
        }
        if count > 0 {
            ranges.push(LineRange { start, end: start + count - 1 });
        } else if added > 0 {
            ranges.push(LineRange { start, end: start });
        }
    }
    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

fn merge_ranges(ranges: &[LineRange]) -> Vec<LineRange> {
    let mut sorted = ranges.to_vec();
    sorted.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    let mut out: Vec<LineRange> = Vec::new();
    for r in sorted {
        match out.last_mut() {
            Some(last) if r.start <= last.end + 1 => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

#[derive(Clone, Debug)]
struct FileInfo {
    filename: String,
    lines: i64,
    ranges: Option<Vec<LineRange>>,
}

#[derive(Clone, Debug, Serialize)]
struct Part {
    filename: String,
    ranges: Option<Vec<LineRange>>,
    unsplittable: bool,
}

#[derive(Clone, Debug)]
struct Bucket {
    login: String,
    lines: i64,
    parts: Vec<Part>,
}

struct Unit {
    filename: String,
    ranges: Option<Vec<LineRange>>,
    weight: i64,
    unsplittable: bool,
}

/// แบ่งขอบเขตรีวิว: ไฟล์ใหญ่ก่อน, ไฟล์ที่ใหญ่กว่าส่วนแบ่งถูกแบ่งเป็นช่วงบรรทัด
fn plan_scope(files: &[FileInfo], logins: &[String]) -> Vec<Bucket> {
    let total: i64 = files.iter().map(|f| f.lines).sum();
    let target = if logins.is_empty() { total as f64 } else { total as f64 / logins.len() as f64 };
    let mut sorted: Vec<&FileInfo> = files.iter().collect();
    sorted.sort_by(|a, b| b.lines.cmp(&a.lines).then(a.filename.cmp(&b.filename)));

    let mut units: Vec<Unit> = Vec::new();
    for f in sorted {
        let has_ranges = f.ranges.as_ref().map_or(false, |r| !r.is_empty());
        if logins.len() > 1 && (f.lines as f64) > target && has_ranges {
            let ranges = f.ranges.clone().unwrap_or_default();
            let span: i64 = ranges.iter().map(|r| r.end - r.start + 1).sum::<i64>().max(1);
            let mut k = ((f.lines as f64 / target).ceil() as i64).min(8).min(span);
            if k < 2 {
                k = 2;
            }
            let per = span as f64 / k as f64;
            let mut groups: Vec<Vec<LineRange>> = Vec::new();
            let mut cur: Vec<LineRange> = Vec::new();
            let mut cur_span: i64 = 0;
            for r in ranges.iter() {
                let mut start = r.start;
                while start <= r.end {
                    let room = ((per - cur_span as f64).ceil() as i64).max(1);
                    let end = (start + room - 1).min(r.end);
                    cur.push(LineRange { start, end });
                    cur_span += end - start + 1;
                    start = end + 1;
                    if (cur_span as f64) >= per && groups.len() + 1 < k as usize {
                        groups.push(std::mem::take(&mut cur));
                        cur_span = 0;
                    }
                }
            }
            if !cur.is_empty() {
                groups.push(cur);
            }
            for g in groups {
                let gspan: i64 = g.iter().map(|r| r.end - r.start + 1).sum();
                let weight = (((gspan as f64 / span as f64) * f.lines as f64).round() as i64).max(1);
                units.push(Unit {
                    filename: f.filename.clone(),
                    ranges: Some(merge_ranges(&g)),
                    weight,
                    unsplittable: false,
                });
            }
        } else {
            units.push(Unit {
                filename: f.filename.clone(),
                ranges: None,
                weight: f.lines,
                unsplittable: logins.len() > 1 && (f.lines as f64) > target,
            });
        }
    }

    units.sort_by(|a, b| b.weight.cmp(&a.weight).then(a.filename.cmp(&b.filename)));
    let mut buckets: Vec<Bucket> = logins
        .iter()
        .map(|l| Bucket { login: l.clone(), lines: 0, parts: Vec::new() })
        .collect();
    for u in units {
        let mut idx = 0;
        for i in 1..buckets.len() {
            if buckets[i].lines < buckets[idx].lines {
                idx = i;
            }
        }
        buckets[idx].lines += u.weight;
        buckets[idx].parts.push(Part {
            filename: u.filename,
            ranges: u.ranges,
            unsplittable: u.unsplittable,
        });
    }
    buckets
}

fn parse_state(body: &str) -> Option<Value> {
    let i = body.find(MARK)?;
    let rest = &body[i + MARK.len()..];
    let j = rest.find(MARK_END)?;
    serde_json::from_str(rest[..j].trim()).ok()
}

/* ------------------------------ model ------------------------------ */

#[derive(Clone, Debug, Serialize)]
struct Reviewer {
    login: String,
    status: &'static str,
    lines: i64,
    parts: Vec<Part>,
}

#[derive(Clone, Debug, Serialize)]
struct Pull {
    number: i64,
    title: String,
    url: Option<String>,
    author: String,
    labels: Vec<String>,
    updated_at: Option<String>,
    updated_label: String,
    total: i64,
    file_count: usize,
    large: bool,
    needs: Option<String>,
    split: bool,
    status: &'static str,
    reviewers: Vec<Reviewer>,
    unsplittable: bool,
}

#[derive(Serialize)]
struct Summary {
    open: i64,
    approved: i64,
    changes: i64,
    review: i64,
    needs: i64,
    lines: i64,
    large: i64,
}

#[derive(Serialize)]
struct Workload {
    login: String,
    pending: i64,
    done: i64,
    prs: i64,
}

#[derive(Serialize)]
struct Model {
    repo: String,
    fetched_label: String,
    threshold: i64,
    summary: Summary,
    workload: Vec<Workload>,
    pulls: Vec<Pull>,
}

fn status_label(status: &str) -> &'static str {
    match status {
        "approved" => "พร้อม merge",
        "changes" => "ขอแก้ไข",
        "review" => "รอรีวิว",
        "needs" => "ยังไม่มี Reviewer",
        _ => "Draft",
    }
}

fn status_class(status: &str) -> &'static str {
    match status {
        "approved" => "ok",
        "changes" | "needs" => "bad",
        "review" => "warn",
        _ => "muted",
    }
}

fn status_rank(status: &str) -> i32 {
    match status {
        "changes" => 0,
        "approved" => 1,
        "review" => 2,
        "needs" => 3,
        _ => 4,
    }
}

fn review_status(review: Option<&(String, String)>) -> &'static str {
    match review {
        None => "pending",
        Some((state, _)) if state == "APPROVED" => "approved",
        Some((state, _)) if state == "CHANGES_REQUESTED" => "changes",
        Some(_) => "commented",
    }
}

fn review_label(status: &str) -> &'static str {
    match status {
        "approved" => "อนุมัติแล้ว",
        "changes" => "ขอแก้ไข",
        "commented" => "คอมเมนต์แล้ว",
        _ => "ยังไม่รีวิว",
    }
}

fn varr<'a>(v: &'a Value, key: &str) -> Vec<&'a Value> {
    v.get(key).and_then(|x| x.as_array()).map(|a| a.iter().collect()).unwrap_or_default()
}

fn vstr(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn vint(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|x| x.as_i64()).unwrap_or(0)
}

fn login_of(v: &Value) -> String {
    vstr(v, "login")
        .or_else(|| v.get("user").and_then(|u| u.get("login")).and_then(|x| x.as_str()).map(|s| s.to_string()))
        .unwrap_or_default()
}

fn build_model(snapshot: &Value, cfg: &Cfg) -> Model {
    let threshold = if cfg.large_pr_threshold > 0 { cfg.large_pr_threshold } else { DEFAULT_THRESHOLD };
    let now = now_epoch();
    let mut pulls: Vec<Pull> = Vec::new();

    for p in varr(snapshot, "pulls") {
        let author = norm(&vstr(p, "author").unwrap_or_default());
        let mut files: Vec<FileInfo> = Vec::new();
        for f in varr(p, "files") {
            let filename = vstr(f, "filename").unwrap_or_default();
            if cfg.exclude_patterns.iter().any(|g| glob_match(g, &filename)) {
                continue;
            }
            let lines = vint(f, "additions") + vint(f, "deletions");
            let ranges = hunks_from_patch(vstr(f, "patch").as_deref());
            files.push(FileInfo { filename, lines, ranges });
        }
        let total: i64 = files.iter().map(|f| f.lines).sum();
        let state = p.get("bot_state").filter(|v| !v.is_null()).cloned();
        let state_lines: Vec<(String, i64)> = state
            .as_ref()
            .and_then(|s| s.get("lines"))
            .and_then(|l| l.as_object())
            .map(|m| m.iter().map(|(k, v)| (norm(k), v.as_i64().unwrap_or(0))).collect())
            .unwrap_or_default();
        let state_lines_map: HashMap<String, i64> = state_lines.iter().cloned().collect();

        // รีวิวล่าสุดของแต่ละคน (ตัด PENDING และของ author)
        let mut reviews: HashMap<String, (String, String)> = HashMap::new();
        for r in varr(p, "reviews") {
            let login = norm(&login_of(r));
            if login.is_empty() || login == author {
                continue;
            }
            let st = vstr(r, "state").unwrap_or_default().to_uppercase();
            let at = match vstr(r, "submitted_at") {
                Some(a) => a,
                None => continue,
            };
            if st == "PENDING" {
                continue;
            }
            let replace = match reviews.get(&login) {
                None => true,
                Some(prev) => iso_epoch(&at).unwrap_or(0) > iso_epoch(&prev.1).unwrap_or(0),
            };
            if replace {
                reviews.insert(login, (st, at));
            }
        }

        let requested: Vec<String> = varr(p, "requested")
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .collect();

        // ชื่อที่ใช้แสดง (ตัวพิมพ์ตามต้นทาง)
        let mut display: HashMap<String, String> = HashMap::new();
        for l in &requested {
            display.entry(norm(l)).or_insert_with(|| l.clone());
        }
        for r in varr(p, "reviews") {
            let orig = login_of(r);
            if !orig.is_empty() {
                display.entry(norm(&orig)).or_insert(orig);
            }
        }
        for (k, _) in &state_lines {
            display.entry(k.clone()).or_insert_with(|| k.clone());
        }

        // logins เรียงตามลำดับ: requested → reviews → state.lines
        let mut logins: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for l in &requested {
            let n = norm(l);
            if !n.is_empty() && n != author && seen.insert(n.clone()) {
                logins.push(n);
            }
        }
        for r in varr(p, "reviews") {
            let n = norm(&login_of(r));
            if !n.is_empty() && n != author && seen.insert(n.clone()) {
                logins.push(n);
            }
        }
        for (n, _) in &state_lines {
            if !n.is_empty() && *n != author && seen.insert(n.clone()) {
                logins.push(n.clone());
            }
        }

        let state_split = state.as_ref().and_then(|s| s.get("split")).and_then(|v| v.as_bool()).unwrap_or(false);
        let split = state_split || (logins.len() >= 2 && total > threshold);
        let buckets = if logins.len() >= 2 { plan_scope(&files, &logins) } else { Vec::new() };

        let mut reviewers: Vec<Reviewer> = Vec::new();
        for login in &logins {
            let bucket = buckets.iter().find(|b| &b.login == login);
            let parts: Vec<Part> = if split && bucket.is_some() {
                bucket.map(|b| b.parts.clone()).unwrap_or_default()
            } else {
                files
                    .iter()
                    .map(|f| Part {
                        filename: f.filename.clone(),
                        ranges: None,
                        unsplittable: false,
                    })
                    .collect()
            };
            let lines = state_lines_map
                .get(login)
                .copied()
                .or_else(|| bucket.map(|b| b.lines))
                .unwrap_or(total);
            let status = review_status(reviews.get(login));
            reviewers.push(Reviewer {
                login: display.get(login).cloned().unwrap_or_else(|| login.clone()),
                status,
                lines,
                parts,
            });
        }

        let needs = state.as_ref().and_then(|s| s.get("needs")).and_then(|v| v.as_str()).map(|s| s.to_string());
        let has_changes = reviewers.iter().any(|r| r.status == "changes");
        let has_approved = reviewers.iter().any(|r| r.status == "approved");
        let draft = p.get("draft").and_then(|v| v.as_bool()).unwrap_or(false);
        let status = if draft {
            "draft"
        } else if has_changes {
            "changes"
        } else if has_approved {
            "approved"
        } else if reviewers.is_empty() || needs.is_some() {
            "needs"
        } else {
            "review"
        };

        let labels: Vec<String> = varr(p, "labels")
            .iter()
            .filter_map(|l| match l {
                Value::String(s) => Some(s.clone()),
                _ => vstr(l, "name"),
            })
            .filter(|s| !s.is_empty())
            .collect();

        let updated_at = vstr(p, "updated_at");
        let unsplittable = reviewers.iter().any(|r| r.parts.iter().any(|x| x.unsplittable));

        pulls.push(Pull {
            number: vint(p, "number"),
            title: vstr(p, "title").unwrap_or_default(),
            url: vstr(p, "url"),
            author: vstr(p, "author").unwrap_or_default(),
            labels,
            updated_label: rel_time(updated_at.as_deref(), now),
            updated_at,
            total,
            file_count: files.len(),
            large: total > threshold,
            needs,
            split: split && reviewers.len() >= 2,
            status,
            reviewers,
            unsplittable,
        });
    }

    pulls.sort_by(|a, b| {
        status_rank(a.status)
            .cmp(&status_rank(b.status))
            .then(iso_epoch(b.updated_at.as_deref().unwrap_or("")).unwrap_or(0).cmp(&iso_epoch(a.updated_at.as_deref().unwrap_or("")).unwrap_or(0)))
    });

    let count = |s: &str| pulls.iter().filter(|p| p.status == s).count() as i64;
    let summary = Summary {
        open: pulls.len() as i64,
        approved: count("approved"),
        changes: count("changes"),
        review: count("review"),
        needs: count("needs"),
        lines: pulls.iter().map(|p| p.total).sum(),
        large: pulls.iter().filter(|p| p.large).count() as i64,
    };

    let mut workload: Vec<Workload> = Vec::new();
    for p in &pulls {
        for r in &p.reviewers {
            let idx = match workload.iter().position(|w| w.login == r.login) {
                Some(i) => i,
                None => {
                    workload.push(Workload { login: r.login.clone(), pending: 0, done: 0, prs: 0 });
                    workload.len() - 1
                }
            };
            let w = &mut workload[idx];
            w.prs += 1;
            if r.status == "approved" {
                w.done += r.lines;
            } else {
                w.pending += r.lines;
            }
        }
    }
    workload.sort_by(|a, b| b.pending.cmp(&a.pending).then(b.done.cmp(&a.done)));

    Model {
        repo: vstr(snapshot, "repo").unwrap_or_else(|| "(ไม่ระบุ repo)".to_string()),
        fetched_label: rel_time(vstr(snapshot, "fetched_at").as_deref(), now),
        threshold,
        summary,
        workload,
        pulls,
    }
}

/* ------------------------------ render ------------------------------ */

fn format_ranges(ranges: &[LineRange]) -> String {
    ranges
        .iter()
        .map(|r| {
            if r.start == r.end {
                r.start.to_string()
            } else {
                format!("{}–{}", r.start, r.end)
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn reviewer_parts(r: &Reviewer) -> String {
    if r.parts.is_empty() {
        return "<div class=\"parts muted\">ไม่มีไฟล์ (ถูกยกเว้นทั้งหมด)</div>".to_string();
    }
    let mut items: Vec<String> = Vec::new();
    for p in r.parts.iter().take(6) {
        let range = match &p.ranges {
            Some(rs) if !rs.is_empty() => format!("บรรทัด {}", format_ranges(&merge_ranges(rs))),
            _ => format!("ทั้งไฟล์{}", if p.unsplittable { " · แยกช่วงไม่ได้" } else { "" }),
        };
        items.push(format!(
            "<li><code>{}</code> <span class=\"muted\">{}</span></li>",
            escape_html(&p.filename),
            escape_html(&range)
        ));
    }
    if r.parts.len() > 6 {
        items.push(format!("<li class=\"muted\">และอีก {} รายการ</li>", r.parts.len() - 6));
    }
    format!("<ul class=\"parts\">{}</ul>", items.join(""))
}

fn render_pr(p: &Pull) -> String {
    let reviewers = if p.reviewers.is_empty() {
        "<div class=\"rv muted\">ยังไม่มี Reviewer — บอทจะลองใหม่ทุกวัน หรือเลือกเองใน PR</div>".to_string()
    } else {
        p.reviewers
            .iter()
            .map(|r| {
                format!(
                    "<div class=\"rv\">\n        <div class=\"rv-head\"><b>@{}</b>\n          <span class=\"tag {}\">{}</span>\n          <span class=\"lines\">{} บรรทัด</span></div>\n        {}\n      </div>",
                    escape_html(&r.login),
                    r.status,
                    escape_html(review_label(r.status)),
                    r.lines,
                    reviewer_parts(r)
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let mut search_parts: Vec<String> = vec![p.number.to_string(), p.title.clone(), p.author.clone()];
    search_parts.extend(p.reviewers.iter().map(|r| r.login.clone()));
    search_parts.extend(p.labels.iter().cloned());
    let search = search_parts.join(" ").to_lowercase();

    let has_ranges = p.reviewers.iter().any(|r| r.parts.iter().any(|x| x.ranges.as_ref().map_or(false, |v| !v.is_empty())));
    let mut badges = String::new();
    badges.push_str(&format!(
        "<span class=\"badge {}\">{}</span>",
        status_class(p.status),
        escape_html(status_label(p.status))
    ));
    if p.large {
        badges.push_str("<span class=\"badge warn\">PR ใหญ่</span>");
    }
    if p.split {
        badges.push_str(&format!(
            "<span class=\"badge info\">{}</span>",
            if has_ranges { "แบ่งช่วงบรรทัด" } else { "แบ่งไฟล์" }
        ));
    }
    if p.needs.as_deref() == Some("needs_second_reviewer") {
        badges.push_str("<span class=\"badge bad\">ขาดคนที่สอง</span>");
    }
    if p.unsplittable {
        badges.push_str("<span class=\"badge bad\">มีไฟล์แยกช่วงไม่ได้</span>");
    }

    let labels = if p.labels.is_empty() {
        String::new()
    } else {
        format!(
            "<div class=\"labels\">{}</div>",
            p.labels
                .iter()
                .map(|l| format!("<span class=\"label\">{}</span>", escape_html(l)))
                .collect::<Vec<_>>()
                .join("")
        )
    };

    let title = match &p.url {
        Some(u) => format!(
            "<a class=\"title\" href=\"{}\" target=\"_blank\" rel=\"noreferrer\">{}</a>",
            escape_html(u),
            escape_html(&p.title)
        ),
        None => format!("<span class=\"title\">{}</span>", escape_html(&p.title)),
    };

    format!(
        "<article class=\"pr\" data-status=\"{}\" data-search=\"{}\">\n  <header class=\"pr-head\">\n    <span class=\"num\">#{}</span>\n    {}\n    {}\n  </header>\n  <div class=\"meta\">@{} · <b>{}</b> บรรทัด · {} ไฟล์ · อัปเดต {}</div>\n  {}\n  <div class=\"reviewers\">{}</div>\n</article>",
        p.status,
        escape_html(&search),
        p.number,
        title,
        badges,
        escape_html(&p.author),
        p.total,
        p.file_count,
        escape_html(&p.updated_label),
        labels,
        reviewers
    )
}

const HEAD_A: &str = r###"<!doctype html>
<html lang="th">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>Auto-review · "###;

const HEAD_B: &str = r###"</title>
<style>
:root{color-scheme:light dark;--bg:#ffffff;--fg:#1a1a1a;--muted:#5c5c5c;--line:#d4d4d4;--hover:#f5f5f5;--accent:#0969da;--mono:ui-monospace,SFMono-Regular,"SF Mono",Menlo,Consolas,"Liberation Mono",monospace}
@media (prefers-color-scheme:dark){:root{--bg:#0d0d0d;--fg:#e8e8e8;--muted:#a1a1a1;--line:#2c2c2c;--hover:#171717;--accent:#4493f8}}
*{box-sizing:border-box}
body{margin:0;width:100%;background:var(--bg);color:var(--fg);font:14px/1.55 -apple-system,"Segoe UI",Roboto,"Noto Sans Thai",sans-serif;padding:20px clamp(16px,2.5vw,48px) 40px}
a{color:var(--accent);text-decoration:none}
a:hover{text-decoration:underline}
h1{font-size:20px;font-weight:600;letter-spacing:-.01em;margin:0 0 2px;color:var(--accent)}
.top{display:flex;flex-wrap:wrap;gap:8px;align-items:baseline;justify-content:space-between;margin-bottom:18px}
.muted{color:var(--muted)}
.kpis{display:flex;flex-wrap:wrap;border:1px solid var(--line);border-radius:6px;overflow:hidden;margin-bottom:16px}
.kpi{flex:1 1 110px;padding:10px 14px;border-left:1px solid var(--line);min-width:96px}
.kpi:first-child{border-left:0}
.kpi-v{font-family:var(--mono);font-size:19px;font-weight:600;line-height:1.25}
.kpi-l{font-size:10.5px;letter-spacing:.06em;text-transform:uppercase;color:var(--muted);margin-top:2px}
.toolbar{display:flex;flex-wrap:wrap;gap:8px;align-items:center;margin-bottom:8px}
input[type=search]{flex:1 1 220px;min-width:160px;background:var(--bg);border:1px solid var(--line);color:var(--fg);border-radius:6px;padding:7px 10px;font-size:14px;font-family:inherit}
input[type=search]:focus{outline:none;border-color:var(--fg)}
.chips{display:flex;gap:2px;flex-wrap:wrap}
.chip{background:transparent;border:0;color:var(--muted);border-radius:6px;padding:5px 9px;font-size:12.5px;cursor:pointer;font-family:inherit}
.chip:hover{background:var(--hover);color:var(--fg)}
.chip[aria-pressed=true]{background:var(--fg);color:var(--bg);font-weight:600}
.pr{border-bottom:1px solid var(--line);padding:14px 0}
.pr-head{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.num{font-family:var(--mono);font-size:12px;color:var(--muted)}
.title{font-size:15px;font-weight:600;flex:1 1 220px;min-width:0;overflow-wrap:anywhere;color:var(--accent)}
.badge,.tag,.label{display:inline-flex;align-items:center;gap:5px;font-size:11px;border-radius:999px;padding:1px 8px;border:1px solid var(--line);color:var(--muted);white-space:nowrap;font-family:inherit}
.badge::before,.tag::before{content:"";width:6px;height:6px;border-radius:50%;background:currentColor}
.badge.ok,.tag.approved{color:var(--fg);border-color:var(--fg)}
.badge.bad,.tag.changes{color:var(--fg);border-style:dashed;border-color:var(--muted)}
.badge.warn,.tag.commented{color:var(--muted)}
.badge.info{border-style:dotted;color:var(--fg)}
.badge.muted,.tag.pending{color:var(--muted);border-style:dashed}
.tag.pending::before{background:transparent;box-shadow:inset 0 0 0 1px currentColor}
.meta{font-size:12.5px;color:var(--muted);margin:6px 0 0}
.meta b{font-family:var(--mono);font-weight:600;color:var(--fg)}
.labels{display:flex;gap:5px;flex-wrap:wrap;margin:6px 0 0}
.label{font-size:11px;color:var(--muted)}
.reviewers{display:grid;margin-top:8px}
.rv{border-top:1px solid var(--line);padding:8px 0}
.rv:first-child{border-top:0}
.rv-head{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:13px}
.rv-head b{font-weight:600}
.lines{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.parts{list-style:none;margin:6px 0 0;padding:0;font-size:12px}
.parts li{border-left:2px solid var(--line);padding:1px 0 1px 8px;margin:3px 0;overflow-wrap:anywhere}
code{font-family:var(--mono);font-size:11.5px;background:var(--hover);border-radius:4px;padding:1px 4px}
table{width:100%;border-collapse:collapse;font-size:12.5px;margin-top:8px}
th,td{text-align:left;padding:7px 8px;border-bottom:1px solid var(--line)}
th{font-size:10.5px;letter-spacing:.06em;text-transform:uppercase;color:var(--muted);font-weight:600}
td:not(:first-child){font-family:var(--mono);text-align:right}
h2{font-size:12px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;color:var(--accent);margin:28px 0 0;padding-bottom:6px;border-bottom:1px solid var(--line)}
footer{margin-top:24px;padding-top:12px;border-top:1px solid var(--line);font-size:11.5px;color:var(--muted)}
@media (max-width:560px){body{padding:16px 12px 32px}.kpi{flex-basis:50%;border-top:1px solid var(--line)}.kpi:nth-child(-n+2){border-top:0}.kpi:nth-child(odd){border-left:0}.title{flex-basis:100%}}
</style>
</head>
<body>
<div class="top">
  <div>
    <h1>Auto-review dashboard</h1>
    <div class="muted">"###;

const TOOLBAR: &str = r###"</div>
  </div>
</div>

<div class="toolbar">
  <input id="q" type="search" placeholder="ค้นหา: เลข PR, ชื่อ, ผู้เขียน, reviewer, label" autocomplete="off">
  <div class="chips">
    <button class="chip" data-filter="all" aria-pressed="true">ทั้งหมด</button>
    <button class="chip" data-filter="approved" aria-pressed="false">พร้อม merge</button>
    <button class="chip" data-filter="review" aria-pressed="false">รอรีวิว</button>
    <button class="chip" data-filter="changes" aria-pressed="false">ขอแก้ไข</button>
    <button class="chip" data-filter="needs" aria-pressed="false">ไม่มี Reviewer</button>
    <button class="chip" data-filter="draft" aria-pressed="false">Draft</button>
  </div>
</div>

"###;

const TABLE_HEAD: &str = r###"
<div id="empty" class="pr muted" style="display:none">ไม่พบ PR ที่ตรงกับตัวกรอง</div>

<h2>ภาระงานค้างต่อคน</h2>
<table>
  <thead><tr><th>Reviewer</th><th>PR</th><th>ค้าง (บรรทัด)</th><th>รีวิวแล้ว (บรรทัด)</th></tr></thead>
  <tbody>"###;

const TAIL: &str = r###"</tbody>
</table>

<footer>อ่านอย่างเดียว · สถานะรีวิวจาก GitHub review ล่าสุดของแต่ละคน · ขอบเขตบรรทัดคำนวณด้วย <code>planScope</code> ตัวเดียวกับบอท</footer>

<script>
(function () {
  var q = document.getElementById('q');
  var chips = Array.prototype.slice.call(document.querySelectorAll('.chip[data-filter]'));
  var cards = Array.prototype.slice.call(document.querySelectorAll('.pr[data-status]'));
  var filter = 'all';
  function apply() {
    var term = (q.value || '').trim().toLowerCase();
    var shown = 0;
    cards.forEach(function (c) {
      var okStatus = filter === 'all' || (c.getAttribute('data-status') || '') === filter;
      var okTerm = !term || (c.getAttribute('data-search') || '').indexOf(term) >= 0;
      var ok = okStatus && okTerm;
      c.style.display = ok ? '' : 'none';
      if (ok) shown++;
    });
    document.getElementById('empty').style.display = cards.length && !shown ? '' : 'none';
  }
  q.addEventListener('input', apply);
  chips.forEach(function (b) {
    b.addEventListener('click', function () {
      filter = b.getAttribute('data-filter');
      chips.forEach(function (x) { x.setAttribute('aria-pressed', String(x === b)); });
      apply();
    });
  });
})();
</script>
</body>
</html>
"###;

fn kpi(label: &str, value: i64, cls: &str) -> String {
    format!(
        "<div class=\"kpi {}\"><div class=\"kpi-v\">{}</div><div class=\"kpi-l\">{}</div></div>",
        cls, value, label
    )
}

fn render_html(model: &Model) -> String {
    let s = &model.summary;
    let mut html = String::with_capacity(24 * 1024);
    html.push_str(HEAD_A);
    html.push_str(&escape_html(&model.repo));
    html.push_str(HEAD_B);
    html.push_str(&format!(
        "{} · ข้อมูล {} · เกณฑ์ PR ใหญ่ {} บรรทัด",
        escape_html(&model.repo),
        escape_html(&model.fetched_label),
        model.threshold
    ));
    html.push_str("\n\n<section class=\"kpis\">\n  ");
    html.push_str(&kpi("PR เปิดอยู่", s.open, ""));
    html.push_str("\n  ");
    html.push_str(&kpi("พร้อม merge", s.approved, "ok"));
    html.push_str("\n  ");
    html.push_str(&kpi("ขอแก้ไข", s.changes, "bad"));
    html.push_str("\n  ");
    html.push_str(&kpi("รอรีวิว", s.review, "warn"));
    html.push_str("\n  ");
    html.push_str(&kpi("ยังไม่มี Reviewer", s.needs, "bad"));
    html.push_str("\n  ");
    html.push_str(&kpi("บรรทัดรวม", s.lines, "info"));
    html.push_str("\n  ");
    html.push_str(&kpi("PR ใหญ่", s.large, "warn"));
    html.push_str("\n</section>\n\n");
    html.push_str(TOOLBAR);

    if model.pulls.is_empty() {
        html.push_str("<div class=\"pr muted\">ยังไม่มี PR ที่เปิดอยู่</div>");
    } else {
        for p in &model.pulls {
            html.push_str(&render_pr(p));
            html.push('\n');
        }
    }

    html.push_str(TABLE_HEAD);
    if model.workload.is_empty() {
        html.push_str("<tr><td colspan=\"4\" class=\"muted\">ยังไม่มีงานในมือ</td></tr>");
    } else {
        for w in &model.workload {
            html.push_str(&format!(
                "<tr><td>@{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                escape_html(&w.login),
                w.prs,
                w.pending,
                w.done
            ));
        }
    }
    html.push_str(TAIL);
    html
}

/* ------------------------------ data ------------------------------ */

fn github_token() -> Option<String> {
    for k in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(k) {
            let t = v.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    use std::io::Write;
    let mut child = std::process::Command::new("git")
        .args(["credential", "fill"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.as_mut()?.write_all(b"protocol=https\nhost=github.com\n\n").ok()?;
    let out = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        if let Some(p) = line.strip_prefix("password=") {
            let t = p.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    None
}

fn api_get(path: &str, token: Option<&str>) -> Result<Value, String> {
    let url = format!("{API}{path}");
    let mut req = ureq::get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "auto-review-dashboard-rs")
        .timeout(Duration::from_secs(25));
    if let Some(t) = token {
        req = req.set("Authorization", &format!("Bearer {t}"));
    }
    match req.call() {
        Ok(res) => {
            let body = res.into_string().map_err(|e| e.to_string())?;
            serde_json::from_str(&body).map_err(|e| e.to_string())
        }
        Err(ureq::Error::Status(code, res)) => {
            let body = res.into_string().unwrap_or_default();
            let short: String = body.chars().take(200).collect();
            Err(format!("GitHub {code} {path}: {short}"))
        }
        Err(e) => Err(format!("{e}")),
    }
}

fn fetch_snapshot(repo: &str, max_pulls: usize) -> Result<Value, String> {
    let valid = {
        let parts: Vec<&str> = repo.split('/').collect();
        parts.len() == 2
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            })
    };
    if !valid {
        return Err("ต้องระบุ --repo owner/name (หรือตั้ง GITHUB_REPOSITORY)".to_string());
    }
    let token = github_token();
    let tk = token.as_deref();
    let path = format!("/repos/{repo}/pulls?state=open&sort=updated&direction=desc&per_page={max_pulls}");
    let pulls = api_get(&path, tk)?;
    let arr = pulls.as_array().cloned().unwrap_or_default();

    let mut out: Vec<Value> = Vec::new();
    for pr in &arr {
        let number = pr.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
        let files_path = format!("/repos/{repo}/pulls/{number}/files?per_page=100");
        let reviews_path = format!("/repos/{repo}/pulls/{number}/reviews?per_page=100");
        let comments_path = format!("/repos/{repo}/issues/{number}/comments?per_page=100");
        let (files, reviews, comments) = std::thread::scope(|s| {
            let f = s.spawn(|| api_get(&files_path, tk));
            let r = s.spawn(|| api_get(&reviews_path, tk));
            let c = s.spawn(|| api_get(&comments_path, tk));
            (
                f.join().unwrap_or_else(|_| Err("thread panic".to_string())),
                r.join().unwrap_or_else(|_| Err("thread panic".to_string())),
                c.join().unwrap_or_else(|_| Err("thread panic".to_string())),
            )
        });
        let files = files?;
        let reviews = reviews?;
        let comments = comments?;

        let mut best: Option<&Value> = None;
        let mut best_id = i64::MIN;
        for c in comments.as_array().map(|a| a.iter()).into_iter().flatten() {
            let login = c.get("user").and_then(|u| u.get("login")).and_then(|v| v.as_str()).unwrap_or("");
            let body = c.get("body").and_then(|v| v.as_str()).unwrap_or("");
            if login != BOT_LOGIN || !body.contains(MARK) {
                continue;
            }
            let id = c.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
            if id > best_id {
                best_id = id;
                best = Some(c);
            }
        }
        let bot_state = best.and_then(|c| c.get("body").and_then(|b| b.as_str())).and_then(parse_state);

        let files_out: Vec<Value> = files
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|f| {
                        json!({
                            "filename": f.get("filename").cloned().unwrap_or(Value::Null),
                            "additions": f.get("additions").cloned().unwrap_or(Value::from(0)),
                            "deletions": f.get("deletions").cloned().unwrap_or(Value::from(0)),
                            "patch": f.get("patch").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let reviews_out: Vec<Value> = reviews
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|r| {
                        json!({
                            "login": r.get("user").and_then(|u| u.get("login")).cloned().unwrap_or(Value::Null),
                            "state": r.get("state").cloned().unwrap_or(Value::Null),
                            "submitted_at": r.get("submitted_at").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let requested: Vec<Value> = pr
            .get("requested_reviewers")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|u| u.get("login").cloned().unwrap_or(Value::Null)).collect())
            .unwrap_or_default();
        let labels: Vec<Value> = pr
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().map(|l| l.get("name").cloned().unwrap_or(Value::Null)).collect())
            .unwrap_or_default();

        out.push(json!({
            "number": number,
            "title": pr.get("title").cloned().unwrap_or(Value::Null),
            "url": pr.get("html_url").cloned().unwrap_or(Value::Null),
            "author": pr.get("user").and_then(|u| u.get("login")).cloned().unwrap_or(Value::Null),
            "draft": pr.get("draft").cloned().unwrap_or(Value::Bool(false)),
            "created_at": pr.get("created_at").cloned().unwrap_or(Value::Null),
            "updated_at": pr.get("updated_at").cloned().unwrap_or(Value::Null),
            "labels": labels,
            "files": files_out,
            "reviews": reviews_out,
            "requested": requested,
            "bot_state": bot_state.unwrap_or(Value::Null),
        }));
    }

    Ok(json!({ "repo": repo, "fetched_at": iso_now(), "pulls": out }))
}

/* ------------------------------ CLI ------------------------------ */

#[derive(Clone, Debug)]
struct Args {
    repo: Option<String>,
    out: String,
    snapshot: Option<String>,
    serve: Option<u16>,
    host: String,
    web: Option<String>,
    max_pulls: usize,
    config: Option<String>,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            repo: std::env::var("GITHUB_REPOSITORY").ok(),
            out: "dashboard.html".to_string(),
            snapshot: None,
            serve: None,
            host: "127.0.0.1".to_string(),
            web: None,
            max_pulls: 30,
            config: None,
            help: false,
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (a.as_str(), None),
        };
        let next = |i: &mut usize| -> Option<String> {
            if let Some(v) = &inline {
                return Some(v.clone());
            }
            *i += 1;
            argv.get(*i).cloned()
        };
        match key {
            "--repo" | "-r" => args.repo = next(&mut i),
            "--out" | "-o" => {
                if let Some(v) = next(&mut i) {
                    args.out = v;
                }
            }
            "--snapshot" | "-s" => args.snapshot = next(&mut i),
            "--config" | "-c" => args.config = next(&mut i),
            "--max-pulls" => {
                if let Some(v) = next(&mut i) {
                    args.max_pulls = v.parse().unwrap_or(30);
                }
            }
            "--serve" => {
                if let Some(v) = &inline {
                    args.serve = Some(v.parse().unwrap_or(7788));
                } else if argv.get(i + 1).map_or(false, |n| n.chars().all(|c| c.is_ascii_digit())) {
                    i += 1;
                    args.serve = Some(argv[i].parse().unwrap_or(7788));
                } else {
                    args.serve = Some(7788);
                }
            }
            "--host" => {
                if let Some(v) = next(&mut i) {
                    args.host = v;
                }
            }
            "--web" => args.web = next(&mut i),
            "--help" | "-h" => args.help = true,
            _ => return Err(format!("ไม่รู้จัก flag: {a}")),
        }
        i += 1;
    }
    Ok(args)
}

fn print_help() {
    println!(
        "ใช้: dashboard-rs [options]
  --repo owner/name   repo ที่จะอ่าน (หรือ env GITHUB_REPOSITORY)
  --out file          ไฟล์ HTML ผลลัพธ์ (ค่าเริ่มต้น dashboard.html)
  --snapshot file     อ่านข้อมูลจาก JSON ที่บันทึกไว้ แทนการยิง GitHub
  --config file       config (ค่าเริ่มต้น .github/auto-review.yml)
  --max-pulls n       จำนวน PR ที่ดึง (ค่าเริ่มต้น 30)
  --serve [port]      เสิร์ฟที่ 127.0.0.1 (ค่าเริ่มต้น 7788) รีเฟรชข้อมูลทุก 30 วิ
  --host addr         bind address ของ --serve (127.0.0.1 หรือ 0.0.0.0)
  --web dir           โฟลเดอร์ frontend ที่ build แล้ว (ค่าเริ่มต้น dashboard-web/dist)
  token: env GH_TOKEN / GITHUB_TOKEN หรือ git credential ของ github.com"
    );
}

fn snapshot_from(args: &Args) -> Result<Value, String> {
    match &args.snapshot {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("อ่าน snapshot ไม่ได้: {e}"))?;
            serde_json::from_str(&raw).map_err(|e| format!("snapshot ไม่ใช่ JSON: {e}"))
        }
        None => {
            let repo = args.repo.clone().unwrap_or_default();
            fetch_snapshot(&repo, args.max_pulls)
        }
    }
}

fn build_page(args: &Args, cfg: &Cfg) -> Result<(String, Model), String> {
    let snapshot = snapshot_from(args)?;
    let model = build_model(&snapshot, cfg);
    let html = render_html(&model);
    Ok((html, model))
}

/* ------------------------------ serve ------------------------------ */

struct Cached {
    html: String,
    json: String,
    at: Instant,
}

struct AppState {
    args: Args,
    cfg: Cfg,
    cache: Mutex<Option<Cached>>,
    web_root: Option<PathBuf>,
}

/// คืนทั้ง HTML (สำหรับ `/`) และ JSON (สำหรับ `/api/data.json`) จาก cache เดียวกัน
async fn get_page(st: &Arc<AppState>) -> Result<(String, String), String> {
    {
        let cache = st.cache.lock().await;
        if let Some(c) = cache.as_ref() {
            if c.at.elapsed() < Duration::from_secs(30) {
                return Ok((c.html.clone(), c.json.clone()));
            }
        }
    }
    let args = st.args.clone();
    let cfg = st.cfg.clone();
    let res = tokio::task::spawn_blocking(move || {
        build_page(&args, &cfg).and_then(|(html, model)| {
            let json = serde_json::to_string(&model).map_err(|e| e.to_string())?;
            Ok((html, json))
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    let (html, json) = res?;
    let mut cache = st.cache.lock().await;
    *cache = Some(Cached { html: html.clone(), json: json.clone(), at: Instant::now() });
    Ok((html, json))
}

/* ---------- static frontend (Vite dist) ---------- */

/// หาโฟลเดอร์ dist ของ frontend: `--web` มาก่อน แล้วลอง path มาตรฐาน
fn resolve_web_root(explicit: Option<&str>) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match explicit {
        Some(p) => vec![PathBuf::from(p)],
        None => vec![
            PathBuf::from("dashboard-web/dist"),
            PathBuf::from("../dashboard-web/dist"),
            PathBuf::from("../../dashboard-web/dist"),
            PathBuf::from("dist"),
        ],
    };
    candidates.into_iter().find(|p| p.join("index.html").is_file())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "map" => "application/json; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn text_response(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        msg.to_string(),
    )
        .into_response()
}

async fn serve_file(path: &Path, immutable: bool) -> Response {
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let cache = if immutable { "public, max-age=31536000, immutable" } else { "no-store" };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, content_type(path)), (header::CACHE_CONTROL, cache)],
                bytes,
            )
                .into_response()
        }
        Err(_) => text_response(StatusCode::NOT_FOUND, "ไม่พบไฟล์"),
    }
}

/// `/` — ถ้ามี dist ให้เสิร์ฟ index.html ของ Vite, ถ้าไม่มีใช้ HTML ที่เรนเดอร์จาก server
async fn root(State(st): State<Arc<AppState>>) -> Response {
    if let Some(web) = st.web_root.as_ref() {
        return serve_file(&web.join("index.html"), false).await;
    }
    match get_page(&st).await {
        Ok((html, _)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            html,
        )
            .into_response(),
        Err(e) => text_response(StatusCode::BAD_GATEWAY, &format!("โหลดข้อมูลไม่สำเร็จ: {e}")),
    }
}

/// ไฟล์ static (assets, favicon) + SPA fallback ไป index.html
async fn files(State(st): State<Arc<AppState>>, uri: Uri) -> Response {
    let Some(web) = st.web_root.as_ref() else {
        return text_response(
            StatusCode::NOT_FOUND,
            "ยังไม่มี dashboard-web/dist — ใช้ / เพื่อดู HTML ที่เรนเดอร์จาก server",
        );
    };
    let rel = uri.path().trim_start_matches('/');
    if rel.split('/').any(|s| s == "..") {
        return text_response(StatusCode::BAD_REQUEST, "path ไม่ถูกต้อง");
    }
    let mut file = if rel.is_empty() { web.join("index.html") } else { web.join(rel) };
    if !file.is_file() {
        let has_ext = Path::new(rel).extension().is_some();
        if !has_ext && !rel.starts_with("assets/") {
            file = web.join("index.html");
        } else {
            return text_response(StatusCode::NOT_FOUND, "ไม่พบไฟล์");
        }
    }
    serve_file(&file, rel.starts_with("assets/")).await
}

async fn api_data(State(st): State<Arc<AppState>>) -> impl IntoResponse {
    match get_page(&st).await {
        Ok((_, json)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            json,
        ),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            format!("โหลดข้อมูลไม่สำเร็จ: {e}"),
        ),
    }
}

fn run_server(args: Args, cfg: Cfg) -> Result<(), String> {
    let port = args.serve.unwrap_or(7788);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move {
        let web_root = resolve_web_root(args.web.as_deref());
        let web_label = web_root.as_ref().map(|p| p.display().to_string());
        let state = Arc::new(AppState { args: args.clone(), cfg, cache: Mutex::new(None), web_root });
        if let Err(e) = get_page(&state).await {
            eprintln!("⚠️  โหลดข้อมูลรอบแรกไม่สำเร็จ: {e}");
        }
        let app = Router::new()
            .route("/api/data.json", get(api_data))
            .route("/", get(root))
            .fallback(get(files))
            .with_state(state);
        let bind = format!("{}:{}", args.host, port);
        let listener = tokio::net::TcpListener::bind(&bind).await.map_err(|e| format!("bind {bind} ไม่ได้: {e}"))?;
        match web_label {
            Some(dir) => println!(
                "dashboard: {}:{} — frontend จาก {dir} + API /api/data.json (รีเฟรช 30 วิ, Ctrl+C เพื่อหยุด)",
                args.host, port
            ),
            None => println!(
                "dashboard: {}:{} — HTML จาก server (ไม่พบ dashboard-web/dist) (Ctrl+C เพื่อหยุด)",
                args.host, port
            ),
        }
        axum::serve(listener, app).await.map_err(|e| e.to_string())
    })
}

/* ------------------------------ main ------------------------------ */

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("❌ {e}");
            std::process::exit(2);
        }
    };
    if args.help {
        print_help();
        return;
    }
    let (cfg, warning) = load_config(args.config.as_deref());
    if let Some(w) = warning {
        eprintln!("⚠️  {w}");
    }
    if args.serve.is_some() {
        if let Err(e) = run_server(args, cfg) {
            eprintln!("❌ {e}");
            std::process::exit(1);
        }
        return;
    }
    match build_page(&args, &cfg) {
        Ok((html, model)) => {
            let out = std::env::current_dir().map(|d| d.join(&args.out)).unwrap_or_else(|_| args.out.clone().into());
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = std::fs::write(&out, html) {
                eprintln!("❌ เขียนไฟล์ไม่ได้: {e}");
                std::process::exit(1);
            }
            let s = &model.summary;
            println!("✅ {}\n   {}: เปิด {} · พร้อม merge {} · รอรีวิว {} · ขอแก้ไข {} · ไม่มี reviewer {} · {} บรรทัด",
                out.display(), model.repo, s.open, s.approved, s.review, s.changes, s.needs, s.lines);
        }
        Err(e) => {
            eprintln!("❌ {e}");
            std::process::exit(1);
        }
    }
}

/* ------------------------------ tests ------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    fn pr(number: i64, files: Value, requested: Vec<&str>) -> Value {
        json!({
            "number": number,
            "title": "test",
            "author": "GitNonta",
            "files": files,
            "reviews": [],
            "requested": requested,
            "bot_state": Value::Null
        })
    }

    #[test]
    fn split_single_big_file_into_ranges() {
        let snap = json!({
            "repo": "o/r",
            "pulls": [pr(21, json!([{ "filename": "src/Huge.php", "additions": 700, "deletions": 0, "patch": "@@ -0,0 +1,700 @@\n+<?php" }]), vec!["Themynon2", "Nontawatpkru17"])]
        });
        let model = build_model(&snap, &Cfg::default());
        let p = &model.pulls[0];
        assert_eq!(p.total, 700);
        assert!(p.split);
        assert_eq!(p.reviewers.len(), 2);
        assert_eq!(format_ranges(&merge_ranges(&p.reviewers[0].parts[0].ranges.clone().unwrap())), "1–350");
        assert_eq!(format_ranges(&merge_ranges(&p.reviewers[1].parts[0].ranges.clone().unwrap())), "351–700");
    }

    #[test]
    fn exclude_lockfile_matches_glob() {
        let cfg = Cfg { large_pr_threshold: 600, exclude_patterns: vec!["**/*.lock".to_string(), "package-lock.json".to_string()] };
        assert!(glob_match("**/*.lock", "deep/dir/a.lock"));
        assert!(glob_match("package-lock.json", "package-lock.json"));
        assert!(!glob_match("**/*.lock", "src/a.php"));
        assert_eq!(cfg.large_pr_threshold, 600);
    }

    #[test]
    fn status_and_author_review_ignored() {
        let snap = json!({
            "repo": "o/r",
            "pulls": [{
                "number": 5, "title": "x", "author": "GitNonta",
                "files": [{ "filename": "a.ts", "additions": 10, "deletions": 0, "patch": "@@ -0,0 +1,10 @@" }],
                "requested": ["Themynon2"],
                "reviews": [
                    { "login": "GitNonta", "state": "APPROVED", "submitted_at": "2026-11-12T05:00:00Z" },
                    { "login": "Themynon2", "state": "CHANGES_REQUESTED", "submitted_at": "2026-11-12T06:00:00Z" }
                ],
                "bot_state": Value::Null
            }]
        });
        let model = build_model(&snap, &Cfg::default());
        assert_eq!(model.pulls[0].status, "changes");
        assert_eq!(model.pulls[0].reviewers.len(), 1);
    }

    #[test]
    fn escape_blocks_injection() {
        assert_eq!(escape_html("<img onerror=x>"), "&lt;img onerror=x&gt;");
        let snap = json!({
            "repo": "o/r",
            "pulls": [{
                "number": 1, "title": "<img src=x onerror=alert(1)>", "author": "a",
                "files": [{ "filename": "a.ts", "additions": 1, "deletions": 0, "patch": "@@ -0,0 +1 @@\n+x" }],
                "requested": ["b"], "reviews": [], "bot_state": Value::Null
            }]
        });
        let html = render_html(&build_model(&snap, &Cfg::default()));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img src=x"));
    }

    #[test]
    fn config_lite_reads_threshold_and_lists() {
        let cfg = read_config_lite("large_pr_threshold: 800\nexclude_patterns:\n  - '**/*.lock'\n  - dist/**\nteam: []\n");
        assert_eq!(cfg.large_pr_threshold, 800);
        assert_eq!(cfg.exclude_patterns, vec!["**/*.lock".to_string(), "dist/**".to_string()]);
    }

    #[test]
    fn static_content_types() {
        assert_eq!(content_type(Path::new("index.html")), "text/html; charset=utf-8");
        assert_eq!(content_type(Path::new("assets/index-abc.js")), "text/javascript; charset=utf-8");
        assert_eq!(content_type(Path::new("assets/index-abc.css")), "text/css; charset=utf-8");
        assert_eq!(content_type(Path::new("favicon.ico")), "image/x-icon");
        assert_eq!(content_type(Path::new("blob.bin")), "application/octet-stream");
        assert!(resolve_web_root(Some("/ไม่มีโฟลเดอร์นี้")).is_none());
    }

    #[test]
    fn rel_time_formats_thai() {
        let now = iso_epoch("2026-11-12T10:00:00Z").unwrap();
        assert_eq!(rel_time(Some("2026-11-12T09:59:30Z"), now), "เมื่อสักครู่");
        assert_eq!(rel_time(Some("2026-11-12T07:00:00Z"), now), "3 ชม.ที่แล้ว");
        assert_eq!(rel_time(Some("2026-11-10T10:00:00Z"), now), "2 วันที่แล้ว");
    }
}
