'use strict';
// รัน: node --test .github/scripts/
const test = require('node:test');
const assert = require('node:assert');
const { pure } = require('./auto-review.cjs');
const { globToRegex, splitFiles, planScope, hunksFromPatch, chooseReviewer, gap, parseState, renderBody, DEFAULTS } = pure;

const cfg = { ...DEFAULTS };
const team = ['a', 'b', 'c', 'd'];
const teamIdx = Object.fromEntries(team.map((l, i) => [l, i]));
const stats = (o) => Object.fromEntries(team.map((l) => [l, { C: 0, P: 0, ...(o[l] || {}) }]));

test('glob: lock/generated/snapshot ถูกยกเว้น', () => {
  const rx = ['**/package-lock.json', '**/*.snap', '**/dist/**'].map(globToRegex);
  const ex = (f) => rx.some((r) => r.test(f));
  assert.ok(ex('package-lock.json'));
  assert.ok(ex('web/package-lock.json'));
  assert.ok(ex('src/__tests__/a.snap'));
  assert.ok(ex('app/dist/x/y.js'));
  assert.ok(!ex('src/index.js'));
});

test('splitFiles: ไม่ซ้ำ ครบ และใกล้เคียงกัน', () => {
  const files = [400, 300, 200, 100, 50].map((lines, i) => ({ filename: `f${i}`, lines }));
  const b = splitFiles(files, ['x', 'y']);
  const all = b.flatMap((x) => x.files).sort();
  assert.deepStrictEqual(all, ['f0', 'f1', 'f2', 'f3', 'f4']);
  assert.strictEqual(b[0].lines + b[1].lines, 1050);
  assert.ok(Math.abs(b[0].lines - b[1].lines) <= 100);
});

test('hunksFromPatch: แปลง hunk เป็นช่วงบรรทัดฝั่งใหม่', () => {
  const patch = '@@ -1,3 +1,4 @@\n a\n+b\n c\n@@ -20,0 +22,2 @@\n+x\n+y\n';
  const r = hunksFromPatch(patch);
  assert.deepStrictEqual(r.map((x) => [x.start, x.end]), [
    [1, 4],
    [22, 23],
  ]);
  assert.strictEqual(hunksFromPatch(''), null);
  assert.strictEqual(hunksFromPatch(null), null);
});

test('planScope: ไฟล์เดียว 1200 บรรทัด → แบ่งช่วงบรรทัด 2 คน ครอบคุลม ไม่ทับกัน', () => {
  const ranges = [];
  for (let s = 1; s <= 1200; s += 20) ranges.push({ start: s, end: s + 19, weight: 20 });
  const b = planScope([{ filename: 'big.php', lines: 1200, ranges }], ['x', 'y']);
  assert.strictEqual(b.length, 2);
  const flat = b.flatMap((x) => x.parts.flatMap((p) => p.ranges)).sort((a, c) => a.start - c.start);
  assert.strictEqual(flat[0].start, 1);
  assert.strictEqual(flat[flat.length - 1].end, 1200);
  for (let i = 1; i < flat.length; i++) assert.ok(flat[i].start > flat[i - 1].end, 'ช่วงต้องไม่ทับกัน');
  assert.strictEqual(flat.length, 2);
  assert.ok(Math.abs(b[0].lines - b[1].lines) <= 60, `${b[0].lines}/${b[1].lines}`);
  assert.ok(b.every((x) => x.parts.every((p) => p.ranges && p.ranges.length)));
});

test('planScope: หลายไฟล์เล็ก → แบ่งตามไฟล์ ไม่มีช่วงบรรทัด', () => {
  const files = [300, 250, 150, 100].map((lines, i) => ({ filename: `f${i}`, lines, ranges: null }));
  const b = planScope(files, ['x', 'y']);
  assert.strictEqual(b.flatMap((x) => x.files).length, 4);
  assert.ok(b.every((x) => x.parts.every((p) => p.ranges === null)));
  assert.ok(Math.abs(b[0].lines - b[1].lines) <= 100);
});

test('planScope: ไฟล์ใหญ่ที่ไม่มี patch → ยกทั้งไฟล์และทำเครื่องหมาย unsplittable', () => {
  const b = planScope([{ filename: 'bin.dat', lines: 900, ranges: null }], ['x', 'y']);
  const flagged = b.flatMap((x) => x.parts).filter((p) => p.unsplittable);
  assert.strictEqual(flagged.length, 1);
  assert.strictEqual(flagged[0].filename, 'bin.dat');
});

test('เลือกคน Score ต่ำสุด (ห่างเกิน tie_threshold)', () => {
  const r = chooseReviewer({ candidates: ['b', 'c', 'd'], stats: stats({ b: { C: 1000 }, c: { C: 900 }, d: { C: 100 } }), ex: {}, author: 'a', teamIdx, prNumber: 1, cfg });
  assert.strictEqual(r.login, 'd');
});

test('งานค้างถ่วง x2: คนถือ PR ใหญ่ถูกข้าม', () => {
  const r = chooseReviewer({ candidates: ['b', 'c'], stats: stats({ b: { P: 700 }, c: { C: 1000 } }), ex: {}, author: 'a', teamIdx, prNumber: 1, cfg });
  assert.strictEqual(r.login, 'c'); // b = 1400, c = 1000 (ห่าง 400 > tie_threshold)
});

test('Score ใกล้เคียง: เลือกคนที่ไม่ได้รีวิว Author นานที่สุด', () => {
  const ex = { b: { a: { count: 3, last: 3000 } }, c: { a: { count: 1, last: 1000 } } };
  const r = chooseReviewer({ candidates: ['b', 'c'], stats: stats({ b: { C: 100 }, c: { C: 250 } }), ex, author: 'a', teamIdx, prNumber: 1, cfg });
  assert.strictEqual(r.login, 'c');
});

test('Score เท่ากันและไม่มีประวัติ: Round-Robin หมุนตามเลข PR', () => {
  const picks = new Set();
  for (let n = 1; n <= 4; n++) picks.add(chooseReviewer({ candidates: ['b', 'c', 'd'], stats: stats({}), ex: {}, author: 'a', teamIdx, prNumber: n, cfg }).login);
  assert.ok(picks.size >= 3);
});

test('ทุกคนงานค้างเกินเพดาน: ยังเลือกคนต่ำสุดและตั้ง overloaded', () => {
  const r = chooseReviewer({ candidates: ['b', 'c'], stats: stats({ b: { P: 900 }, c: { P: 1200 } }), ex: {}, author: 'a', teamIdx, prNumber: 1, cfg });
  assert.strictEqual(r.login, 'b');
  assert.strictEqual(r.overloaded, true);
});

test('ไม่มี candidates: คืน null', () => {
  assert.strictEqual(chooseReviewer({ candidates: [], stats: stats({}), ex: {}, author: 'a', teamIdx, prNumber: 1, cfg }), null);
});

test('Gap = (Max-Min)/Avg*100', () => {
  assert.strictEqual(gap([100, 100, 100, 100]), 0);
  assert.strictEqual(Math.round(gap([700, 500, 500, 700])), 33);
  assert.strictEqual(gap([0, 0]), 0);
});

test('marker state: render/parse ครบรอบ', () => {
  const s = { v: 1, total: 850, lines: { b: 400, c: 450 }, needs: null };
  assert.deepStrictEqual(parseState(renderBody('ข้อความ', s)), s);
  assert.strictEqual(parseState('ไม่มี marker'), null);
});

test('จำลอง 80 PR: Gap ของภาระไม่เกิน 20% และไม่มีใครรีวิวตัวเอง', () => {
  let seed = 42;
  const rnd = () => ((seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296);
  const st = stats({});
  const ex = {};
  for (let n = 1; n <= 80; n++) {
    const author = team[Math.floor(rnd() * 4)];
    const size = Math.floor(50 + rnd() * 350);
    const r = chooseReviewer({ candidates: team.filter((l) => l !== author), stats: st, ex, author, teamIdx, prNumber: n, cfg });
    assert.notStrictEqual(r.login, author);
    st[r.login].C += size;
    ex[r.login] ??= {};
    (ex[r.login][author] ??= { count: 0, last: 0 }).last = n;
  }
  const g = gap(team.map((l) => st[l].C));
  console.log('simulated gap %', g.toFixed(1));
  assert.ok(g <= 20, `gap ${g}`);
});
