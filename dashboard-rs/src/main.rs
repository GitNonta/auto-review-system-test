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
use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::extract::Request;
use axum::middleware::Next;
use axum::routing::{get, post};
use axum::{extract::Query, extract::State, Json, Router};
use axum::http::HeaderMap as HttpHeaders;
use axum::http::HeaderValue as HttpHeaderValue;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

const MARK: &str = "<!-- auto-review-state:";
const MARK_END: &str = "-->";
const BOT_LOGIN: &str = "github-actions[bot]";
const API: &str = "https://api.github.com";
const DEFAULT_THRESHOLD: i64 = 600;

/// ประวัติ commit บน branch หลัก + PR ที่ merge แล้ว (พร้อมคนที่กด merge)
const HISTORY_QUERY: &str = r#"
query($owner:String!,$name:String!){
  repository(owner:$owner,name:$name){
    defaultBranchRef{
      target{
        ... on Commit{
          history(first:20){
            nodes{
              oid
              messageHeadline
              committedDate
              author{name user{login}}
              committer{name user{login}}
            }
          }
        }
      }
    }
    pullRequests(states:MERGED,first:20,orderBy:{field:UPDATED_AT,direction:DESC}){
      nodes{
        number
        title
        url
        mergedAt
        mergedBy{login}
        mergeCommit{oid}
        author{login}
        baseRefName
        headRefName
      }
    }
  }
}"#;

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
    patch: Option<String>,
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
struct ReviewEvent {
    state: String,
    label: &'static str,
    at: String,
    at_label: String,
    /// ข้อความคอมเมนต์ของรีวิว (ตัดช่องว่างหัวท้าย, ว่าง = None, สูงสุด 2000 ตัวอักษร)
    body: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct Reviewer {
    login: String,
    status: &'static str,
    lines: i64,
    parts: Vec<Part>,
    /// รีวิวล่าสุด (ISO) / ป้ายเวลาแบบไทย — None ถ้ายังไม่เคยรีวิว
    reviewed_at: Option<String>,
    reviewed_label: Option<String>,
    /// ประวัติรีวิวทั้งหมดของคนนี้ใน PR นี้ (ใหม่สุดก่อน)
    history: Vec<ReviewEvent>,
}

#[derive(Clone, Debug, Serialize)]
struct FileEntry {
    filename: String,
    lines: i64,
    /// unified diff ของไฟล์ (ใช้แสดง diff + วาง inline comment ตรงบรรทัด)
    patch: Option<String>,
}

/// คอมเมนต์รีวิวแบบ inline (ผูกกับไฟล์+บรรทัด)
#[derive(Clone, Debug, Serialize)]
struct InlineComment {
    filename: String,
    line: Option<i64>,
    side: Option<String>,
    login: String,
    body: String,
    at: String,
    at_label: String,
    reply_to: Option<String>,
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
    files: Vec<FileEntry>,
    inline: Vec<InlineComment>,
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

/// ป้ายของ event รีวิว (state แบบ GitHub: APPROVED / CHANGES_REQUESTED / ...)
fn review_event_label(state: &str) -> &'static str {
    match state {
        "APPROVED" => "อนุมัติแล้ว",
        "CHANGES_REQUESTED" => "ขอแก้ไข",
        "COMMENTED" => "คอมเมนต์แล้ว",
        "DISMISSED" => "ถูก dismiss",
        _ => "รีวิว",
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
            let patch = vstr(f, "patch");
            let ranges = hunks_from_patch(patch.as_deref());
            files.push(FileInfo { filename, lines, patch, ranges });
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

        // ประวัติรีวิวทุกครั้งของแต่ละคน (ตัด PENDING และของ author) — ใหม่สุดก่อน
        let mut histories: HashMap<String, Vec<ReviewEvent>> = HashMap::new();
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
            let at_label = rel_time(Some(&at), now);
            let body = vstr(r, "body")
                .map(|b| b.trim().to_string())
                .filter(|b| !b.is_empty())
                .map(|b| b.chars().take(2000).collect::<String>());
            histories.entry(login).or_default().push(ReviewEvent {
                label: review_event_label(&st),
                state: st,
                at,
                at_label,
                body,
            });
        }
        for events in histories.values_mut() {
            events.sort_by(|a, b| iso_epoch(&b.at).unwrap_or(0).cmp(&iso_epoch(&a.at).unwrap_or(0)));
        }
        // รีวิวล่าสุดของแต่ละคน (ใช้ตัดสินสถานะ)
        let reviews: HashMap<String, (String, String)> = histories
            .iter()
            .filter_map(|(login, events)| events.first().map(|e| (login.clone(), (e.state.clone(), e.at.clone()))))
            .collect();

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
            let history = histories.get(login).cloned().unwrap_or_default();
            let reviewed_at = history.first().map(|e| e.at.clone());
            let reviewed_label = history.first().map(|e| e.at_label.clone());
            reviewers.push(Reviewer {
                login: display.get(login).cloned().unwrap_or_else(|| login.clone()),
                status,
                lines,
                parts,
                reviewed_at,
                reviewed_label,
                history,
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

        let file_list: Vec<FileEntry> = files
            .iter()
            .map(|f| FileEntry { filename: f.filename.clone(), lines: f.lines, patch: f.patch.clone() })
            .collect();

        let mut inline: Vec<InlineComment> = varr(p, "review_comments")
            .iter()
            .filter_map(|c| {
                let body = vstr(c, "body").map(|b| b.trim().to_string()).filter(|b| !b.is_empty())?;
                let filename = vstr(c, "path").unwrap_or_default();
                if filename.is_empty() {
                    return None;
                }
                let at = vstr(c, "created_at")?;
                Some(InlineComment {
                    filename,
                    line: c.get("line").and_then(|v| v.as_i64()),
                    side: vstr(c, "side"),
                    login: login_of(c),
                    at_label: rel_time(Some(&at), now),
                    at,
                    body,
                    reply_to: c
                        .get("reply_to")
                        .and_then(|v| if v.is_null() { None } else { v.as_i64().map(|n| n.to_string()) }),
                })
            })
            .collect();
        inline.sort_by(|a, b| {
            a.filename
                .cmp(&b.filename)
                .then(a.line.unwrap_or(0).cmp(&b.line.unwrap_or(0)))
                .then(iso_epoch(&a.at).unwrap_or(0).cmp(&iso_epoch(&b.at).unwrap_or(0)))
        });

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
            files: file_list,
            inline,
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

.picker{display:flex;gap:10px;align-items:center;flex-wrap:wrap}
select{font-family:var(--mono);font-size:12.5px;background:var(--bg);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:6px 8px;max-width:min(420px,60vw)}
select:focus{outline:none;border-color:var(--fg)}
.who{background:transparent;border:0;padding:0;font:inherit;font-weight:600;color:var(--accent);cursor:pointer;font-family:inherit}
.who:hover{text-decoration:underline}
.person{display:flex;flex-wrap:wrap;gap:12px;align-items:baseline;border:1px solid var(--line);border-radius:6px;padding:10px 14px;margin:0 0 12px;background:var(--hover)}
.person b{font-family:var(--mono);font-weight:600}
.person .muted{margin-left:auto}
.person .close{background:transparent;border:1px solid var(--line);border-radius:6px;color:var(--muted);font-size:12px;padding:3px 9px;cursor:pointer;font-family:inherit}
.person .close:hover{color:var(--fg);border-color:var(--fg)}
.hint{color:var(--muted)}
tr[data-login]{cursor:pointer}
tr[data-login]:hover td{background:var(--hover)}

.back{background:transparent;border:0;color:var(--accent);font:inherit;font-size:12.5px;cursor:pointer;padding:0;margin:0 0 4px;font-family:inherit}
.back:hover{text-decoration:underline}
.d-title{font-size:20px;font-weight:600;letter-spacing:-.01em;margin:2px 0 2px;color:var(--accent);overflow-wrap:anywhere}
.detail{border-top:1px solid var(--line);padding-top:14px}
.d-top{display:flex;gap:6px;flex-wrap:wrap;align-items:center}
.files td:first-child{font-family:var(--mono);font-size:12px}
.files td:first-child code{background:transparent;padding:0}

.prs td:first-child{font-family:inherit}
.prs td:nth-child(2){text-align:left}
.prs .row-title{margin-left:6px;font-weight:500}
.file-list{list-style:none;margin:8px 0 0;padding:0;font-size:12.5px}
.file-list li{border-left:2px solid var(--line);padding:2px 0 2px 8px;margin:3px 0;overflow-wrap:anywhere}
.file-list code{background:transparent;padding:0}

.history{list-style:none;margin:6px 0 0;padding:0;font-size:12px}
.history li{display:flex;gap:8px;align-items:baseline;flex-wrap:wrap;border-left:2px solid var(--line);padding:2px 0 2px 8px;margin:3px 0}
.history a{font-family:var(--mono)}

.rv-body{flex-basis:100%;white-space:pre-wrap;font-size:12px;color:var(--muted);border-left:2px solid var(--line);padding:4px 0 2px 8px;margin:4px 0 0;max-height:14em;overflow:auto;overflow-wrap:anywhere}

.inline td:first-child{font-family:var(--mono);font-size:11.5px}
.inline td:last-child{white-space:nowrap}
.inline .rv-body{margin:0;border-left:0;padding:0;max-height:none}

.file-block{margin:10px 0 18px;border:1px solid var(--line);border-radius:6px;overflow:hidden}
.file-head{display:flex;gap:10px;align-items:baseline;padding:7px 10px;background:var(--hover);border-bottom:1px solid var(--line)}
.file-head code{background:transparent;padding:0}
.diff{width:100%;border-collapse:collapse;font-family:var(--mono);font-size:11.5px;margin:0}
.diff td{padding:0 8px;border-bottom:0;vertical-align:top}
.diff td.ln{width:1%;min-width:34px;text-align:right;color:var(--muted);user-select:none;padding-top:2px}
.diff td.code{white-space:pre-wrap;overflow-wrap:anywhere;text-align:left;padding-top:2px;padding-bottom:2px}
.diff td.code .sign{display:inline-block;width:1ch;color:var(--muted)}
.diff tr.hunk td.code{color:var(--muted);background:var(--hover);padding-top:3px;padding-bottom:3px}
.diff tr.add td.code{background:var(--hover)}
.diff tr.del td.code{color:var(--muted)}
.diff tr.cmt td.code{padding:6px 8px 8px}
.cmt-box{border:1px solid var(--line);border-radius:6px;padding:6px 8px;background:var(--bg)}
.cmt-box .rv-body{border-left:0;padding-left:0;margin-top:3px}

.review-form{margin:10px 0 0;border:1px solid var(--line);border-radius:6px;padding:12px}
.rf-actions{display:flex;gap:14px;flex-wrap:wrap;font-size:13px;margin-bottom:8px}
.rf-actions label{display:inline-flex;gap:6px;align-items:center;cursor:pointer}
.review-form textarea{width:100%;background:var(--bg);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:8px 10px;font:inherit;font-size:13px;resize:vertical}
.review-form textarea:focus{outline:none;border-color:var(--fg)}
.rf-row{display:flex;gap:12px;align-items:center;flex-wrap:wrap;margin-top:8px;font-size:12px}
.rf-submit{background:var(--fg);color:var(--bg);border:0;border-radius:6px;padding:7px 14px;font:inherit;font-size:13px;font-weight:600;cursor:pointer}
.rf-submit:hover{opacity:.85}

.merge-form{margin:10px 0 0;border:1px solid var(--line);border-radius:6px;padding:12px}
.mf-row{display:flex;gap:10px;align-items:center;flex-wrap:wrap;margin-bottom:8px;font-size:13px}
.mf-row:last-child{margin-bottom:0}
.mf-row label{min-width:120px;color:var(--muted)}
.mf-row select,.mf-row input{background:var(--bg);color:var(--fg);border:1px solid var(--line);border-radius:6px;padding:6px 8px;font:inherit;font-size:13px;min-width:0;flex:1 1 200px;max-width:420px}
.mf-row select:focus,.mf-row input:focus{outline:none;border-color:var(--fg)}
.mf-submit{background:var(--fg);color:var(--bg);border:0;border-radius:6px;padding:7px 14px;font:inherit;font-size:13px;font-weight:600;cursor:pointer}
.mf-submit:hover{opacity:.85}
.mf-submit.danger{background:#cf222e;color:#fff}
.mf-submit:disabled{opacity:.6;cursor:default}
.nav{display:flex;gap:4px;flex-wrap:wrap;align-items:center;margin:0 0 16px;padding-bottom:8px;border-bottom:1px solid var(--line)}
.nav a{font-size:13px;color:var(--muted);padding:5px 10px;border-radius:6px;display:inline-flex;align-items:center;gap:6px}
.nav a:hover{background:var(--hover);color:var(--fg);text-decoration:none}
.nav a[aria-current=page]{background:var(--fg);color:var(--bg);font-weight:600}
.nav .count{font-family:var(--mono);font-size:10.5px;background:var(--hover);color:var(--fg);border-radius:999px;padding:0 6px;line-height:1.6}
.nav a[aria-current=page] .count{background:var(--bg);color:var(--fg)}
.workload td.wl{text-align:left;font-family:inherit;white-space:nowrap}
.wbar{width:min(160px,100%);height:8px;display:inline-block;vertical-align:middle}
.wbar .track{fill:var(--line)}
.wbar .pend{fill:var(--muted);opacity:.6}
.wbar .done{fill:var(--accent)}
.key{display:inline-block;width:14px;height:8px;border-radius:2px;vertical-align:middle;margin:0 3px 0 6px}
.key.pend{background:var(--muted);opacity:.6}
.key.done{background:var(--accent)}
/* สีโค้ดใน diff: เพิ่ม/ลบ + ไฮไลต์พื้นฐาน */
:root{--add-bg:#e6ffec;--del-bg:#ffebe9;--add-fg:#1a7f37;--del-fg:#cf222e;--syn-kw:#cf222e;--syn-str:#0a3069;--syn-com:#6e7781;--syn-num:#0550ae;--syn-type:#8250df}
@media (prefers-color-scheme:dark){:root{--add-bg:#12261e;--del-bg:#2d1214;--add-fg:#3fb950;--del-fg:#f85149;--syn-kw:#ff7b72;--syn-str:#a5d6ff;--syn-com:#8b949e;--syn-num:#79c0ff;--syn-type:#d2a8ff}}
.diff tr.add td.code{background:var(--add-bg)}
.diff tr.del td.code{background:var(--del-bg);color:var(--fg)}
.diff tr.add td.ln{background:var(--add-bg);color:var(--add-fg)}
.diff tr.del td.ln{background:var(--del-bg);color:var(--del-fg)}
.diff tr.add td.code .sign{color:var(--add-fg)}
.diff tr.del td.code .sign{color:var(--del-fg)}
.syn-k{color:var(--syn-kw)}
.syn-s{color:var(--syn-str)}
.syn-c{color:var(--syn-com);font-style:italic}
.syn-n{color:var(--syn-num)}
.wl-tools{display:flex;flex-wrap:wrap;gap:12px;align-items:center;margin:10px 0 0;font-size:12.5px;color:var(--muted)}
.wl-tools label{display:inline-flex;gap:6px;align-items:center;cursor:pointer}
.wl-tools input[type=checkbox]{accent-color:var(--accent);margin:0}
.wl-tools select{font-size:12px;padding:4px 6px;max-width:190px}
#wl-count{font-family:var(--mono);font-size:11.5px}
.table-wrap{max-width:100%;overflow-x:auto}
.syn-t{color:var(--syn-type)}
@media (max-width:560px){
  .wl-tools{gap:8px 10px;font-size:12px}
  .workload th:nth-child(2),.workload td:nth-child(2),.workload th:nth-child(4),.workload td:nth-child(4){display:none}
  .workload th,.workload td{padding:6px 5px}
  .workload td.wl{white-space:normal}
  .wbar{width:110px}
  .diff td.code{font-size:10.5px}
  .diff td.ln{min-width:26px;padding:0 4px}
  .nav a{padding:5px 7px;font-size:12.5px}
  .mf-row label{min-width:auto}
  .mf-row select,.mf-row input{flex-basis:100%}
}
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

/// เรียก GitHub GraphQL (ใช้ตัวเดียวกับ REST token)
fn api_graphql(query: &str, vars: Value, token: &str) -> Result<Value, String> {
    let body = json!({ "query": query, "variables": vars }).to_string();
    let req = ureq::post("https://api.github.com/graphql")
        .set("Accept", "application/vnd.github+json")
        .set("Content-Type", "application/json")
        .set("User-Agent", "auto-review-dashboard-rs")
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30));
    match req.send_string(&body) {
        Ok(res) => {
            let text = res.into_string().map_err(|e| e.to_string())?;
            let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if let Some(errs) = v.get("errors").and_then(|e| e.as_array()) {
                if !errs.is_empty() {
                    let msg = errs[0].get("message").and_then(|m| m.as_str()).unwrap_or("GraphQL error");
                    return Err(format!("GitHub GraphQL: {msg}"));
                }
            }
            Ok(v.get("data").cloned().unwrap_or(Value::Null))
        }
        Err(ureq::Error::Status(code, res)) => {
            let body = res.into_string().unwrap_or_default();
            let short: String = body.chars().take(200).collect();
            Err(format!("GitHub {code} /graphql: {short}"))
        }
        Err(e) => Err(format!("{e}")),
    }
}

struct PrMeta {
    number: i64,
    title: Value,
    url: Value,
    author: Value,
    draft: Value,
    created_at: Value,
    updated_at: Value,
    labels: Vec<Value>,
    requested: Vec<Value>,
}

fn pr_meta(pr: &Value) -> PrMeta {
    let requested = pr
        .get("requested_reviewers")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|u| u.get("login").cloned().unwrap_or(Value::Null)).collect())
        .unwrap_or_default();
    let labels = pr
        .get("labels")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|l| l.get("name").cloned().unwrap_or(Value::Null)).collect())
        .unwrap_or_default();
    PrMeta {
        number: pr.get("number").and_then(|v| v.as_i64()).unwrap_or(0),
        title: pr.get("title").cloned().unwrap_or(Value::Null),
        url: pr.get("html_url").cloned().unwrap_or(Value::Null),
        author: pr.get("user").and_then(|u| u.get("login")).cloned().unwrap_or(Value::Null),
        draft: pr.get("draft").cloned().unwrap_or(Value::Bool(false)),
        created_at: pr.get("created_at").cloned().unwrap_or(Value::Null),
        updated_at: pr.get("updated_at").cloned().unwrap_or(Value::Null),
        labels,
        requested,
    }
}

const GRAPHQL_QUERY: &str = r#"query($owner: String!, $name: String!, $first: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: $first, orderBy: {field: UPDATED_AT, direction: DESC}) {
      nodes {
        number
        title
        url
        isDraft
        createdAt
        updatedAt
        author { login }
        labels(first: 50) { nodes { name } }
        reviewRequests(first: 50) { nodes { requestedReviewer { ... on User { login } } } }
        files(first: 100) { nodes { path additions deletions } }
        reviews(first: 100) { nodes { state submittedAt body author { login } } }
        comments(first: 100) { nodes { body createdAt author { login } } }
        reviewThreads(first: 50) {
          nodes { comments(first: 50) { nodes { path line originalLine body createdAt author { login } replyTo { id } } } }
        }
      }
    }
  }
}"#;

/// POST ไป GitHub GraphQL API
fn api_post_graphql(payload: &Value, token: &str) -> Result<Value, String> {
    let body = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let res = ureq::post("https://api.github.com/graphql")
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "auto-review-dashboard-rs")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(30))
        .send_string(&body);
    match res {
        Ok(r) => {
            let text = r.into_string().map_err(|e| e.to_string())?;
            serde_json::from_str(&text).map_err(|e| e.to_string())
        }
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            Err(format!("GraphQL {code}: {}", text.chars().take(200).collect::<String>()))
        }
        Err(e) => Err(format!("{e}")),
    }
}

/// แปลง node ของ GraphQL เป็น snapshot ต่อ PR (รูปเดียวกับที่ได้จาก REST)
fn graphql_pr_to_snapshot(n: &Value) -> Value {
    let nodes = |path: &str| n.pointer(path).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let labels: Vec<Value> = nodes("/labels/nodes").iter().filter_map(|l| l.get("name").cloned()).collect();
    let requested: Vec<Value> = nodes("/reviewRequests/nodes")
        .iter()
        .filter_map(|r| r.pointer("/requestedReviewer/login").cloned())
        .collect();
    let files: Vec<Value> = nodes("/files/nodes")
        .iter()
        .map(|f| {
            json!({
                "filename": f.get("path").cloned().unwrap_or(Value::Null),
                "additions": f.get("additions").cloned().unwrap_or(Value::from(0)),
                "deletions": f.get("deletions").cloned().unwrap_or(Value::from(0)),
                "patch": Value::Null,
            })
        })
        .collect();
    let reviews: Vec<Value> = nodes("/reviews/nodes")
        .iter()
        .map(|r| {
            json!({
                "login": r.pointer("/author/login").cloned().unwrap_or(Value::Null),
                "state": r.get("state").cloned().unwrap_or(Value::Null),
                "submitted_at": r.get("submittedAt").cloned().unwrap_or(Value::Null),
                "body": r.get("body").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let mut inline: Vec<Value> = Vec::new();
    for thread in nodes("/reviewThreads/nodes") {
        for c in thread
            .pointer("/comments/nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            inline.push(json!({
                "login": c.pointer("/author/login").cloned().unwrap_or(Value::Null),
                "path": c.get("path").cloned().unwrap_or(Value::Null),
                "line": c.get("line").cloned().or_else(|| c.get("originalLine").cloned()).unwrap_or(Value::Null),
                "side": Value::Null,
                "body": c.get("body").cloned().unwrap_or(Value::Null),
                "created_at": c.get("createdAt").cloned().unwrap_or(Value::Null),
                "reply_to": c.pointer("/replyTo/id").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    // bot_state = คอมเมนต์ล่าสุดของบอทที่มี marker
    let mut best_at = String::new();
    let mut bot_body: Option<String> = None;
    for c in nodes("/comments/nodes") {
        let login = c.pointer("/author/login").and_then(|v| v.as_str()).unwrap_or("");
        let body = c.get("body").and_then(|v| v.as_str()).unwrap_or("");
        if login != BOT_LOGIN || !body.contains(MARK) {
            continue;
        }
        let at = c.get("createdAt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if bot_body.is_none() || at > best_at {
            best_at = at;
            bot_body = Some(body.to_string());
        }
    }
    let bot_state = bot_body.as_deref().and_then(parse_state);
    json!({
        "number": n.get("number").cloned().unwrap_or(Value::Null),
        "title": n.get("title").cloned().unwrap_or(Value::Null),
        "url": n.get("url").cloned().unwrap_or(Value::Null),
        "author": n.pointer("/author/login").cloned().unwrap_or(Value::Null),
        "draft": n.get("isDraft").cloned().unwrap_or(Value::Bool(false)),
        "created_at": n.get("createdAt").cloned().unwrap_or(Value::Null),
        "updated_at": n.get("updatedAt").cloned().unwrap_or(Value::Null),
        "labels": labels,
        "files": files,
        "reviews": reviews,
        "review_comments": inline,
        "requested": requested,
        "bot_state": bot_state.unwrap_or(Value::Null),
    })
}

/// ดึง snapshot ผ่าน GraphQL (1 request ต่อ repo) + patch ผ่าน REST เฉพาะ PR ที่เปลี่ยน
fn fetch_graphql_repo(
    repo: &str,
    max_pulls: usize,
    token: &str,
    prev: &HashMap<String, (String, Value)>,
) -> Result<(Value, HashMap<String, (String, Value)>), String> {
    let mut parts = repo.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    if owner.is_empty() || name.is_empty() {
        return Err("repo ต้องเป็น owner/name".to_string());
    }
    let payload = json!({
        "query": GRAPHQL_QUERY,
        "variables": { "owner": owner, "name": name, "first": max_pulls }
    });
    let raw = api_post_graphql(&payload, token)?;
    if let Some(errors) = raw.get("errors") {
        return Err(format!("errors: {}", errors));
    }
    let nodes = raw
        .pointer("/data/repository/pullRequests/nodes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "GraphQL ไม่มี pullRequests".to_string())?;

    let mut slots: Vec<Option<Value>> = vec![None; nodes.len()];
    let mut next: HashMap<String, (String, Value)> = HashMap::new();
    let mut todo: Vec<(usize, i64, Value)> = Vec::new();
    for (i, n) in nodes.iter().enumerate() {
        let number = n.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
        let updated = n.get("updatedAt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let key = number.to_string();
        if let Some((cached_at, value)) = prev.get(&key) {
            if cached_at == &updated {
                slots[i] = Some(value.clone());
                next.insert(key, (updated, value.clone()));
                continue;
            }
        }
        todo.push((i, number, graphql_pr_to_snapshot(n)));
    }

    let changed = todo.len();
    for (i, number, pr) in todo {
        let updated = pr.get("updated_at").and_then(|x| x.as_str()).unwrap_or("").to_string();
        next.insert(number.to_string(), (updated, pr.clone()));
        slots[i] = Some(pr);
    }

    let mut pulls: Vec<Value> = Vec::with_capacity(slots.len());
    for slot in slots {
        if let Some(v) = slot {
            pulls.push(v);
        }
    }
    println!(
        "dashboard: (GraphQL) {} PR · ใช้ cache {} · เปลี่ยน {}",
        nodes.len(),
        nodes.len() - changed,
        changed
    );
    Ok((json!({ "repo": repo, "fetched_at": iso_now(), "pulls": pulls }), next))
}

/// ดึงรายละเอียดของ PR หนึ่งตัว: files / reviews / issue comments / review comments (ขนานกัน)
fn fetch_one_pr(repo: &str, meta: &PrMeta, tk: Option<&str>) -> Result<Value, String> {
    let number = meta.number;
    let files_path = format!("/repos/{repo}/pulls/{number}/files?per_page=100");
    let reviews_path = format!("/repos/{repo}/pulls/{number}/reviews?per_page=100");
    let comments_path = format!("/repos/{repo}/issues/{number}/comments?per_page=100");
    let review_comments_path = format!("/repos/{repo}/pulls/{number}/comments?per_page=100");
    let (files, reviews, comments, review_comments) = std::thread::scope(|s| {
        let f = s.spawn(|| api_get(&files_path, tk));
        let r = s.spawn(|| api_get(&reviews_path, tk));
        let c = s.spawn(|| api_get(&comments_path, tk));
        let rc = s.spawn(|| api_get(&review_comments_path, tk));
        (
            f.join().unwrap_or_else(|_| Err("thread panic".to_string())),
            r.join().unwrap_or_else(|_| Err("thread panic".to_string())),
            c.join().unwrap_or_else(|_| Err("thread panic".to_string())),
            rc.join().unwrap_or_else(|_| Err("thread panic".to_string())),
        )
    });
    let files = files?;
    let reviews = reviews?;
    let comments = comments?;
    let review_comments = review_comments.unwrap_or_else(|_| Value::Array(Vec::new()));

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
                        "body": r.get("body").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let inline_out: Vec<Value> = review_comments
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| {
                    json!({
                        "login": c.get("user").and_then(|u| u.get("login")).cloned().unwrap_or(Value::Null),
                        "path": c.get("path").cloned().unwrap_or(Value::Null),
                        "line": c.get("line").or_else(|| c.get("original_line")).cloned().unwrap_or(Value::Null),
                        "side": c.get("side").cloned().unwrap_or(Value::Null),
                        "body": c.get("body").cloned().unwrap_or(Value::Null),
                        "created_at": c.get("created_at").cloned().unwrap_or(Value::Null),
                        "reply_to": c.get("in_reply_to_id").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(json!({
        "number": meta.number,
        "title": meta.title,
        "url": meta.url,
        "author": meta.author,
        "draft": meta.draft,
        "created_at": meta.created_at,
        "updated_at": meta.updated_at,
        "labels": meta.labels,
        "files": files_out,
        "reviews": reviews_out,
        "review_comments": inline_out,
        "requested": meta.requested,
        "bot_state": bot_state.unwrap_or(Value::Null),
    }))
}

/// ดึง snapshot ของ repo — ใช้ cache ต่อ PR (key = เลข PR) เมื่อ `updated_at` ไม่เปลี่ยน
fn fetch_snapshot(
    repo: &str,
    max_pulls: usize,
    prev: &HashMap<String, (String, Value)>,
) -> Result<(Value, HashMap<String, (String, Value)>), String> {
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
    if let Some(token) = tk {
        match fetch_graphql_repo(repo, max_pulls, token, prev) {
            Ok(res) => return Ok(res),
            Err(e) => eprintln!("⚠️  GraphQL ใช้ไม่ได้ ({e}) — กลับไปใช้ REST"),
        }
    }
    let path = format!("/repos/{repo}/pulls?state=open&sort=updated&direction=desc&per_page={max_pulls}");
    let pulls = api_get(&path, tk)?;
    let arr = pulls.as_array().cloned().unwrap_or_default();
    let metas: Vec<PrMeta> = arr.iter().map(pr_meta).collect();

    let mut slots: Vec<Option<Value>> = vec![None; metas.len()];
    let mut next: HashMap<String, (String, Value)> = HashMap::new();
    let mut todo: Vec<usize> = Vec::new();
    for (i, meta) in metas.iter().enumerate() {
        let key = meta.number.to_string();
        let updated = meta.updated_at.as_str().unwrap_or("").to_string();
        if let Some((cached_at, value)) = prev.get(&key) {
            if cached_at == &updated {
                slots[i] = Some(value.clone());
                next.insert(key, (updated, value.clone()));
                continue;
            }
        }
        todo.push(i);
    }

    // ดึงเฉพาะ PR ที่เปลี่ยน — แบ่งเป็นก้อนแล้วยิงพร้อมกัน (สูงสุด 6 เธรด)
    const WORKERS: usize = 6;
    let chunk = (todo.len() + WORKERS - 1) / WORKERS.max(1);
    let mut results: Vec<(usize, Result<Value, String>)> = Vec::new();
    if !todo.is_empty() {
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for part in todo.chunks(chunk.max(1)) {
                handle_chunk(&mut handles, s, repo, &metas, part, tk);
            }
            for h in handles {
                results.extend(h.join().unwrap_or_default());
            }
        });
    }

    fn handle_chunk<'scope, 'env, 'a>(
        handles: &mut Vec<std::thread::ScopedJoinHandle<'scope, Vec<(usize, Result<Value, String>)>>>,
        s: &'scope std::thread::Scope<'scope, 'env>,
        repo: &'env str,
        metas: &'env [PrMeta],
        part: &'env [usize],
        tk: Option<&'env str>,
    ) {
        handles.push(s.spawn(move || part.iter().map(|&i| (i, fetch_one_pr(repo, &metas[i], tk))).collect()));
    }

    for (i, value) in results {
        match value {
            Ok(v) => {
                let updated = metas[i].updated_at.as_str().unwrap_or("").to_string();
                next.insert(metas[i].number.to_string(), (updated, v.clone()));
                slots[i] = Some(v);
            }
            Err(e) => eprintln!("⚠️  PR #{}: {e}", metas[i].number),
        }
    }

    let mut with_pulls: Vec<Value> = Vec::with_capacity(slots.len());
    for slot in slots {
        if let Some(v) = slot {
            with_pulls.push(v);
        }
    }
    println!(
        "dashboard: {} PR · ใช้ cache {} · ดึงใหม่ {}",
        metas.len(),
        metas.len() - todo.len(),
        todo.len()
    );
    Ok((json!({ "repo": repo, "fetched_at": iso_now(), "pulls": with_pulls }), next))
}

/* ------------------------------ CLI ------------------------------ */

#[derive(Clone, Debug)]
struct Args {
    repo: Option<String>,
    repos: Vec<String>,
    allow_write: bool,
    oauth_client_id: Option<String>,
    oauth_client_secret: Option<String>,
    public_url: Option<String>,
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
            repos: std::env::var("DASHBOARD_REPOS").ok().map(|s| parse_repos(&s)).unwrap_or_default(),
            allow_write: std::env::var("DASHBOARD_ALLOW_WRITE").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false),
            oauth_client_id: std::env::var("GITHUB_CLIENT_ID").ok(),
            oauth_client_secret: std::env::var("GITHUB_CLIENT_SECRET").ok(),
            public_url: std::env::var("DASHBOARD_PUBLIC_URL").ok(),
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
            "--allow-write" => args.allow_write = true,
            "--oauth-client-id" => args.oauth_client_id = next(&mut i),
            "--oauth-client-secret" => args.oauth_client_secret = next(&mut i),
            "--public-url" => args.public_url = next(&mut i),
            "--repos" => {
                if let Some(v) = next(&mut i) {
                    args.repos = parse_repos(&v);
                }
            }
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
  --repos a/b,c/d     หลายโปรเจคให้เลือกในหน้าเว็บ (คั่นด้วย , หรือ env DASHBOARD_REPOS)
  --allow-write       เปิดให้ approve / ขอแก้ไข / คอมเมนต์จากหน้าเว็บ (ปิดเป็นค่าเริ่มต้น)
  --oauth-client-id / --oauth-client-secret / --public-url
                      เปิด GitHub OAuth login (หรือ env GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET / DASHBOARD_PUBLIC_URL)
                      callback URL = <public-url>/auth/callback
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

/// แยก `owner/name` หลายโปรเจค (คั่นด้วย , หรือ whitespace) — ตัดค่าที่รูปแบบไม่ถูกต้องทิ้ง
fn parse_repos(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace())
        .map(|x| x.trim().to_string())
        .filter(|x| {
            let mut it = x.split('/');
            matches!((it.next(), it.next(), it.next()), (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty())
        })
        .collect()
}

/// รายการโปรเจคให้เลือก: `--repos` มาก่อน แล้วค่อย fallback เป็น `--repo`
fn repo_list(args: &Args) -> Vec<String> {
    if !args.repos.is_empty() {
        return args.repos.clone();
    }
    args.repo.clone().filter(|r| r.contains('/')).into_iter().collect()
}

fn snapshot_from(
    args: &Args,
    repo: &str,
    prev: &HashMap<String, (String, Value)>,
) -> Result<(Value, HashMap<String, (String, Value)>), String> {
    match &args.snapshot {
        Some(path) => {
            let raw = std::fs::read_to_string(path).map_err(|e| format!("อ่าน snapshot ไม่ได้: {e}"))?;
            let value = serde_json::from_str(&raw).map_err(|e| format!("snapshot ไม่ใช่ JSON: {e}"))?;
            Ok((value, HashMap::new()))
        }
        None => fetch_snapshot(repo, args.max_pulls, prev),
    }
}

fn build_page(args: &Args, cfg: &Cfg, repo: &str) -> Result<(String, Model), String> {
    let (snapshot, _) = snapshot_from(args, repo, &HashMap::new())?;
    let model = build_model(&snapshot, cfg);
    let html = render_html(&model);
    Ok((html, model))
}

/// เหมือน build_page แต่คืน PR cache ที่อัปเดตแล้ว (ใช้ในโหมด serve)
fn build_page_cached(
    args: &Args,
    cfg: &Cfg,
    repo: &str,
    prev: &HashMap<String, (String, Value)>,
) -> Result<(String, Model, HashMap<String, (String, Value)>), String> {
    let (snapshot, prs) = snapshot_from(args, repo, prev)?;
    let model = build_model(&snapshot, cfg);
    let html = render_html(&model);
    Ok((html, model, prs))
}

/* ------------------------------ serve ------------------------------ */

const CACHE_TTL: Duration = Duration::from_secs(10);

/// FNV-1a 64-bit — ใช้เป็น version ของข้อมูล (เท่าเดิม = client ไม่ต้องดึงใหม่)
fn content_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[derive(Clone)]
struct Cached {
    html: String,
    json: String,
    at: Instant,
    version: String,
}

struct AppState {
    args: Args,
    cfg: Cfg,
    repos: Vec<String>,
    cache: Mutex<HashMap<String, Cached>>,
    /// snapshot ต่อ PR: key = เลข PR, value = (updated_at, snapshot)
    pr_cache: Mutex<HashMap<String, (String, Value)>>,
    /// กัน refresh ซ้ำซ้อนต่อ repo
    refreshing: Mutex<HashSet<String>>,
    /// ไฟล์+patch ต่อ PR: key = "repo#pr" (โหลดเมื่อเปิดหน้า detail)
    diff_cache: Mutex<HashMap<String, (Instant, Vec<Value>)>>,
    /// commit ของ PR: key = "repo#pr"
    commits_cache: Mutex<HashMap<String, (Instant, Vec<Value>)>>,
    /// ประวัติ commit/merge ต่อ repo
    history_cache: Mutex<HashMap<String, (Instant, Value)>>,
    /// รายละเอียด PR เดียว (เปิดจากประวัติ รวม PR ที่ merge แล้ว)
    pr_detail_cache: Mutex<HashMap<String, (Instant, Value)>>,
    /// เปิดให้เขียนรีวิวกลับ GitHub (ต้องสั่ง --allow-write)
    allow_write: bool,
    /// login ของบัญชี token (ใช้ซ่อนฟอร์มรีวิวเมื่อ PR เป็นของตัวเอง)
    viewer: Option<String>,
    /// ตั้งค่า GitHub OAuth (None = ใช้ token ฝั่ง server แบบเดิม)
    auth: Option<AuthConfig>,
    web_root: Option<PathBuf>,
}

/// โปรเจคเริ่มต้น (ตัวแรกในลิสต์) — สตริงว่างถ้ายังไม่ได้ตั้งค่า
fn default_repo(st: &AppState) -> String {
    st.repos.first().cloned().unwrap_or_default()
}

/// สร้างหน้าใหม่ของ repo แล้วเก็บลง cache (คืน html, json, version)
async fn refresh(st: &Arc<AppState>, repo: &str) -> Result<(String, String, String), String> {
    let args = st.args.clone();
    let cfg = st.cfg.clone();
    let repo_owned = repo.to_string();
    let prev = { st.pr_cache.lock().await.clone() };
    let res = tokio::task::spawn_blocking(move || {
        build_page_cached(&args, &cfg, &repo_owned, &prev).and_then(|(html, model, prs)| {
            let json = serde_json::to_string(&model).map_err(|e| e.to_string())?;
            Ok((html, json, prs))
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    let (html, json, prs) = res?;
    let version = format!("{:x}", content_hash(&json));
    {
        let mut cache = st.cache.lock().await;
        cache.insert(
            repo.to_string(),
            Cached { html: html.clone(), json: json.clone(), at: Instant::now(), version: version.clone() },
        );
    }
    if !prs.is_empty() {
        let mut pc = st.pr_cache.lock().await;
        pc.extend(prs);
    }
    Ok((html, json, version))
}

/// refresh เบื้องหลัง (ไม่บล็อก request ที่เสิร์ฟข้อมูลเก่า)
fn spawn_refresh(st: Arc<AppState>, repo: String) {
    tokio::spawn(async move {
        {
            let mut guard = st.refreshing.lock().await;
            if !guard.insert(repo.clone()) {
                return;
            }
        }
        if let Err(e) = refresh(&st, &repo).await {
            eprintln!("⚠️  refresh {repo} ไม่สำเร็จ: {e}");
        }
        st.refreshing.lock().await.remove(&repo);
    });
}

/// คืน HTML/JSON จาก cache ทันทีถ้ามี (แม้หมดอายุ) แล้ว refresh เบื้องหลัง
async fn get_page(st: &Arc<AppState>, repo: &str) -> Result<(String, String), String> {
    if repo.is_empty() {
        return Err("ยังไม่ได้ระบุโปรเจค (ใช้ --repo owner/name หรือ --repos)".to_string());
    }
    let existing = {
        let cache = st.cache.lock().await;
        cache.get(repo).cloned()
    };
    if let Some(c) = existing {
        if c.at.elapsed() < CACHE_TTL {
            return Ok((c.html, c.json));
        }
        spawn_refresh(Arc::clone(st), repo.to_string());
        return Ok((c.html, c.json));
    }
    refresh(st, repo).await.map(|(html, json, _)| (html, json))
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
    match get_page(&st, &default_repo(&st)).await {
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

#[derive(Deserialize)]
struct DataQuery {
    repo: Option<String>,
}

/// `/api/projects` — รายการโปรเจคที่เปิดให้เลือกในหน้าเว็บ
/// `/api/profile` — ข้อมูลโปรไฟล์ GitHub ทั้งหมดของผู้ใช้ที่ล็อกอิน (ใช้ token ใน session)
async fn api_profile(State(st): State<Arc<AppState>>, headers: HttpHeaders) -> Response {
    let Some(session) = session_of(&st, &headers).await else {
        return json_error(StatusCode::UNAUTHORIZED, "ต้องเข้าสู่ระบบด้วย GitHub ก่อน");
    };
    let token = session.token;
    let result = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let user = api_get("/user", Some(&token))?;
        // endpoint เหล่านี้อาจถูกปฏิเสธถ้าผู้ใช้ไม่ได้ให้ scope ครบ — ไม่ทำให้หน้าโปรไฟล์ล้มทั้งหน้า
        let organizations = api_get("/user/orgs?per_page=100", Some(&token)).unwrap_or_else(|_| json!([]));
        let emails = api_get("/user/emails", Some(&token)).unwrap_or(Value::Null);
        let social_accounts = api_get("/user/social_accounts", Some(&token)).unwrap_or(Value::Null);
        Ok(json!({
            "user": user,
            "organizations": organizations,
            "emails": emails,
            "social_accounts": social_accounts,
        }))
    })
    .await;
    match result {
        Ok(Ok(v)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            v.to_string(),
        )
            .into_response(),
        Ok(Err(e)) => json_error(StatusCode::BAD_GATEWAY, &e),
        Err(e) => json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn api_projects(State(st): State<Arc<AppState>>, headers: HttpHeaders) -> Response {
    if st.repos.is_empty() {
        return text_response(StatusCode::BAD_REQUEST, "ยังไม่ได้ตั้งค่าโปรเจค (--repo/--repos)");
    }
    let projects: Vec<Value> = st
        .repos
        .iter()
        .map(|r| json!({ "repo": r, "label": r }))
        .collect();
    let session = session_of(&st, &headers).await;
    // โหมด OAuth: ต้องมี session เท่านั้น (ไม่ fallback ไป token ฝั่ง server)
    let login = if st.auth.is_some() {
        session.as_ref().map(|s| s.login.clone())
    } else {
        st.viewer.clone()
    };
    let body = json!({
        "projects": projects,
        "default": default_repo(&st),
        "write": st.allow_write,
        "viewer": st.viewer,
        "auth": { "enabled": st.auth.is_some(), "login": login },
    })
    .to_string();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        body,
    )
        .into_response()
}

/// `/api/data.json?repo=owner/name` — ข้อมูลของโปรเจคที่เลือก (ไม่ระบุ = โปรเจคแรก)
async fn api_data(State(st): State<Arc<AppState>>, Query(q): Query<DataQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return text_response(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    match get_page(&st, &repo).await {
        Ok((_, json)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            json,
        )
            .into_response(),
        Err(e) => text_response(StatusCode::BAD_GATEWAY, &format!("โหลดข้อมูลไม่สำเร็จ: {e}")),
    }
}

#[derive(Deserialize)]
struct DiffQuery {
    repo: Option<String>,
    pr: Option<i64>,
}

/// ไฟล์+patch ของ PR เดียวผ่าน REST
fn fetch_pr_files(repo: &str, number: i64, tk: Option<&str>) -> Result<Vec<Value>, String> {
    let path = format!("/repos/{repo}/pulls/{number}/files?per_page=100");
    let files = api_get(&path, tk)?;
    Ok(files
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| {
                    json!({
                        "filename": f.get("filename").cloned().unwrap_or(Value::Null),
                        "lines": f.get("additions").and_then(|v| v.as_i64()).unwrap_or(0)
                            + f.get("deletions").and_then(|v| v.as_i64()).unwrap_or(0),
                        "patch": f.get("patch").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// `/api/diff?repo=&pr=` — diff ของ PR เดียว (โหลดเมื่อเปิดหน้า detail, cache 60 วิ)
async fn api_diff(State(st): State<Arc<AppState>>, Query(q): Query<DiffQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return text_response(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = q.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return text_response(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let key = format!("{repo}#{number}");
    {
        let cache = st.diff_cache.lock().await;
        if let Some((at, files)) = cache.get(&key) {
            if at.elapsed() < Duration::from_secs(60) {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                    json!({ "repo": repo, "pr": number, "files": files }).to_string(),
                )
                    .into_response();
            }
        }
    }
    let repo_owned = repo.clone();
    let res = tokio::task::spawn_blocking(move || {
        let token = github_token();
        fetch_pr_files(&repo_owned, number, token.as_deref())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    match res {
        Ok(files) => {
            let mut cache = st.diff_cache.lock().await;
            cache.insert(key, (Instant::now(), files.clone()));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                json!({ "repo": repo, "pr": number, "files": files }).to_string(),
            )
                .into_response()
        }
        Err(e) => text_response(StatusCode::BAD_GATEWAY, &format!("โหลด diff ไม่สำเร็จ: {e}")),
    }
}

struct AuthConfig {
    client_id: String,
    client_secret: String,
    /// ตั้งค่าตายตัวด้วย DASHBOARD_PUBLIC_URL / --public-url (ถ้าไม่ตั้ง จะเดาจาก Host header)
    public_url: Option<String>,
    /// ใช้เมื่อไม่มีทั้ง public_url และ Host header
    fallback_url: String,
    states: Mutex<HashMap<String, PendingAuth>>,
    sessions: Mutex<HashMap<String, Session>>,
}

struct PendingAuth {
    at: Instant,
    redirect_uri: String,
    next: String,
}

/// origin ของ callback: public_url ที่ตั้งไว้ > X-Forwarded-* / Host ที่เบราว์เซอร์เรียกเข้ามา > fallback
fn callback_origin(cfg: &AuthConfig, headers: &HttpHeaders) -> String {
    if let Some(p) = cfg.public_url.as_deref() {
        return p.trim_end_matches('/').to_string();
    }
    let first = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    if let Some(host) = first("x-forwarded-host").or_else(|| first("host")) {
        let proto = first("x-forwarded-proto").unwrap_or_else(|| "http".to_string());
        return format!("{}://{}", proto, host);
    }
    cfg.fallback_url.trim_end_matches('/').to_string()
}

#[derive(Clone)]
struct Session {
    login: String,
    token: String,
    expires: Instant,
}

fn random_id() -> String {
    use std::hash::{Hash, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (nanos, n, std::process::id()).hash(&mut h);
    format!("{:016x}{:x}", h.finish(), n)
}

fn urlencode(v: &str) -> String {
    let mut out = String::new();
    for b in v.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn cookie_value(headers: &HttpHeaders, name: &str) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let mut it = part.trim().splitn(2, '=');
        if it.next() == Some(name) {
            return it.next().map(|v| v.to_string());
        }
    }
    None
}

async fn session_of(st: &AppState, headers: &HttpHeaders) -> Option<Session> {
    let cfg = st.auth.as_ref()?;
    let sid = cookie_value(headers, "dash_session")?;
    let mut sessions = cfg.sessions.lock().await;
    if let Some(s) = sessions.get(&sid) {
        if s.expires > Instant::now() {
            return Some(s.clone());
        }
        sessions.remove(&sid);
    }
    None
}

/// ด่านบังคับล็อกอิน — ทุกหน้า/ทุก API ต้องมี session ยกเว้น /auth/* และ static assets
async fn require_auth(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let path = req.uri().path().to_string();
    if path == "/login" || path.starts_with("/auth/") || path.starts_with("/assets/") || path == "/favicon.ico" {
        return next.run(req).await;
    }
    if st.auth.is_none() {
        let msg = "ยังไม่ได้ตั้งค่า GitHub OAuth (GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET) — ปิดการเข้าถึงข้อมูลทั้งหมด";
        return if path.starts_with("/api/") {
            json_error(StatusCode::SERVICE_UNAVAILABLE, msg)
        } else {
            text_response(StatusCode::SERVICE_UNAVAILABLE, msg)
        };
    }
    if session_of(&st, req.headers()).await.is_some() {
        return next.run(req).await;
    }
    if path.starts_with("/api/") {
        return json_error(StatusCode::UNAUTHORIZED, "ต้องเข้าสู่ระบบด้วย GitHub ก่อน");
    }
    let target = req
        .uri()
        .path_and_query()
        .map(|x| x.as_str())
        .unwrap_or("/")
        .to_string();
    redirect_to(&format!("/login?next={}", urlencode(&target)))
}

/// อนุญาตเฉพาะ path ภายในเว็บ (กัน open redirect)
fn safe_next(raw: Option<&str>) -> String {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return "/".to_string();
    };
    if !s.starts_with('/')
        || s.starts_with("//")
        || s.starts_with("/login")
        || s.starts_with("/auth/")
        || s.contains(['\n', '\r', '\\'])
        || s.len() > 2048
    {
        return "/".to_string();
    }
    s.to_string()
}

fn redirect_to(url: &str) -> Response {
    let mut res = StatusCode::FOUND.into_response();
    if let Ok(v) = HttpHeaderValue::from_str(url) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

#[derive(Deserialize)]
struct OAuthQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Deserialize)]
struct LoginQuery {
    next: Option<String>,
}

const LOGIN_HTML: &str = r##"<!doctype html>
<html lang="th">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>เข้าสู่ระบบ · Auto-review dashboard</title>
<style>
:root{color-scheme:light dark;--bg:#ffffff;--card:#ffffff;--surface:#f5f6f9;--fg:#18202f;--muted:#5b6475;--line:#dfe3ec;--accent:#4f46e5;--accent-strong:#3730a3;--accent-soft:#eef2ff;--accent-2:#7c3aed;--btn:#18202f;--btn-h:#000000;--btn-fg:#ffffff;--err:#b91c1c;--err-soft:#fee2e2;--ok:#15803d}
@media (prefers-color-scheme:dark){:root{--bg:#0d0d0d;--card:#141414;--surface:#1c1c1c;--fg:#e8e8e8;--muted:#a1a1a1;--line:#2c2c2c;--accent:#818cf8;--accent-strong:#a5b4fc;--accent-soft:#1e1b4b;--accent-2:#a78bfa;--btn:#e8e8e8;--btn-h:#ffffff;--btn-fg:#0d0d0d;--err:#f87171;--err-soft:#2c1518;--ok:#4ade80}}
*{box-sizing:border-box}
body{margin:0;min-height:100vh;display:grid;place-items:center;background:var(--bg);color:var(--fg);font:14px/1.6 -apple-system,"Segoe UI",Roboto,"Noto Sans Thai",sans-serif;padding:24px}
.card{position:relative;overflow:hidden;width:min(460px,100%);background:var(--card);border:1px solid var(--line);border-radius:20px;padding:30px 30px 24px;box-shadow:0 2px 4px rgba(0,0,0,.04),0 24px 60px rgba(0,0,0,.1)}
.card::before{content:"";position:absolute;inset:0 0 auto;height:5px;background:linear-gradient(90deg,var(--accent),var(--accent-2))}
.brand{display:flex;align-items:center;gap:9px;font-weight:600;font-size:13px;color:var(--muted);margin-bottom:22px}
.brand .logo{display:grid;place-items:center;width:34px;height:34px;border-radius:10px;background:var(--fg);color:var(--bg);box-shadow:none}
.brand svg{width:20px;height:20px;fill:currentColor}
h1{font-size:28px;line-height:1.2;letter-spacing:-.035em;margin:0 0 7px;font-weight:800;color:var(--accent-strong)}
.sub{margin:0 0 20px;color:var(--muted);font-size:13.5px;line-height:1.65}
.gh{display:flex;align-items:center;justify-content:center;gap:9px;width:100%;min-height:46px;background:var(--btn);color:var(--btn-fg)!important;border-radius:12px;padding:11px 15px;font-weight:700;text-decoration:none!important;margin:0 0 18px;box-shadow:none;transition:background .16s ease,transform .16s ease}
.gh:hover{background:var(--btn-h);transform:translateY(-1px);box-shadow:none;text-decoration:none!important}
.gh:focus-visible{outline:3px solid var(--fg);outline-offset:3px}
.gh svg{width:19px;height:19px;fill:currentColor}
.gh.disabled{background:var(--surface);color:var(--muted)!important;cursor:not-allowed;box-shadow:none;transform:none}
.err{border:1px solid color-mix(in srgb,var(--err) 35%,var(--line));border-radius:12px;padding:11px 13px;color:var(--err);background:var(--err-soft);font-size:12.5px;margin:0 0 14px;overflow-wrap:anywhere}
.perks{list-style:none;margin:0 0 18px;padding:0;display:grid;gap:9px}
.perks li{display:flex;align-items:flex-start;gap:10px;padding:9px 10px;border:1px solid var(--line);border-radius:12px;background:var(--surface)}
.perks .perk-ico{display:grid;place-items:center;width:30px;height:30px;border-radius:9px;background:var(--accent-soft);color:var(--accent-strong);flex:none}
.perks .perk-ico svg{width:17px;height:17px;fill:none;stroke:currentColor;stroke-width:1.8;stroke-linecap:round;stroke-linejoin:round}
.perks b{display:block;font-size:12.5px;line-height:1.35;color:var(--fg)}
.perks span{display:block;font-size:11.5px;line-height:1.45;color:var(--muted);margin-top:1px}
.scopes{font-size:11.5px;color:var(--muted);margin:0 0 12px;overflow-wrap:anywhere}
.scopes code{background:var(--surface);border:1px solid var(--line);border-radius:5px;padding:1px 4px;font-size:10.5px}
.fine{display:flex;align-items:flex-start;gap:6px;font-size:11px;line-height:1.5;color:var(--muted);margin:0;border-top:1px solid var(--line);padding-top:12px}
.fine svg{width:14px;height:14px;flex:none;fill:none;stroke:currentColor;stroke-width:1.8;stroke-linecap:round;stroke-linejoin:round;margin-top:1px}
@media (prefers-reduced-motion:reduce){*{transition:none!important}}
</style>
</head>
<body>
<main class="card">
  <div class="brand">
    <span class="logo"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg></span>
    <span>Auto-review dashboard</span>
  </div>
  <h1>ยินดีต้อนรับ</h1>
  <p class="sub">รวม PR, ประวัติ merge และโปรไฟล์ GitHub ไว้ในที่เดียว — เริ่มจากเข้าสู่ระบบด้วยบัญชี GitHub ของคุณ</p>
  __NOTICE__
  __ACTION__
  <ul class="perks">
    <li><span class="perk-ico"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12z"/><circle cx="12" cy="12" r="2.5"/></svg></span><div><b>เห็นงานที่ต้องรีวิว</b><span>รวม PR, สถานะ และภาระงานของแต่ละคนไว้ในหน้าจอเดียว</span></div></li>
    <li><span class="perk-ico"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l3 2"/></svg></span><div><b>ย้อนดูประวัติได้ทันที</b><span>ดู commit, merge และบทสนทนาโดยไม่ต้องสลับแท็บไป GitHub</span></div></li>
    <li><span class="perk-ico"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 3 4 6v5c0 5 3.5 8.5 8 10 4.5-1.5 8-5 8-10V6z"/><path d="m8.5 12 2.2 2.2 4.8-4.8"/></svg></span><div><b>ปลอดภัยกว่าเดิม</b><span>token อยู่ใน session ฝั่ง server 12 ชั่วโมง ไม่ถูกส่งไปเบราว์เซอร์</span></div></li>
  </ul>
  <p class="scopes">สิทธิ์ที่ขอ: <code>repo</code> <code>read:user</code> <code>user:email</code> <code>read:org</code></p>
  <p class="fine"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/></svg><span>token ถูกเก็บไว้ฝั่ง server และไม่ถูกส่งไปยังเบราว์เซอร์</span></p>
</main>
</body>
</html>"##;

/// `GET /login` — หน้าเข้าสู่ระบบของ dashboard
async fn login_page(State(st): State<Arc<AppState>>, headers: HttpHeaders, Query(q): Query<LoginQuery>) -> Response {
    let next = safe_next(q.next.as_deref());
    if session_of(&st, &headers).await.is_some() {
        return redirect_to(&next);
    }
    let (notice, action) = if st.auth.is_some() {
        (
            String::new(),
            format!(
                r#"<a class="gh" href="/auth/login?next={}"><svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/></svg>Continue with GitHub</a>"#,
                escape_html(&urlencode(&next))
            ),
        )
    } else {
        (
            "<p class=\"err\">ยังไม่ได้ตั้งค่า GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET — ระบบปิดการเข้าถึงข้อมูลทั้งหมด</p>".to_string(),
            "<span class=\"gh disabled\">ยังไม่ได้ตั้งค่า OAuth</span>".to_string(),
        )
    };
    let html = LOGIN_HTML.replace("__NOTICE__", &notice).replace("__ACTION__", &action);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        html,
    )
        .into_response()
}

/// `GET /auth/login` — พาไปหน้า authorize ของ GitHub
async fn auth_login(State(st): State<Arc<AppState>>, headers: HttpHeaders, Query(q): Query<LoginQuery>) -> Response {
    let Some(cfg) = st.auth.as_ref() else {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "ยังไม่ได้ตั้งค่า GitHub OAuth (GITHUB_CLIENT_ID / GITHUB_CLIENT_SECRET)");
    };
    let next = safe_next(q.next.as_deref());
    let state_tok = random_id();
    let redirect_uri = format!("{}/auth/callback", callback_origin(cfg, &headers));
    {
        let mut m = cfg.states.lock().await;
        m.retain(|_, p| p.at.elapsed() < Duration::from_secs(900));
        m.insert(
            state_tok.clone(),
            PendingAuth { at: Instant::now(), redirect_uri: redirect_uri.clone(), next },
        );
    }
    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope={}&state={}",
        urlencode(&cfg.client_id),
        urlencode(&redirect_uri),
        urlencode("repo read:user user:email read:org"),
        state_tok
    );
    redirect_to(&url)
}

/// `GET /auth/callback` — แลก code เป็น token, เก็บ session ฝั่ง server
async fn auth_callback(State(st): State<Arc<AppState>>, Query(q): Query<OAuthQuery>) -> Response {
    let Some(cfg) = st.auth.as_ref() else {
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "ยังไม่ได้ตั้งค่า GitHub OAuth");
    };
    if let Some(err) = q.error {
        let msg = q.error_description.unwrap_or(err);
        return json_error(StatusCode::BAD_REQUEST, &format!("GitHub ปฏิเสธการเข้าสู่ระบบ: {msg}"));
    }
    let (Some(code), Some(state_tok)) = (q.code, q.state) else {
        return json_error(StatusCode::BAD_REQUEST, "callback ไม่ครบ (code/state)");
    };
    // ใช้ redirect_uri ตัวเดียวกับตอน /auth/login (GitHub ต้องได้ค่าเดิมตอนแลก code)
    let pending = {
        let mut m = cfg.states.lock().await;
        m.remove(&state_tok).filter(|p| p.at.elapsed() < Duration::from_secs(900))
    };
    let Some(pending) = pending else {
        return json_error(StatusCode::BAD_REQUEST, "state ไม่ถูกต้องหรือหมดอายุ — เริ่มเข้าสู่ระบบใหม่");
    };
    let redirect_uri = pending.redirect_uri;
    let next = pending.next;

    let secure = if redirect_uri.starts_with("https://") { "; Secure" } else { "" };
    let client_id = cfg.client_id.clone();
    let client_secret = cfg.client_secret.clone();
    let result = tokio::task::spawn_blocking(move || -> Result<(String, String), String> {
        let form = format!(
            "client_id={}&client_secret={}&code={}&redirect_uri={}",
            urlencode(&client_id),
            urlencode(&client_secret),
            urlencode(&code),
            urlencode(&redirect_uri)
        );
        let res = ureq::post("https://github.com/login/oauth/access_token")
            .set("Accept", "application/json")
            .set("User-Agent", "auto-review-dashboard-rs")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .timeout(Duration::from_secs(20))
            .send_string(&form)
            .map_err(|e| format!("แลก code ไม่สำเร็จ: {e}"))?;
        let v: Value = serde_json::from_str(&res.into_string().unwrap_or_default()).map_err(|e| e.to_string())?;
        let token = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .ok_or_else(|| {
                format!(
                    "ไม่ได้ access_token: {}",
                    v.get("error_description").and_then(|x| x.as_str()).unwrap_or("unknown")
                )
            })?
            .to_string();
        let ures = ureq::get(&format!("{API}/user"))
            .set("Accept", "application/vnd.github+json")
            .set("User-Agent", "auto-review-dashboard-rs")
            .set("Authorization", &format!("Bearer {token}"))
            .timeout(Duration::from_secs(15))
            .call()
            .map_err(|e| format!("อ่าน /user ไม่สำเร็จ: {e}"))?;
        let uv: Value = serde_json::from_str(&ures.into_string().unwrap_or_default()).map_err(|e| e.to_string())?;
        let login = uv
            .get("login")
            .and_then(|x| x.as_str())
            .ok_or_else(|| "ไม่พบ login ใน /user".to_string())?
            .to_string();
        Ok((token, login))
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    let (token, login) = match result {
        Ok(v) => v,
        Err(e) => return json_error(StatusCode::BAD_GATEWAY, &e),
    };
    let sid = random_id();
    {
        let mut m = cfg.sessions.lock().await;
        m.retain(|_, s| s.expires > Instant::now());
        m.insert(
            sid.clone(),
            Session { login, token, expires: Instant::now() + Duration::from_secs(12 * 3600) },
        );
    }
    let cookie = format!("dash_session={sid}; Path=/; HttpOnly; SameSite=Lax; Max-Age=43200{secure}");
    let mut res = redirect_to(&next);
    if let Ok(v) = HttpHeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

/// `POST /auth/logout` — ลบ session แล้วล้าง cookie
async fn auth_logout(State(st): State<Arc<AppState>>, headers: HttpHeaders) -> Response {
    if let Some(cfg) = st.auth.as_ref() {
        if let Some(sid) = cookie_value(&headers, "dash_session") {
            let mut m = cfg.sessions.lock().await;
            m.remove(&sid);
        }
    }
    let mut res = StatusCode::OK.into_response();
    res.headers_mut().insert(
        header::SET_COOKIE,
        HttpHeaderValue::from_static("dash_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    res
}

/// login ของบัญชีที่ token ใช้ (เรียก /user ครั้งเดียวตอน start)
fn fetch_viewer_login() -> Option<String> {
    let token = github_token()?;
    let res = ureq::get(&format!("{API}/user"))
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "auto-review-dashboard-rs")
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(15))
        .call()
        .ok()?;
    let v: Value = serde_json::from_str(&res.into_string().ok()?).ok()?;
    v.get("login").and_then(|x| x.as_str()).map(|x| x.to_string())
}

/// ส่ง JSON ไป GitHub REST (POST/PUT) — ใช้สำหรับเขียนรีวิวและ merge
fn api_write_json(method: &str, path: &str, payload: &Value, token: &str) -> Result<Value, String> {
    let body = serde_json::to_string(payload).map_err(|e| e.to_string())?;
    let url = format!("{API}{path}");
    let req = if method.eq_ignore_ascii_case("PUT") { ureq::put(&url) } else { ureq::post(&url) };
    let res = req
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", "auto-review-dashboard-rs")
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(25))
        .send_string(&body);
    match res {
        Ok(r) => {
            let text = r.into_string().unwrap_or_default();
            serde_json::from_str(&text).or_else(|_| Ok(json!({ "raw": text })))
        }
        Err(ureq::Error::Status(code, r)) => {
            let text = r.into_string().unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(|x| x.to_string()))
                .unwrap_or_else(|| text.chars().take(200).collect());
            Err(format!("GitHub {code}: {msg}"))
        }
        Err(e) => Err(format!("{e}")),
    }
}

fn api_post_json(path: &str, payload: &Value, token: &str) -> Result<Value, String> {
    api_write_json("POST", path, payload, token)
}

#[derive(Deserialize)]
struct ReviewRequest {
    repo: Option<String>,
    pr: Option<i64>,
    action: Option<String>,
    body: Option<String>,
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        json!({ "error": msg }).to_string(),
    )
        .into_response()
}

/// `POST /api/review` — approve / request changes / comment จากหน้าเว็บ (ต้องเปิด --allow-write)
async fn api_review(
    State(st): State<Arc<AppState>>,
    headers: HttpHeaders,
    Json(req): Json<ReviewRequest>,
) -> Response {
    if !st.allow_write {
        return json_error(StatusCode::FORBIDDEN, "โหมดอ่านอย่างเดียว — ต้องเริ่ม server ด้วย --allow-write");
    }
    let session = session_of(&st, &headers).await;
    if st.auth.is_some() && session.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "ต้องเข้าสู่ระบบด้วย GitHub ก่อนจึงจะรีวิวได้");
    }
    let repo = match req.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = req.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let (event, label) = match req.action.as_deref() {
        Some("approve") => ("APPROVE", "อนุมัติ"),
        Some("request_changes") => ("REQUEST_CHANGES", "ขอแก้ไข"),
        Some("comment") => ("COMMENT", "คอมเมนต์"),
        _ => return json_error(StatusCode::BAD_REQUEST, "action ต้องเป็น approve / request_changes / comment"),
    };
    let body = req.body.unwrap_or_default().trim().to_string();
    if event != "APPROVE" && body.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "ต้องใส่ข้อความเมื่อขอแก้ไขหรือคอมเมนต์");
    }
    let repo_owned = repo.clone();
    let body_owned = body.clone();
    let session_token = session.as_ref().map(|s| s.token.clone());
    let viewer_login = session.as_ref().map(|s| s.login.clone()).or_else(|| st.viewer.clone());
    let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let token = match session_token {
            Some(t) => t,
            None => github_token().ok_or_else(|| "ไม่มี token (GH_TOKEN/GITHUB_TOKEN/git credential)".to_string())?,
        };
        // กัน approve/ขอแก้ไข PR ของตัวเอง (GitHub จะตอบ 422 อยู่แล้ว แต่ให้ข้อความชัดเจนกว่า)
        if event != "COMMENT" {
            if let Some(me) = viewer_login {
                let author = ureq::get(&format!("{API}/repos/{repo_owned}/pulls/{number}"))
                    .set("Accept", "application/vnd.github+json")
                    .set("User-Agent", "auto-review-dashboard-rs")
                    .set("Authorization", &format!("Bearer {token}"))
                    .timeout(Duration::from_secs(15))
                    .call()
                    .ok()
                    .and_then(|r| serde_json::from_str::<Value>(&r.into_string().unwrap_or_default()).ok())
                    .and_then(|v| v.get("user").and_then(|u| u.get("login")).and_then(|l| l.as_str()).map(|s| s.to_string()));
                if author.map(|a| a.eq_ignore_ascii_case(&me)).unwrap_or(false) {
                    return Err("__SELF__".to_string());
                }
            }
        }
        let path = format!("/repos/{repo_owned}/pulls/{number}/reviews");
        let payload = json!({ "event": event, "body": body_owned });
        api_post_json(&path, &payload, &token)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    match res {
        Ok(review) => {
            // ล้าง cache ของ repo/PR นี้ แล้ว refresh เบื้องหลังให้เห็นสถานะล่าสุด
            {
                let mut cache = st.cache.lock().await;
                cache.remove(&repo);
            }
            {
                let mut pc = st.pr_cache.lock().await;
                pc.remove(&number.to_string());
            }
            {
                let mut dc = st.diff_cache.lock().await;
                dc.remove(&format!("{repo}#{number}"));
            }
            spawn_refresh(Arc::clone(&st), repo.clone());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                json!({
                    "ok": true,
                    "repo": repo,
                    "pr": number,
                    "event": event,
                    "label": label,
                    "review": review,
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) if e == "__SELF__" => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "PR นี้เป็นของคุณเอง — GitHub ไม่อนุญาตให้อนุมัติ/ขอแก้ไข PR ของตัวเอง (คอมเมนต์ได้)",
        ),
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &e),
    }
}

#[derive(Deserialize)]
struct CommentRequest {
    repo: Option<String>,
    pr: Option<i64>,
    path: Option<String>,
    line: Option<i64>,
    side: Option<String>,
    body: Option<String>,
}

/// คำอธิบายภาษาไทยสำหรับ error ที่พบบ่อยจาก GitHub ตอนคอมเมนต์รายบรรทัด
fn comment_hint(e: &str) -> &'static str {
    let l = e.to_lowercase();
    if l.contains("line must be part of the diff") || l.contains("position") || l.contains("diff hunk") {
        "บรรทัดนี้ไม่พบใน diff ล่าสุด — refresh หน้าแล้วลองอีกครั้ง — "
    } else if e.contains("GitHub 403") {
        "บัญชีนี้ไม่มีสิทธิ์คอมเมนต์ใน repo/PR นี้ — "
    } else if e.contains("GitHub 404") {
        "ไม่พบ PR หรือไม่มีสิทธิ์เข้าถึง — "
    } else if e.contains("GitHub 422") {
        "GitHub ไม่รับคอมเมนต์นี้ — ตรวจว่าบรรทัดยังอยู่ใน diff ล่าสุด — "
    } else {
        ""
    }
}

/// `POST /api/comment` — สร้าง review comment รายบรรทัด (ต้องเปิด --allow-write)
async fn api_comment(
    State(st): State<Arc<AppState>>,
    headers: HttpHeaders,
    Json(req): Json<CommentRequest>,
) -> Response {
    if !st.allow_write {
        return json_error(StatusCode::FORBIDDEN, "โหมดอ่านอย่างเดียว — ต้องเริ่ม server ด้วย --allow-write");
    }
    let session = session_of(&st, &headers).await;
    if st.auth.is_some() && session.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "ต้องเข้าสู่ระบบด้วย GitHub ก่อนจึงจะคอมเมนต์ได้");
    }
    let repo = match req.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = req.pr.unwrap_or(0);
    let path = req.path.unwrap_or_default().trim().to_string();
    let line = req.line.unwrap_or(0);
    let side = req.side.unwrap_or_else(|| "RIGHT".to_string()).to_ascii_uppercase();
    let body = req.body.unwrap_or_default().trim().to_string();
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    if path.is_empty() || path.chars().count() > 1024 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ path ของไฟล์");
    }
    if line <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "line ต้องเป็นเลขบรรทัดที่มากกว่า 0");
    }
    if side != "LEFT" && side != "RIGHT" {
        return json_error(StatusCode::BAD_REQUEST, "side ต้องเป็น LEFT หรือ RIGHT");
    }
    if body.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "ต้องใส่ข้อความคอมเมนต์");
    }
    if body.chars().count() > 60_000 {
        return json_error(StatusCode::BAD_REQUEST, "คอมเมนต์ยาวเกิน 60,000 ตัวอักษร");
    }

    let repo_owned = repo.clone();
    let path_owned = path.clone();
    let side_owned = side.clone();
    let body_owned = body.clone();
    let session_token = session.as_ref().map(|s| s.token.clone());
    let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let token = match session_token {
            Some(t) => t,
            None => github_token().ok_or_else(|| "ไม่มี token (GH_TOKEN/GITHUB_TOKEN/git credential)".to_string())?,
        };
        let meta = api_get(&format!("/repos/{repo_owned}/pulls/{number}"), Some(&token))?;
        let pr_state = meta.get("state").and_then(|v| v.as_str()).unwrap_or("");
        if pr_state == "closed" {
            return Err("__CLOSED__".to_string());
        }
        let commit_id = meta
            .pointer("/head/sha")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "อ่าน commit ล่าสุดของ PR ไม่ได้".to_string())?
            .to_string();
        let payload = json!({
            "body": body_owned,
            "commit_id": commit_id,
            "path": path_owned,
            "line": line,
            "side": side_owned,
        });
        api_post_json(&format!("/repos/{repo_owned}/pulls/{number}/comments"), &payload, &token)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    match res {
        Ok(comment) => {
            {
                let mut cache = st.cache.lock().await;
                cache.remove(&repo);
            }
            {
                let mut pc = st.pr_cache.lock().await;
                pc.remove(&number.to_string());
            }
            {
                let mut dc = st.diff_cache.lock().await;
                dc.remove(&format!("{repo}#{number}"));
            }
            {
                let mut pc = st.pr_detail_cache.lock().await;
                pc.remove(&format!("{repo}#{number}"));
            }
            spawn_refresh(Arc::clone(&st), repo.clone());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                json!({
                    "ok": true,
                    "repo": repo,
                    "pr": number,
                    "path": path,
                    "line": line,
                    "side": side,
                    "label": "คอมเมนต์แล้ว",
                    "comment": comment,
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) if e == "__CLOSED__" => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "PR นี้ปิด/merge แล้ว — คอมเมนต์รายบรรทัดได้เฉพาะ PR ที่เปิดอยู่",
        ),
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &format!("{}{e}", comment_hint(&e))),
    }
}

#[derive(Deserialize)]
struct MergeRequest {
    repo: Option<String>,
    pr: Option<i64>,
    method: Option<String>,
    title: Option<String>,
    message: Option<String>,
}

/// คำอธิบายภาษาไทยสำหรับ error ที่พบบ่อยจาก GitHub ตอน merge
fn merge_hint(e: &str) -> &'static str {
    let l = e.to_lowercase();
    if l.contains("approving review") || l.contains("review is required") {
        "ต้องมี approving review จากผู้มีสิทธิ์เขียนก่อน (branch protection) — "
    } else if l.contains("not mergeable") || l.contains("merge conflict") {
        "PR ยัง merge ไม่ได้ (มี conflict หรือเงื่อนไขยังไม่ผ่าน) — "
    } else if l.contains("squash") {
        "repo นี้ปิดการ squash merge — "
    } else if l.contains("rebase") {
        "repo นี้ปิดการ rebase merge — "
    } else if e.contains("GitHub 401") {
        "session/token หมดอายุหรือไม่ถูกต้อง — ออกจากระบบแล้วเข้าสู่ระบบใหม่ — "
    } else if e.contains("GitHub 403") {
        "บัญชีที่ล็อกอินไม่มีสิทธิ์เขียน repo นี้ — "
    } else if e.contains("GitHub 404") {
        "ไม่พบ PR หรือบัญชีไม่มีสิทธิ์เข้าถึง repo — "
    } else {
        ""
    }
}

/// `POST /api/merge` — merge PR จากหน้าเว็บ (ต้องเปิด --allow-write)
async fn api_merge(
    State(st): State<Arc<AppState>>,
    headers: HttpHeaders,
    Json(req): Json<MergeRequest>,
) -> Response {
    if !st.allow_write {
        return json_error(StatusCode::FORBIDDEN, "โหมดอ่านอย่างเดียว — ต้องเริ่ม server ด้วย --allow-write");
    }
    let session = session_of(&st, &headers).await;
    if st.auth.is_some() && session.is_none() {
        return json_error(StatusCode::UNAUTHORIZED, "ต้องเข้าสู่ระบบด้วย GitHub ก่อนจึงจะ merge ได้");
    }
    let repo = match req.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = req.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let method = match req.method.as_deref() {
        Some("merge") => "merge",
        Some("squash") => "squash",
        Some("rebase") => "rebase",
        Some(m) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                &format!("method ต้องเป็น merge / squash / rebase (ได้: {m})"),
            )
        }
        None => "merge",
    };
    let title: String = req.title.unwrap_or_default().trim().chars().take(200).collect();
    let message: String = req.message.unwrap_or_default().trim().chars().take(2000).collect();
    // ใครเป็นคนสั่ง merge (session > token ของ server) — GitHub จะเห็นเป็น token admin จึงต้อง log เอง
    let actor = session
        .as_ref()
        .map(|s| s.login.clone())
        .or_else(|| st.viewer.clone())
        .unwrap_or_else(|| "server-token".to_string());
    let audit_title = title.clone();
    let repo_owned = repo.clone();
    // merge ใช้ admin token ของ server เสมอ (ไม่ใช้ session ของผู้ใช้) เพื่อให้ผู้ที่ล็อกอินแต่ไม่มีสิทธิ์เขียน merge ได้
    let session_token: Option<String> = None;
    let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let token = match session_token {
            Some(t) => t,
            None => github_token().ok_or_else(|| "ไม่มี token (GH_TOKEN/GITHUB_TOKEN/git credential)".to_string())?,
        };
        let mut payload = json!({ "merge_method": method });
        if !title.is_empty() {
            payload["commit_title"] = json!(title);
        }
        if !message.is_empty() {
            payload["commit_message"] = json!(message);
        }
        let path = format!("/repos/{repo_owned}/pulls/{number}/merge");
        api_write_json("PUT", &path, &payload, &token)
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);

    match res {
        Ok(v) => {
            {
                let mut cache = st.cache.lock().await;
                cache.remove(&repo);
            }
            {
                let mut pc = st.pr_cache.lock().await;
                pc.remove(&number.to_string());
            }
            {
                let mut dc = st.diff_cache.lock().await;
                dc.remove(&format!("{repo}#{number}"));
            }
            {
                let mut hc = st.history_cache.lock().await;
                hc.remove(&repo);
            }
            append_audit(&json!({
                "at": iso_now(),
                "repo": repo,
                "pr": number,
                "action": "merge",
                "actor": actor.clone(),
                "method": method,
                "sha": v.get("sha").cloned().unwrap_or(Value::Null),
                "title": if audit_title.is_empty() { Value::Null } else { json!(audit_title) },
            }));
            spawn_refresh(Arc::clone(&st), repo.clone());
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                json!({
                    "ok": true,
                    "repo": repo,
                    "pr": number,
                    "method": method,
                    "actor": actor,
                    "merged": v.get("merged").cloned().unwrap_or(json!(true)),
                    "sha": v.get("sha").cloned().unwrap_or(Value::Null),
                    "message": v.get("message").cloned().unwrap_or(Value::Null),
                })
                .to_string(),
            )
                .into_response()
        }
        Err(e) => {
            eprintln!("merge {repo}#{number} ({method}) ไม่สำเร็จ: {e}");
            let status = if e.contains("GitHub 405") || e.contains("GitHub 409") {
                StatusCode::CONFLICT
            } else if e.contains("GitHub 422") {
                StatusCode::UNPROCESSABLE_ENTITY
            } else if e.contains("GitHub 401") {
                StatusCode::UNAUTHORIZED
            } else if e.contains("GitHub 403") {
                StatusCode::FORBIDDEN
            } else if e.contains("GitHub 404") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            };
            json_error(status, &format!("{}{e}", merge_hint(&e)))
        }
    }
}

fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// path ของ audit log (บันทึกว่าใครสั่ง merge ผ่าน dashboard)
fn audit_log_path() -> PathBuf {
    std::env::var("DASHBOARD_AUDIT_LOG")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("dashboard-audit.jsonl"))
}

fn append_audit(entry: &Value) {
    let path = audit_log_path();
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(dir);
        }
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{entry}");
    }
}

/// audit ล่าสุดก่อน (อ่านท้ายไฟล์ ไม่เกิน 1000 บรรทัด)
fn read_audit() -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(audit_log_path()) else {
        return Vec::new();
    };
    text.lines()
        .rev()
        .take(1000)
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

fn gql_commit_json(repo: &str, c: &Value) -> Value {
    let sha = c.get("oid").and_then(|v| v.as_str()).unwrap_or("");
    let author = c.get("author").cloned().unwrap_or(Value::Null);
    let committer = c.get("committer").cloned().unwrap_or(Value::Null);
    let date = c.get("committedDate").and_then(|v| v.as_str());
    let message = c.get("messageHeadline").cloned().unwrap_or(Value::Null);
    json!({
        "sha": sha,
        "short": short_sha(sha),
        "message": message,
        "summary": c.get("messageHeadline").cloned().unwrap_or(Value::Null),
        "author_name": author.get("name").cloned().unwrap_or(Value::Null),
        "author_login": author.pointer("/user/login").cloned().unwrap_or(Value::Null),
        "committer_name": committer.get("name").cloned().unwrap_or(Value::Null),
        "committer_login": committer.pointer("/user/login").cloned().unwrap_or(Value::Null),
        "date": date,
        "date_label": rel_time(date, now_epoch()),
        "url": format!("https://github.com/{repo}/commit/{sha}"),
        "parents": Value::Null,
        "verified": Value::Null
    })
}

fn rest_commit_json(repo: &str, c: &Value) -> Value {
    let sha = c.get("sha").and_then(|v| v.as_str()).unwrap_or("");
    let commit = c.get("commit").cloned().unwrap_or(Value::Null);
    let author = commit.get("author").cloned().unwrap_or(Value::Null);
    let committer = commit.get("committer").cloned().unwrap_or(Value::Null);
    let message = commit.get("message").and_then(|v| v.as_str()).unwrap_or("");
    let summary = message.lines().next().unwrap_or("");
    let date = author.get("date").and_then(|v| v.as_str());
    json!({
        "sha": sha,
        "short": short_sha(sha),
        "message": message,
        "summary": summary,
        "author_name": author.get("name").cloned().unwrap_or(Value::Null),
        "author_login": c.pointer("/author/login").cloned().unwrap_or(Value::Null),
        "committer_name": committer.get("name").cloned().unwrap_or(Value::Null),
        "committer_login": c.pointer("/committer/login").cloned().unwrap_or(Value::Null),
        "date": date,
        "date_label": rel_time(date, now_epoch()),
        "url": c.get("html_url").cloned().unwrap_or_else(|| json!(format!("https://github.com/{repo}/commit/{sha}"))),
        "parents": c.get("parents").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
        "verified": commit.pointer("/verification/verified").cloned().unwrap_or(Value::Null)
    })
}

/// commit ของ PR (REST: /pulls/{n}/commits)
fn fetch_pr_commits(repo: &str, number: i64, tk: Option<&str>) -> Result<Vec<Value>, String> {
    let path = format!("/repos/{repo}/pulls/{number}/commits?per_page=100");
    let arr = api_get(&path, tk)?;
    Ok(arr
        .as_array()
        .map(|a| a.iter().map(|c| rest_commit_json(repo, c)).collect())
        .unwrap_or_default())
}

/// ประวัติของ repo: commit บน branch หลัก + PR ที่ merge แล้ว (GraphQL ครั้งเดียว)
fn fetch_history(repo: &str) -> Result<Value, String> {
    let token = github_token().ok_or_else(|| "ไม่มี token (GH_TOKEN/GITHUB_TOKEN/git credential)".to_string())?;
    let (owner, name) = repo
        .split_once('/')
        .ok_or_else(|| format!("repo ต้องอยู่ในรูป owner/name: {repo}"))?;
    let data = api_graphql(HISTORY_QUERY, json!({ "owner": owner, "name": name }), &token)?;
    let r = data
        .get("repository")
        .filter(|v| !v.is_null())
        .ok_or_else(|| format!("ไม่พบ repo {repo} หรือ token ไม่มีสิทธิ์อ่าน"))?;

    let commits: Vec<Value> = r
        .pointer("/defaultBranchRef/target/history/nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|c| gql_commit_json(repo, c)).collect())
        .unwrap_or_default();

    // audit ของ dashboard: ใช้ระบุคนที่สั่ง merge จริง (GitHub เห็นเป็น token ของ server)
    let mut audit_by_pr: HashMap<String, Value> = HashMap::new();
    for a in read_audit() {
        let same_repo = a.get("repo").and_then(|v| v.as_str()).map(|r| r.eq_ignore_ascii_case(repo)).unwrap_or(false);
        if !same_repo || a.get("action").and_then(|v| v.as_str()) != Some("merge") {
            continue;
        }
        if let Some(pr) = a.get("pr").and_then(|v| v.as_i64()) {
            audit_by_pr.entry(pr.to_string()).or_insert(a);
        }
    }

    let mut merges: Vec<Value> = r
        .pointer("/pullRequests/nodes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .map(|p| {
                    let num = p.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                    let merged_at = p.get("mergedAt").cloned().unwrap_or(Value::Null);
                    let sha = p.pointer("/mergeCommit/oid").cloned().unwrap_or(Value::Null);
                    let audit = audit_by_pr.get(&num.to_string());
                    json!({
                        "number": num,
                        "title": p.get("title").cloned().unwrap_or(Value::Null),
                        "url": p.get("url").cloned().unwrap_or(Value::Null),
                        "author": p.pointer("/author/login").cloned().unwrap_or(Value::Null),
                        "merged_by": p.pointer("/mergedBy/login").cloned().unwrap_or(Value::Null),
                        "actor": audit.and_then(|a| a.get("actor")).cloned().unwrap_or(Value::Null),
                        "via_dashboard": audit.is_some(),
                        "merged_at": merged_at,
                        "merged_at_label": rel_time(merged_at.as_str(), now_epoch()),
                        "merge_commit_sha": sha,
                        "short_sha": sha.as_str().map(short_sha).unwrap_or_default(),
                        "base": p.get("baseRefName").cloned().unwrap_or(Value::Null),
                        "head": p.get("headRefName").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    merges.sort_by(|a, b| {
        let ta = a.get("merged_at").and_then(|v| v.as_str()).and_then(iso_epoch).unwrap_or(0);
        let tb = b.get("merged_at").and_then(|v| v.as_str()).and_then(iso_epoch).unwrap_or(0);
        tb.cmp(&ta)
    });

    Ok(json!({ "repo": repo, "commits": commits, "merges": merges }))
}

#[derive(Deserialize)]
struct MergeInfoQuery {
    repo: Option<String>,
    pr: Option<i64>,
}

/// `GET /api/mergeable` — สถานะ merge จริงจาก GitHub ให้ UI เตือนก่อนกด merge
async fn api_mergeable(
    State(st): State<Arc<AppState>>,
    headers: HttpHeaders,
    Query(q): Query<MergeInfoQuery>,
) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = q.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let session = session_of(&st, &headers).await;
    let session_token = session.as_ref().map(|s| s.token.clone());
    let res = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let token = match session_token {
            Some(t) => t,
            None => github_token().ok_or_else(|| "ไม่มี token (GH_TOKEN/GITHUB_TOKEN/git credential)".to_string())?,
        };
        let v = api_get(&format!("/repos/{repo}/pulls/{number}"), Some(&token))?;
        Ok(json!({
            "ok": true,
            "repo": repo,
            "pr": number,
            "title": v.get("title").cloned().unwrap_or(Value::Null),
            "state": v.get("state").cloned().unwrap_or(Value::Null),
            "merged": v.get("merged").and_then(|x| x.as_bool()).unwrap_or(false),
            "draft": v.get("draft").and_then(|x| x.as_bool()).unwrap_or(false),
            "mergeable": v.get("mergeable").cloned().unwrap_or(Value::Null),
            "mergeable_state": v.get("mergeable_state").cloned().unwrap_or(Value::Null),
            "merged_by": v.pointer("/merged_by/login").cloned().unwrap_or(Value::Null),
            "merged_at": v.get("merged_at").cloned().unwrap_or(Value::Null),
            "merge_commit_sha": v.get("merge_commit_sha").cloned().unwrap_or(Value::Null),
            "commits": v.get("commits").cloned().unwrap_or(Value::Null),
            "changed_files": v.get("changed_files").cloned().unwrap_or(Value::Null),
            "additions": v.get("additions").cloned().unwrap_or(Value::Null),
            "deletions": v.get("deletions").cloned().unwrap_or(Value::Null),
            "base": v.pointer("/base/ref").cloned().unwrap_or(Value::Null),
            "head": v.pointer("/head/ref").cloned().unwrap_or(Value::Null),
            "head_sha": v.pointer("/head/sha").cloned().unwrap_or(Value::Null),
            "created_at": v.get("created_at").cloned().unwrap_or(Value::Null),
            "updated_at": v.get("updated_at").cloned().unwrap_or(Value::Null),
        }))
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    match res {
        Ok(v) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
            v.to_string(),
        )
            .into_response(),
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &e),
    }
}

/// รายละเอียด PR เดียว (ใช้เปิด PR ที่ merge แล้วจากหน้าประวัติ)
fn fetch_pr_detail(repo: &str, number: i64, tk: Option<&str>) -> Result<Value, String> {
    let v = api_get(&format!("/repos/{repo}/pulls/{number}"), tk)?;
    let files = fetch_pr_files(repo, number, tk)?;
    let merged = v.get("merged").and_then(|x| x.as_bool()).unwrap_or(false);
    let merged_at = v.get("merged_at").cloned().unwrap_or(Value::Null);
    let updated_at = v.get("updated_at").cloned().unwrap_or(Value::Null);
    let sha = v.get("merge_commit_sha").cloned().unwrap_or(Value::Null);
    let audit = read_audit().into_iter().find(|a| {
        a.get("repo").and_then(|x| x.as_str()).map(|r| r.eq_ignore_ascii_case(repo)).unwrap_or(false)
            && a.get("pr").and_then(|x| x.as_i64()) == Some(number)
            && a.get("action").and_then(|x| x.as_str()) == Some("merge")
    });
    let labels: Vec<Value> = v
        .get("labels")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().map(|l| l.get("name").cloned().unwrap_or(Value::Null)).collect())
        .unwrap_or_default();

    let reviews_raw = api_get(&format!("/repos/{repo}/pulls/{number}/reviews?per_page=100"), tk)
        .ok()
        .and_then(|x| x.as_array().cloned())
        .unwrap_or_default();
    let issue_raw = api_get(&format!("/repos/{repo}/issues/{number}/comments?per_page=100"), tk)
        .ok()
        .and_then(|x| x.as_array().cloned())
        .unwrap_or_default();
    let inline_raw = api_get(&format!("/repos/{repo}/pulls/{number}/comments?per_page=100"), tk)
        .ok()
        .and_then(|x| x.as_array().cloned())
        .unwrap_or_default();

    let reviews: Vec<Value> = reviews_raw
        .iter()
        .map(|r| {
            let state = r.get("state").and_then(|x| x.as_str()).unwrap_or("");
            let at = r.get("submitted_at").and_then(|x| x.as_str());
            json!({
                "id": r.get("id").cloned().unwrap_or(Value::Null),
                "author": r.pointer("/user/login").cloned().unwrap_or(Value::Null),
                "state": state,
                "label": review_event_label(state),
                "body": r.get("body").cloned().unwrap_or(Value::Null),
                "at": at,
                "at_label": rel_time(at, now_epoch()),
                "url": r.get("html_url").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    let mut conversation: Vec<Value> = Vec::new();
    for r in &reviews_raw {
        let state = r.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let at = r.get("submitted_at").and_then(|x| x.as_str());
        conversation.push(json!({
            "kind": "review",
            "author": r.pointer("/user/login").cloned().unwrap_or(Value::Null),
            "body": r.get("body").cloned().unwrap_or(Value::Null),
            "at": at,
            "at_label": rel_time(at, now_epoch()),
            "state": state,
            "label": review_event_label(state),
            "path": Value::Null,
            "line": Value::Null,
            "url": r.get("html_url").cloned().unwrap_or(Value::Null),
        }));
    }
    for c in &issue_raw {
        let at = c.get("created_at").and_then(|x| x.as_str());
        conversation.push(json!({
            "kind": "comment",
            "author": c.pointer("/user/login").cloned().unwrap_or(Value::Null),
            "body": c.get("body").cloned().unwrap_or(Value::Null),
            "at": at,
            "at_label": rel_time(at, now_epoch()),
            "state": Value::Null,
            "label": "คอมเมนต์",
            "path": Value::Null,
            "line": Value::Null,
            "url": c.get("html_url").cloned().unwrap_or(Value::Null),
        }));
    }
    for c in &inline_raw {
        let at = c.get("created_at").and_then(|x| x.as_str());
        conversation.push(json!({
            "kind": "inline",
            "author": c.pointer("/user/login").cloned().unwrap_or(Value::Null),
            "body": c.get("body").cloned().unwrap_or(Value::Null),
            "at": at,
            "at_label": rel_time(at, now_epoch()),
            "state": Value::Null,
            "label": "คอมเมนต์ในไฟล์",
            "path": c.get("path").cloned().unwrap_or(Value::Null),
            "line": c.get("line").cloned().or_else(|| c.get("original_line").cloned()).unwrap_or(Value::Null),
            "url": c.get("html_url").cloned().unwrap_or(Value::Null),
        }));
    }
    conversation.sort_by(|a, b| {
        a.get("at").and_then(|v| v.as_str()).unwrap_or("").cmp(b.get("at").and_then(|v| v.as_str()).unwrap_or(""))
    });

    let head_sha = v.pointer("/head/sha").and_then(|x| x.as_str()).unwrap_or("");
    let combined = if head_sha.is_empty() {
        None
    } else {
        api_get(&format!("/repos/{repo}/commits/{head_sha}/status"), tk).ok()
    };
    let checks_raw = if head_sha.is_empty() {
        None
    } else {
        api_get(&format!("/repos/{repo}/commits/{head_sha}/check-runs?per_page=100"), tk).ok()
    };
    let status_items: Vec<Value> = combined
        .as_ref()
        .and_then(|c| c.get("statuses"))
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .map(|s| {
                    json!({
                        "context": s.get("context").cloned().unwrap_or(Value::Null),
                        "state": s.get("state").cloned().unwrap_or(Value::Null),
                        "description": s.get("description").cloned().unwrap_or(Value::Null),
                        "url": s.get("target_url").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // GitHub คืน check run ทุกครั้งที่รัน (รวมรอบเก่าที่ fail) — เอาเฉพาะ run ล่าสุดต่อชื่อ check
    let mut latest_runs: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    if let Some(a) = checks_raw.as_ref().and_then(|c| c.get("check_runs")).and_then(|x| x.as_array()) {
        for r in a {
            let name = r.get("name").and_then(|x| x.as_str()).unwrap_or("check").to_string();
            let id = r.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
            let newer = latest_runs
                .get(&name)
                .and_then(|old| old.get("id"))
                .and_then(|x| x.as_i64())
                .map(|old_id| id > old_id)
                .unwrap_or(true);
            if newer {
                latest_runs.insert(name, r.clone());
            }
        }
    }
    let runs: Vec<Value> = latest_runs
        .values()
        .map(|r| {
            json!({
                "name": r.get("name").cloned().unwrap_or(Value::Null),
                "status": r.get("status").cloned().unwrap_or(Value::Null),
                "conclusion": r.get("conclusion").cloned().unwrap_or(Value::Null),
                "url": r.get("details_url").cloned().unwrap_or(Value::Null),
                "started_at": r.get("started_at").cloned().unwrap_or(Value::Null),
                "completed_at": r.get("completed_at").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    let mut check_state = if status_items.is_empty() {
        "none".to_string()
    } else {
        combined
            .as_ref()
            .and_then(|c| c.get("state"))
            .and_then(|x| x.as_str())
            .unwrap_or("none")
            .to_string()
    };
    for r in &runs {
        let status = r.get("status").and_then(|x| x.as_str()).unwrap_or("");
        let conclusion = r.get("conclusion").and_then(|x| x.as_str()).unwrap_or("");
        if matches!(conclusion, "failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure") {
            check_state = "failure".to_string();
        } else if status != "completed" && check_state != "failure" {
            check_state = "pending".to_string();
        }
    }
    if check_state == "none" && !runs.is_empty() {
        check_state = "success".to_string();
    }

    Ok(json!({
        "repo": repo,
        "number": number,
        "title": v.get("title").cloned().unwrap_or(Value::Null),
        "url": v.get("html_url").cloned().unwrap_or(Value::Null),
        "author": v.pointer("/user/login").cloned().unwrap_or(Value::Null),
        "state": v.get("state").cloned().unwrap_or(Value::Null),
        "status": if merged { "merged" } else { v.get("state").and_then(|x| x.as_str()).unwrap_or("closed") },
        "merged": merged,
        "draft": v.get("draft").and_then(|x| x.as_bool()).unwrap_or(false),
        "merged_by": v.pointer("/merged_by/login").cloned().unwrap_or(Value::Null),
        "actor": audit.as_ref().and_then(|a| a.get("actor")).cloned().unwrap_or(Value::Null),
        "via_dashboard": audit.is_some(),
        "merged_at": merged_at,
        "merged_at_label": rel_time(merged_at.as_str(), now_epoch()),
        "merge_commit_sha": sha,
        "short_sha": sha.as_str().map(short_sha).unwrap_or_default(),
        "commits": v.get("commits").cloned().unwrap_or(Value::Null),
        "additions": v.get("additions").cloned().unwrap_or(Value::Null),
        "deletions": v.get("deletions").cloned().unwrap_or(Value::Null),
        "changed_files": v.get("changed_files").cloned().unwrap_or(Value::Null),
        "base": v.pointer("/base/ref").cloned().unwrap_or(Value::Null),
        "head": v.pointer("/head/ref").cloned().unwrap_or(Value::Null),
        "head_sha": v.pointer("/head/sha").cloned().unwrap_or(Value::Null),
        "created_at": v.get("created_at").cloned().unwrap_or(Value::Null),
        "updated_at": updated_at,
        "updated_label": rel_time(updated_at.as_str(), now_epoch()),
        "labels": labels,
        "files": files,
        "reviews": reviews,
        "conversation": conversation,
        "checks": {
            "state": check_state,
            "total": status_items.len() + runs.len(),
            "statuses": status_items,
            "runs": runs,
        },
    }))
}

/// `GET /api/pr?repo=&pr=` — รายละเอียด PR เดียว รวม PR ที่ปิด/merge แล้ว
async fn api_pr(State(st): State<Arc<AppState>>, Query(q): Query<MergeInfoQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = q.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let key = format!("{repo}#{number}");
    {
        let cache = st.pr_detail_cache.lock().await;
        if let Some((at, v)) = cache.get(&key) {
            if at.elapsed() < Duration::from_secs(60) {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                    v.to_string(),
                )
                    .into_response();
            }
        }
    }
    let repo_owned = repo.clone();
    let res = tokio::task::spawn_blocking(move || {
        let token = github_token();
        fetch_pr_detail(&repo_owned, number, token.as_deref())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    match res {
        Ok(v) => {
            let mut cache = st.pr_detail_cache.lock().await;
            cache.insert(key, (Instant::now(), v.clone()));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                v.to_string(),
            )
                .into_response()
        }
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &e),
    }
}

#[derive(Deserialize)]
struct RepoQuery {
    repo: Option<String>,
}

/// `GET /api/history?repo=` — commit ล่าสุดบน branch หลัก + ประวัติ merge พร้อมคนที่ merge
async fn api_history(State(st): State<Arc<AppState>>, Query(q): Query<RepoQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    if repo.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "ยังไม่ได้ตั้งค่าโปรเจค");
    }
    {
        let cache = st.history_cache.lock().await;
        if let Some((at, v)) = cache.get(&repo) {
            if at.elapsed() < Duration::from_secs(60) {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                    v.to_string(),
                )
                    .into_response();
            }
        }
    }
    let repo_owned = repo.clone();
    let res = tokio::task::spawn_blocking(move || fetch_history(&repo_owned))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
    match res {
        Ok(v) => {
            let mut cache = st.history_cache.lock().await;
            cache.insert(repo, (Instant::now(), v.clone()));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                v.to_string(),
            )
                .into_response()
        }
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &e),
    }
}

/// `GET /api/commits?repo=&pr=` — commit ทั้งหมดของ PR
async fn api_commits(State(st): State<Arc<AppState>>, Query(q): Query<MergeInfoQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return json_error(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    let number = q.pr.unwrap_or(0);
    if repo.is_empty() || number <= 0 {
        return json_error(StatusCode::BAD_REQUEST, "ต้องระบุ repo และ pr");
    }
    let key = format!("{repo}#{number}");
    {
        let cache = st.commits_cache.lock().await;
        if let Some((at, v)) = cache.get(&key) {
            if at.elapsed() < Duration::from_secs(60) {
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                    json!({ "repo": repo, "pr": number, "commits": v }).to_string(),
                )
                    .into_response();
            }
        }
    }
    let repo_owned = repo.clone();
    let res = tokio::task::spawn_blocking(move || {
        let token = github_token();
        fetch_pr_commits(&repo_owned, number, token.as_deref())
    })
    .await
    .map_err(|e| e.to_string())
    .and_then(|r| r);
    match res {
        Ok(commits) => {
            let mut cache = st.commits_cache.lock().await;
            cache.insert(key, (Instant::now(), commits.clone()));
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
                json!({ "repo": repo, "pr": number, "commits": commits }).to_string(),
            )
                .into_response()
        }
        Err(e) => json_error(StatusCode::BAD_GATEWAY, &e),
    }
}

/// `/api/version` — version (hash) ของเนื้อหาล่าสุด ใช้ poll เบา ๆ แทนการดึงข้อมูลเต็ม
async fn api_version(State(st): State<Arc<AppState>>, Query(q): Query<DataQuery>) -> Response {
    let repo = match q.repo {
        Some(r) if st.repos.iter().any(|x| x.eq_ignore_ascii_case(&r)) => r,
        Some(r) => return text_response(StatusCode::BAD_REQUEST, &format!("ไม่รู้จักโปรเจค: {r}")),
        None => default_repo(&st),
    };
    if repo.is_empty() {
        return text_response(StatusCode::BAD_REQUEST, "ยังไม่ได้ตั้งค่าโปรเจค (--repo/--repos)");
    }
    if let Err(e) = get_page(&st, &repo).await {
        return text_response(StatusCode::BAD_GATEWAY, &format!("โหลดข้อมูลไม่สำเร็จ: {e}"));
    }
    let version = {
        let cache = st.cache.lock().await;
        cache.get(&repo).map(|c| c.version.clone()).unwrap_or_default()
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8"), (header::CACHE_CONTROL, "no-store")],
        json!({ "repo": repo, "version": version }).to_string(),
    )
        .into_response()
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
        let repos = repo_list(&args);
        let viewer = if args.allow_write {
        tokio::task::spawn_blocking(fetch_viewer_login).await.ok().flatten()
    } else {
        None
    };
    if let Some(v) = &viewer {
        eprintln!("dashboard: รีวิวจากเว็บจะโพสต์ในนาม @{v}");
    }
    let auth = {
        let client_id = args.oauth_client_id.clone().filter(|s| !s.trim().is_empty());
        let client_secret = args.oauth_client_secret.clone().filter(|s| !s.trim().is_empty());
        let public_url = args
            .public_url
            .clone()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim_end_matches('/').to_string());
        let fallback_url = {
            let host = if args.host == "0.0.0.0" { "127.0.0.1" } else { args.host.as_str() };
            format!("http://{host}:{}", args.serve.unwrap_or(7788))
        };
        match (client_id, client_secret) {
            (Some(id), Some(secret)) => {
                match &public_url {
                    Some(u) => eprintln!("dashboard: GitHub OAuth เปิดใช้งาน — callback {u}/auth/callback"),
                    None => eprintln!(
                        "dashboard: GitHub OAuth เปิดใช้งาน — callback ตาม Host ที่เบราว์เซอร์เรียก (เช่น http://192.168.x.x:{}/auth/callback) ตั้ง DASHBOARD_PUBLIC_URL เพื่อล็อกค่าตายตัว",
                        args.serve.unwrap_or(7788)
                    ),
                }
                Some(AuthConfig {
                    client_id: id,
                    client_secret: secret,
                    public_url,
                    fallback_url,
                    states: Mutex::new(HashMap::new()),
                    sessions: Mutex::new(HashMap::new()),
                })
            }
            _ => None,
        }
    };
    let state = Arc::new(AppState {
        auth,
        viewer,
            args: args.clone(),
            cfg,
            repos,
            cache: Mutex::new(HashMap::new()),
            pr_cache: Mutex::new(HashMap::new()),
            refreshing: Mutex::new(HashSet::new()),
            diff_cache: Mutex::new(HashMap::new()),
            commits_cache: Mutex::new(HashMap::new()),
            history_cache: Mutex::new(HashMap::new()),
            pr_detail_cache: Mutex::new(HashMap::new()),
            allow_write: args.allow_write,
            web_root,
        });
        let first = default_repo(&state);
        if first.is_empty() {
            eprintln!("⚠️  ยังไม่ได้ระบุ --repo/--repos — /api/data.json จะตอบ 400");
        } else if let Err(e) = get_page(&state, &first).await {
            eprintln!("⚠️  โหลดข้อมูลรอบแรกไม่สำเร็จ: {e}");
        }
        // preload โปรเจคที่เหลือเบื้องหลัง — ครั้งแรกที่สลับโปรเจคจะตอบจาก cache ทันที
        for repo in state.repos.iter().skip(1) {
            spawn_refresh(Arc::clone(&state), repo.clone());
        }
        let app = Router::new()
            .route("/api/data.json", get(api_data))
            .route("/api/projects", get(api_projects))
            .route("/api/profile", get(api_profile))
            .route("/api/version", get(api_version))
            .route("/api/diff", get(api_diff))
            .route("/api/review", post(api_review))
            .route("/api/comment", post(api_comment))
            .route("/api/merge", post(api_merge))
            .route("/api/mergeable", get(api_mergeable))
            .route("/api/commits", get(api_commits))
            .route("/api/history", get(api_history))
            .route("/api/pr", get(api_pr))
            .route("/auth/login", get(auth_login))
            .route("/auth/callback", get(auth_callback))
            .route("/auth/logout", post(auth_logout))
            .route("/login", get(login_page))
            .route("/", get(root))
            .fallback(get(files))
            .layer(axum::middleware::from_fn_with_state(Arc::clone(&state), require_auth))
            .with_state(state);
        let bind = format!("{}:{}", args.host, port);
        let listener = tokio::net::TcpListener::bind(&bind).await.map_err(|e| format!("bind {bind} ไม่ได้: {e}"))?;
        match web_label {
            Some(dir) => println!(
                "dashboard: {}:{} — frontend จาก {dir} + API /api/data.json, /api/version (แคช 10 วิ + refresh เบื้องหลัง, Ctrl+C เพื่อหยุด)",
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
    let repo = repo_list(&args).first().cloned().unwrap_or_default();
    match build_page(&args, &cfg, &repo) {
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
    fn safe_next_allows_only_local_paths() {
        assert_eq!(safe_next(Some("/?view=history&pr=16")), "/?view=history&pr=16");
        assert_eq!(safe_next(Some("/profile")), "/profile");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("/\\evil.example")), "/");
        assert_eq!(safe_next(Some("/auth/login")), "/");
        assert_eq!(safe_next(None), "/");
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
    fn graphql_node_maps_to_snapshot() {
        let node = json!({
            "number": 3, "title": "g", "url": "u", "isDraft": false,
            "createdAt": "2026-11-12T01:00:00Z", "updatedAt": "2026-11-12T02:00:00Z",
            "author": {"login": "a"},
            "labels": {"nodes": [{"name": "x"}]},
            "reviewRequests": {"nodes": [{"requestedReviewer": {"login": "b"}}]},
            "files": {"nodes": [{"path": "f.ts", "additions": 3, "deletions": 1}]},
            "reviews": {"nodes": [{"state": "APPROVED", "submittedAt": "2026-11-12T02:00:00Z", "body": "ok", "author": {"login": "b"}}]},
            "comments": {"nodes": []},
            "reviewThreads": {"nodes": [{"comments": {"nodes": [{"path": "f.ts", "line": 2, "body": "nice", "createdAt": "2026-11-12T02:10:00Z", "author": {"login": "b"}, "replyTo": null}]}}]}
        });
        let pr = graphql_pr_to_snapshot(&node);
        assert_eq!(pr["number"], 3);
        assert_eq!(pr["author"], "a");
        assert_eq!(pr["files"][0]["filename"], "f.ts");
        assert_eq!(pr["reviews"][0]["login"], "b");
        assert_eq!(pr["review_comments"][0]["path"], "f.ts");
        assert_eq!(pr["review_comments"][0]["line"], 2);
    }

    #[test]
    fn inline_comments_sorted_by_file_and_line() {
        let snap = json!({
            "repo": "o/r",
            "pulls": [{
                "number": 8, "title": "z", "author": "a",
                "files": [{ "filename": "b.ts", "additions": 5, "deletions": 0, "patch": "@@ -0,0 +1,5 @@\n+x" }],
                "requested": ["b"], "reviews": [], "bot_state": Value::Null,
                "review_comments": [
                    { "login": "b", "path": "b.ts", "line": 9, "side": "RIGHT", "body": "บรรทัด 9", "created_at": "2026-11-12T05:00:00Z", "reply_to": null },
                    { "login": "b", "path": "a.ts", "line": 2, "side": "RIGHT", "body": "บรรทัด 2", "created_at": "2026-11-12T05:30:00Z", "reply_to": null }
                ]
            }]
        });
        let model = build_model(&snap, &Cfg::default());
        let inline = &model.pulls[0].inline;
        assert_eq!(inline.len(), 2);
        assert_eq!(inline[0].filename, "a.ts");
        assert_eq!(inline[0].line, Some(2));
        assert_eq!(inline[1].filename, "b.ts");
        assert!(!inline[0].at_label.is_empty());
        assert!(model.pulls[0].files[0].patch.as_deref().unwrap_or("").contains("@@"), "patch ต้องติดไปกับ FileEntry");
    }

    #[test]
    fn review_history_is_newest_first() {
        let snap = json!({
            "repo": "o/r",
            "pulls": [{
                "number": 7, "title": "y", "author": "GitNonta",
                "files": [{ "filename": "a.ts", "additions": 10, "deletions": 0, "patch": "@@ -0,0 +1,10 @@\n+x" }],
                "requested": ["Themynon2"],
                "reviews": [
                    { "login": "Themynon2", "state": "CHANGES_REQUESTED", "submitted_at": "2026-11-12T05:00:00Z" },
                    { "login": "Themynon2", "state": "APPROVED", "submitted_at": "2026-11-12T06:00:00Z", "body": "ดูแล้ว โอเค" },
                    { "login": "Themynon2", "state": "PENDING", "submitted_at": "2026-11-12T07:00:00Z" }
                ],
                "bot_state": Value::Null
            }]
        });
        let model = build_model(&snap, &Cfg::default());
        let rv = &model.pulls[0].reviewers[0];
        assert_eq!(rv.status, "approved");
        assert_eq!(rv.history.len(), 2, "PENDING ต้องถูกตัดออก");
        assert_eq!(rv.history[0].state, "APPROVED");
        assert_eq!(rv.history[1].state, "CHANGES_REQUESTED");
        assert_eq!(rv.reviewed_at.as_deref(), Some("2026-11-12T06:00:00Z"));
        assert!(!rv.reviewed_label.as_deref().unwrap_or("").is_empty());
        assert_eq!(rv.history[0].body.as_deref(), Some("ดูแล้ว โอเค"));
        assert_eq!(rv.history[1].body, None, "ไม่มีข้อความ = None");
    }

    #[test]
    fn rel_time_formats_thai() {
        let now = iso_epoch("2026-11-12T10:00:00Z").unwrap();
        assert_eq!(rel_time(Some("2026-11-12T09:59:30Z"), now), "เมื่อสักครู่");
        assert_eq!(rel_time(Some("2026-11-12T07:00:00Z"), now), "3 ชม.ที่แล้ว");
        assert_eq!(rel_time(Some("2026-11-10T10:00:00Z"), now), "2 วันที่แล้ว");
    }
}
