'use strict';
/**
 * Dashboard (อ่านอย่างเดียว) สำหรับระบบ Auto-assign Reviewer
 *
 * แสดงต่อ PR ที่เปิดอยู่: ใครเป็น reviewer, รีวิวแล้วหรือยัง, ได้กี่บรรทัด,
 * ช่วงบรรทัด/ไฟล์ที่แต่ละคนต้องดู (คำนวณด้วย planScope ตัวเดียวกับที่บอทใช้),
 * ป้ายสถานะ และภาระงานค้างต่อคน
 *
 * ใช้:
 *   node .github/scripts/dashboard.cjs                       # อ่าน PR จาก GitHub (token จาก env/git credential)
 *   node .github/scripts/dashboard.cjs --out dashboard.html  # กำหนดไฟล์ผลลัพธ์
 *   node .github/scripts/dashboard.cjs --serve 7788          # เปิด http://127.0.0.1:7788 (รีเฟรชข้อมูลทุก 30 วิ)
 *   node .github/scripts/dashboard.cjs --snapshot snap.json  # โหมดออฟไลน์ (ไม่ต้องมี token)
 *
 * ไม่อ้างไลบรารีภายนอก: parse config แบบเบาเอง + ใช้ pure helper จาก auto-review.cjs
 */
const fs = require('fs');
const path = require('path');
const http = require('http');
const { execFileSync } = require('child_process');

const { pure } = require('./auto-review.cjs');
const { globToRegex, hunksFromPatch, mergeRanges, planScope, parseState, sumLines, DEFAULTS } = pure;

const MARK = '<!-- auto-review-state:';
const BOT_LOGIN = 'github-actions[bot]';
const ROOT = path.resolve(__dirname, '..', '..');
const API = 'https://api.github.com';

const STATUS = {
  approved: { label: 'พร้อม merge', cls: 'ok', rank: 1 },
  changes: { label: 'ขอแก้ไข', cls: 'bad', rank: 0 },
  review: { label: 'รอรีวิว', cls: 'warn', rank: 2 },
  needs: { label: 'ยังไม่มี Reviewer', cls: 'bad', rank: 3 },
  draft: { label: 'Draft', cls: 'muted', rank: 4 },
};

const REVIEW_LABEL = {
  approved: 'อนุมัติแล้ว',
  changes: 'ขอแก้ไข',
  commented: 'คอมเมนต์แล้ว',
  pending: 'ยังไม่รีวิว',
};

const norm = (s) => String(s || '').toLowerCase();

/* ------------------------------ helpers ------------------------------ */

const escapeHtml = (v) =>
  String(v ?? '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');

const UNITS = { s: 1e3, m: 60e3, h: 36e5, d: 864e5 };

function relTime(iso, now = Date.now()) {
  const t = Date.parse(iso || '');
  if (!t) return '—';
  const diff = Math.max(0, now - t);
  const mins = diff / UNITS.m;
  if (mins < 1) return 'เมื่อสักครู่';
  if (mins < 60) return `${Math.round(mins)} นาทีที่แล้ว`;
  const hours = diff / UNITS.h;
  if (hours < 48) return `${Math.round(hours)} ชม.ที่แล้ว`;
  const days = diff / UNITS.d;
  if (days < 60) return `${Math.round(days)} วันที่แล้ว`;
  return new Date(t).toISOString().slice(0, 10);
}

/** อ่านค่า config แบบเบา (เฉพาะ key แบน + list) — ไม่ต้องพึ่ง js-yaml */
function readConfigLite(text) {
  const cfg = { ...DEFAULTS };
  const lines = String(text || '').split('\n');
  let key = null;
  for (const raw of lines) {
    if (/^\s*#/.test(raw) || /^\s*$/.test(raw)) continue;
    const kv = /^([A-Za-z_][\w-]*):\s*(.*)$/.exec(raw);
    if (kv) {
      key = kv[1];
      const val = kv[2].replace(/\s+#.*$/, '').trim();
      if (val === '') continue; // ค่าอยู่ใน list ด้านล่าง
      cfg[key] = parseScalar(val);
      key = null;
      continue;
    }
    const item = /^\s+-\s*(.+)$/.exec(raw);
    if (item && key) {
      const v = parseScalar(item[1].trim());
      if (typeof v === 'string' || typeof v === 'number') {
        if (!Array.isArray(cfg[key])) cfg[key] = [];
        cfg[key].push(v);
      }
    }
  }
  return cfg;
}

function parseScalar(v) {
  const s = String(v).trim().replace(/\s+#.*$/, '').trim();
  if (s === '[]') return [];
  if (s === '{}') return {};
  if (s === 'true') return true;
  if (s === 'false') return false;
  if (s === 'null' || s === '~') return null;
  if (/^-?\d+(?:\.\d+)?$/.test(s)) return Number(s);
  return s.replace(/^['"]|['"]$/g, '');
}

function loadConfig(file) {
  const full = path.resolve(ROOT, file || '.github/auto-review.yml');
  try {
    const cfg = readConfigLite(fs.readFileSync(full, 'utf8'));
    return { cfg, source: full };
  } catch (e) {
    return { cfg: { ...DEFAULTS }, source: null, warning: `อ่าน config ไม่ได้ (${e.message}) — ใช้ค่าเริ่มต้น` };
  }
}

function reviewerStatus(review) {
  if (!review) return 'pending';
  if (review.state === 'APPROVED') return 'approved';
  if (review.state === 'CHANGES_REQUESTED') return 'changes';
  return 'commented';
}

function formatRanges(ranges) {
  return (ranges || []).map((r) => (r.start === r.end ? String(r.start) : `${r.start}–${r.end}`)).join(', ');
}

/** รวมรีวิวของแต่ละคน (ใช้สถานะล่าสุด ไม่สน PENDING/ของ author) */
function latestReviews(reviews, author) {
  const map = {};
  for (const r of reviews || []) {
    const login = norm(r.login || r.user?.login);
    if (!login || login === author) continue;
    if (String(r.state || '').toUpperCase() === 'PENDING' || !r.submitted_at) continue;
    const prev = map[login];
    if (!prev || Date.parse(r.submitted_at) > Date.parse(prev.at)) {
      map[login] = { state: String(r.state || '').toUpperCase(), at: r.submitted_at };
    }
  }
  return map;
}

/* ------------------------------ model ------------------------------ */

function buildModel(snapshot, cfgIn = {}) {
  const cfg = { ...DEFAULTS, ...cfgIn };
  const threshold = Number(cfg.large_pr_threshold) || DEFAULTS.large_pr_threshold;
  const exRegex = (cfg.exclude_patterns || []).map(globToRegex);
  const now = Date.now();

  const pulls = (snapshot.pulls || []).map((p) => {
    const author = norm(p.author);
    const files = (p.files || [])
      .filter((f) => !exRegex.some((re) => re.test(f.filename)))
      .map((f) => ({
        filename: f.filename,
        additions: f.additions || 0,
        deletions: f.deletions || 0,
        lines: (f.additions || 0) + (f.deletions || 0),
        ranges: hunksFromPatch(f.patch),
      }));
    const total = sumLines(files);
    const state = p.bot_state || null;
    const reviews = latestReviews(p.reviews, author);
    const requested = (p.requested || []).map((l) => String(l || '')).filter(Boolean);

    const display = {};
    for (const l of requested) if (l) display[norm(l)] ??= l;
    for (const l of Object.keys(reviews)) display[l] ??= p.reviews.find((r) => norm(r.login || r.user?.login) === l)?.login || l;
    for (const l of Object.keys(state?.lines || {})) display[norm(l)] ??= l;

    const logins = [...new Set([...requested.map(norm), ...Object.keys(reviews), ...Object.keys(state?.lines || {}).map(norm)])].filter(
      (l) => l && l !== author
    );
    const split = !!state?.split || (logins.length >= 2 && total > threshold);
    const buckets = logins.length >= 2 ? new Map(planScope(files, logins).map((b) => [b.login, b])) : new Map();

    const reviewers = logins.map((login) => {
      const b = buckets.get(login);
      const parts = split && b
        ? b.parts
        : files.map((f) => ({ filename: f.filename, ranges: null, weight: f.lines, unsplittable: false }));
      const lines = Number(state?.lines?.[login] ?? b?.lines ?? total);
      const status = reviewerStatus(reviews[login]);
      return {
        login: display[login] || login,
        status,
        statusLabel: REVIEW_LABEL[status],
        lines,
        parts,
        submitted_at: reviews[login]?.at || null,
      };
    });

    const rvStatus = reviewers.map((r) => r.status);
    let status;
    if (p.draft) status = 'draft';
    else if (rvStatus.includes('changes')) status = 'changes';
    else if (rvStatus.includes('approved')) status = 'approved';
    else if (!reviewers.length || state?.needs) status = 'needs';
    else status = 'review';

    return {
      number: Number(p.number),
      title: String(p.title || ''),
      url: p.url || null,
      author: p.author || '',
      draft: !!p.draft,
      labels: (p.labels || []).map((l) => (typeof l === 'string' ? l : l.name)).filter(Boolean),
      created_at: p.created_at || null,
      updated_at: p.updated_at || null,
      updated_label: relTime(p.updated_at, now),
      total,
      fileCount: files.length,
      files,
      large: total > threshold,
      needs: state?.needs || null,
      split: split && reviewers.length >= 2,
      status,
      statusLabel: STATUS[status].label,
      statusClass: STATUS[status].cls,
      reviewers,
      unsplittable: reviewers.some((r) => r.parts.some((x) => x.unsplittable)),
    };
  });

  pulls.sort((a, b) => STATUS[a.status].rank - STATUS[b.status].rank || Date.parse(b.updated_at || 0) - Date.parse(a.updated_at || 0));

  const count = (s) => pulls.filter((p) => p.status === s).length;
  const summary = {
    open: pulls.length,
    approved: count('approved'),
    changes: count('changes'),
    review: count('review'),
    needs: count('needs'),
    draft: count('draft'),
    lines: pulls.reduce((s, p) => s + p.total, 0),
    large: pulls.filter((p) => p.large).length,
    unsplittable: pulls.filter((p) => p.unsplittable).length,
  };

  const workload = {};
  for (const p of pulls) {
    for (const r of p.reviewers) {
      const w = (workload[r.login] ??= { login: r.login, pending: 0, done: 0, prs: 0 });
      w.prs++;
      if (r.status === 'approved') w.done += r.lines;
      else w.pending += r.lines;
    }
  }

  return {
    repo: snapshot.repo || '(ไม่ระบุ repo)',
    fetched_at: snapshot.fetched_at || null,
    fetched_label: relTime(snapshot.fetched_at, now),
    threshold,
    summary,
    workload: Object.values(workload).sort((a, b) => b.pending - a.pending || b.done - a.done),
    pulls,
  };
}

/* ------------------------------ render ------------------------------ */

const reviewerParts = (r) => {
  if (!r.parts.length) return '<div class="parts muted">ไม่มีไฟล์ (ถูกยกเว้นทั้งหมด)</div>';
  const shown = r.parts.slice(0, 6).map((p) => {
    const range = p.ranges && p.ranges.length ? `บรรทัด ${formatRanges(mergeRanges(p.ranges))}` : `ทั้งไฟล์${p.unsplittable ? ' · แยกช่วงไม่ได้' : ''}`;
    return `<li><code>${escapeHtml(p.filename)}</code> <span class="muted">${escapeHtml(range)}</span></li>`;
  });
  if (r.parts.length > 6) shown.push(`<li class="muted">และอีก ${r.parts.length - 6} รายการ</li>`);
  return `<ul class="parts">${shown.join('')}</ul>`;
};

function renderPr(p) {
  const reviewers = p.reviewers.length
    ? p.reviewers
        .map(
          (r) => `<div class="rv">
        <div class="rv-head"><b>@${escapeHtml(r.login)}</b>
          <span class="tag ${r.status}">${escapeHtml(r.statusLabel)}</span>
          <span class="lines">${r.lines} บรรทัด</span></div>
        ${reviewerParts(r)}
      </div>`
        )
        .join('')
    : '<div class="rv muted">ยังไม่มี Reviewer — บอทจะลองใหม่ทุกวัน หรือเลือกเองใน PR</div>';

  const search = [p.number, p.title, p.author, ...p.reviewers.map((r) => r.login), ...p.labels].join(' ').toLowerCase();
  const badges = [
    `<span class="badge ${p.statusClass}">${escapeHtml(p.statusLabel)}</span>`,
    p.large ? '<span class="badge warn">PR ใหญ่</span>' : '',
    p.split ? `<span class="badge info">${p.reviewers.some((r) => r.parts.some((x) => x.ranges?.length)) ? 'แบ่งช่วงบรรทัด' : 'แบ่งไฟล์'}</span>` : '',
    p.needs === 'needs_second_reviewer' ? '<span class="badge bad">ขาดคนที่สอง</span>' : '',
    p.unsplittable ? '<span class="badge bad">มีไฟล์แยกช่วงไม่ได้</span>' : '',
  ].join('');

  const labels = p.labels.length ? `<div class="labels">${p.labels.map((l) => `<span class="label">${escapeHtml(l)}</span>`).join('')}</div>` : '';

  return `<article class="pr" data-status="${p.status}" data-search="${escapeHtml(search)}">
  <header class="pr-head">
    <span class="num">#${p.number}</span>
    ${p.url ? `<a class="title" href="${escapeHtml(p.url)}" target="_blank" rel="noreferrer">${escapeHtml(p.title)}</a>` : `<span class="title">${escapeHtml(p.title)}</span>`}
    ${badges}
  </header>
  <div class="meta">@${escapeHtml(p.author)} · <b>${p.total}</b> บรรทัด · ${p.fileCount} ไฟล์ · อัปเดต ${escapeHtml(p.updated_label)}</div>
  ${labels}
  <div class="reviewers">${reviewers}</div>
</article>`;
}

function renderHtml(model) {
  const s = model.summary;
  const kpi = (label, value, cls) => `<div class="kpi ${cls || ''}"><div class="kpi-v">${value}</div><div class="kpi-l">${label}</div></div>`;
  const workload = model.workload.length
    ? model.workload
        .map(
          (w) =>
            `<tr><td>@${escapeHtml(w.login)}</td><td>${w.prs}</td><td>${w.pending}</td><td>${w.done}</td></tr>`
        )
        .join('')
    : '<tr><td colspan="4" class="muted">ยังไม่มีงานในมือ</td></tr>';

  const cards = model.pulls.map(renderPr).join('\n');

  return `<!doctype html>
<html lang="th">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>Auto-review · ${escapeHtml(model.repo)}</title>
<style>
:root{--bg:#0d1117;--panel:#161b22;--panel2:#1c2430;--line:#2a3441;--fg:#e6edf3;--mut:#8b98ad;
--ok:#3fb950;--warn:#d29922;--bad:#f85149;--info:#4493f8;--accent:#4493f8}
*{box-sizing:border-box}
body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.5 -apple-system,"Segoe UI",Roboto,"Noto Sans Thai",sans-serif;padding:16px;padding-bottom:calc(16px + env(safe-area-inset-bottom))}
a{color:var(--info);text-decoration:none}
h1{font-size:19px;margin:0 0 4px}
.top{display:flex;flex-wrap:wrap;gap:8px;align-items:baseline;justify-content:space-between;margin-bottom:12px}
.muted{color:var(--mut)}
.kpis{display:grid;grid-template-columns:repeat(auto-fit,minmax(110px,1fr));gap:8px;margin-bottom:14px}
.kpi{background:var(--panel);border:1px solid var(--line);border-radius:10px;padding:10px 12px}
.kpi-v{font-size:22px;font-weight:700;line-height:1.2}
.kpi-l{font-size:12px;color:var(--mut)}
.kpi.ok .kpi-v{color:var(--ok)} .kpi.bad .kpi-v{color:var(--bad)} .kpi.warn .kpi-v{color:var(--warn)} .kpi.info .kpi-v{color:var(--info)}
.toolbar{display:flex;flex-wrap:wrap;gap:8px;margin-bottom:12px;align-items:center}
input[type=search]{flex:1 1 200px;min-width:160px;background:var(--panel);border:1px solid var(--line);color:var(--fg);border-radius:8px;padding:9px 12px;font-size:15px}
.chips{display:flex;gap:6px;flex-wrap:wrap}
.chip{background:var(--panel);border:1px solid var(--line);color:var(--fg);border-radius:999px;padding:6px 12px;font-size:13px;cursor:pointer}
.chip[aria-pressed=true]{border-color:var(--accent);color:var(--accent);background:var(--panel2)}
.pr{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:12px 14px;margin-bottom:10px}
.pr-head{display:flex;flex-wrap:wrap;gap:8px;align-items:center}
.num{font-weight:700;color:var(--mut)}
.title{font-weight:600;flex:1 1 220px;min-width:0;overflow-wrap:anywhere}
.badge,.tag,.label{font-size:12px;border-radius:999px;padding:2px 9px;border:1px solid transparent;white-space:nowrap}
.badge.ok,.tag.approved{color:var(--ok);border-color:color-mix(in srgb,var(--ok) 45%,transparent)}
.badge.bad,.tag.changes{color:var(--bad);border-color:color-mix(in srgb,var(--bad) 45%,transparent)}
.badge.warn,.tag.commented{color:var(--warn);border-color:color-mix(in srgb,var(--warn) 45%,transparent)}
.badge.info{color:var(--info);border-color:color-mix(in srgb,var(--info) 45%,transparent)}
.badge.muted,.tag.pending{color:var(--mut);border-color:var(--line)}
.meta{font-size:13px;color:var(--mut);margin:6px 0}
.labels{display:flex;gap:6px;flex-wrap:wrap;margin:4px 0 8px}
.label{color:var(--mut);border-color:var(--line)}
.reviewers{display:grid;gap:8px}
.rv{border-top:1px dashed var(--line);padding-top:8px}
.rv:first-child{border-top:0;padding-top:0}
.rv-head{display:flex;gap:8px;align-items:center;flex-wrap:wrap;font-size:14px}
.lines{color:var(--mut);font-size:13px}
.parts{margin:4px 0 0;padding-left:18px;font-size:13px}
.parts li{margin:2px 0;overflow-wrap:anywhere}
code{font-size:12.5px;background:var(--panel2);border:1px solid var(--line);border-radius:5px;padding:1px 5px}
table{width:100%;border-collapse:collapse;font-size:13.5px;margin-top:6px}
th,td{text-align:left;padding:6px 8px;border-bottom:1px solid var(--line)}
th{color:var(--mut);font-weight:600}
td:not(:first-child),th:not(:first-child){text-align:right}
h2{font-size:15px;margin:18px 0 6px}
footer{margin-top:18px;font-size:12px;color:var(--mut)}
@media (max-width:520px){body{padding:12px} .kpi-v{font-size:19px} .title{flex-basis:100%}}
</style>
</head>
<body>
<div class="top">
  <div>
    <h1>Auto-review dashboard</h1>
    <div class="muted">${escapeHtml(model.repo)} · ข้อมูล ${escapeHtml(model.fetched_label)} · เกณฑ์ PR ใหญ่ ${model.threshold} บรรทัด</div>
  </div>
</div>

<section class="kpis">
  ${kpi('PR เปิดอยู่', s.open, '')}
  ${kpi('พร้อม merge', s.approved, 'ok')}
  ${kpi('ขอแก้ไข', s.changes, 'bad')}
  ${kpi('รอรีวิว', s.review, 'warn')}
  ${kpi('ยังไม่มี Reviewer', s.needs, 'bad')}
  ${kpi('บรรทัดรวม', s.lines, 'info')}
  ${kpi('PR ใหญ่', s.large, 'warn')}
</section>

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

${cards || '<div class="pr muted">ยังไม่มี PR ที่เปิดอยู่</div>'}
<div id="empty" class="pr muted" style="display:none">ไม่พบ PR ที่ตรงกับตัวกรอง</div>

<h2>ภาระงานค้างต่อคน</h2>
<table>
  <thead><tr><th>Reviewer</th><th>PR</th><th>ค้าง (บรรทัด)</th><th>รีวิวแล้ว (บรรทัด)</th></tr></thead>
  <tbody>${workload}</tbody>
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
`;
}

/* ------------------------------ data ------------------------------ */

function githubToken() {
  const env = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
  if (env) return env.trim();
  try {
    const out = execFileSync('git', ['credential', 'fill'], {
      input: 'protocol=https\nhost=github.com\n\n',
      encoding: 'utf8',
      stdio: ['pipe', 'pipe', 'ignore'],
    });
    const m = /^password=(.+)$/m.exec(out);
    if (m) return m[1].trim();
  } catch {
    /* ไม่มี credential — จะลองแบบไม่ใส่ token */
  }
  return null;
}

async function fetchSnapshot({ repo, token, cfg, maxPulls = 30 }) {
  if (!repo || !/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error('ต้องระบุ --repo owner/name (หรือตั้ง GITHUB_REPOSITORY)');
  const headers = { Accept: 'application/vnd.github+json', 'User-Agent': 'auto-review-dashboard' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const get = async (p) => {
    const res = await fetch(`${API}${p}`, { headers });
    if (!res.ok) throw new Error(`GitHub ${res.status} ${p}: ${(await res.text()).slice(0, 200)}`);
    return res.json();
  };

  const pulls = await get(`/repos/${repo}/pulls?state=open&sort=updated&direction=desc&per_page=${maxPulls}`);
  const out = [];
  for (const pr of pulls) {
    const [files, reviews, comments] = await Promise.all([
      get(`/repos/${repo}/pulls/${pr.number}/files?per_page=100`),
      get(`/repos/${repo}/pulls/${pr.number}/reviews?per_page=100`),
      get(`/repos/${repo}/issues/${pr.number}/comments?per_page=100`),
    ]);
    const bot = comments
      .filter((c) => c.user?.login === BOT_LOGIN && String(c.body || '').includes(MARK))
      .sort((a, b) => Number(b.id) - Number(a.id))[0];
    out.push({
      number: pr.number,
      title: pr.title,
      url: pr.html_url,
      author: pr.user?.login || '',
      draft: !!pr.draft,
      created_at: pr.created_at,
      updated_at: pr.updated_at,
      labels: (pr.labels || []).map((l) => l.name),
      files: files.map((f) => ({ filename: f.filename, additions: f.additions, deletions: f.deletions, patch: f.patch })),
      reviews: reviews.map((r) => ({ login: r.user?.login, state: r.state, submitted_at: r.submitted_at })),
      requested: (pr.requested_reviewers || []).map((u) => u.login),
      bot_state: bot ? parseState(bot.body) : null,
    });
  }
  return { repo, fetched_at: new Date().toISOString(), pulls: out, config: cfg };
}

/* ------------------------------ CLI ------------------------------ */

function parseArgs(argv) {
  const out = { repo: process.env.GITHUB_REPOSITORY || null, out: 'dashboard.html', snapshot: null, serve: null, host: '127.0.0.1', maxPulls: 30, config: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const [key, inline] = a.split('=');
    const next = () => (inline !== undefined ? inline : argv[++i]);
    if (key === '--repo' || key === '-r') out.repo = next();
    else if (key === '--out' || key === '-o') out.out = next();
    else if (key === '--snapshot' || key === '-s') out.snapshot = next();
    else if (key === '--config' || key === '-c') out.config = next();
    else if (key === '--max-pulls') out.maxPulls = Number(next()) || 30;
    else if (key === '--serve') out.serve = inline !== undefined ? Number(inline) || 7788 : /^\d+$/.test(argv[i + 1] || '') ? Number(argv[++i]) : 7788;
    else if (key === '--host') out.host = next();
    else if (key === '--help' || key === '-h') out.help = true;
    else throw new Error(`ไม่รู้จัก flag: ${a}`);
  }
  return out;
}

async function main(argv = process.argv.slice(2)) {
  const args = parseArgs(argv);
  if (args.help) {
    console.log(`ใช้: node .github/scripts/dashboard.cjs [options]
  --repo owner/name   repo ที่จะอ่าน (หรือ env GITHUB_REPOSITORY)
  --out file          ไฟล์ HTML ผลลัพธ์ (ค่าเริ่มต้น dashboard.html)
  --snapshot file     อ่านข้อมูลจาก JSON ที่บันทึกไว้ แทนการยิง GitHub
  --config file       config (ค่าเริ่มต้น .github/auto-review.yml)
  --max-pulls n       จำนวน PR ที่ดึง (ค่าเริ่มต้น 30)
  --serve [port]      เสิร์ฟที่ 127.0.0.1 (ค่าเริ่มต้น 7788) รีเฟรชข้อมูลทุก 30 วิ
  --host addr         bind address ของ --serve (127.0.0.1 หรือ 0.0.0.0 สำหรับดูจากเครื่องอื่น)
  token: env GH_TOKEN / GITHUB_TOKEN หรือ git credential ของ github.com`);
    return;
  }

  const { cfg, warning } = loadConfig(args.config);
  if (warning) console.error(`⚠️  ${warning}`);

  const snapshotFrom = async () => {
    if (args.snapshot) {
      const raw = JSON.parse(fs.readFileSync(path.resolve(args.snapshot), 'utf8'));
      return { ...raw, config: raw.config || null };
    }
    return fetchSnapshot({ repo: args.repo, token: githubToken(), cfg, maxPulls: args.maxPulls });
  };

  if (args.serve) {
    let cache = null;
    let cachedAt = 0;
    let building = null;
    // สร้าง cache รอบแรกก่อน listen เพื่อไม่ให้ request แรกค้างนาน (ช้าเฉพาะตอนข้อมูลหมดอายุ)
    const refresh = async (force = false) => {
      if (!force && cache && Date.now() - cachedAt <= 30e3) return cache;
      if (!building) {
        building = (async () => {
          cache = renderHtml(buildModel(await snapshotFrom(), cfg));
          cachedAt = Date.now();
          return cache;
        })().finally(() => {
          building = null;
        });
      }
      return building;
    };
    try {
      await refresh();
    } catch (e) {
      console.error(`⚠️  โหลดข้อมูลรอบแรกไม่สำเร็จ: ${e.message}`);
    }
    const server = http.createServer(async (req, res) => {
      try {
        const html = await refresh();
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Cache-Control': 'no-store' });
        res.end(html);
      } catch (e) {
        res.writeHead(502, { 'Content-Type': 'text/plain; charset=utf-8' });
        res.end(`โหลดข้อมูลไม่สำเร็จ: ${e.message}`);
      }
    });
    server.listen(args.serve, args.host, () => {
      const where = args.host === '0.0.0.0' ? `0.0.0.0:${args.serve} (เปิดจากเครื่องอื่นที่ http://<ip-ของเครื่อง>:${args.serve})` : `${args.host}:${args.serve}`;
      console.log(`dashboard: ${where} — รีเฟรชทุก 30 วิ (Ctrl+C เพื่อหยุด)`);
    });
    return;
  }

  const snap = await snapshotFrom();
  const model = buildModel(snap, cfg);
  const html = renderHtml(model);
  const outFile = path.resolve(process.cwd(), args.out);
  fs.mkdirSync(path.dirname(outFile), { recursive: true });
  fs.writeFileSync(outFile, html);
  const s = model.summary;
  console.log(
    `✅ ${outFile}\n   ${model.repo}: เปิด ${s.open} · พร้อม merge ${s.approved} · รอรีวิว ${s.review} · ขอแก้ไข ${s.changes} · ไม่มี reviewer ${s.needs} · ${s.lines} บรรทัด`
  );
}

module.exports = main;
module.exports.pure = { buildModel, renderHtml, readConfigLite, parseScalar, reviewerStatus, relTime, formatRanges, escapeHtml, parseArgs };

if (require.main === module) {
  main().catch((e) => {
    console.error(`❌ ${e.message}`);
    process.exit(1);
  });
}
