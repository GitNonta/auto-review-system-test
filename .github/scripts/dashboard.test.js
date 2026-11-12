'use strict';
// รัน: node --test .github/scripts/dashboard.test.js
const test = require('node:test');
const assert = require('node:assert');
const { pure } = require('./dashboard.cjs');
const { buildModel, renderHtml, readConfigLite, parseArgs, formatRanges, relTime } = pure;

const now = new Date().toISOString();

function pr(over = {}) {
  return {
    number: 1,
    title: 'PR ทดสอบ',
    url: 'https://github.com/o/r/pull/1',
    author: 'GitNonta',
    draft: false,
    created_at: now,
    updated_at: now,
    labels: [],
    files: [],
    reviews: [],
    requested: [],
    bot_state: null,
    ...over,
  };
}

const bigFile = (lines = 700, patch) => ({
  filename: 'src/HugeTest.php',
  additions: lines,
  deletions: 0,
  patch:
    patch ??
    `@@ -0,0 +1,${lines} @@\n` + Array.from({ length: lines }, (_, i) => `+line ${i + 1}`).join('\n'),
});

test('readConfigLite: อ่าน key แบนและ list โดยไม่สน comment/nested key', () => {
  const cfg = readConfigLite(`# comment
team:
  - GitNonta
  - Themynon2   # trailing
unavailable: []
large_pr_threshold: 600
labels:
  needs_reviewer: needs-reviewer
exclude_patterns:
  - "**/package-lock.json"
  - "**/*.snap"
`);
  assert.deepStrictEqual(cfg.team, ['GitNonta', 'Themynon2']);
  assert.strictEqual(cfg.large_pr_threshold, 600);
  assert.deepStrictEqual(cfg.unavailable, []);
  assert.deepStrictEqual(cfg.exclude_patterns, ['**/package-lock.json', '**/*.snap']);
});

test('buildModel: ไฟล์เดียว 700 บรรทัด (hunk เดียว) แบ่งช่วงบรรทัดตาม state ของบอท', () => {
  const m = buildModel({
    repo: 'o/r',
    fetched_at: now,
    pulls: [
      pr({
        number: 21,
        files: [bigFile(700)],
        requested: ['Themynon2', 'Nontawatpkru17'],
        bot_state: { v: 1, total: 700, split: true, lines: { themynon2: 350, nontawatpkru17: 350 }, needs: null },
      }),
    ],
  });
  assert.strictEqual(m.pulls[0].total, 700);
  assert.strictEqual(m.pulls[0].split, true);
  assert.strictEqual(m.pulls[0].reviewers.length, 2);
  const ranges = m.pulls[0].reviewers.map((r) => formatRanges(r.parts[0].ranges));
  assert.deepStrictEqual(ranges, ['1–350', '351–700']);
  assert.deepStrictEqual(
    m.pulls[0].reviewers.map((r) => r.lines),
    [350, 350]
  );
});

test('buildModel: ไฟล์ที่ถูก exclude ไม่ถูกนับบรรทัด/ไม่โผล่ในขอบเขต', () => {
  const m = buildModel({
    pulls: [
      pr({
        files: [
          { filename: 'package-lock.json', additions: 5000, deletions: 0, patch: '@@ -1,1 +1,5000 @@' },
          { filename: 'src/a.ts', additions: 50, deletions: 0, patch: '@@ -0,0 +1,50 @@' },
        ],
      }),
    ],
  });
  assert.strictEqual(m.pulls[0].total, 50);
  assert.strictEqual(m.pulls[0].fileCount, 1);
});

test('buildModel: สถานะรีวิว (approved / changes / pending) และตัดรีวิวของ author ออก', () => {
  const m = buildModel({
    pulls: [
      pr({
        number: 1,
        files: [{ filename: 'a.ts', additions: 10, deletions: 0, patch: '@@ -0,0 +1,10 @@' }],
        requested: ['Themynon2'],
        reviews: [{ login: 'Themynon2', state: 'APPROVED', submitted_at: now }],
      }),
      pr({
        number: 2,
        files: [{ filename: 'b.ts', additions: 10, deletions: 0, patch: '@@ -0,0 +1,10 @@' }],
        requested: ['Themynon2'],
        reviews: [{ login: 'Themynon2', state: 'CHANGES_REQUESTED', submitted_at: now }],
      }),
      pr({
        number: 3,
        files: [{ filename: 'c.ts', additions: 10, deletions: 0, patch: '@@ -0,0 +1,10 @@' }],
        requested: ['GitNonta'],
        reviews: [{ login: 'GitNonta', state: 'APPROVED', submitted_at: now }],
      }),
      pr({ number: 4, files: [{ filename: 'd.ts', additions: 10, deletions: 0, patch: '@@ -0,0 +1,10 @@' }] }),
    ],
  });
  const byNum = Object.fromEntries(m.pulls.map((p) => [p.number, p.status]));
  assert.strictEqual(byNum[1], 'approved');
  assert.strictEqual(byNum[2], 'changes');
  assert.strictEqual(byNum[3], 'needs'); // รีวิวของ author ถูกตัด -> ไม่มี reviewer
  assert.strictEqual(byNum[4], 'needs');
  assert.strictEqual(m.summary.approved, 1);
  assert.strictEqual(m.summary.changes, 1);
  assert.strictEqual(m.summary.needs, 2);
});

test('buildModel: ไฟล์ใหญ่ที่ไม่มี patch ติดธง unsplittable + เตือนในหน้า', () => {
  const m = buildModel({
    pulls: [
      pr({
        files: [{ filename: 'assets/logo.png', additions: 900, deletions: 0 }],
        requested: ['a', 'b'],
      }),
    ],
  });
  assert.strictEqual(m.pulls[0].unsplittable, true);
  assert.strictEqual(m.summary.unsplittable, 1);
  assert.match(renderHtml(m), /แยกช่วงไม่ได้/);
});

test('buildModel: ภาระงานค้างแยก approved กับ pending', () => {
  const m = buildModel({
    pulls: [
      pr({
        number: 1,
        files: [{ filename: 'a.ts', additions: 100, deletions: 0, patch: '@@ -0,0 +1,100 @@' }],
        requested: ['Themynon2'],
        reviews: [{ login: 'Themynon2', state: 'APPROVED', submitted_at: now }],
      }),
      pr({
        number: 2,
        files: [{ filename: 'b.ts', additions: 40, deletions: 0, patch: '@@ -0,0 +1,40 @@' }],
        requested: ['Themynon2'],
      }),
    ],
  });
  const w = m.workload.find((x) => x.login === 'Themynon2');
  assert.strictEqual(w.prs, 2);
  assert.strictEqual(w.done, 100);
  assert.strictEqual(w.pending, 40);
});

test('renderHtml: escape ชื่อ PR และมี KPI/ตัวกรองครบ', () => {
  const m = buildModel({
    repo: 'o/r',
    fetched_at: now,
    pulls: [pr({ title: '<img src=x onerror=alert(1)>', files: [{ filename: 'a.ts', additions: 5, deletions: 0, patch: '@@ -0,0 +1,5 @@' }] })],
  });
  const html = renderHtml(m);
  assert.ok(!html.includes('<img src=x'));
  assert.match(html, /&lt;img src=x/);
  assert.match(html, /Auto-review dashboard/);
  assert.match(html, /data-filter="approved"/);
  assert.match(html, /PR เปิดอยู่/);
});

test('formatRanges / relTime / parseArgs', () => {
  assert.strictEqual(formatRanges([{ start: 1, end: 1 }, { start: 5, end: 9 }]), '1, 5–9');
  assert.match(relTime(new Date(Date.now() - 3 * 36e5).toISOString()), /3 ชม\.ที่แล้ว/);
  const a = parseArgs(['--repo', 'o/r', '--out=d.html', '--serve', '7799']);
  assert.strictEqual(a.repo, 'o/r');
  assert.strictEqual(a.out, 'd.html');
  assert.strictEqual(a.serve, 7799);
  assert.strictEqual(a.host, '127.0.0.1');
  assert.strictEqual(parseArgs(['--serve', '7788', '--host', '0.0.0.0']).host, '0.0.0.0');
});
