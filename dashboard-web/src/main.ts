/* Auto-review dashboard (Vite frontend)
   ข้อมูลทั้งหมดมาจาก dashboard-rs: GET /api/data.json (proxy ผ่าน vite dev server)
   หน้านี้เป็น read-only: เรนเดอร์ KPI, การ์ด PR, reviewer/ช่วงบรรทัด และภาระงานค้าง */
import './style.css';

type Range = { start: number; end: number };
type Part = { filename: string; ranges: Range[] | null; unsplittable: boolean };
type Reviewer = { login: string; status: string; lines: number; parts: Part[] };
type Pull = {
  number: number;
  title: string;
  url: string | null;
  author: string;
  labels: string[];
  updated_label: string;
  total: number;
  file_count: number;
  large: boolean;
  needs: string | null;
  split: boolean;
  status: string;
  reviewers: Reviewer[];
  unsplittable: boolean;
};
type Workload = { login: string; pending: number; done: number; prs: number };
type Summary = {
  open: number;
  approved: number;
  changes: number;
  review: number;
  needs: number;
  lines: number;
  large: number;
};
type Model = {
  repo: string;
  fetched_label: string;
  threshold: number;
  summary: Summary;
  workload: Workload[];
  pulls: Pull[];
};

const STATUS: Record<string, { label: string; cls: string }> = {
  approved: { label: 'พร้อม merge', cls: 'ok' },
  changes: { label: 'ขอแก้ไข', cls: 'bad' },
  review: { label: 'รอรีวิว', cls: 'warn' },
  needs: { label: 'ยังไม่มี Reviewer', cls: 'bad' },
  draft: { label: 'Draft', cls: 'muted' },
};

const REVIEW_LABEL: Record<string, string> = {
  approved: 'อนุมัติแล้ว',
  changes: 'ขอแก้ไข',
  commented: 'คอมเมนต์แล้ว',
  pending: 'ยังไม่รีวิว',
};

const esc = (v: unknown): string =>
  String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c] as string));

const fmtRanges = (ranges: Range[] | null): string => {
  if (!ranges || !ranges.length) return '';
  return ranges.map((r) => (r.start === r.end ? String(r.start) : `${r.start}–${r.end}`)).join(', ');
};

const state = { model: null as Model | null, filter: 'all', q: '' };

function partsHtml(r: Reviewer): string {
  if (!r.parts.length) return '<div class="parts muted">ไม่มีไฟล์ (ถูกยกเว้นทั้งหมด)</div>';
  const items = r.parts.slice(0, 6).map((p) => {
    const range = p.ranges && p.ranges.length
      ? `บรรทัด ${fmtRanges(p.ranges)}`
      : `ทั้งไฟล์${p.unsplittable ? ' · แยกช่วงไม่ได้' : ''}`;
    return `<li><code>${esc(p.filename)}</code> <span class="muted">${esc(range)}</span></li>`;
  });
  if (r.parts.length > 6) items.push(`<li class="muted">และอีก ${r.parts.length - 6} รายการ</li>`);
  return `<ul class="parts">${items.join('')}</ul>`;
}

function prHtml(p: Pull): string {
  const st = STATUS[p.status] ?? STATUS.draft;
  const badges = [
    `<span class="badge ${st.cls}">${esc(st.label)}</span>`,
    p.large ? '<span class="badge warn">PR ใหญ่</span>' : '',
    p.split ? `<span class="badge info">${p.reviewers.some((r) => r.parts.some((x) => x.ranges && x.ranges.length)) ? 'แบ่งช่วงบรรทัด' : 'แบ่งไฟล์'}</span>` : '',
    p.needs === 'needs_second_reviewer' ? '<span class="badge bad">ขาดคนที่สอง</span>' : '',
    p.unsplittable ? '<span class="badge bad">มีไฟล์แยกช่วงไม่ได้</span>' : '',
  ].join('');

  const reviewers = p.reviewers.length
    ? p.reviewers
        .map(
          (r) => `<div class="rv">
        <div class="rv-head"><b>@${esc(r.login)}</b>
          <span class="tag ${esc(r.status)}">${esc(REVIEW_LABEL[r.status] ?? r.status)}</span>
          <span class="lines">${r.lines} บรรทัด</span></div>
        ${partsHtml(r)}
      </div>`
        )
        .join('')
    : '<div class="rv muted">ยังไม่มี Reviewer — บอทจะลองใหม่ทุกวัน หรือเลือกเองใน PR</div>';

  const labels = p.labels.length
    ? `<div class="labels">${p.labels.map((l) => `<span class="label">${esc(l)}</span>`).join('')}</div>`
    : '';
  const title = p.url
    ? `<a class="title" href="${esc(p.url)}" target="_blank" rel="noreferrer">${esc(p.title)}</a>`
    : `<span class="title">${esc(p.title)}</span>`;
  const search = [p.number, p.title, p.author, ...p.reviewers.map((r) => r.login), ...p.labels].join(' ').toLowerCase();

  return `<article class="pr" data-status="${esc(p.status)}" data-search="${esc(search)}">
  <header class="pr-head">
    <span class="num">#${p.number}</span>
    ${title}
    ${badges}
  </header>
  <div class="meta">@${esc(p.author)} · <b>${p.total}</b> บรรทัด · ${p.file_count} ไฟล์ · อัปเดต ${esc(p.updated_label)}</div>
  ${labels}
  <div class="reviewers">${reviewers}</div>
</article>`;
}

function kpi(label: string, value: number, cls: string): string {
  return `<div class="kpi ${cls}"><div class="kpi-v">${value}</div><div class="kpi-l">${label}</div></div>`;
}

function render(model: Model): void {
  const app = document.getElementById('app');
  if (!app) return;
  const s = model.summary;
  const rows = model.workload.length
    ? model.workload.map((w) => `<tr><td>@${esc(w.login)}</td><td>${w.prs}</td><td>${w.pending}</td><td>${w.done}</td></tr>`).join('')
    : '<tr><td colspan="4" class="muted">ยังไม่มีงานในมือ</td></tr>';

  app.innerHTML = `
<div class="top">
  <div>
    <h1>Auto-review dashboard</h1>
    <div class="muted">${esc(model.repo)} · ข้อมูล ${esc(model.fetched_label)} · เกณฑ์ PR ใหญ่ ${model.threshold} บรรทัด</div>
  </div>
  <span class="muted" id="stamp">อัปเดต ${new Date().toLocaleTimeString('th-TH')}</span>
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
  <input id="q" type="search" placeholder="ค้นหา: เลข PR, ชื่อ, ผู้เขียน, reviewer, label" autocomplete="off" value="${esc(state.q)}">
  <div class="chips">
    ${[['all', 'ทั้งหมด'], ['approved', 'พร้อม merge'], ['review', 'รอรีวิว'], ['changes', 'ขอแก้ไข'], ['needs', 'ไม่มี Reviewer'], ['draft', 'Draft']]
      .map(([k, label]) => `<button class="chip" data-filter="${k}" aria-pressed="${String(state.filter === k)}">${label}</button>`)
      .join('')}
  </div>
</div>

${model.pulls.map(prHtml).join('\n') || '<div class="pr muted">ยังไม่มี PR ที่เปิดอยู่</div>'}
<div id="empty" class="pr muted" style="display:none">ไม่พบ PR ที่ตรงกับตัวกรอง</div>

<h2>ภาระงานค้างต่อคน</h2>
<table>
  <thead><tr><th>Reviewer</th><th>PR</th><th>ค้าง (บรรทัด)</th><th>รีวิวแล้ว (บรรทัด)</th></tr></thead>
  <tbody>${rows}</tbody>
</table>

<footer>อ่านอย่างเดียว · สถานะรีวิวจาก GitHub review ล่าสุดของแต่ละคน · ขอบเขตบรรทัดคำนวณด้วย <code>planScope</code> ตัวเดียวกับบอท</footer>`;

  wire();
  applyFilter();
}

function applyFilter(): void {
  const cards = Array.from(document.querySelectorAll<HTMLElement>('.pr[data-status]'));
  const term = state.q.trim().toLowerCase();
  let shown = 0;
  cards.forEach((c) => {
    const okStatus = state.filter === 'all' || c.dataset.status === state.filter;
    const okTerm = !term || (c.dataset.search || '').includes(term);
    const ok = okStatus && okTerm;
    c.style.display = ok ? '' : 'none';
    if (ok) shown++;
  });
  const empty = document.getElementById('empty');
  if (empty) empty.style.display = cards.length && !shown ? '' : 'none';
}

function wire(): void {
  const q = document.getElementById('q') as HTMLInputElement | null;
  if (q) {
    q.addEventListener('input', () => {
      state.q = q.value;
      applyFilter();
    });
  }
  document.querySelectorAll<HTMLButtonElement>('.chip[data-filter]').forEach((btn) => {
    btn.addEventListener('click', () => {
      state.filter = btn.dataset.filter || 'all';
      document.querySelectorAll<HTMLButtonElement>('.chip[data-filter]').forEach((b) => b.setAttribute('aria-pressed', String(b === btn)));
      applyFilter();
    });
  });
}

async function load(): Promise<void> {
  try {
    const res = await fetch('/api/data.json', { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const model = (await res.json()) as Model;
    state.model = model;
    render(model);
  } catch (e) {
    const loading = document.getElementById('loading');
    if (loading) loading.textContent = `โหลดข้อมูลไม่สำเร็จ: ${(e as Error).message}`;
    const sub = document.getElementById('sub');
    if (sub) sub.textContent = 'ต้องรัน dashboard-rs (--serve) แล้วเปิดผ่าน Vite dev server หรือ reverse proxy';
  }
}

load();
// backend cache 30 วิ — รีเฟรชหน้าให้ตรงกัน
setInterval(load, 30_000);
