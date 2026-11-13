/* Auto-review dashboard (Vite frontend)
   ข้อมูลทั้งหมดมาจาก dashboard-rs:
     GET /api/projects              → รายการโปรเจคที่เลือกได้
     GET /api/data.json?repo=a/b    → ข้อมูลของโปรเจคที่เลือก
   หน้าจอมีสองโหมด (จำใน URL):
     ?repo=a/b              → รายการ PR + ภาระงานต่อคน + เปิดดูงานรายคน (?reviewer=)
     ?repo=a/b&pr=123       → หน้ารายละเอียด PR ในตัว (ไม่ต้องเด้งไป GitHub)
   หน้านี้เป็น read-only */
import './style.css';

type Range = { start: number; end: number };
type Part = { filename: string; ranges: Range[] | null; unsplittable: boolean };
type FileEntry = { filename: string; lines: number; patch: string | null };
type ReviewEvent = { state: string; label: string; at: string; at_label: string; body: string | null };
type InlineComment = {
  filename: string;
  line: number | null;
  side: string | null;
  login: string;
  body: string;
  at: string;
  at_label: string;
  reply_to: string | null;
};
type LineDraft = {
  filename: string;
  line: number;
  side: 'LEFT' | 'RIGHT';
  body: string;
};
type Reviewer = {
  login: string;
  status: string;
  lines: number;
  parts: Part[];
  reviewed_at: string | null;
  reviewed_label: string | null;
  history: ReviewEvent[];
};
type Pull = {
  number: number;
  title: string;
  url: string | null;
  author: string;
  labels: string[];
  updated_at: string | null;
  updated_label: string;
  total: number;
  file_count: number;
  files: FileEntry[];
  inline: InlineComment[];
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
type ProjectsResponse = { projects: { repo: string; label: string }[]; default: string; write?: boolean; viewer?: string | null; auth?: { enabled?: boolean; login?: string | null } };
type CommitEntry = {
  sha: string;
  short: string;
  message: string;
  summary: string;
  author_login: string | null;
  author_name: string | null;
  committer_login: string | null;
  committer_name: string | null;
  date: string | null;
  date_label: string;
  url: string | null;
  parents: number | null;
  verified: boolean | null;
};
type MergeEntry = {
  number: number;
  title: string;
  url: string | null;
  author: string | null;
  merged_by: string | null;
  actor: string | null;
  via_dashboard: boolean;
  merged_at: string | null;
  merged_at_label: string;
  merge_commit_sha: string | null;
  short_sha: string;
  base: string | null;
  head: string | null;
};
type HistoryResponse = { repo: string; commits: CommitEntry[]; merges: MergeEntry[] };
type ReviewItem = {
  id: number | null;
  author: string | null;
  state: string;
  label: string;
  body: string | null;
  at: string | null;
  at_label: string;
  url: string | null;
};
type ConversationItem = {
  kind: 'review' | 'comment' | 'inline';
  author: string | null;
  body: string | null;
  at: string | null;
  at_label: string;
  state: string | null;
  label: string;
  path: string | null;
  line: number | null;
  url: string | null;
};
type CheckStatus = { context: string | null; state: string | null; description: string | null; url: string | null };
type CheckRun = {
  name: string | null;
  status: string | null;
  conclusion: string | null;
  url: string | null;
  started_at: string | null;
  completed_at: string | null;
};
type ChecksInfo = { state: string; total: number; statuses: CheckStatus[]; runs: CheckRun[] };
type ProfileOrg = {
  login?: string;
  avatar_url?: string;
  description?: string | null;
  html_url?: string;
  url?: string;
};
type ProfileEmail = { email?: string; primary?: boolean; verified?: boolean; visibility?: string | null };
type ProfileSocial = { provider?: string; url?: string };
type ProfileResponse = {
  user: Record<string, unknown>;
  organizations?: ProfileOrg[] | null;
  emails?: ProfileEmail[] | null;
  social_accounts?: ProfileSocial[] | null;
};
type HistoryPr = {
  repo: string;
  number: number;
  title: string;
  url: string | null;
  author: string | null;
  state: string;
  status: string;
  merged: boolean;
  draft: boolean;
  merged_by: string | null;
  actor: string | null;
  via_dashboard: boolean;
  merged_at: string | null;
  merged_at_label: string;
  merge_commit_sha: string | null;
  short_sha: string;
  commits: number | null;
  additions: number | null;
  deletions: number | null;
  changed_files: number | null;
  base: string | null;
  head: string | null;
  head_sha: string | null;
  created_at: string | null;
  updated_at: string | null;
  updated_label: string;
  labels: string[];
  files: FileEntry[];
  reviews: ReviewItem[];
  conversation: ConversationItem[];
  checks: ChecksInfo;
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

/* ไอคอน SVG แบบ inline — ไม่ต้องโหลดฟอนต์/ไลบรารีเพิ่ม และรับสีจาก currentColor */
const ICONS = {
  grid: '<rect x="3" y="3" width="7" height="7" rx="1.6"/><rect x="14" y="3" width="7" height="7" rx="1.6"/><rect x="3" y="14" width="7" height="7" rx="1.6"/><rect x="14" y="14" width="7" height="7" rx="1.6"/>',
  inbox: '<path d="M4 5h16v14H4z"/><path d="M4 14h4l2 3h4l2-3h4"/>',
  users: '<path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/>',
  history: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l3 2"/>',
  user: '<circle cx="12" cy="8" r="4"/><path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m21 21-4.3-4.3"/>',
  filter: '<path d="M4 6h16"/><path d="M7 12h10"/><path d="M10 18h4"/>',
  arrowLeft: '<path d="M19 12H5"/><path d="m12 19-7-7 7-7"/>',
  external: '<path d="M14 3h7v7"/><path d="M10 14 21 3"/><path d="M21 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5"/>',
  chevronRight: '<path d="m9 18 6-6-6-6"/>',
  gitPr: '<circle cx="6" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><circle cx="18" cy="18" r="2.4"/><path d="M6 8.4v7.2"/><path d="M18 15.6v-6a3 3 0 0 0-3-3h-3"/><path d="m14 4-2 2 2 2"/>',
  gitMerge: '<circle cx="6" cy="6" r="2.4"/><circle cx="6" cy="18" r="2.4"/><circle cx="18" cy="6" r="2.4"/><path d="M6 8.4v7.2"/><path d="M18 8.4v2a4 4 0 0 1-4 4H9"/>',
  check: '<path d="M20 6 9 17l-5-5"/>',
  checkCircle: '<circle cx="12" cy="12" r="9"/><path d="m8.5 12 2.5 2.5 4.5-5"/>',
  clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
  alert: '<circle cx="12" cy="12" r="9"/><path d="M12 8v5"/><path d="M12 16h.01"/>',
  code: '<path d="m9 18-6-6 6-6"/><path d="m15 6 6 6-6 6"/>',
  message: '<path d="M21 15a4 4 0 0 1-4 4H7l-4 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"/>',
  sparkles: '<path d="m12 3 1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="m19 15 .8 2.2L22 18l-2.2.8L19 21l-.8-2.2L16 18l2.2-.8z"/>',
  logout: '<path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><path d="m16 17 5-5-5-5"/><path d="M21 12H9"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2 5.3"/><path d="M20 4v7h-7"/>',
  lock: '<rect x="4" y="10" width="16" height="11" rx="2"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
  shield: '<path d="M12 3 4 6v5c0 5 3.5 8.5 8 10 4.5-1.5 8-5 8-10V6z"/><path d="m8.5 12 2.2 2.2 4.8-4.8"/>',
  eye: '<path d="M2 12s3.5-6 10-6 10 6 10 6-3.5 6-10 6S2 12 2 12z"/><circle cx="12" cy="12" r="2.5"/>',
  mail: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="m3 7 9 6 9-6"/>',
  building: '<path d="M4 21V4h10v17"/><path d="M14 9h6v12"/><path d="M8 8h2M8 12h2M8 16h2M17 13h.01M17 17h.01"/>',
  link: '<path d="M10 13a5 5 0 0 0 7 0l2-2a5 5 0 0 0-7-7l-1 1"/><path d="M14 11a5 5 0 0 0-7 0l-2 2a5 5 0 0 0 7 7l1-1"/>',
  calendar: '<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M16 3v4M8 3v4M3 11h18"/>',
  file: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z"/><path d="M14 3v6h6"/>',
  folder: '<path d="M3 6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5z"/><path d="m3 12 9 5 9-5"/><path d="m3 16 9 5 9-5"/>',
} as const;

type IconName = keyof typeof ICONS;

function icon(name: IconName, cls = 'ico'): string {
  return `<svg class="${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${ICONS[name]}</svg>`;
}

/** empty state ที่มีไอคอนและคำอธิบายสั้น ๆ แทนข้อความเทา ๆ */
function emptyState(name: IconName, title: string, text: string, compact = false): string {
  return `<div class="empty-state${compact ? ' compact' : ''}"><span class="empty-ico">${icon(name)}</span><div><b>${esc(title)}</b><p>${esc(text)}</p></div></div>`;
}

const fmtRanges = (ranges: Range[] | null): string => {
  if (!ranges || !ranges.length) return '';
  return ranges.map((r) => (r.start === r.end ? String(r.start) : `${r.start}–${r.end}`)).join(', ');
};

const query = (): URLSearchParams => new URLSearchParams(location.search);

function setParam(key: string, value: string | null): void {
  const p = query();
  if (value) p.set(key, value);
  else p.delete(key);
  const qs = p.toString();
  history.replaceState(null, '', qs ? `?${qs}` : location.pathname);
}

const parsedPr = Number(query().get('pr'));

type View = 'list' | 'mine' | 'people' | 'history' | 'profile';
type DetailTab = 'changes' | 'status' | 'conversation';
type WlSort = 'pending' | 'total' | 'prs' | 'done' | 'name';
const viewOf = (v: string | null): View =>
  v === 'mine' || v === 'people' || v === 'history' || v === 'profile' ? v : 'list';

const state = {
  model: null as Model | null,
  view: viewOf(query().get('view')),
  projects: [] as string[],
  repo: query().get('repo') || '',
  filter: 'all',
  wlHide: false,
  wlSort: 'pending' as WlSort,
  q: '',
  person: query().get('person'),
  pr: viewOf(query().get('view')) !== 'history' && Number.isFinite(parsedPr) && parsedPr > 0 ? parsedPr : null,
  historyPr: viewOf(query().get('view')) === 'history' && Number.isFinite(parsedPr) && parsedPr > 0 ? parsedPr : null,
  canWrite: false,
  viewer: '',
  authEnabled: false,
  login: '',
  mergeConfirming: false,
  merging: false,
  history: null as HistoryResponse | null,
  historyRepo: '',
  historyError: '',
  historyLoading: false,
  historyPrError: '',
  detailTab: 'changes' as DetailTab,
  lineDraft: null as LineDraft | null,
  profile: null as ProfileResponse | null,
  profileLoading: false,
  profileError: '',
};

// session หมดอายุ/ยังไม่ล็อกอินระหว่างใช้งาน → กลับไปล็อกอินทันที
const nativeFetch = window.fetch.bind(window);
window.fetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
  const res = await nativeFetch(input, init);
  if (res.status === 401) {
    const next = location.pathname + location.search;
    location.assign(`/login?next=${encodeURIComponent(next)}`);
    throw new Error('ต้องเข้าสู่ระบบ');
  }
  return res;
};

/** ลิงก์ภายในสำหรับ PR ที่เปิดจากหน้าประวัติ */
function historyPrHref(n: number): string {
  const p = query();
  p.set('view', 'history');
  p.set('pr', String(n));
  if (state.repo) p.set('repo', state.repo);
  p.delete('person');
  p.delete('reviewer');
  return `?${p.toString()}`;
}

/** ลิงก์ภายในสำหรับหน้า detail ของ PR (คง repo ไว้, ไม่พา reviewer ติดไป) */
function prHref(n: number): string {
  const p = query();
  p.set('pr', String(n));
  if (state.repo) p.set('repo', state.repo);
  p.delete('reviewer');
  return `?${p.toString()}`;
}

function partsHtml(r: Reviewer, limit = 6): string {
  if (!r.parts.length) return '<div class="parts muted">ไม่มีไฟล์ (ถูกยกเว้นทั้งหมด)</div>';
  const items = r.parts.slice(0, limit).map((p) => {
    const range = p.ranges && p.ranges.length
      ? `บรรทัด ${fmtRanges(p.ranges)}`
      : `ทั้งไฟล์${p.unsplittable ? ' · แยกช่วงไม่ได้' : ''}`;
    return `<li><code>${esc(p.filename)}</code> <span class="muted">${esc(range)}</span></li>`;
  });
  if (r.parts.length > limit) items.push(`<li class="muted">และอีก ${r.parts.length - limit} รายการ</li>`);
  return `<ul class="parts">${items.join('')}</ul>`;
}

const who = (login: string): string => `<button class="who" data-login="${esc(login)}" type="button">${icon('user', 'ico-xs')}<span>@${esc(login)}</span></button>`;

const snippet = (s: string, n = 180): string => (s.length > n ? `${s.slice(0, n).trimEnd()}…` : s);

function eventClass(state: string): string {
  if (state === 'APPROVED') return 'approved';
  if (state === 'CHANGES_REQUESTED') return 'changes';
  if (state === 'COMMENTED') return 'commented';
  return 'pending';
}

/** ประวัติรีวิวของคนหนึ่งใน PR หนึ่ง (ใหม่สุดก่อน) */
function historyHtml(r: Reviewer): string {
  if (!r.history || !r.history.length) return '';
  const items = r.history
    .map(
      (ev) => `<li><span class="tag ${eventClass(ev.state)}">${esc(ev.label)}</span> <span class="muted">${esc(ev.at_label)} · ${esc(ev.at.slice(0, 10))}</span>${
        ev.body ? `<div class="rv-body">${esc(ev.body)}</div>` : ''
      }</li>`
    )
    .join('');
  return `<ul class="history">${items}</ul>`;
}

function badgesFor(p: Pull): string {
  const st = STATUS[p.status] ?? STATUS.draft;
  return [
    `<span class="badge ${st.cls}">${esc(st.label)}</span>`,
    p.large ? '<span class="badge warn">PR ใหญ่</span>' : '',
    p.split ? `<span class="badge info">${p.reviewers.some((r) => r.parts.some((x) => x.ranges && x.ranges.length)) ? 'แบ่งช่วงบรรทัด' : 'แบ่งไฟล์'}</span>` : '',
    p.needs === 'needs_second_reviewer' ? '<span class="badge bad">ขาดคนที่สอง</span>' : '',
    p.unsplittable ? '<span class="badge bad">มีไฟล์แยกช่วงไม่ได้</span>' : '',
  ].join('');
}

function labelsFor(p: Pull): string {
  return p.labels.length
    ? `<div class="labels">${p.labels.map((l) => `<span class="label">${esc(l)}</span>`).join('')}</div>`
    : '';
}

function prHtml(p: Pull): string {
  const reviewers = p.reviewers.length
    ? p.reviewers
        .map(
          (r) => `<div class="rv">
        <div class="rv-head">${who(r.login)}
          <span class="tag ${esc(r.status)}">${esc(REVIEW_LABEL[r.status] ?? r.status)}</span>
          <span class="lines">${r.lines} บรรทัด</span></div>
        ${partsHtml(r)}
      </div>`
        )
        .join('')
    : '<div class="rv muted">ยังไม่มี Reviewer — บอทจะลองใหม่ทุกวัน หรือเลือกเองใน PR</div>';

  const search = [p.number, p.title, p.author, ...p.reviewers.map((r) => r.login), ...p.labels].join(' ').toLowerCase();
  const people = p.reviewers.map((r) => r.login.toLowerCase()).join('|');
  const days = ageDays(p.updated_at);

  return `<article class="pr" data-status="${esc(p.status)}" data-search="${esc(search)}" data-reviewers="${esc(people)}" data-pr-number="${p.number}" data-large="${p.large ? '1' : '0'}" data-stale="${days != null && days >= 7 ? '1' : '0'}">
  <header class="pr-head">
    <span class="num">#${p.number}</span>
    <a class="title" data-pr="${p.number}" href="${esc(prHref(p.number))}"><span>${esc(p.title)}</span>${icon('chevronRight', 'ico-xs title-chev')}</a>
    ${badgesFor(p)}
  </header>
  <div class="meta">@${esc(p.author)} · <b>${p.total}</b> บรรทัด · ${p.file_count} ไฟล์ · อัปเดต ${esc(p.updated_label)}</div>
  ${labelsFor(p)}
  <div class="reviewers">${reviewers}</div>
</article>`;
}

/** แท่งภาระงานต่อคน (SVG): เทา = ค้าง, ฟ้า = รีวิวแล้ว; ยาวตามคนที่งานมากสุด */
function workloadBar(w: Workload, max: number): string {
  const total = w.pending + w.done;
  const scale = Math.max(max, 1);
  const pw = Math.min(100, Math.round((w.pending / scale) * 1000) / 10);
  const dw = Math.min(100 - pw, Math.round((w.done / scale) * 1000) / 10);
  const label = `ค้าง ${w.pending} บรรทัด · รีวิวแล้ว ${w.done} บรรทัด`;
  return `<svg class="wbar" viewBox="0 0 100 8" preserveAspectRatio="none" role="img" aria-label="${esc(label)}">
      <title>${esc(label)}</title>
      <rect class="track" x="0" y="0" width="100" height="8" rx="4"></rect>
      ${w.pending ? `<rect class="pend" x="0" y="0" width="${pw}" height="8"></rect>` : ''}
      ${w.done ? `<rect class="done" x="${pw}" y="0" width="${dw}" height="8"></rect>` : ''}
    </svg>${total ? '' : '<span class="muted">—</span>'}`;
}

function wlList(model: Model, hide: boolean, sort: WlSort): Workload[] {
  const rows = model.workload.filter((w) => !hide || w.pending + w.done + w.prs > 0);
  const key = (w: Workload): number => (sort === 'done' ? w.done : sort === 'total' ? w.pending + w.done : sort === 'prs' ? w.prs : w.pending);
  if (sort === 'name') return rows.sort((a, b) => a.login.toLowerCase().localeCompare(b.login.toLowerCase()));
  return rows.sort((a, b) => key(b) - key(a) || b.pending - a.pending || a.login.localeCompare(b.login));
}

function workloadRowsHtml(model: Model): string {
  const list = wlList(model, state.wlHide, state.wlSort);
  if (!list.length) return `<tr><td colspan="5">${emptyState('users', 'ไม่มีคนที่ตรงกับตัวกรอง', 'ลองเปิดคนที่ยังไม่มีงานหรือล้างตัวกรอง', true)}</td></tr>`;
  const max = Math.max(1, ...list.map((w) => w.pending + w.done));
  return list
    .map(
      (w) => `<tr data-login="${esc(w.login)}"><td>${who(w.login)}</td><td>${w.prs}</td><td>${w.pending}</td><td>${w.done}</td><td class="wl">${workloadBar(w, max)}</td></tr>`
    )
    .join('');
}

const WL_SORTS: [WlSort, string][] = [
  ['pending', 'ค้างมากสุด'],
  ['total', 'งานรวมมากสุด'],
  ['prs', 'จำนวน PR'],
  ['done', 'รีวิวแล้วมากสุด'],
  ['name', 'ชื่อ A→Z'],
];

function workloadTableHtml(model: Model): string {
  const shown = wlList(model, state.wlHide, state.wlSort).length;
  return `<div class="wl-tools">
  <label><input type="checkbox" id="wl-hide"${state.wlHide ? ' checked' : ''}> ซ่อนคนที่ไม่มีงาน</label>
  <label>เรียงตาม <select id="wl-sort">${WL_SORTS.map(([v, label]) => `<option value="${v}"${state.wlSort === v ? ' selected' : ''}>${label}</option>`).join('')}</select></label>
  <span id="wl-count">แสดง ${shown}/${model.workload.length} คน</span>
</div>
<div class="table-wrap">
<table class="workload">
  <thead><tr><th>Reviewer</th><th>PR</th><th>ค้าง (บรรทัด)</th><th>รีวิวแล้ว (บรรทัด)</th><th>สัดส่วน</th></tr></thead>
  <tbody id="wl-body">${workloadRowsHtml(model)}</tbody>
</table>
</div>`;
}

function renderWorkloadBody(): void {
  if (!state.model) return;
  const body = document.getElementById('wl-body');
  if (body) body.innerHTML = workloadRowsHtml(state.model);
  const count = document.getElementById('wl-count');
  if (count) count.textContent = `แสดง ${wlList(state.model, state.wlHide, state.wlSort).length}/${state.model.workload.length} คน`;
  wireRows();
}

function wireRows(): void {
  document.querySelectorAll<HTMLTableRowElement>('tr[data-login]').forEach((row) => {
    row.addEventListener('click', () => openPerson(row.dataset.login || null));
  });
}

const KPI_ICONS: Record<string, IconName> = {
  'PR เปิดอยู่': 'gitPr',
  'พร้อม merge': 'checkCircle',
  'ขอแก้ไข': 'alert',
  'รอรีวิว': 'clock',
  'ยังไม่มี Reviewer': 'alert',
  'บรรทัดรวม': 'file',
  'PR ใหญ่': 'layers',
  'PR ที่ได้รับ': 'gitPr',
  'ค้าง (บรรทัด)': 'clock',
  'รีวิวแล้ว (บรรทัด)': 'checkCircle',
  'ไฟล์ที่ต้องดู': 'file',
  'PR ของฉัน': 'gitPr',
  'รอฉันรีวิว': 'clock',
  'รีวิวแล้ว': 'checkCircle',
  'บรรทัดรอรีวิว': 'file',
  'PR merge แล้ว': 'gitMerge',
  'Commit บน branch หลัก': 'code',
  'Merge ผ่าน dashboard': 'sparkles',
  'Merge บน GitHub': 'external',
  'Public repos': 'folder',
  'Public gists': 'file',
  'Followers': 'users',
  'Following': 'user',
};

type MetricAction =
  | { kind: 'filter'; filter: string; anchor?: string }
  | { kind: 'hash'; anchor: string }
  | { kind: 'link'; href: string; nav?: View; external?: boolean };

/** การ์ดตัวเลขที่กดได้: filter = กรองรายการ, hash = เลื่อนไป section, link = เปิดหน้าปลายทาง */
function metricCard(tag: 'div' | 'a' | 'button', cls: string, action: MetricAction | undefined, inner: string): string {
  if (!action) return `<${tag} class="${cls}">${inner}</${tag}>`;
  if (action.kind === 'filter') {
    return `<button type="button" class="${cls}" data-metric-filter="${esc(action.filter)}"${
      action.anchor ? ` data-metric-anchor="${esc(action.anchor)}"` : ''
    } aria-pressed="${String(state.filter === action.filter)}">${inner}</button>`;
  }
  if (action.kind === 'hash') return `<a class="${cls}" href="#${esc(action.anchor)}">${inner}</a>`;
  return `<a class="${cls}" href="${esc(action.href)}"${action.nav ? ` data-nav="${esc(action.nav)}"` : ''}${
    action.external ? ' target="_blank" rel="noreferrer"' : ''
  }>${inner}</a>`;
}

function kpi(label: string, value: number, cls: string, action?: MetricAction): string {
  const inner = `<span class="kpi-ico">${icon(KPI_ICONS[label] || 'grid')}</span><div class="kpi-v">${value}</div><div class="kpi-l">${label}</div>`;
  return metricCard(action ? (action.kind === 'filter' ? 'button' : 'a') : 'div', `kpi ${cls}`.trim(), action, inner);
}

const pct = (n: number, d: number): number => (d > 0 ? Math.round((n / d) * 100) : 0);

function median(values: number[]): number {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : Math.round((sorted[mid - 1] + sorted[mid]) / 2);
}

/** อายุของ PR เป็นวัน (นับจาก updated_at) */
function ageDays(iso: string | null | undefined): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return null;
  return Math.max(0, Math.floor((Date.now() - t) / 86_400_000));
}

const STATUS_VIEW: { key: string; label: string; cls: string }[] = [
  { key: 'approved', label: 'พร้อม merge', cls: 'approved' },
  { key: 'review', label: 'รอรีวิว', cls: 'review' },
  { key: 'changes', label: 'ขอแก้ไข', cls: 'changes' },
  { key: 'needs', label: 'ไม่มี Reviewer', cls: 'needs' },
  { key: 'draft', label: 'Draft', cls: 'draft' },
];

/** เหตุผลความเสี่ยงของ PR หนึ่งรายการ (ใช้จัดอันดับ “จุดที่ควรดูก่อน”) */
function riskReasons(p: Pull): string[] {
  const reasons: string[] = [];
  if (p.status === 'needs') reasons.push('ยังไม่มี reviewer');
  if (p.status === 'changes') reasons.push('ขอแก้ไขค้างอยู่');
  if (p.large) reasons.push(`ใหญ่ ${p.total} บรรทัด`);
  const days = ageDays(p.updated_at);
  if (days != null && days >= 7) reasons.push(`ไม่ขยับ ${days} วัน`);
  return reasons;
}

/** ภาพรวมเชิงวิเคราะห์: สถิติ + กราฟสัดส่วน + บทวิเคราะห์ + จุดที่ควรดูก่อน */
function analysisHtml(model: Model): string {
  const pulls = model.pulls;
  const total = pulls.length;
  if (!total) return '';
  const authors = new Set(pulls.map((p) => p.author.toLowerCase()));
  const totalFiles = pulls.reduce((n, p) => n + p.file_count, 0);
  const lines = pulls.map((p) => p.total);
  const avgLines = Math.round(lines.reduce((a, b) => a + b, 0) / total);
  const medLines = median(lines);
  const largeLines = pulls.filter((p) => p.large).reduce((n, p) => n + p.total, 0);
  const largePct = pct(largeLines, model.summary.lines);
  const assigned = pulls.filter((p) => p.reviewers.length > 0).length;
  const activeReviewers = model.workload.filter((w) => w.pending + w.done > 0);
  const pendingLines = model.workload.reduce((n, w) => n + w.pending, 0);
  const top = [...model.workload].sort((a, b) => b.pending - a.pending)[0];
  const topShare = top ? pct(top.pending, pendingLines) : 0;
  const avgPending = activeReviewers.length ? Math.round(pendingLines / activeReviewers.length) : 0;
  const imbalance = avgPending > 0 && top ? top.pending / avgPending : 0;
  const overAvg = activeReviewers.filter((w) => w.pending > avgPending).length;
  const authorCounts = new Map<string, number>();
  for (const p of pulls) authorCounts.set(p.author, (authorCounts.get(p.author) || 0) + 1);
  const topAuthor = [...authorCounts.entries()].sort((a, b) => b[1] - a[1])[0];
  const withAge = pulls
    .map((p) => ({ p, days: ageDays(p.updated_at) }))
    .filter((x): x is { p: Pull; days: number } => x.days != null);
  const oldest = [...withAge].sort((a, b) => b.days - a.days)[0];
  const stale = withAge.filter((x) => x.days >= 7);

  const statusCounts = STATUS_VIEW.map((s) => ({ ...s, n: pulls.filter((p) => p.status === s.key).length }));
  const segs = statusCounts
    .filter((s) => s.n > 0)
    .map((s) => `<span class="ov-seg ${s.cls}" style="width:${(s.n / total) * 100}%" title="${esc(s.label)} ${s.n} PR (${pct(s.n, total)}%)"></span>`)
    .join('');
  const legend = statusCounts
    .map(
      (s) => `<button type="button" class="ov-lg" data-metric-filter="${s.key}" data-metric-anchor="pr-list" aria-pressed="${String(
        state.filter === s.key
      )}"><i class="ov-dot ${s.cls}"></i>${esc(s.label)} <b>${s.n}</b> <span class="muted">${pct(s.n, total)}%</span></button>`
    )
    .join('');

  const readyPct = pct(model.summary.approved, total);
  const needsPct = pct(model.summary.needs, total);
  const healthy = needsPct === 0 && readyPct >= 40;
  const verdict = healthy
    ? 'ภาพรวมอยู่ในเกณฑ์ดี คิวรีวิวไม่ติดขัด'
    : model.summary.needs > 0
      ? 'ภาพรวมยังมีคอขวดจาก PR ที่ไม่มีผู้รีวิว'
      : 'ภาพรวมยังต้องเร่งรีวิว PR ที่ค้างอยู่';

  const bottleneck = model.summary.needs
    ? `คอขวดหลักคือ PR ${model.summary.needs} รายการที่ยังไม่มี reviewer (${needsPct}% ของทั้งหมด) ซึ่งจะไม่มีทางไปถึงสถานะพร้อม merge จนกว่าจะมีคนรับงาน`
    : model.summary.review
      ? `คอขวดหลักคือ PR ${model.summary.review} รายการที่รอรีวิว (${pct(model.summary.review, total)}%) แม้จะมี reviewer แล้วแต่ยังไม่มีผลอนุมัติ`
      : 'ไม่มี PR ที่ติดค้างเพราะขาด reviewer หรือรอผลรีวิวในขณะนี้';
  const ageLabel = oldest ? (oldest.days === 0 ? 'วันนี้' : `${oldest.days} วันก่อน`) : '';
  const ageNote = oldest
    ? `PR ที่เปิดนานที่สุดคือ #${oldest.p.number} อัปเดตล่าสุดเมื่อ ${esc(oldest.p.updated_label)} (${ageLabel}) และมี ${stale.length} รายการที่ไม่มีการอัปเดตเกิน 7 วัน`
    : 'ยังไม่มีข้อมูลเวลาอัปเดตพอจะวิเคราะห์ความเก่าของ PR';

  const workloadNote = top
    ? `มี reviewer ที่มีงานอยู่ในระบบ ${activeReviewers.length} คน ภาระค้างรวม ${pendingLines} บรรทัด เฉลี่ย ${avgPending} บรรทัด/คน โดย @${esc(top.login)} แบกมากที่สุด ${top.pending} บรรทัด${
        topShare >= 60 || imbalance >= 2
          ? ` คิดเป็น ${topShare}% ของภาระค้างและสูงกว่าค่าเฉลี่ย ${imbalance.toFixed(1)} เท่า — ควรกระจายงานเพิ่ม`
          : ' การกระจายตัวอยู่ในระดับที่รับได้'
      }${overAvg ? ` และมี ${overAvg} คนที่มีงานค้างเกินค่าเฉลี่ย` : ''}`
    : 'ยังไม่มี reviewer ที่ถูกมอบหมายงานในโปรเจคนี้';

  const riskNote = `PR ใหญ่ ${model.summary.large} รายการคิดเป็น ${largePct}% ของบรรทัดที่เปิดอยู่ทั้งหมด และแตะรวม ${totalFiles} ไฟล์ (เฉลี่ย ${(totalFiles / total).toFixed(1)} ไฟล์/PR) — ${
    model.summary.large ? 'ควรแบ่งรีวิวเป็นช่วงหรือแบ่ง PR ก่อนรีวิวเพื่อลดความเสี่ยงที่ reviewer จะพลาดจุดสำคัญ' : 'ขนาด PR ยังอยู่ในระดับที่รีวิวได้ในการอ่านครั้งเดียว'
  }`;

  const risks = pulls
    .map((p) => ({ p, reasons: riskReasons(p) }))
    .filter((x) => x.reasons.length)
    .sort((a, b) => b.reasons.length - a.reasons.length || b.p.total - a.p.total)
    .slice(0, 5);

  const stat = (value: string, label: string, sub: string, action?: MetricAction): string =>
    metricCard('div', 'ov-stat', action, `<b>${value}</b><span>${label}</span><em>${sub}</em>`);

  return `<section class="overview">
  <h2>ภาพรวมเชิงวิเคราะห์</h2>
  <div class="ov-grid">
    ${stat(`${readyPct}%`, 'อัตราพร้อม merge', `${model.summary.approved}/${total} PR`, { kind: 'filter', filter: 'approved', anchor: 'pr-list' })}
    ${stat(String(avgLines), 'เฉลี่ยบรรทัด/PR', `มัธยฐาน ${medLines} บรรทัด`, { kind: 'hash', anchor: 'overview-risk' })}
    ${stat(String(totalFiles), 'ไฟล์ที่เปลี่ยนแปลง', `เฉลี่ย ${(totalFiles / total).toFixed(1)} ไฟล์/PR`, { kind: 'hash', anchor: 'overview-risk' })}
    ${stat(String(model.summary.changes + model.summary.needs), 'ต้องตามต่อ', `ขอแก้ไข ${model.summary.changes} · ไม่มี reviewer ${model.summary.needs}`, { kind: 'filter', filter: 'attention', anchor: 'pr-list' })}
    ${stat(String(model.summary.large), 'PR ใหญ่', `${largePct}% ของบรรทัดทั้งหมด`, { kind: 'filter', filter: 'large', anchor: 'pr-list' })}
    ${stat(String(authors.size), 'ผู้เขียน', topAuthor ? `สูงสุด @${esc(topAuthor[0])} (${topAuthor[1]} PR)` : '—', { kind: 'link', href: navHref('people'), nav: 'people' })}
    ${stat(String(activeReviewers.length), 'Reviewer ที่มีงาน', `ค้างรวม ${pendingLines} บรรทัด`, { kind: 'link', href: navHref('people'), nav: 'people' })}
    ${stat(String(stale.length), 'ค้างเกิน 7 วัน', oldest ? `เก่าสุด ${oldest.days === 0 ? 'วันนี้' : `${oldest.days} วัน`} · #${oldest.p.number}` : 'ไม่ทราบอายุ PR', { kind: 'filter', filter: 'stale', anchor: 'pr-list' })}
  </div>

  <div class="ov-stack" role="img" aria-label="สัดส่วนสถานะ PR: ${statusCounts.map((s) => `${s.label} ${s.n}`).join(', ')}">${segs}</div>
  <div class="ov-legend">${legend}</div>

  <div class="ov-notes">
    <p><b>สุขภาพโดยรวม —</b> ${verdict} ปัจจุบันมี PR เปิดอยู่ ${total} รายการ รวม ${model.summary.lines} บรรทัด โดย ${model.summary.approved} รายการ (${readyPct}%) พร้อม merge, ${model.summary.review} รายการรอรีวิว, ${model.summary.changes} รายการถูกขอแก้ไข และ ${model.summary.needs} รายการยังไม่มีผู้รับรีวิว</p>
    <p><b>คอขวดการรีวิว —</b> ${bottleneck}${assigned < total ? ` ทั้งนี้มีเพียง ${assigned}/${total} PR (${pct(assigned, total)}%) ที่มี reviewer ถูกมอบหมายแล้ว` : ''} ${ageNote}</p>
    <p><b>ภาระงานของ Reviewer —</b> ${workloadNote}</p>
    <p id="overview-risk"><b>ความเสี่ยงด้านขนาดและความสด —</b> ${riskNote}${stale.length ? ` โดยมี ${stale.length} PR ที่ไม่ถูกแตะเกิน 7 วัน เสี่ยงตกรอบหรือมี conflict สะสม` : ''}</p>
  </div>

  ${risks.length ? `<h3>จุดที่ควรดูก่อน</h3>
  <ol class="ov-do">${risks
    .map(({ p, reasons }) => `<li><a href="${esc(prHref(p.number))}" data-pr-link="${p.number}">#${p.number} ${esc(p.title)}</a> <span class="muted">${esc(reasons.join(' · '))}</span></li>`)
    .join('')}</ol>` : ''}

  <p class="ov-note muted">สถิติคำนวณจาก PR ที่เปิดอยู่ทั้งหมด ${total} รายการ · ไม่อิงตัวกรองด้านล่าง · ข้อมูล ${esc(model.fetched_label)}</p>
</section>`;
}

function pickerHtml(model: Model): string {
  const select = state.projects.length > 1
    ? `<select id="project" aria-label="เลือกโปรเจค">${state.projects
        .map((r) => `<option value="${esc(r)}"${r.toLowerCase() === model.repo.toLowerCase() ? ' selected' : ''}>${esc(r)}</option>`)
        .join('')}</select>`
    : '';
  return `<div class="picker">${select}<span class="muted stamp" id="stamp">${icon('clock', 'ico-xs')} อัปเดต ${new Date().toLocaleTimeString('th-TH')}</span></div>`;
}

type PersonRow = { pull: Pull; rv: Reviewer };

/** งานทั้งหมดของคนหนึ่งในโปรเจคปัจจุบัน: PR ที่ได้รับ + ไฟล์ที่ต้องดู (รวมซ้ำ) */
function personData(model: Model, login: string): { rows: PersonRow[]; files: { filename: string; prs: number[]; ranges: string }[] } {
  const wanted = login.toLowerCase();
  const rows: PersonRow[] = [];
  for (const pull of model.pulls) {
    const rv = pull.reviewers.find((r) => r.login.toLowerCase() === wanted);
    if (rv) rows.push({ pull, rv });
  }
  const files = new Map<string, { prs: number[]; ranges: string[] }>();
  for (const { pull, rv } of rows) {
    for (const part of rv.parts) {
      const cur = files.get(part.filename) || { prs: [], ranges: [] };
      if (!cur.prs.includes(pull.number)) cur.prs.push(pull.number);
      cur.ranges.push(part.ranges && part.ranges.length ? fmtRanges(part.ranges) : 'ทั้งไฟล์');
      files.set(part.filename, cur);
    }
  }
  return {
    rows,
    files: Array.from(files.entries())
      .map(([filename, v]) => ({ filename, prs: v.prs, ranges: v.ranges.slice(0, 3).join(' · ') }))
      .sort((a, b) => b.prs.length - a.prs.length || a.filename.localeCompare(b.filename)),
  };
}

function renderPerson(app: HTMLElement, model: Model, login: string): boolean {
  const { rows, files } = personData(model, login);
  if (!rows.length) return false;
  const w = model.workload.find((x) => x.login.toLowerCase() === login.toLowerCase());
  const pending = w ? w.pending : rows.filter((r) => r.rv.status !== 'approved').reduce((n, r) => n + r.rv.lines, 0);
  const done = w ? w.done : rows.filter((r) => r.rv.status === 'approved').reduce((n, r) => n + r.rv.lines, 0);

  const prRows = rows
    .map(
      ({ pull, rv }) => `<tr data-pr-number="${pull.number}">
      <td><a href="${esc(prHref(pull.number))}" data-pr-link="${pull.number}">#${pull.number}</a> <a class="row-title" href="${esc(prHref(pull.number))}" data-pr-link="${pull.number}">${esc(pull.title)}</a></td>
      <td><span class="tag ${esc(rv.status)}">${esc(REVIEW_LABEL[rv.status] ?? rv.status)}</span></td>
      <td>${rv.lines}</td>
      <td>${rv.parts.length}</td>
      <td>${rv.reviewed_label ? `${esc(rv.reviewed_label)}` : '<span class="muted">—</span>'}</td>
    </tr>`
    )
    .join('');

  const fileItems = files.length
    ? files
        .map((f) => {
          const prs = f.prs.map((n) => `#${n}`).join(', ');
          return `<li><code>${esc(f.filename)}</code> <span class="muted">${prs}${f.ranges ? ` · ${esc(f.ranges)}` : ''}</span></li>`;
        })
        .join('')
    : '<li class="muted">ไม่มีไฟล์ที่ต้องดู</li>';

  const events = rows
    .flatMap(({ pull, rv }) => rv.history.map((ev) => ({ pull, ev })))
    .sort((a, b) => (a.ev.at < b.ev.at ? 1 : a.ev.at > b.ev.at ? -1 : 0))
    .slice(0, 25);
  const historyItems = events.length
    ? events
        .map(
          ({ pull, ev }) => `<li><span class="tag ${eventClass(ev.state)}">${esc(ev.label)}</span> <a href="${esc(prHref(pull.number))}" data-pr-link="${pull.number}">#${pull.number}</a> <span class="muted">${esc(ev.at_label)} · ${esc(ev.at.slice(0, 10))}</span>${
            ev.body ? `<div class="rv-body">${esc(snippet(ev.body))}</div>` : ''
          }</li>`
        )
        .join('')
    : '<li class="muted">ยังไม่มีประวัติรีวิว</li>';

  app.innerHTML = `
${navHtml(model)}
<div class="top">
  <div>
    <button class="back" id="back-list" type="button">${icon('arrowLeft', 'ico-xs')} กลับไปรายการ PR</button>
    <h1 class="d-title">@${esc(login)}</h1>
    <div class="muted">${esc(model.repo)} · ${rows.length} PR · ข้อมูล ${esc(model.fetched_label)}</div>
  </div>
  ${pickerHtml(model)}
</div>

<section class="kpis">
  ${kpi('PR ที่ได้รับ', rows.length, '', { kind: 'hash', anchor: 'person-prs' })}
  ${kpi('ค้าง (บรรทัด)', pending, 'warn', { kind: 'hash', anchor: 'person-prs' })}
  ${kpi('รีวิวแล้ว (บรรทัด)', done, 'ok', { kind: 'hash', anchor: 'person-prs' })}
  ${kpi('ไฟล์ที่ต้องดู', files.length, 'info', { kind: 'hash', anchor: 'person-files' })}
</section>

<h2 id="person-prs">PR ที่ได้รับ (${rows.length})</h2>
<div class="table-wrap"><table class="prs">
  <thead><tr><th>PR</th><th>สถานะรีวิว</th><th>บรรทัด</th><th>ไฟล์</th><th>รีวิวล่าสุด</th></tr></thead>
  <tbody>${prRows}</tbody>
</table></div>

<h2 id="person-files">ไฟล์ที่ต้องดู (${files.length})</h2>
<ul class="file-list">${fileItems}</ul>

<h2>ประวัติรีวิว (${events.length})</h2>
<ul class="history">${historyItems}</ul>

<footer>อ่านอย่างเดียว · ตัวเลขมาจาก planScope ของบอท · คลิกเลข PR เพื่อเปิดรายละเอียดในหน้านี้</footer>`;

  wire();
  return true;
}

function renderList(app: HTMLElement, model: Model): void {
  const s = model.summary;
  app.innerHTML = `
${navHtml(model)}
<div class="top">
  <div>
    <h1>Auto-review dashboard</h1>
    <div class="muted">${esc(model.repo)} · ข้อมูล ${esc(model.fetched_label)} · เกณฑ์ PR ใหญ่ ${model.threshold} บรรทัด</div>
  </div>
  ${pickerHtml(model)}
</div>

<section class="kpis">
  ${kpi('PR เปิดอยู่', s.open, '', { kind: 'filter', filter: 'all', anchor: 'pr-list' })}
  ${kpi('พร้อม merge', s.approved, 'ok', { kind: 'filter', filter: 'approved', anchor: 'pr-list' })}
  ${kpi('ขอแก้ไข', s.changes, 'bad', { kind: 'filter', filter: 'changes', anchor: 'pr-list' })}
  ${kpi('รอรีวิว', s.review, 'warn', { kind: 'filter', filter: 'review', anchor: 'pr-list' })}
  ${kpi('ยังไม่มี Reviewer', s.needs, 'bad', { kind: 'filter', filter: 'needs', anchor: 'pr-list' })}
  ${kpi('บรรทัดรวม', s.lines, 'info', { kind: 'hash', anchor: 'overview-risk' })}
  ${kpi('PR ใหญ่', s.large, 'warn', { kind: 'filter', filter: 'large', anchor: 'pr-list' })}
</section>

${analysisHtml(model)}

<div class="toolbar">
  <label class="search-box">${icon('search', 'search-ico')}<input id="q" type="search" aria-label="ค้นหา PR" placeholder="ค้นหาเลข PR, ชื่อ, ผู้เขียน, reviewer หรือ label" autocomplete="off" value="${esc(state.q)}"></label>
  <div class="chips" role="group" aria-label="กรองตามสถานะ">
    ${[['all', 'ทั้งหมด'], ['approved', 'พร้อม merge'], ['review', 'รอรีวิว'], ['changes', 'ขอแก้ไข'], ['needs', 'ไม่มี Reviewer'], ['large', 'PR ใหญ่'], ['attention', 'ต้องตามต่อ'], ['stale', 'ค้างเกิน 7 วัน'], ['draft', 'Draft']]
      .map(([k, label]) => `<button class="chip" data-metric-filter="${k}" aria-pressed="${String(state.filter === k)}">${label}</button>`)
      .join('')}
  </div>
</div>

<div id="pr-list">${model.pulls.map(prHtml).join('\n') || emptyState('inbox', 'ยังไม่มี PR ที่เปิดอยู่', 'เมื่อมี PR ใหม่ ระบบจะแสดงสถานะรีวิวและผู้รับผิดชอบที่นี่')}</div>
<div id="empty" class="empty-wrap" style="display:none">${emptyState('search', 'ไม่พบ PR ที่ตรงกับตัวกรอง', 'ลองล้างคำค้นหาหรือเลือกสถานะอื่น')}</div>

<h2>ภาระงานค้างต่อคน</h2>
<p class="muted">คลิกชื่อเพื่อเปิดโปรไฟล์ของคนนั้น • คลิกชื่อ PR เพื่อเปิดรายละเอียดในหน้านี้ • แท่ง: <span class="key pend"></span> ค้าง <span class="key done"></span> รีวิวแล้ว (เทียบคนที่งานมากสุด)</p>
${workloadTableHtml(model)}

<footer>อ่านอย่างเดียว · สถานะรีวิวจาก GitHub review ล่าสุดของแต่ละคน · ขอบเขตบรรทัดคำนวณด้วย <code>planScope</code> ตัวเดียวกับบอท</footer>`;

  wire();
  applyFilter();
}

type DiffRow = { kind: 'hunk' | 'add' | 'del' | 'ctx'; oldNo: number | null; newNo: number | null; text: string };

/** แยก unified diff (patch) เป็นแถว พร้อมเลขบรรทัดฝั่งเก่า/ใหม่ */
function diffRows(patch: string | null): DiffRow[] {
  if (!patch) return [];
  const rows: DiffRow[] = [];
  let oldNo = 0;
  let newNo = 0;
  for (const raw of patch.split('\n')) {
    if (raw.startsWith('@@')) {
      const plus = /\+(\d+)/.exec(raw);
      const minus = /-(\d+)/.exec(raw);
      if (plus) newNo = Number(plus[1]);
      if (minus) oldNo = Number(minus[1]);
      rows.push({ kind: 'hunk', oldNo: null, newNo: null, text: raw });
      continue;
    }
    if (raw.startsWith('+')) {
      rows.push({ kind: 'add', oldNo: null, newNo, text: raw.slice(1) });
      newNo++;
    } else if (raw.startsWith('-')) {
      rows.push({ kind: 'del', oldNo, newNo: null, text: raw.slice(1) });
      oldNo++;
    } else if (raw.startsWith(' ')) {
      rows.push({ kind: 'ctx', oldNo, newNo, text: raw.slice(1) });
      oldNo++;
      newNo++;
    }
  }
  return rows;
}

/** ชุดภาษา/คำสั่งที่ใช้ไฮไลต์ (ไม่พึ่ง dependency ภายนอก) */
type LangSpec = { kw: Set<string>; types?: Set<string>; line: string[]; block?: [string, string]; str: string[] };
const set = (v) => new Set(v.split(' ').filter(Boolean));

const JS_KW = 'const let var function return if else for while do switch case break continue class extends new this super import export default async await try catch finally throw typeof instanceof in of delete void yield static get set interface type enum implements public private protected readonly declare namespace abstract keyof never unknown any null undefined true false';
const PY_KW = 'and as assert async await break class continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return True try while with yield match case';
const RUST_KW = 'as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while';
const GO_KW = 'break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var nil true false';
const JAVA_KW = 'abstract assert boolean break byte case catch char class const continue default do double else enum extends final finally float for goto if implements import instanceof int interface long native new package private protected public return short static strictfp super switch synchronized this throw throws transient try void volatile while var record sealed permits yield';
const C_KW = 'auto break case char const continue default do double else enum extern float for goto if inline int long register restrict return short signed sizeof static struct switch typedef union unsigned void volatile while';
const CS_KW = C_KW + ' as base bool byte checked decimal delegate event fixed foreach implicit in interface internal is lock namespace object operator out override params readonly ref sbyte sealed stackalloc string throw true false try typeof uint ulong unchecked unsafe ushort using virtual async await record';
const PHP_KW = 'abstract and array as break callable case catch class clone const continue declare default do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile eval exit extends final finally fn for foreach function global goto if implements include include_once instanceof insteadof interface isset list match namespace new or print private protected public readonly require require_once return static switch throw trait try unset use var while xor yield';
const RB_KW = 'alias and begin break case class def defined do else elsif end ensure false for if in module next nil not or redo rescue retry return self super then true undef unless until when while yield attr_accessor attr_reader private public protected require require_relative';
const SH_KW = 'if then else elif fi for in do done case esac while until function select time export local readonly return exit set unset echo cd source alias declare typeset shift trap';
const SQL_KW = 'select from where join left right inner outer on group by order having limit offset insert into values update set delete create table alter drop index view as and or not null is in like between distinct union all case when then else end primary key foreign references default returning with asc desc exists';
const YAML_KW = 'true false null yes no on off';
const TOML_KW = 'true false';

const LANGS: Record<string, LangSpec> = {
  ts: { kw: set(JS_KW), types: set('string number boolean any unknown never void object symbol bigint'), line: ['//'], block: ['/*', '*/'], str: ['"', "'", '`'] },
  rust: { kw: set(RUST_KW), types: set('i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f32 f64 bool char str String Vec Option Result Box HashMap'), line: ['//'], block: ['/*', '*/'], str: ['"', '`'] },
  py: { kw: set(PY_KW), types: set('int float str bool list dict tuple set bytes object'), line: ['#'], str: ['"', "'"] },
  go: { kw: set(GO_KW), types: set('string int int8 int16 int32 int64 uint8 uint16 uint32 uint64 float32 float64 bool byte rune error any'), line: ['//'], block: ['/*', '*/'], str: ['"', "'", '`'] },
  java: { kw: set(JAVA_KW), types: set('String Integer Long Double Float Boolean Object List Map Set Optional'), line: ['//'], block: ['/*', '*/'], str: ['"', "'"] },
  c: { kw: set(C_KW), types: set('int char short long float double void unsigned signed size_t bool uint8_t uint16_t uint32_t uint64_t'), line: ['//'], block: ['/*', '*/'], str: ['"', "'"] },
  cs: { kw: set(CS_KW), types: set('int string bool double float decimal object byte long short char void var'), line: ['//'], block: ['/*', '*/'], str: ['"', "'"] },
  php: { kw: set(PHP_KW), types: set('int string bool float array object callable iterable void mixed null'), line: ['//', '#'], block: ['/*', '*/'], str: ['"', "'"] },
  rb: { kw: set(RB_KW), types: set('String Integer Float Array Hash Symbol Proc'), line: ['#'], str: ['"', "'"] },
  sh: { kw: set(SH_KW), types: set('true false'), line: ['#'], str: ['"', "'"] },
  sql: { kw: set(SQL_KW), types: set('int varchar text boolean timestamp date numeric jsonb uuid'), line: ['--'], block: ['/*', '*/'], str: ["'", '"', '`'] },
  json: { kw: set('true false null'), line: ['//'], str: ['"'] },
  yaml: { kw: set(YAML_KW), line: ['#'], str: ['"', "'"] },
  toml: { kw: set(TOML_KW), line: ['#'], str: ['"', "'"] },
  html: { kw: set(''), line: [], block: ['<!--', '-->'], str: ['"', "'"] },
  css: { kw: set('important media supports keyframes import charset'), line: [], block: ['/*', '*/'], str: ['"', "'"] },
  md: { kw: set(''), line: [], block: ['<!--', '-->'], str: ['`', '"', "'"] },
  other: { kw: set(JS_KW), line: ['//', '#'], block: ['/*', '*/'], str: ['"', "'", '`'] },
};

const EXT: Record<string, string> = {
  ts: 'ts', tsx: 'ts', mts: 'ts', cts: 'ts', js: 'ts', jsx: 'ts', mjs: 'ts', cjs: 'ts',
  rs: 'rust', py: 'py', pyi: 'py', go: 'go', java: 'java', kt: 'java', kts: 'java', scala: 'java', swift: 'java', dart: 'java',
  c: 'c', h: 'c', cc: 'c', cpp: 'c', cxx: 'c', hpp: 'c', hh: 'c', m: 'c', mm: 'c', cs: 'cs',
  php: 'php', rb: 'rb', rake: 'rb', ex: 'rb', exs: 'rb', sh: 'sh', bash: 'sh', zsh: 'sh', fish: 'sh', sql: 'sql',
  json: 'json', jsonc: 'json', json5: 'json', yaml: 'yaml', yml: 'yaml', toml: 'toml', ini: 'toml', conf: 'toml', env: 'toml',
  html: 'html', htm: 'html', vue: 'html', svelte: 'html', xml: 'html', svg: 'html', css: 'css', scss: 'css', less: 'css',
  md: 'md', markdown: 'md', mdx: 'md', txt: 'other', proto: 'c', gradle: 'java',
};

const SPECIAL: Record<string, string> = { dockerfile: 'sh', makefile: 'sh', 'cmakelists.txt': 'sh' };

function langOf(filename: string): string {
  const base = filename.split('/').pop()!.toLowerCase();
  if (SPECIAL[base]) return SPECIAL[base];
  const ext = base.includes('.') ? base.split('.').pop()! : '';
  return EXT[ext] || 'other';
}

const synSpan = (cls: string, text: string): string => `<span class="syn-${cls}">${esc(text)}</span>`;

/** ไฮไลต์ตามภาษาของไฟล์ (คอมเมนต์/สตริง/คีย์เวิร์ด/ชนิดข้อมูล/ตัวเลข) — escape ทุก chunk */
function highlightCode(src: string, lang = 'other'): string {
  const spec = LANGS[lang] || LANGS.other;
  let out = '';
  let i = 0;
  while (i < src.length) {
    const rest = src.slice(i);
    if (spec.block && rest.startsWith(spec.block[0])) {
      const close = src.indexOf(spec.block[1], i + spec.block[0].length);
      const end = close < 0 ? src.length : close + spec.block[1].length;
      out += synSpan('c', src.slice(i, end));
      i = end;
      continue;
    }
    if (spec.line.some((p) => rest.startsWith(p))) {
      out += synSpan('c', rest);
      break;
    }
    if (lang === 'html' && src[i] === '<' && /[a-zA-Z/!]/.test(src[i + 1] || '')) {
      const tag = /^<\/?[A-Za-z][\w:.-]*/.exec(rest);
      if (tag) {
        out += synSpan('t', tag[0]);
        i += tag[0].length;
        continue;
      }
    }
    const q = spec.str.find((c) => src[i] === c);
    if (q) {
      let j = i + 1;
      while (j < src.length && src[j] !== q) j += src[j] === '\\' ? 2 : 1;
      if (j < src.length) {
        out += synSpan('s', src.slice(i, j + 1));
        i = j + 1;
        continue;
      }
    }
    const ch = src[i];
    if (/[A-Za-z_$]/.test(ch)) {
      let j = i + 1;
      while (j < src.length && /[\w$]/.test(src[j])) j++;
      const word = src.slice(i, j);
      const lower = word.toLowerCase();
      out += spec.kw.has(lower) ? synSpan('k', word) : spec.types?.has(word) || spec.types?.has(lower) ? synSpan('t', word) : esc(word);
      i = j;
      continue;
    }
    if (/[0-9]/.test(ch)) {
      let j = i + 1;
      while (j < src.length && /[\w.]/.test(src[j])) j++;
      out += synSpan('n', src.slice(i, j));
      i = j;
      continue;
    }
    out += esc(ch);
    i++;
  }
  return out;
}

function commentBox(c: InlineComment): string {
  return `<div class="cmt-box">${who(c.login)} <span class="muted">${esc(c.at_label)}</span>${
    c.reply_to ? ' <span class="muted">· ตอบกลับ</span>' : ''
  }<div class="rv-body">${esc(c.body)}</div></div>`;
}

function lineCommentButton(file: FileEntry, side: 'LEFT' | 'RIGHT', line: number): string {
  return `<button type="button" class="line-comment-btn" data-line-comment data-comment-file="${esc(file.filename)}" data-comment-line="${line}" data-comment-side="${side}" title="คอมเมนต์บรรทัด ${line}" aria-label="คอมเมนต์บรรทัด ${line}">${icon('message', 'ico-xs')}</button>`;
}

function lineCommentComposerHtml(draft: LineDraft): string {
  return `<tr class="cmt-compose"><td class="ln"></td><td class="code">
  <form class="line-comment-form" id="line-comment-form">
    <div class="lcf-head">${icon('message', 'ico-xs')}<b>คอมเมนต์บรรทัด ${draft.line}</b><span class="muted">${draft.side === 'LEFT' ? 'ฝั่งเดิม' : 'ฝั่งใหม่'} · ${esc(draft.filename)}</span></div>
    <textarea id="line-comment-body" rows="3" placeholder="เขียนคอมเมนต์ถึงบรรทัดนี้…" required>${esc(draft.body)}</textarea>
    <div class="lcf-actions">
      <button class="rf-submit" type="submit">ส่งคอมเมนต์</button>
      <button class="chip" type="button" id="line-comment-cancel">ยกเลิก</button>
      <span class="muted" id="line-comment-status" aria-live="polite">โพสต์เป็น review comment บน GitHub</span>
    </div>
  </form>
</td></tr>`;
}

/** ไฟล์หนึ่งไฟล์: diff + inline comment แทรกใต้บรรทัดที่ตรงกัน (ใช้ Set กันแสดงซ้ำ) */
function fileBlockHtml(
  file: FileEntry,
  comments: InlineComment[],
  used: Set<InlineComment>,
  draft: LineDraft | null,
  commentable: boolean
): string {
  const rows = diffRows(file.patch);
  const byNew = new Map<number, InlineComment[]>();
  const byOld = new Map<number, InlineComment[]>();
  for (const c of comments) {
    if (c.line == null) continue;
    const target = c.side === 'LEFT' ? byOld : byNew;
    const list = target.get(c.line) || [];
    list.push(c);
    target.set(c.line, list);
  }
  const out: string[] = [];
  let draftPlaced = false;
  const emit = (list?: InlineComment[]): void => {
    if (!list) return;
    for (const c of list) {
      if (used.has(c)) continue;
      used.add(c);
      out.push(`<tr class="cmt"><td class="ln"></td><td class="code">${commentBox(c)}</td></tr>`);
    }
  };
  const emitDraft = (side: 'LEFT' | 'RIGHT', line: number | null): void => {
    if (draftPlaced || !draft || !commentable || line == null) return;
    if (draft.filename !== file.filename || draft.side !== side || draft.line !== line) return;
    draftPlaced = true;
    out.push(lineCommentComposerHtml(draft));
  };
  if (!rows.length) {
    out.push('<tr><td class="ln"></td><td class="code muted">ไม่มี diff ในข้อมูล (ไฟล์ใหม่งหรือ patch ถูกตัด)</td></tr>');
    for (const c of comments) emit([c]);
  } else {
    for (const r of rows) {
      if (r.kind === 'hunk') {
        out.push(`<tr class="hunk"><td class="ln"></td><td class="code">${esc(r.text)}</td></tr>`);
        continue;
      }
      const sign = r.kind === 'add' ? '+' : r.kind === 'del' ? '−' : ' ';
      const no = r.newNo ?? r.oldNo ?? '';
      const side: 'LEFT' | 'RIGHT' = r.kind === 'del' ? 'LEFT' : 'RIGHT';
      const line = side === 'LEFT' ? r.oldNo : r.newNo;
      const button = commentable && line != null ? lineCommentButton(file, side, line) : '';
      const target =
        commentable && line != null
          ? ` class="${r.kind} line-target" data-line-target data-comment-file="${esc(file.filename)}" data-comment-line="${line}" data-comment-side="${side}"`
          : ` class="${r.kind}"`;
      out.push(`<tr${target}><td class="ln">${no}</td><td class="code"><span class="sign">${sign}</span>${highlightCode(r.text, langOf(file.filename))}${button}</td></tr>`);
      if (r.newNo != null) emit(byNew.get(r.newNo));
      if (r.oldNo != null) emit(byOld.get(r.oldNo));
      emitDraft('RIGHT', r.newNo);
      emitDraft('LEFT', r.oldNo);
    }
  }
  return `<div class="file-block">
  <div class="file-head"><code>${esc(file.filename)}</code> <span class="muted">${file.lines} บรรทัด</span></div>
  <table class="diff"><tbody>${out.join('')}</tbody></table>
</div>`;
}

type DiffResponse = { repo: string; pr: number; files: FileEntry[] };
const diffCache = new Map<string, FileEntry[]>();
const diffLoading = new Set<string>();

function authHtml(): string {
  if (!state.authEnabled) return '';
  if (state.login) {
    return `<div class="auth"><a class="auth-user" href="${esc(navHref('profile'))}" data-nav="profile"${
      state.view === 'profile' && !state.person ? ' aria-current="page"' : ''
    }>${icon('user', 'ico-xs')}<span>@${esc(state.login)}</span></a><button class="auth-logout" id="logout" type="button">${icon('logout', 'ico-xs')}<span>ออก</span></button></div>`;
  }
  return `<div class="auth"><a class="auth-login" href="/login">${icon('shield', 'ico-xs')} เข้าสู่ระบบ GitHub</a></div>`;
}

function reviewFormHtml(p: Pull): string {
  if (state.authEnabled && !state.login) {
    return '<h2>รีวิว</h2>\n  <p class="muted">ต้อง <a href="/login">เข้าสู่ระบบด้วย GitHub</a> ก่อนจึงจะรีวิวได้</p>';
  }
  if (state.viewer && p.author.toLowerCase() === state.viewer.toLowerCase()) {
    return `<h2>รีวิว</h2>\n  <p class="muted">PR นี้เป็นของคุณเอง (@${esc(p.author)}) — GitHub ไม่อนุญาตให้อนุมัติ/ขอแก้ไข PR ของตัวเอง จึงปิดช่องรีวิวไว้</p>`;
  }
  if (!state.canWrite) {
    return '<h2>รีวิว</h2>\n  <p class="muted">โหมดอ่านอย่างเดียว — เริ่ม server ด้วย <code>--allow-write</code> เพื่อรีวิวจากหน้านี้</p>';
  }
  return `<h2>รีวิวจากหน้านี้</h2>
<form class="review-form" id="review-form">
  <div class="rf-actions">
    <label><input type="radio" name="action" value="approve" checked> อนุมัติ</label>
    <label><input type="radio" name="action" value="request_changes"> ขอแก้ไข</label>
    <label><input type="radio" name="action" value="comment"> คอมเมนต์</label>
  </div>
  <textarea id="review-body" rows="4" placeholder="ข้อความรีวิว (ต้องใส่เมื่อขอแก้ไข/คอมเมนต์)"></textarea>
  <div class="rf-row">
    <button type="submit" class="rf-submit">ส่งรีวิว</button>
    <span class="muted" id="review-status">โพสต์ในนามบัญชีของ token ที่ server ใช้</span>
  </div>
</form>`;
}

async function submitReview(form: HTMLFormElement): Promise<void> {
  if (!state.pr || !state.repo) return;
  const status = document.getElementById('review-status');
  const action = String(new FormData(form).get('action') || 'comment');
  const bodyEl = document.getElementById('review-body') as HTMLTextAreaElement | null;
  const text = (bodyEl?.value || '').trim();
  if (status) status.textContent = 'กำลังส่งรีวิว…';
  try {
    const res = await fetch('/api/review', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ repo: state.repo, pr: state.pr, action, body: text }),
    });
    const payload = (await res.json().catch(() => ({}))) as { error?: string; label?: string };
    if (!res.ok) throw new Error(payload.error || `HTTP ${res.status}`);
    if (bodyEl) bodyEl.value = '';
    diffCache.delete(`${state.repo}#${state.pr}`);
    mergeInfoCache.delete(`${state.repo}#${state.pr}`);
    if (status) status.textContent = `ส่งแล้ว: ${payload.label || action}`;
    await load();
  } catch (e) {
    if (status) status.textContent = `ไม่สำเร็จ: ${(e as Error).message}`;
  }
}

async function submitLineComment(form: HTMLFormElement): Promise<void> {
  const draft = state.lineDraft;
  if (!draft || !state.pr || !state.repo) return;
  const bodyEl = document.getElementById('line-comment-body') as HTMLTextAreaElement | null;
  const text = (bodyEl?.value || draft.body).trim();
  const status = document.getElementById('line-comment-status');
  if (!text) {
    if (status) {
      status.textContent = 'ต้องใส่ข้อความก่อนส่ง';
      status.classList.add('is-error');
    }
    bodyEl?.focus();
    return;
  }
  if (status) {
    status.textContent = 'กำลังส่ง…';
    status.classList.remove('is-error');
  }
  const submit = form.querySelector<HTMLButtonElement>('button[type=submit]');
  if (bodyEl) bodyEl.disabled = true;
  if (submit) submit.disabled = true;
  try {
    const res = await fetch('/api/comment', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        repo: state.repo,
        pr: state.pr,
        path: draft.filename,
        line: draft.line,
        side: draft.side,
        body: text,
      }),
    });
    const payload = (await res.json().catch(() => ({}))) as { error?: string };
    if (!res.ok) throw new Error(payload.error || `HTTP ${res.status}`);
    state.lineDraft = null;
    diffCache.delete(`${state.repo}#${state.pr}`);
    await load();
    if (state.model) render(state.model);
  } catch (e) {
    if (status) {
      status.textContent = `ไม่สำเร็จ: ${(e as Error).message}`;
      status.classList.add('is-error');
    }
    if (bodyEl) bodyEl.disabled = false;
    if (submit) submit.disabled = false;
  }
}

function wireLineCommentComposer(): void {
  const form = document.getElementById('line-comment-form') as HTMLFormElement | null;
  if (!form) return;
  const bodyEl = document.getElementById('line-comment-body') as HTMLTextAreaElement | null;
  bodyEl?.addEventListener('input', () => {
    if (state.lineDraft) state.lineDraft.body = bodyEl.value;
  });
  bodyEl?.addEventListener('keydown', (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      form.requestSubmit();
    }
  });
  form.addEventListener('submit', (e) => {
    e.preventDefault();
    void submitLineComment(form);
  });
  document.getElementById('line-comment-cancel')?.addEventListener('click', () => {
    state.lineDraft = null;
    form.closest('.cmt-compose')?.remove();
  });
}

function openLineComment(target: HTMLElement): void {
  const tr = target.closest('tr');
  const line = Number(target.dataset.commentLine);
  const filename = target.dataset.commentFile || '';
  const side = target.dataset.commentSide === 'LEFT' ? 'LEFT' : 'RIGHT';
  if (!tr || !filename || !Number.isFinite(line) || line <= 0) return;
  const draft: LineDraft = { filename, line, side, body: '' };
  const current = state.lineDraft;
  const existing = document.getElementById('line-comment-form');
  if (
    existing &&
    current &&
    current.filename === filename &&
    current.line === line &&
    current.side === side
  ) {
    (document.getElementById('line-comment-body') as HTMLTextAreaElement | null)?.focus();
    return;
  }
  state.lineDraft = draft;
  document.querySelectorAll('.cmt-compose').forEach((el) => el.remove());
  tr.insertAdjacentHTML('afterend', lineCommentComposerHtml(draft));
  wireLineCommentComposer();
  (document.getElementById('line-comment-body') as HTMLTextAreaElement | null)?.focus();
}

const MERGE_METHODS: { value: string; label: string }[] = [
  { value: 'merge', label: 'Merge commit (เก็บทุก commit)' },
  { value: 'squash', label: 'Squash and merge (รวมเป็น commit เดียว)' },
  { value: 'rebase', label: 'Rebase and merge (ไม่สร้าง merge commit)' },
];

function mergePanelHtml(p: Pull, history?: HistoryPr): string {
  if (history) {
    const info: MergeInfo = {
      state: history.state || (history.merged ? 'closed' : 'open'),
      merged: history.merged,
      draft: history.draft,
      mergeable: null,
      mergeable_state: null,
      merged_by: history.merged_by,
      actor: history.actor,
      via_dashboard: history.via_dashboard,
      merged_at: history.merged_at,
      merge_commit_sha: history.merge_commit_sha,
      commits: history.commits,
      changed_files: history.changed_files,
      additions: history.additions,
      deletions: history.deletions,
      base: history.base,
      head: history.head,
      head_sha: history.head_sha,
    };
    const label = history.merged
      ? `merge แล้ว${history.merged_at_label ? ` · ${esc(history.merged_at_label)}` : ''}`
      : `PR ถูกปิดโดยไม่ merge${history.merged_at_label ? ` · ${esc(history.merged_at_label)}` : ''}`;
    return `<h2>สถานะ merge</h2>
<div class="merge-status-panel">
  <b>${label}</b>
  <div class="merge-facts muted">${mergeFactsHtml(info)}</div>
</div>`;
  }
  const status = `<h2>สถานะ merge</h2>
<div class="merge-status-panel">
  <span class="muted" id="merge-status">กำลังตรวจสถานะ merge…</span>
  <div class="merge-facts muted" id="merge-facts"></div>
</div>`;
  if (state.authEnabled && !state.login) {
    return `${status}<h2>Merge</h2>\n  <p class="muted">ต้อง <a href="/login">เข้าสู่ระบบด้วย GitHub</a> ก่อนจึงจะ merge ได้</p>`;
  }
  if (!state.canWrite) {
    return `${status}<h2>Merge</h2>\n  <p class="muted">โหมดอ่านอย่างเดียว — เริ่ม server ด้วย <code>--allow-write</code> เพื่อ merge จากหน้านี้</p>`;
  }
  const opts = MERGE_METHODS.map(
    (m) => `<option value="${m.value}"${m.value === 'merge' ? ' selected' : ''}>${esc(m.label)}</option>`
  ).join('');
  return `${status}<h2>Merge</h2>
<form class="merge-form" id="merge-form">
  <div class="mf-row">
    <label for="merge-method">วิธี merge</label>
    <select id="merge-method" name="method">${opts}</select>
  </div>
  <div class="mf-row">
    <label for="merge-title">ชื่อ commit</label>
    <input id="merge-title" name="title" type="text" maxlength="200" placeholder="${esc(p.title)} (เว้นว่างเพื่อใช้ค่าเริ่มต้น)">
  </div>
  <div class="mf-row">
    <button type="submit" class="mf-submit" id="merge-btn">Merge PR #${p.number}</button>
  </div>
</form>`;
}

async function submitMerge(form: HTMLFormElement): Promise<void> {
  if (!state.pr || !state.repo) return;
  const btn = document.getElementById('merge-btn') as HTMLButtonElement | null;
  const status = document.getElementById('merge-status');
  if (!btn) return;
  if (btn.disabled) return;
  // ยืนยันสองจังหวะ: กดครั้งแรกเปลี่ยนปุ่มเป็น "ยืนยัน" กดอีกครั้งจึงส่ง
  if (!state.mergeConfirming) {
    state.mergeConfirming = true;
    btn.classList.remove('ready');
    btn.classList.add('danger');
    btn.textContent = `ยืนยัน merge #${state.pr}`;
    if (status) status.textContent = 'กดอีกครั้งเพื่อยืนยัน — merge แล้วย้อนกลับไม่ได้';
    return;
  }
  const fd = new FormData(form);
  const method = String(fd.get('method') || 'merge');
  const title = String(fd.get('title') || '').trim();
  state.mergeConfirming = false;
  state.merging = true;
  btn.disabled = true;
  btn.textContent = 'กำลัง merge…';
  if (status) status.textContent = 'กำลังส่งคำขอไป GitHub…';
  try {
    const res = await fetch('/api/merge', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ repo: state.repo, pr: state.pr, method, title }),
    });
    const payload = (await res.json().catch(() => ({}))) as { error?: string; sha?: string; method?: string; actor?: string };
    if (!res.ok) throw new Error(payload.error || `HTTP ${res.status}`);
    diffCache.delete(`${state.repo}#${state.pr}`);
    mergeInfoCache.delete(`${state.repo}#${state.pr}`);
    state.history = null;
    state.historyRepo = '';
    const successMsg = `Merge สำเร็จ${payload.sha ? ` · ${payload.sha.slice(0, 7)}` : ''} (${payload.method || method})${payload.actor ? ` · โดย @${payload.actor}` : ''}`;
    if (status) status.textContent = successMsg;
    lastJson = '';
    await load();
    state.merging = false;
    const statusAfter = document.getElementById('merge-status');
    if (statusAfter) statusAfter.textContent = successMsg;
  } catch (e) {
    state.merging = false;
    const failMsg = `ไม่สำเร็จ: ${(e as Error).message}`;
    btn.disabled = false;
    btn.classList.remove('danger');
    btn.textContent = `Merge PR #${state.pr}`;
    const p = state.model?.pulls.find((x) => x.number === state.pr);
    if (p) await loadMergeInfo(p);
    const statusAfterFail = document.getElementById('merge-status');
    if (statusAfterFail) statusAfterFail.textContent = failMsg;
  }
}

type MergeInfo = {
  state: string;
  merged: boolean;
  draft: boolean;
  mergeable: boolean | null;
  mergeable_state: string | null;
  merged_by: string | null;
  actor?: string | null;
  via_dashboard?: boolean;
  merged_at: string | null;
  merge_commit_sha: string | null;
  commits: number | null;
  changed_files: number | null;
  additions: number | null;
  deletions: number | null;
  base: string | null;
  head: string | null;
  head_sha: string | null;
};

const fmtIso = (iso: string | null | undefined): string =>
  iso ? `${iso.slice(0, 16).replace('T', ' ')} UTC` : '—';

function mergeFactsHtml(info: MergeInfo): string {
  const facts: string[] = [];
  facts.push(`<span>สถานะ: <b>${info.merged ? 'merge แล้ว' : info.state === 'open' ? 'เปิดอยู่' : esc(info.state)}</b></span>`);
  if (info.mergeable_state) facts.push(`<span>mergeable: <b>${esc(info.mergeable_state)}</b></span>`);
  if (info.commits != null) facts.push(`<span><b>${info.commits}</b> commits</span>`);
  if (info.changed_files != null) {
    const diff = info.additions != null || info.deletions != null ? ` · +${info.additions ?? 0}/−${info.deletions ?? 0}` : '';
    facts.push(`<span><b>${info.changed_files}</b> ไฟล์${diff}</span>`);
  }
  if (info.base || info.head) facts.push(`<span><code>${esc(info.head || '?')}</code> → <code>${esc(info.base || '?')}</code></span>`);
  if (info.head_sha) facts.push(`<span>head <code>${esc(info.head_sha.slice(0, 7))}</code></span>`);
  if (info.merged) {
    const by = info.actor || info.merged_by;
    if (by) facts.push(`<span>merge โดย <b>@${esc(by)}</b>${info.via_dashboard ? ' (ผ่าน dashboard)' : ''}${info.merged_at ? ` · ${esc(fmtIso(info.merged_at))}` : ''}</span>`);
    if (info.actor && info.merged_by && info.actor !== info.merged_by) {
      facts.push(`<span class="muted">GitHub เห็นเป็น @${esc(info.merged_by)} (token ของ server)</span>`);
    }
  }
  if (info.merged && info.merge_commit_sha) facts.push(`<span>merge commit <code>${esc(info.merge_commit_sha.slice(0, 7))}</code></span>`);
  return facts.join('');
}

const mergeInfoCache = new Map<string, { info: MergeInfo; at: number }>();
const MERGE_INFO_TTL = 20_000;

/** เหตุผลที่ยัง merge ไม่ได้ (อิง mergeable_state จริงจาก GitHub) */
function mergeBlockReason(info: MergeInfo): string | null {
  if (info.merged) return 'PR นี้ merge ไปแล้ว';
  if (info.state !== 'open') return 'PR ปิดอยู่ — merge ไม่ได้';
  if (info.draft) return 'Draft PR — กด “Ready for review” บน GitHub ก่อน';
  const ms = info.mergeable_state;
  if (ms === null) return 'GitHub กำลังคำนวณสถานะ merge — ลองรีเฟรชอีกครั้ง';
  if (ms === 'blocked') return 'branch protection/conversation ยังไม่ผ่าน — ตรวจ approving review / resolved conversation';
  if (ms === 'dirty') return 'มี conflict กับ base branch — ต้องแก้ก่อน merge';
  if (ms === 'behind') return 'branch ตาม base ไม่ทัน — ต้องอัปเดต branch ก่อน merge';
  if (ms === 'unstable') return 'CI/check ยังไม่ผ่าน หรือยังรันไม่เสร็จ';
  if (ms === 'unknown') return 'GitHub กำลังคำนวณสถานะ merge — ลองรีเฟรชอีกครั้ง';
  if (info.mergeable === false) return 'GitHub รายงานว่า PR นี้ยัง merge ไม่ได้';
  if (ms !== 'clean') return `สถานะ merge ยังไม่พร้อม (${ms})`;
  return null;
}

/** เหตุผล merge ที่เกี่ยวกับ Reviewer ที่ถูกมอบหมาย (ไม่นับ self-review) */
function assignedReviewBlockReason(p: Pull): string | null {
  const assigned = p.reviewers.filter((r) => r.login.toLowerCase() !== p.author.toLowerCase());
  if (!assigned.length) return 'ยังไม่มี Reviewer ที่ถูกมอบหมาย';
  if (assigned.some((r) => r.status === 'approved')) return null;
  const pending = assigned.filter((r) => r.status !== 'approved').map((r) => `@${r.login}`);
  const names = pending.length ? pending.join(', ') : assigned.map((r) => `@${r.login}`).join(', ');
  return `รอรีวิวจาก ${names}`;
}

async function loadMergeInfo(p: Pull): Promise<void> {
  if (!state.repo) return;
  const key = `${state.repo}#${p.number}`;
  const status = document.getElementById('merge-status');
  const cached = mergeInfoCache.get(key);
  let info = cached && Date.now() - cached.at < MERGE_INFO_TTL ? cached.info : null;
  if (!info) {
    try {
      const res = await fetch(`/api/mergeable?repo=${encodeURIComponent(state.repo)}&pr=${p.number}`, { cache: 'no-store' });
      if (!res.ok) {
        if (status) status.textContent = 'ตรวจสถานะ merge ไม่สำเร็จ — กด merge ได้เลย GitHub จะแจ้งเหตุผลให้อีกครั้ง';
        return;
      }
      info = (await res.json()) as MergeInfo;
      mergeInfoCache.set(key, { info, at: Date.now() });
    } catch {
      if (status) status.textContent = 'ตรวจสถานะ merge ไม่สำเร็จ — กด merge ได้เลย GitHub จะแจ้งเหตุผลให้อีกครั้ง';
      return;
    }
  }
  if (state.pr !== p.number) return;
  const facts = document.getElementById('merge-facts');
  if (facts) facts.innerHTML = mergeFactsHtml(info);
  const btn = document.getElementById('merge-btn') as HTMLButtonElement | null;
  const canMergeHere = !!document.getElementById('merge-form');
  const reason = assignedReviewBlockReason(p) || mergeBlockReason(info);
  if (reason) {
    state.mergeConfirming = false;
    if (btn) {
      btn.disabled = true;
      btn.classList.remove('ready', 'danger');
      btn.textContent = `Merge ไม่ได้ (${info.mergeable_state || info.state})`;
    }
    if (status) status.textContent = reason;
  } else if (state.mergeConfirming) {
    if (btn) {
      btn.disabled = false;
      btn.classList.remove('ready');
      btn.classList.add('danger');
      btn.textContent = `ยืนยัน merge #${p.number}`;
    }
    if (status) status.textContent = 'กดอีกครั้งเพื่อยืนยัน — merge แล้วย้อนกลับไม่ได้';
  } else {
    if (btn) {
      btn.disabled = false;
      btn.classList.add('ready');
      btn.classList.remove('danger');
      btn.textContent = `Merge PR #${p.number}`;
    }
    if (status) {
      status.textContent = canMergeHere
        ? 'พร้อม merge · merge ด้วย token ของ server · ระบบบันทึกชื่อผู้สั่งในประวัติ'
        : `สถานะพร้อม merge (${info.mergeable_state || info.state}) — ต้องเข้าสู่ระบบ/เปิดโหมดเขียนเพื่อสั่ง merge จากหน้านี้`;
    }
  }
}

const commitsCache = new Map<string, CommitEntry[]>();
const commitsLoading = new Set<string>();

/** ชื่อคนแบบมี @login ถ้ามี และชื่อจริงเป็นข้อมูลรอง */
function personHtml(login: string | null | undefined, name: string | null | undefined): string {
  const usable = !!login && login !== 'invalid-email-address' && login !== 'web-flow';
  const l = usable ? `<b>@${esc(login)}</b>` : '';
  const n = name ? `<span class="muted">${esc(name)}</span>` : '';
  return [l, n].filter(Boolean).join(' ') || '<span class="muted">ไม่ทราบ</span>';
}

function commitItem(c: CommitEntry): string {
  const isMerge = (c.parents ?? 0) > 1 || /^Merge pull request/i.test(c.summary || '');
  const body = c.message && c.message !== c.summary ? `<div class="c-body">${esc(c.message.slice(0, 400))}${c.message.length > 400 ? '…' : ''}</div>` : '';
  return `<li class="commit">
  <div class="c-head"><a href="${esc(c.url || '#')}" target="_blank" rel="noreferrer"><code>${esc(c.short)}</code></a>
    <span class="c-msg">${esc(c.summary || '(ไม่มีข้อความ)')}</span>${isMerge ? '<span class="badge info">merge commit</span>' : ''}${c.verified ? '<span class="badge ok">verified</span>' : ''}</div>
  <div class="c-meta">ผู้เขียน ${personHtml(c.author_login, c.author_name)}${
    c.committer_login || c.committer_name ? ` · ผู้ commit ${personHtml(c.committer_login, c.committer_name)}` : ''
  } · ${esc(c.date_label || fmtIso(c.date))}</div>
  ${body}
</li>`;
}

function commitsListHtml(list: CommitEntry[]): string {
  if (!list.length) return '<p class="muted">ไม่พบ commit ใน PR นี้</p>';
  return `<ol class="commit-list">${list.map(commitItem).join('')}</ol>`;
}

async function loadCommits(p: Pull): Promise<void> {
  const key = `${state.repo}#${p.number}`;
  if (commitsCache.has(key) || commitsLoading.has(key)) return;
  commitsLoading.add(key);
  try {
    const res = await fetch(`/api/commits?repo=${encodeURIComponent(state.repo)}&pr=${p.number}`, { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = (await res.json()) as { commits: CommitEntry[] };
    commitsCache.set(key, data.commits || []);
    if (state.pr === p.number || state.historyPr === p.number) {
      const panel = document.getElementById('commits-panel');
      if (panel) panel.innerHTML = commitsListHtml(data.commits || []);
      const count = document.getElementById('commits-count');
      if (count) count.textContent = String((data.commits || []).length);
    }
  } catch {
    if (state.pr === p.number || state.historyPr === p.number) {
      const panel = document.getElementById('commits-panel');
      if (panel) panel.innerHTML = '<p class="muted">โหลด commit ไม่สำเร็จ — เปิด PR บน GitHub เพื่อดูรายละเอียด</p>';
    }
  } finally {
    commitsLoading.delete(key);
  }
}

const DETAIL_TABS: [DetailTab, string, IconName][] = [
  ['changes', 'Changes', 'file'],
  ['status', 'Status', 'checkCircle'],
  ['conversation', 'Conversation', 'message'],
];

function detailTabsHtml(p: Pull, h: HistoryPr): string {
  const counts: Record<DetailTab, number> = {
    changes: p.file_count,
    status: h.checks?.total || 0,
    conversation: (h.conversation || []).length,
  };
  return `<div class="dtabs" role="tablist">${DETAIL_TABS.map(
    ([key, label, ico]) => `<button type="button" class="dtab" data-dtab="${key}" role="tab" aria-selected="${state.detailTab === key}">${icon(ico, 'ico tab-ico')}${label}<span class="count">${counts[key]}</span></button>`
  ).join('')}</div>`;
}

function checkClass(state: string | null | undefined): string {
  const s = (state || '').toLowerCase();
  if (s === 'success') return 'ok';
  if (s === 'failure' || s === 'error' || s === 'timed_out' || s === 'cancelled' || s === 'action_required') return 'bad';
  if (s === 'pending' || s === 'queued' || s === 'in_progress' || s === 'expected') return 'warn';
  return 'muted';
}

function checkLabel(state: string | null | undefined): string {
  const s = (state || '').toLowerCase();
  if (s === 'success') return 'ผ่าน';
  if (s === 'failure' || s === 'error') return 'ไม่ผ่าน';
  if (s === 'pending' || s === 'queued' || s === 'in_progress') return 'กำลังรัน';
  if (s === 'none') return 'ไม่มี checks';
  return state || '—';
}

function checksHtml(checks: ChecksInfo | undefined): string {
  if (!checks || !checks.total) return '<p class="muted">ไม่มี checks/status จาก GitHub สำหรับ PR นี้</p>';
  const statusRows = checks.statuses.map(
    (s) => `<li><span class="tag ${checkClass(s.state)}">${esc(checkLabel(s.state))}</span> <b>${esc(s.context || '—')}</b>${
      s.description ? ` <span class="muted">${esc(s.description)}</span>` : ''
    }${s.url ? ` <a href="${esc(s.url)}" target="_blank" rel="noreferrer">ดู</a>` : ''}</li>`
  );
  const runRows = checks.runs.map(
    (r) => `<li><span class="tag ${checkClass(r.conclusion || r.status)}">${esc(checkLabel(r.conclusion || r.status))}</span> <b>${esc(r.name || '—')}</b>${
      r.conclusion ? ` <span class="muted">${esc(r.conclusion)}</span>` : r.status ? ` <span class="muted">${esc(r.status)}</span>` : ''
    }${r.url ? ` <a href="${esc(r.url)}" target="_blank" rel="noreferrer">ดู</a>` : ''}</li>`
  );
  return `<div class="check-summary"><span class="badge ${checkClass(checks.state)}">${esc(checkLabel(checks.state))}</span> <span class="muted">${checks.total} รายการ</span></div>
  <ul class="check-list">${statusRows.concat(runRows).join('')}</ul>`;
}

function reviewsSummaryHtml(reviews: ReviewItem[]): string {
  if (!reviews.length) return '<p class="muted">ยังไม่มีรีวิวใน PR นี้</p>';
  const latest = new Map<string, ReviewItem>();
  for (const r of reviews) latest.set((r.author || '?').toLowerCase(), r);
  const counts = {
    approved: reviews.filter((r) => r.state === 'APPROVED').length,
    changes: reviews.filter((r) => r.state === 'CHANGES_REQUESTED').length,
    commented: reviews.filter((r) => r.state === 'COMMENTED').length,
  };
  return `<div class="review-counts"><span class="badge ok">อนุมัติ ${counts.approved}</span> <span class="badge bad">ขอแก้ไข ${counts.changes}</span> <span class="badge muted">คอมเมนต์ ${counts.commented}</span></div>
  <ul class="review-list">${Array.from(latest.values())
    .map(
      (r) => `<li><span class="tag ${eventClass(r.state)}">${esc(r.label)}</span> ${personHtml(r.author, null)} <span class="muted">${esc(r.at_label)}</span>${
        r.body ? `<div class="rv-body">${esc(snippet(r.body, 220))}</div>` : ''
      }</li>`
    )
    .join('')}</ul>`;
}

function conversationHtml(h: HistoryPr): string {
  const items = h.conversation || [];
  if (!items.length) return '<p class="muted">ยังไม่มีความคิดเห็นหรือรีวิวใน PR นี้</p>';
  return `<ol class="convo">${items
    .map((e) => {
      const cls = e.kind === 'review' ? eventClass(e.state || '') : 'commented';
      const where = e.path
        ? `<div class="c-meta"><code>${esc(e.path)}${e.line != null ? `:${e.line}` : ''}</code></div>`
        : '';
      const body = e.body && e.body.trim()
        ? `<div class="rv-body">${esc(e.body)}</div>`
        : '<div class="muted c-empty">ไม่มีข้อความ</div>';
      return `<li class="convo-item">
  <div class="convo-head">${personHtml(e.author, null)} <span class="tag ${cls}">${esc(e.label)}</span> <span class="muted">${esc(e.at_label)}</span>${
        e.url ? ` <a href="${esc(e.url)}" target="_blank" rel="noreferrer">เปิด</a>` : ''
      }</div>
  ${where}${body}
</li>`;
    })
    .join('')}</ol>`;
}

type DetailOpts = { readonly?: boolean; history?: HistoryPr };

function renderDetail(app: HTMLElement, model: Model, p: Pull, opts?: DetailOpts): void {
  const readonly = !!opts?.readonly;
  const cacheKey = `${model.repo}#${p.number}`;
  const fetchedDiff = diffCache.get(cacheKey);
  const filesResolved = p.files.map((f) => ({
    ...f,
    patch: f.patch ?? fetchedDiff?.find((x) => x.filename === f.filename)?.patch ?? null,
  }));
  const needDiff = !fetchedDiff && filesResolved.some((f) => !f.patch);
  const inlineComments = p.inline || [];
  const used = new Set<InlineComment>();
  const commentable = !readonly && state.canWrite && (!state.authEnabled || !!state.login);
  const fileBlocks = filesResolved.length
    ? filesResolved
        .map((f) =>
          fileBlockHtml(
            f,
            inlineComments.filter((c) => c.filename === f.filename),
            used,
            commentable ? state.lineDraft : null,
            commentable
          )
        )
        .join('')
    : '<p class="muted">ไม่มีไฟล์ (ถูกยกเว้นทั้งหมด)</p>';
  const leftovers = inlineComments.filter((c) => !used.has(c));
  const leftoversHtml = leftovers.length
    ? `<h2>คอมเมนต์ที่อยู่นอก diff (${leftovers.length})</h2>
  <ul class="history">${leftovers
    .map(
      (c) => `<li><span class="muted">${esc(c.filename)}${c.line ? `:${c.line}` : ''}</span>${commentBox(c)}</li>`
    )
    .join('')}</ul>`
    : '';

  const reviewers = p.reviewers.length
    ? p.reviewers
        .map(
          (r) => `<div class="rv">
        <div class="rv-head">${who(r.login)}
          <span class="tag ${esc(r.status)}">${esc(REVIEW_LABEL[r.status] ?? r.status)}</span>
          <span class="lines">${r.lines} บรรทัด · ${r.parts.length} ไฟล์</span></div>
        ${partsHtml(r, 200)}
        ${historyHtml(r)}
      </div>`
        )
        .join('')
    : '<div class="rv muted">ยังไม่มี Reviewer — บอทจะลองใหม่ทุกวัน หรือเลือกเองใน PR</div>';

  const topBadges = opts?.history
    ? `<span class="badge ${opts.history.merged ? 'ok' : 'bad'}">${opts.history.merged ? 'merge แล้ว' : 'ปิดอยู่'}</span>${
        opts.history.draft ? '<span class="badge muted">Draft</span>' : ''
      }`
    : badgesFor(p);

  const commitsSection = `<h2>Commits (<span id="commits-count">${commitsCache.get(cacheKey)?.length ?? '…'}</span>)</h2>
  <div id="commits-panel">${commitsCache.has(cacheKey) ? commitsListHtml(commitsCache.get(cacheKey) || []) : '<p class="muted">กำลังโหลด commit…</p>'}</div>`;
  const filesSection = `<h2 id="files-section">ไฟล์ที่เปลี่ยน (${p.files.length})</h2>
  <p class="muted">inline comment ${inlineComments.length} รายการ — วางใต้บรรทัดที่ตรงกันใน diff${commentable ? ' · คลิกที่บรรทัดหรือไอคอนเพื่อคอมเมนต์' : ''}${needDiff ? ' · กำลังโหลด diff…' : ''}</p>
  ${fileBlocks}`;
  const tab = state.detailTab;
  const tabbedBody = opts?.history
    ? `${detailTabsHtml(p, opts.history)}
  <section class="tab-panel" data-tab-panel="changes" style="${tab === 'changes' ? '' : 'display:none'}">
    ${commitsSection}
    ${filesSection}
  </section>
  <section class="tab-panel" data-tab-panel="status" style="${tab === 'status' ? '' : 'display:none'}">
    ${mergePanelHtml(p, opts.history)}
    <h2>Checks / CI</h2>
    ${checksHtml(opts.history.checks)}
    <h2>สรุปรีวิว</h2>
    ${reviewsSummaryHtml(opts.history.reviews || [])}
  </section>
  <section class="tab-panel" data-tab-panel="conversation" style="${tab === 'conversation' ? '' : 'display:none'}">
    <h2>Conversation (${(opts.history.conversation || []).length})</h2>
    ${conversationHtml(opts.history)}
  </section>`
    : `${mergePanelHtml(p)}
    ${commitsSection}
    ${filesSection}
    <h2>ผู้รีวิว (${p.reviewers.length})</h2>
    <div class="reviewers">${reviewers}</div>
    ${reviewFormHtml(p)}`;

  app.innerHTML = `
${navHtml(model)}
<div class="top">
  <div>
    <button class="back" id="back-list" type="button">${icon('arrowLeft', 'ico-xs')} ${readonly ? 'กลับไปประวัติ' : 'กลับไปรายการ PR'}</button>
    <h1 class="d-title">${esc(p.title)}</h1>
    <div class="muted">${esc(model.repo)} · #${p.number} · @${esc(p.author)} · อัปเดต ${esc(p.updated_label)}</div>
  </div>
  ${pickerHtml(model)}
</div>

<article class="detail">
  <div class="d-top">${topBadges}</div>
  <div class="meta"><a class="meta-count" href="#files-section"><b>${p.total}</b> บรรทัด</a> · <a class="meta-count" href="#files-section">${p.file_count} ไฟล์</a>${readonly ? '' : ` · เกณฑ์ PR ใหญ่ ${model.threshold} บรรทัด`}</div>
  ${labelsFor(p)}

  ${tabbedBody}

  ${leftoversHtml}

  ${p.url ? `<footer>${readonly ? 'PR นี้ merge แล้ว' : 'ข้อมูลในหน้านี้ครบแล้ว · ถ้าต้องการคอมเมนต์/approve'} · <a class="ext-link" href="${esc(p.url)}" target="_blank" rel="noreferrer">เปิดบน GitHub ${icon('external', 'ico-xs')}</a></footer>` : ''}
</article>`;

  wire();
  if (document.getElementById('merge-form') && state.mergeConfirming) {
    const mb = document.getElementById('merge-btn') as HTMLButtonElement | null;
    if (mb) {
      mb.classList.remove('ready');
      mb.classList.add('danger');
      mb.textContent = `ยืนยัน merge #${p.number}`;
    }
  }
  if (!readonly) void loadMergeInfo(p);
  if (!commitsCache.has(cacheKey)) void loadCommits(p);

  // diff ถูกโหลดตาม request (ไม่ติดมาใน /api/data.json) — โหลดเสร็จแล้วเรนเดอร์ใหม่
  if (needDiff && !diffLoading.has(cacheKey)) {
    diffLoading.add(cacheKey);
    void (async () => {
      try {
        const res = await fetch(`/api/diff?repo=${encodeURIComponent(model.repo)}&pr=${p.number}`, { cache: 'no-store' });
        if (res.ok) {
          const data = (await res.json()) as DiffResponse;
          diffCache.set(cacheKey, data.files || []);
        }
      } catch {
        /* ปล่อยให้ขึ้นว่าไม่มี diff */
      }
      diffLoading.delete(cacheKey);
      if ((state.pr === p.number || state.historyPr === p.number) && state.model) render(state.model);
    })();
  }
}

function navHref(view: View): string {
  const p = query();
  if (view === 'list') p.delete('view');
  else p.set('view', view);
  p.delete('pr');
  p.delete('person');
  if (state.repo) p.set('repo', state.repo);
  return `?${p.toString()}`;
}

type MyWork = { authored: Pull[]; toReview: { pull: Pull; rv: Reviewer }[]; reviewed: { pull: Pull; rv: Reviewer }[] };

/** งานของผู้ใช้ที่ล็อกอิน/เจ้าของ token: PR ที่ตัวเองเปิด + PR ที่ตัวเองต้องรีวิว */
function myWork(model: Model): MyWork {
  const out: MyWork = { authored: [], toReview: [], reviewed: [] };
  const me = state.viewer.toLowerCase();
  if (!me) return out;
  for (const pull of model.pulls) {
    if (pull.author.toLowerCase() === me) out.authored.push(pull);
    const rv = pull.reviewers.find((r) => r.login.toLowerCase() === me);
    if (rv) (rv.status === 'pending' ? out.toReview : out.reviewed).push({ pull, rv });
  }
  return out;
}

/** เมนูหลักของ dashboard */
function navHtml(model: Model): string {
  const work = myWork(model);
  const items: [View, string, IconName][] = [
    ['list', 'ภาพรวม', 'grid'],
    ['mine', 'งานของฉัน', 'inbox'],
    ['people', 'รายคน', 'users'],
    ['history', 'ประวัติ', 'history'],
  ];
  return `<header class="appbar">
  <a class="brandmark" href="${esc(navHref('list'))}" data-nav="list" aria-label="Auto-review dashboard — กลับหน้าภาพรวม">
    <span class="brand-logo">${icon('gitPr', 'brand-ico')}</span>
    <span class="brand-text">Review<small>dashboard</small></span>
  </a>
  <nav class="nav" aria-label="เมนูหลัก">${items
    .map(([v, label, ico]) => {
      const active = state.person ? v === 'people' : state.view === v;
      const badge = v === 'mine' && work.toReview.length ? `<span class="count">${work.toReview.length}</span>` : '';
      return `<a href="${esc(navHref(v))}" data-nav="${v}"${active ? ' aria-current="page"' : ''}>${icon(ico, 'ico nav-ico')}<span>${label}</span>${badge}</a>`;
    })
    .join('')}</nav>
  ${authHtml()}
</header>`;
}

function reviewRowsHtml(rows: { pull: Pull; rv: Reviewer }[]): string {
  if (!rows.length) return '<tr><td colspan="5" class="muted">ไม่มีรายการ</td></tr>';
  return rows
    .map(
      ({ pull, rv }) => `<tr data-pr-number="${pull.number}">
      <td><a href="${esc(prHref(pull.number))}" data-pr-link="${pull.number}">#${pull.number}</a> <a class="row-title" href="${esc(prHref(pull.number))}" data-pr-link="${pull.number}">${esc(pull.title)}</a></td>
      <td>@${esc(pull.author)}</td>
      <td><span class="tag ${esc(rv.status)}">${esc(REVIEW_LABEL[rv.status] ?? rv.status)}</span></td>
      <td>${rv.lines}</td>
      <td>${rv.parts.length}</td>
    </tr>`
    )
    .join('');
}

function renderMine(app: HTMLElement, model: Model): void {
  const me = state.viewer;
  if (!me) {
    app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <h1>งานของฉัน</h1>
    <div class="muted">${esc(model.repo)} · ยังไม่รู้ว่าคุณคือใคร</div>
  </div>
  ${pickerHtml(model)}
</div>
<div class="empty-state"><span class="empty-ico">${icon('lock')}</span><div><b>${state.authEnabled ? 'เข้าสู่ระบบเพื่อดูงานของคุณ' : 'โหมดนี้ยังไม่ทราบตัวตนผู้ใช้'}</b><p>${
      state.authEnabled
        ? '<a href="/login">เข้าสู่ระบบด้วย GitHub</a> เพื่อดู PR ของคุณและคิวรีวิวของคุณ'
        : 'เปิด GitHub OAuth หรือใช้ token ที่ผูกกับบัญชีคุณ'
    }</p></div></div>`;
    wire();
    return;
  }
  const { authored, toReview, reviewed } = myWork(model);
  const pendingLines = toReview.reduce((n, x) => n + x.rv.lines, 0);
  const authoredHtml = authored.length
    ? authored.map(prHtml).join('\n')
    : emptyState('gitPr', 'ยังไม่มี PR ของคุณในโปรเจกต์นี้', 'เมื่อคุณเปิด PR ระบบจะแสดงสถานะรีวิวไว้ที่นี่');

  app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <h1>งานของฉัน</h1>
    <div class="muted">@${esc(me)} · ${esc(model.repo)} · ข้อมูล ${esc(model.fetched_label)}</div>
  </div>
  ${pickerHtml(model)}
</div>

<section class="kpis">
  ${kpi('PR ของฉัน', authored.length, '', { kind: 'hash', anchor: 'mine-authored' })}
  ${kpi('รอฉันรีวิว', toReview.length, 'warn', { kind: 'hash', anchor: 'mine-review' })}
  ${kpi('รีวิวแล้ว', reviewed.length, 'ok', { kind: 'hash', anchor: 'mine-reviewed' })}
  ${kpi('บรรทัดรอรีวิว', pendingLines, 'info', { kind: 'hash', anchor: 'mine-review' })}
</section>

<h2 id="mine-review">รอฉันรีวิว (${toReview.length})</h2>
<div class="table-wrap"><table class="prs">
  <thead><tr><th>PR</th><th>ผู้เขียน</th><th>สถานะรีวิว</th><th>บรรทัด</th><th>ไฟล์</th></tr></thead>
  <tbody>${reviewRowsHtml(toReview)}</tbody>
</table>
</div>

<h2 id="mine-authored">PR ของฉัน (${authored.length})</h2>
${authoredHtml}

<h2 id="mine-reviewed">รีวิวแล้ว (${reviewed.length})</h2>
<div class="table-wrap"><table class="prs">
  <thead><tr><th>PR</th><th>ผู้เขียน</th><th>สถานะรีวิว</th><th>บรรทัด</th><th>ไฟล์</th></tr></thead>
  <tbody>${reviewRowsHtml(reviewed)}</tbody>
</table>
</div>

<footer>${state.login ? `ล็อกอินในนาม @${esc(state.login)}` : 'อ่านอย่างเดียว · ตัวตนมาจาก token ของ server'} · คลิกเลข PR เพื่อเปิดรายละเอียดในหน้านี้</footer>`;
  wire();
}

function mergeActorHtml(m: MergeEntry): string {
  const actor = m.actor || m.merged_by;
  const who = actor ? `<b>@${esc(actor)}</b>` : '<span class="muted">ไม่ทราบ</span>';
  const src = m.via_dashboard
    ? '<span class="tag approved">ผ่าน dashboard</span>'
    : '<span class="tag pending">บน GitHub</span>';
  const gh = m.via_dashboard && m.merged_by && m.merged_by !== m.actor
    ? `<div class="muted">GitHub เห็นเป็น @${esc(m.merged_by)} (token ของ server)</div>`
    : '';
  return `<div>${who} ${src}</div>${gh}`;
}

function historyBodyHtml(model: Model, h: HistoryResponse): string {
  const via = h.merges.filter((m) => m.via_dashboard).length;
  const latest = h.merges[0];
  const mergeRows = h.merges
    .map((m) => {
      const shaUrl = m.merge_commit_sha ? `https://github.com/${model.repo}/commit/${m.merge_commit_sha}` : m.url;
      return `<tr>
      <td><a href="${esc(historyPrHref(m.number))}" data-history-pr="${m.number}">#${m.number}</a> <a href="${esc(historyPrHref(m.number))}" data-history-pr="${m.number}">${esc(m.title)}</a><div class="muted">โดย ${m.author ? `@${esc(m.author)}` : 'ไม่ทราบ'}</div></td>
      <td>${mergeActorHtml(m)}</td>
      <td>${m.head ? `<code>${esc(m.head)}</code> → ` : ''}${m.base ? `<code>${esc(m.base)}</code>` : '—'}</td>
      <td>${m.short_sha ? `<a href="${esc(shaUrl || '#')}" target="_blank" rel="noreferrer"><code>${esc(m.short_sha)}</code></a>` : '—'}</td>
      <td>${esc(m.merged_at_label || '—')}<div class="muted">${esc(fmtIso(m.merged_at))}</div></td>
    </tr>`;
    })
    .join('');
  const commitRows = h.commits
    .map(
      (c) => `<tr>
      <td><a href="${esc(c.url || '#')}" target="_blank" rel="noreferrer"><code>${esc(c.short)}</code></a></td>
      <td>${esc(c.summary || '(ไม่มีข้อความ)')}</td>
      <td>${personHtml(c.author_login, c.author_name)}${c.committer_login && c.committer_login !== c.author_login ? `<div class="muted">commit โดย ${personHtml(c.committer_login, c.committer_name)}</div>` : ''}</td>
      <td>${esc(c.date_label || '—')}</td>
    </tr>`
    )
    .join('');

  return `<section class="kpis">
  ${kpi('PR merge แล้ว', h.merges.length, 'ok', { kind: 'hash', anchor: 'history-merges' })}
  ${kpi('Commit บน branch หลัก', h.commits.length, 'info', { kind: 'hash', anchor: 'history-commits' })}
  ${kpi('Merge ผ่าน dashboard', via, '', { kind: 'hash', anchor: 'history-merges' })}
  ${kpi('Merge บน GitHub', h.merges.length - via, 'muted', { kind: 'hash', anchor: 'history-merges' })}
</section>
<p class="muted">ประวัติล่าสุด ${h.merges.length} merge · ${h.commits.length} commit${latest ? ` · merge ล่าสุด ${esc(latest.merged_at_label)}` : ''} · ตัวเลขจาก GitHub + audit ของ dashboard</p>

<h2 id="history-merges">ประวัติการ merge (${h.merges.length})</h2>
${h.merges.length ? `<div class="table-wrap"><table class="hist">
  <thead><tr><th>PR</th><th>ผู้ merge</th><th>สาขา</th><th>Commit</th><th>เวลา</th></tr></thead>
  <tbody>${mergeRows}</tbody>
</table></div>` : emptyState('gitMerge', 'ยังไม่มี PR ที่ merge', 'เมื่อมีการ merge ประวัติจะถูกบันทึกไว้ที่นี่', true)}

<h2 id="history-commits">ประวัติ commit บน branch หลัก (${h.commits.length})</h2>
${h.commits.length ? `<div class="table-wrap"><table class="hist">
  <thead><tr><th>SHA</th><th>ข้อความ</th><th>ผู้เขียน</th><th>เวลา</th></tr></thead>
  <tbody>${commitRows}</tbody>
</table></div>` : emptyState('code', 'ยังไม่มี commit', 'commit บน branch หลักจะแสดงที่นี่', true)}

<footer>“ผ่าน dashboard” มาจาก audit log ของระบบ (ระบุคนที่กด merge จริง) · GitHub อาจแสดงชื่อ token ของ server · คลิก SHA/PR เพื่อเปิดบน GitHub</footer>`;
}

async function loadHistory(repo: string): Promise<void> {
  if (state.historyLoading) return;
  state.historyLoading = true;
  state.historyRepo = repo;
  state.historyError = '';
  try {
    const res = await fetch(`/api/history?repo=${encodeURIComponent(repo)}`, { cache: 'no-store' });
    const payload = (await res.json().catch(() => ({}))) as HistoryResponse & { error?: string };
    if (!res.ok) throw new Error(payload.error || `HTTP ${res.status}`);
    if (state.repo === repo) {
      state.history = payload;
      if (state.view === 'history' && !state.pr && state.model) render(state.model);
    }
  } catch (e) {
    state.historyError = (e as Error).message;
    if (state.view === 'history' && state.model) render(state.model);
  } finally {
    state.historyLoading = false;
  }
}

const historyPrCache = new Map<string, HistoryPr>();
const historyPrLoading = new Set<string>();
const historyPrKey = (repo: string, n: number): string => `${repo}#${n}`;

function openHistoryPr(n: number): void {
  state.historyPr = n;
  state.pr = null;
  state.person = null;
  state.lineDraft = null;
  state.historyPrError = '';
  state.detailTab = 'changes';
  setParam('view', 'history');
  setParam('pr', String(n));
  setParam('person', null);
  if (state.model) render(state.model);
}

function closeHistoryPr(): void {
  state.historyPr = null;
  state.lineDraft = null;
  setParam('pr', null);
  setParam('view', 'history');
  if (state.model) render(state.model);
}

async function loadHistoryPr(repo: string, n: number): Promise<void> {
  const key = historyPrKey(repo, n);
  if (historyPrLoading.has(key)) return;
  historyPrLoading.add(key);
  state.historyPrError = '';
  try {
    const res = await fetch(`/api/pr?repo=${encodeURIComponent(repo)}&pr=${n}`, { cache: 'no-store' });
    const data = (await res.json().catch(() => ({}))) as HistoryPr & { error?: string };
    if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
    historyPrCache.set(key, data);
    if (state.historyPr === n && state.repo === repo && state.model) render(state.model);
  } catch (e) {
    state.historyPrError = (e as Error).message;
    if (state.historyPr === n && state.repo === repo && state.model) render(state.model);
  } finally {
    historyPrLoading.delete(key);
  }
}

function renderHistoryPr(app: HTMLElement, model: Model, n: number): void {
  const data = historyPrCache.get(historyPrKey(model.repo, n));
  if (!data) {
    const error = state.historyPrError;
    app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <button class="back" id="back-list" type="button">${icon('arrowLeft', 'ico-xs')} กลับไปประวัติ</button>
    <h1 class="d-title">PR #${n}</h1>
    <div class="muted">${esc(model.repo)} · ${error ? 'โหลดไม่สำเร็จ' : 'กำลังโหลด PR…'}</div>
  </div>
  ${pickerHtml(model)}
</div>
<div class="pr muted">${
      error
        ? `โหลด PR ไม่สำเร็จ: ${esc(error)} <button class="chip" id="history-pr-retry" type="button">${icon('refresh', 'ico-xs')} ลองใหม่</button>`
        : 'กำลังโหลดรายละเอียด PR จาก GitHub…'
    }</div>`;
    wire();
    if (!error && !historyPrLoading.has(historyPrKey(model.repo, n))) void loadHistoryPr(model.repo, n);
    return;
  }

  const total = (data.additions ?? 0) + (data.deletions ?? 0) || data.files.reduce((sum, f) => sum + f.lines, 0);
  const pull: Pull = {
    number: data.number,
    title: data.title,
    url: data.url,
    author: data.author || 'ไม่ทราบ',
    labels: data.labels || [],
    updated_at: data.updated_at,
    updated_label: data.updated_label || data.merged_at_label || '—',
    total,
    file_count: data.changed_files ?? data.files.length,
    files: data.files || [],
    inline: [],
    large: total > model.threshold,
    needs: null,
    split: false,
    status: data.merged ? 'approved' : 'draft',
    reviewers: [],
    unsplittable: false,
  };
  renderDetail(app, model, pull, { readonly: true, history: data });
}

function renderHistory(app: HTMLElement, model: Model): void {
  const ready = state.history && state.history.repo === model.repo;
  const body = ready
    ? historyBodyHtml(model, state.history as HistoryResponse)
    : state.historyError
      ? `<div class="notice error">${icon('alert')}<span>โหลดประวัติไม่สำเร็จ: ${esc(state.historyError)}</span><button class="chip" id="history-retry" type="button">${icon('refresh', 'ico-xs')} ลองใหม่</button></div>`
      : `<div class="loading-card">${icon('refresh', 'ico spin')}<span>กำลังโหลดประวัติ commit และ merge…</span></div>`;
  app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <h1>ประวัติ commit &amp; merge</h1>
    <div class="muted">${esc(model.repo)} · ข้อมูล ${esc(model.fetched_label)}</div>
  </div>
  ${pickerHtml(model)}
</div>
${body}`;
  wire();
  if (!ready && !state.historyLoading && state.historyRepo !== model.repo) void loadHistory(model.repo);
}

function renderPeople(app: HTMLElement, model: Model): void {
  app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <h1>รายคน</h1>
    <div class="muted">${esc(model.repo)} · ${model.workload.length} คน · ข้อมูล ${esc(model.fetched_label)}</div>
  </div>
  ${pickerHtml(model)}
</div>
<p class="muted">คลิกชื่อเพื่อเปิดโปรไฟล์: PR ที่ได้รับ ไฟล์ที่ต้องดู และประวัติรีวิว • แท่ง: <span class="key pend"></span> ค้าง <span class="key done"></span> รีวิวแล้ว</p>
${workloadTableHtml(model)}`;
  wire();
}

const profileText = (v: unknown): string => {
  if (v === null || v === undefined || v === '') return '—';
  if (typeof v === 'boolean') return v ? 'ใช่' : 'ไม่ใช่';
  if (Array.isArray(v)) return v.length ? JSON.stringify(v) : '—';
  if (typeof v === 'object') return JSON.stringify(v);
  return String(v);
};

const profileUrl = (v: unknown): string | null =>
  typeof v === 'string' && /^https?:\/\//i.test(v.trim()) ? v.trim() : null;

function profileValueHtml(v: unknown): string {
  const url = profileUrl(v);
  if (url) return `<a href="${esc(url)}" target="_blank" rel="noreferrer">${esc(url)}</a>`;
  return esc(profileText(v));
}

function profileDate(v: unknown): string {
  if (typeof v !== 'string' || !v) return '—';
  const d = new Date(v);
  if (Number.isNaN(d.getTime())) return v;
  return d.toLocaleString('th-TH', { dateStyle: 'medium', timeStyle: 'short' });
}

function profileAvatar(url: unknown, login: string, cls: string): string {
  if (typeof url === 'string' && url) {
    return `<img class="${cls}" src="${esc(url)}" alt="รูปโปรไฟล์ @${esc(login)}" loading="lazy" referrerpolicy="no-referrer">`;
  }
  return `<span class="${cls} avatar-fallback">${esc(login.slice(0, 1).toUpperCase() || '?')}</span>`;
}

function profileBodyHtml(p: ProfileResponse): string {
  const u = p.user || {};
  const login = typeof u.login === 'string' ? u.login : state.login;
  const name = typeof u.name === 'string' && u.name.trim() ? u.name.trim() : login;
  const bio = typeof u.bio === 'string' ? u.bio.trim() : '';
  const htmlUrl = profileUrl(u.html_url);
  const orgs = Array.isArray(p.organizations) ? p.organizations : [];
  const emails = Array.isArray(p.emails) ? p.emails : [];
  const social = Array.isArray(p.social_accounts) ? p.social_accounts : [];
  const count = (v: unknown): number => (typeof v === 'number' ? v : Number(v) || 0);
  const blog = typeof u.blog === 'string' ? u.blog.trim() : '';
  const blogHref = blog ? (/^https?:\/\//i.test(blog) ? blog : `https://${blog}`) : '';
  const twitter = typeof u.twitter_username === 'string' ? u.twitter_username.trim() : '';
  const email = typeof u.email === 'string' ? u.email.trim() : '';

  const details: [string, string][] = [
    ['ชื่อ', profileValueHtml(u.name)],
    ['Bio', profileValueHtml(u.bio)],
    ['บริษัท', profileValueHtml(u.company)],
    ['ที่อยู่', profileValueHtml(u.location)],
    ['เว็บไซต์', blogHref ? `<a href="${esc(blogHref)}" target="_blank" rel="noreferrer">${esc(blog)}</a>` : '—'],
    ['Twitter/X', twitter ? `<a href="https://x.com/${esc(encodeURIComponent(twitter))}" target="_blank" rel="noreferrer">@${esc(twitter)}</a>` : '—'],
    ['อีเมลสาธารณะ', email ? `<a href="mailto:${esc(email)}">${esc(email)}</a>` : '—'],
    ['รับงาน', profileValueHtml(u.hireable)],
    ['ประเภทบัญชี', profileValueHtml(u.type)],
    ['แอดมิน GitHub', profileValueHtml(u.site_admin)],
    ['สมัครเมื่อ', esc(profileDate(u.created_at))],
    ['อัปเดตล่าสุด', esc(profileDate(u.updated_at))],
  ];

  const rawRows = Object.entries(u)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([k, v]) => `<tr><th>${esc(k)}</th><td>${profileValueHtml(v)}</td></tr>`)
    .join('');

  const orgHtml = orgs.length
    ? `<div class="org-list">${orgs
        .map((o) => {
          const login = typeof o.login === 'string' ? o.login : '';
          const href = profileUrl(o.html_url);
          const inner = `${profileAvatar(o.avatar_url, login, 'org-avatar')}<span><b>@${esc(login)}</b>${
            o.description ? `<span class="muted">${esc(o.description)}</span>` : ''
          }</span>`;
          return href
            ? `<a class="org" href="${esc(href)}" target="_blank" rel="noreferrer">${inner}</a>`
            : `<div class="org">${inner}</div>`;
        })
        .join('')}</div>`
    : emptyState('building', 'ไม่พบองค์กรที่มองเห็นได้', 'องค์กรสาธารณะหรือองค์กรที่ token เข้าถึงได้จะแสดงที่นี่', true);

  const emailHtml = emails.length
    ? `<div class="table-wrap"><table class="profile-table"><thead><tr><th>อีเมล</th><th>หลัก</th><th>ยืนยันแล้ว</th><th>การมองเห็น</th></tr></thead><tbody>${emails
        .map(
          (e) => `<tr><td>${esc(e.email || '—')}</td><td>${e.primary ? 'ใช่' : '—'}</td><td>${
            e.verified ? 'ใช่' : '—'
          }</td><td>${esc(e.visibility || '—')}</td></tr>`
        )
        .join('')}</tbody></table></div>`
    : emptyState('mail', 'ไม่พบอีเมล', 'ต้องอนุญาตสิทธิ์ user:email และล็อกอินใหม่เพื่อให้อ่านอีเมลได้', true);

  const socialHtml = social.length
    ? `<ul class="social-list">${social
        .map((s) => `<li><b>${esc(s.provider || '—')}</b> ${profileValueHtml(s.url)}</li>`)
        .join('')}</ul>`
    : emptyState('link', 'ไม่มีบัญชีโซเชียลที่เชื่อมไว้', 'บัญชีที่เชื่อมกับ GitHub จะแสดงที่นี่', true);

  return `<section class="profile-hero">
  ${profileAvatar(u.avatar_url, login, 'avatar')}
  <div class="profile-id">
    <h1 class="d-title">${esc(name)}</h1>
    <div class="muted">${htmlUrl ? `<a href="${esc(htmlUrl)}" target="_blank" rel="noreferrer">@${esc(login)}</a>` : `@${esc(login)}`}${
      typeof u.type === 'string' ? ` · ${esc(u.type)}` : ''
    }${u.site_admin ? ' · GitHub Staff' : ''}</div>
    ${bio ? `<p class="profile-bio">${esc(bio)}</p>` : ''}
    <div class="profile-links">
      ${htmlUrl ? `<a class="profile-link" href="${esc(htmlUrl)}" target="_blank" rel="noreferrer">${icon('external', 'ico-xs')} เปิดโปรไฟล์บน GitHub</a>` : ''}
      ${blogHref ? `<a class="profile-link" href="${esc(blogHref)}" target="_blank" rel="noreferrer">${icon('link', 'ico-xs')} ${esc(blog)}</a>` : ''}
      ${twitter ? `<a class="profile-link" href="https://x.com/${esc(encodeURIComponent(twitter))}" target="_blank" rel="noreferrer">${icon('external', 'ico-xs')} @${esc(twitter)}</a>` : ''}
    </div>
  </div>
</section>

<section class="kpis">
  ${kpi('Public repos', count(u.public_repos), '', { kind: 'link', href: `https://github.com/${encodeURIComponent(login)}?tab=repositories`, external: true })}
  ${kpi('Public gists', count(u.public_gists), '', { kind: 'link', href: `https://gist.github.com/${encodeURIComponent(login)}`, external: true })}
  ${kpi('Followers', count(u.followers), 'ok', { kind: 'link', href: `https://github.com/${encodeURIComponent(login)}?tab=followers`, external: true })}
  ${kpi('Following', count(u.following), 'info', { kind: 'link', href: `https://github.com/${encodeURIComponent(login)}?tab=following`, external: true })}
</section>

<h2>ข้อมูลโปรไฟล์</h2>
<div class="profile-grid">${details
    .map(([label, value]) => `<div class="profile-field"><span>${esc(label)}</span><b>${value}</b></div>`)
    .join('')}</div>

<h2>องค์กร (${orgs.length})</h2>
${orgHtml}

<h2>อีเมล (${emails.length})</h2>
${emailHtml}

<h2>บัญชีโซเชียล (${social.length})</h2>
${socialHtml}

<details class="profile-raw">
  <summary>ข้อมูลดิบทั้งหมดจาก GitHub (${Object.keys(u).length} ฟิลด์)</summary>
  <div class="table-wrap"><table class="profile-table"><tbody>${rawRows}</tbody></table></div>
</details>`;
}

async function loadProfile(): Promise<void> {
  if (state.profileLoading) return;
  state.profileLoading = true;
  state.profileError = '';
  if (state.view === 'profile' && state.model) render(state.model);
  try {
    const res = await fetch('/api/profile', { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    state.profile = (await res.json()) as ProfileResponse;
  } catch (e) {
    state.profileError = (e as Error).message;
  } finally {
    state.profileLoading = false;
    if (state.view === 'profile' && state.model) render(state.model);
  }
}

function renderProfile(app: HTMLElement, model: Model): void {
  const body = state.profileError
    ? `<div class="profile-error">${icon('alert')}<p>โหลดโปรไฟล์ไม่สำเร็จ: ${esc(state.profileError)}</p><button class="back" id="profile-retry" type="button">${icon('refresh', 'ico-xs')} ลองใหม่</button></div>`
    : state.profile
      ? profileBodyHtml(state.profile)
      : `<div class="profile-loading">${icon('refresh', 'ico spin')} กำลังดึงข้อมูลโปรไฟล์จาก GitHub…</div>`;
  app.innerHTML = `${navHtml(model)}
<div class="top">
  <div>
    <h1>โปรไฟล์ GitHub</h1>
    <div class="muted">${state.login ? `@${esc(state.login)}` : 'ผู้ใช้ที่ล็อกอิน'} · ข้อมูลสดจาก GitHub API</div>
  </div>
  ${pickerHtml(model)}
</div>
${body}`;
  wire();
  const retry = document.getElementById('profile-retry');
  if (retry)
    retry.addEventListener('click', () => {
      state.profileError = '';
      if (state.model) render(state.model);
    });
  if (!state.profile && !state.profileLoading && !state.profileError) void loadProfile();
}

function render(model: Model): void {
  const app = document.getElementById('app');
  if (!app) return;
  if (state.person && renderPerson(app, model, state.person)) return;
  if (state.person) {
    state.person = null;
    setParam('person', null);
  }
  if (state.view === 'history' && state.historyPr != null) {
    renderHistoryPr(app, model, state.historyPr);
    return;
  }
  if (!state.pr && state.view === 'profile') {
    renderProfile(app, model);
    return;
  }
  if (!state.pr && state.view === 'history') {
    renderHistory(app, model);
    return;
  }
  if (!state.pr && state.view === 'mine') {
    renderMine(app, model);
    return;
  }
  if (!state.pr && state.view === 'people') {
    renderPeople(app, model);
    return;
  }
  const target = state.pr ? model.pulls.find((p) => p.number === state.pr) : undefined;
  if (target) {
    renderDetail(app, model, target);
  } else {
    if (state.pr) {
      state.pr = null;
      setParam('pr', null);
    }
    renderList(app, model);
  }
}

function openPerson(login: string | null): void {
  state.person = login;
  state.pr = null;
  state.historyPr = null;
  state.lineDraft = null;
  setParam('person', login);
  setParam('pr', null);
  if (state.model) render(state.model);
}

function filterMatch(card: HTMLElement, filter: string): boolean {
  const status = card.dataset.status || '';
  if (filter === 'all') return true;
  if (filter === 'attention') return status === 'changes' || status === 'needs';
  if (filter === 'large') return card.dataset.large === '1';
  if (filter === 'stale') return card.dataset.stale === '1';
  return status === filter;
}

function syncFilterUi(): void {
  document.querySelectorAll<HTMLButtonElement>('button[data-metric-filter]').forEach((btn) => {
    btn.setAttribute('aria-pressed', String((btn.dataset.metricFilter || 'all') === state.filter));
  });
}

function applyFilter(): void {
  const cards = Array.from(document.querySelectorAll<HTMLElement>('.pr[data-status]'));
  const term = state.q.trim().toLowerCase();
  let shown = 0;
  cards.forEach((c) => {
    const okStatus = filterMatch(c, state.filter);
    const okTerm = !term || (c.dataset.search || '').includes(term);
    const ok = okStatus && okTerm;
    c.style.display = ok ? '' : 'none';
    if (ok) shown++;
  });
  syncFilterUi();
  const empty = document.getElementById('empty');
  if (empty) empty.style.display = cards.length && !shown ? '' : 'none';
}

function setFilter(filter: string, anchor?: string): void {
  state.filter = filter || 'all';
  applyFilter();
  if (!anchor) return;
  requestAnimationFrame(() => document.getElementById(anchor)?.scrollIntoView({ behavior: 'smooth', block: 'start' }));
}

function openPr(number: number | null): void {
  state.pr = number;
  state.person = null;
  state.historyPr = null;
  state.lineDraft = null;
  setParam('person', null);
  setParam('pr', number ? String(number) : null);
  if (state.model) render(state.model);
}

function wire(): void {
  const q = document.getElementById('q') as HTMLInputElement | null;
  if (q) {
    q.addEventListener('input', () => {
      state.q = q.value;
      applyFilter();
    });
  }
  document.querySelectorAll<HTMLButtonElement>('button[data-dtab]').forEach((btn) => {
    btn.addEventListener('click', () => {
      const tab = btn.dataset.dtab as DetailTab;
      state.detailTab = tab;
      document.querySelectorAll<HTMLButtonElement>('button[data-dtab]').forEach((b) =>
        b.setAttribute('aria-selected', String(b === btn))
      );
      document.querySelectorAll<HTMLElement>('[data-tab-panel]').forEach((panel) => {
        panel.style.display = panel.dataset.tabPanel === tab ? '' : 'none';
      });
    });
  });
  document.querySelectorAll<HTMLAnchorElement>('a[data-nav]').forEach((a) => {
    a.addEventListener('click', (e) => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
      e.preventDefault();
      state.view = viewOf(a.dataset.nav || null);
      state.person = null;
      state.pr = null;
      state.historyPr = null;
      state.lineDraft = null;
      setParam('view', state.view === 'list' ? null : state.view);
      setParam('person', null);
      setParam('pr', null);
      if (state.model) render(state.model);
    });
  });
  document.querySelectorAll<HTMLButtonElement>('button[data-metric-filter]').forEach((btn) => {
    btn.addEventListener('click', () => setFilter(btn.dataset.metricFilter || 'all', btn.dataset.metricAnchor));
  });
  document.querySelectorAll<HTMLElement>('article.pr[data-pr-number], tr[data-pr-number]').forEach((el) => {
    el.addEventListener('click', (e) => {
      const target = e.target as HTMLElement;
      if (target.closest('a, button, input, select, textarea, label')) return;
      if (window.getSelection()?.toString().trim()) return;
      const n = Number(el.dataset.prNumber);
      if (Number.isFinite(n) && n > 0) openPr(n);
    });
  });
  document.querySelectorAll<HTMLElement>('.who[data-login]').forEach((btn) => {
    btn.addEventListener('click', (e) => {
      e.preventDefault();
      openPerson(btn.dataset.login || null);
    });
  });
  wireRows();
  const wlHide = document.getElementById('wl-hide') as HTMLInputElement | null;
  if (wlHide) {
    wlHide.addEventListener('change', () => {
      state.wlHide = wlHide.checked;
      renderWorkloadBody();
    });
  }
  const wlSort = document.getElementById('wl-sort') as HTMLSelectElement | null;
  if (wlSort) {
    wlSort.addEventListener('change', () => {
      state.wlSort = wlSort.value as WlSort;
      renderWorkloadBody();
    });
  }
  document.querySelectorAll<HTMLAnchorElement>('a[data-pr-link]').forEach((a) => {
    a.addEventListener('click', (e) => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
      e.preventDefault();
      const n = Number(a.dataset.prLink);
      if (Number.isFinite(n) && n > 0) {
        state.person = null;
        setParam('person', null);
        openPr(n);
      }
    });
  });
  document.querySelectorAll<HTMLAnchorElement>('a.title[data-pr]').forEach((a) => {
    a.addEventListener('click', (e) => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
      e.preventDefault();
      openPr(Number(a.dataset.pr));
    });
  });
  document.querySelectorAll<HTMLAnchorElement>('a[data-history-pr]').forEach((a) => {
    a.addEventListener('click', (e) => {
      if (e.metaKey || e.ctrlKey || e.shiftKey || e.button !== 0) return;
      e.preventDefault();
      const n = Number(a.dataset.historyPr);
      if (Number.isFinite(n) && n > 0) openHistoryPr(n);
    });
  });
  const back = document.getElementById('back-list');
  if (back)
    back.addEventListener('click', () => {
      state.person = null;
      setParam('person', null);
      if (state.historyPr != null) {
        closeHistoryPr();
        return;
      }
      openPr(null);
    });
  const logout = document.getElementById('logout');
  if (logout) {
    logout.addEventListener('click', (e) => {
      e.preventDefault();
      void (async () => {
        await fetch('/auth/logout', { method: 'POST' });
        location.reload();
      })();
    });
  }
  const reviewForm = document.getElementById('review-form') as HTMLFormElement | null;
  if (reviewForm) {
    reviewForm.addEventListener('submit', (e) => {
      e.preventDefault();
      void submitReview(reviewForm);
    });
  }
  document.querySelectorAll<HTMLButtonElement>('button[data-line-comment]').forEach((btn) => {
    btn.addEventListener('click', () => openLineComment(btn));
  });
  document.querySelectorAll<HTMLElement>('tr[data-line-target]').forEach((row) => {
    row.addEventListener('click', (e) => {
      const target = e.target as HTMLElement;
      if (target.closest('button, a, textarea, input, select, label')) return;
      if (window.getSelection()?.toString().trim()) return;
      openLineComment(row);
    });
  });
  wireLineCommentComposer();
  const mergeForm = document.getElementById('merge-form') as HTMLFormElement | null;
  if (mergeForm) {
    mergeForm.addEventListener('submit', (e) => {
      e.preventDefault();
      void submitMerge(mergeForm);
    });
  }
  const select = document.getElementById('project') as HTMLSelectElement | null;
  if (select) {
    select.addEventListener('change', () => {
      state.repo = select.value;
      state.person = null;
      state.pr = null;
      state.historyPr = null;
      state.lineDraft = null;
      setParam('repo', state.repo);
      setParam('person', null);
      setParam('pr', null);
      void load();
    });
  }
  const clear = document.getElementById('clear-person');
  if (clear) clear.addEventListener('click', () => openPerson(null));
  const histRetry = document.getElementById('history-retry');
  if (histRetry)
    histRetry.addEventListener('click', () => {
      state.history = null;
      state.historyRepo = '';
      state.historyError = '';
      if (state.model) render(state.model);
    });
  const histPrRetry = document.getElementById('history-pr-retry');
  if (histPrRetry)
    histPrRetry.addEventListener('click', () => {
      if (state.historyPr != null) {
        historyPrCache.delete(historyPrKey(state.repo, state.historyPr));
        state.historyPrError = '';
        if (state.model) render(state.model);
      }
    });
}

async function loadProjects(): Promise<void> {
  try {
    const res = await fetch('/api/projects', { cache: 'no-store' });
    if (!res.ok) return;
    const data = (await res.json()) as ProjectsResponse;
    state.projects = (data.projects || []).map((p) => p.repo);
    state.canWrite = !!data.write;
    state.authEnabled = !!data.auth?.enabled;
    state.login = data.auth?.login || '';
    state.viewer = state.login || data.viewer || '';
    if (!state.repo) state.repo = data.default || state.projects[0] || '';
  } catch {
    /* ไม่มี /api/projects: ใช้โปรเจคเดี่ยวจาก /api/data.json */
  }
}

let lastVersion: string | null = null;
let lastJson = '';

/** version ของเนื้อหา (hash) — เรียกเบามาก ใช้ตัดสินว่าจะดึงข้อมูลใหม่หรือไม่ */
async function currentVersion(): Promise<string | null> {
  const suffix = state.repo ? `?repo=${encodeURIComponent(state.repo)}` : '';
  try {
    const res = await fetch(`/api/version${suffix}`, { cache: 'no-store' });
    if (!res.ok) return null;
    const data = (await res.json()) as { version: string };
    return data.version || null;
  } catch {
    return null;
  }
}

async function load(): Promise<void> {
  if (!state.projects.length) await loadProjects();
  const suffix = state.repo ? `?repo=${encodeURIComponent(state.repo)}` : '';
  try {
    const res = await fetch(`/api/data.json${suffix}`, { cache: 'no-store' });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const model = (await res.json()) as Model;
    const json = JSON.stringify(model);
    state.model = model;
    state.repo = model.repo;
    lastVersion = (await currentVersion()) ?? lastVersion;
    if (json !== lastJson) {
      if (state.pr) mergeInfoCache.delete(`${state.repo}#${state.pr}`);
      lastJson = json;
      render(model);
    }
  } catch (e) {
    const loading = document.getElementById('loading');
    if (loading) loading.textContent = `โหลดข้อมูลไม่สำเร็จ: ${(e as Error).message}`;
    const bootIcon = document.getElementById('boot-icon');
    if (bootIcon) bootIcon.classList.remove('spin');
    const sub = document.getElementById('sub');
    if (sub) sub.textContent = 'ต้องรัน dashboard-rs (--serve) แล้วเปิดผ่าน Vite dev server หรือ reverse proxy';
  }
}

void load();
// เรียลไทม์-ish: poll /api/version ทุก 3 วิ (เบามาก) แล้วดึงข้อมูลเฉพาะเมื่อเนื้อหาเปลี่ยน
setInterval(() => {
  void (async () => {
    const v = await currentVersion();
    if (v && v !== lastVersion) await load();
  })();
}, 3_000);
