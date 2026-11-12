'use strict';
// ทดสอบ flow หลักด้วย GitHub API จำลองในหน่วยความจำ (ไม่ต้องใช้เครือข่าย)
// รัน: npm install --no-save js-yaml@4.1.0 && node --test .github/scripts/auto-review.integration.test.js
const test = require('node:test');
const assert = require('node:assert');
const path = require('path');
const run = require('./auto-review.cjs');

process.env.GITHUB_WORKSPACE = path.resolve(__dirname, '../..');

// ใช้ config ของเทสต์เอง (team สมมติ) ไม่ผูกกับ .github/auto-review.yml จริงที่มีชื่อคนจริง
const fs = require('fs');
const CFG_BASE = path.join(__dirname, 'tmp-base.yml');
fs.writeFileSync(CFG_BASE, 'team: [alice, bob, carol, dave]\nunavailable: []\nexclude_patterns:\n  - "**/package-lock.json"\n');
process.env.AUTO_REVIEW_CONFIG = path.relative(process.env.GITHUB_WORKSPACE, CFG_BASE);
test.after(() => {
  delete process.env.AUTO_REVIEW_CONFIG;
  try {
    fs.unlinkSync(CFG_BASE);
  } catch {}
});

function fakeRepo({ prs = [], files = {}, reviews = {}, comments = {} }) {
  const calls = { requested: [], created: [], updated: [], deleted: [], labels: [] };
  const byNum = (n) => prs.find((p) => p.number === n);
  const rest = {
    pulls: {
      get: async ({ pull_number }) => ({ data: byNum(pull_number) }),
      list: async ({ state }) => ({ data: prs.filter((p) => (state === 'open' ? p.state === 'open' : p.state !== 'open')) }),
      listFiles: async ({ pull_number }) => ({ data: files[pull_number] || [] }),
      listReviews: async ({ pull_number }) => ({ data: reviews[pull_number] || [] }),
      requestReviewers: async ({ pull_number, reviewers }) => {
        calls.requested.push({ pull_number, reviewers });
        const p = byNum(pull_number);
        reviewers.forEach((r) => p.requested_reviewers.push({ login: r }));
        return {};
      },
    },
    issues: {
      listComments: async ({ issue_number }) => ({ data: comments[issue_number] || [] }),
      createComment: async ({ issue_number, body }) => {
        calls.created.push({ issue_number, body });
        (comments[issue_number] ||= []).push({ id: Date.now(), user: { login: 'github-actions[bot]' }, body });
        return {};
      },
      updateComment: async ({ comment_id, body }) => {
        calls.updated.push({ comment_id, body });
        for (const list of Object.values(comments)) list.forEach((c) => c.id === comment_id && (c.body = body));
        return {};
      },
      deleteComment: async ({ comment_id }) => {
        calls.deleted.push(comment_id);
        for (const k of Object.keys(comments)) comments[k] = comments[k].filter((c) => c.id !== comment_id);
        return {};
      },
      createLabel: async () => ({}),
      addLabels: async ({ issue_number, labels }) => {
        calls.labels.push({ issue_number, add: labels });
        byNum(issue_number).labels.push(...labels.map((name) => ({ name })));
        return {};
      },
      removeLabel: async ({ issue_number, name }) => {
        calls.labels.push({ issue_number, remove: name });
        const p = byNum(issue_number);
        p.labels = p.labels.filter((l) => l.name !== name);
        return {};
      },
    },
  };
  const github = {
    rest,
    paginate: Object.assign(async (fn, params) => (await fn(params)).data, {
      iterator: (fn, params) => ({ async *[Symbol.asyncIterator]() { yield await fn(params); } }),
    }),
  };
  return { github, calls };
}

const mkPr = (number, author, extra = {}) => ({ number, state: 'open', draft: false, user: { login: author, type: 'User' }, labels: [], requested_reviewers: [], created_at: new Date().toISOString(), updated_at: new Date().toISOString(), ...extra });
const ctx = (n) => ({ repo: { owner: 'o', repo: 'r' }, eventName: 'pull_request_target', payload: { pull_request: { number: n } } });
const core = { info() {}, warning() {}, setFailed(m) { throw new Error(m); }, summary: { addRaw() { return this; }, addHeading() { return this; }, addList() { return this; }, async write() {} } };
const f = (name, additions, deletions = 0) => ({ filename: name, additions, deletions });

test('PR เล็ก: เลือก 1 คน ไม่ใช่ผู้เขียน และสร้างคอมเมนต์พร้อม marker', async () => {
  const pr = mkPr(1, 'alice');
  const { github, calls } = fakeRepo({ prs: [pr], files: { 1: [f('src/a.js', 100), f('package-lock.json', 5000)] } });
  await run({ github, context: ctx(1), core });
  assert.strictEqual(calls.requested.length, 1);
  assert.strictEqual(calls.requested[0].reviewers.length, 1);
  assert.notStrictEqual(calls.requested[0].reviewers[0].toLowerCase(), 'alice');
  assert.match(calls.created[0].body, /100\*\* บรรทัด/); // lock file ไม่ถูกนับ
  assert.match(calls.created[0].body, /auto-review-state/);
});

test('PR ใหญ่ 800 บรรทัด: 2 คน แบ่งไฟล์ไม่ซ้ำ', async () => {
  const pr = mkPr(2, 'bob');
  const { github, calls } = fakeRepo({ prs: [pr], files: { 2: [f('a', 300), f('b', 250), f('c', 150), f('d', 100)] } });
  await run({ github, context: ctx(2), core });
  assert.strictEqual(calls.requested[0].reviewers.length, 2);
  assert.ok(!calls.requested[0].reviewers.map((x) => x.toLowerCase()).includes('bob'));
  assert.match(calls.created[0].body, /ขอบเขตการรีวิว/);
});

test('คนเลือกเอง 1 คน + PR ใหญ่: คงคนเดิมและเพิ่มอีก 1 คน', async () => {
  const pr = mkPr(3, 'alice', { requested_reviewers: [{ login: 'bob' }] });
  const { github, calls } = fakeRepo({ prs: [pr], files: { 3: [f('a', 500), f('b', 400)] } });
  await run({ github, context: ctx(3), core });
  assert.strictEqual(calls.requested.length, 1);
  assert.strictEqual(calls.requested[0].reviewers.length, 1);
  assert.notStrictEqual(calls.requested[0].reviewers[0].toLowerCase(), 'bob');
});

test('PR เล็กที่มีคนเลือกเองแล้ว: ไม่เพิ่มใคร (ค่าเริ่มต้น)', async () => {
  const pr = mkPr(4, 'alice', { requested_reviewers: [{ login: 'carol' }] });
  const { github, calls } = fakeRepo({ prs: [pr], files: { 4: [f('a', 50)] } });
  await run({ github, context: ctx(4), core });
  assert.strictEqual(calls.requested.length, 0);
});

test('Draft: ไม่ทำอะไร', async () => {
  const pr = mkPr(5, 'alice', { draft: true });
  const { github, calls } = fakeRepo({ prs: [pr], files: { 5: [f('a', 50)] } });
  await run({ github, context: ctx(5), core });
  assert.strictEqual(calls.requested.length + calls.created.length, 0);
});

test('รันซ้ำ (idempotent): ไม่ request ซ้ำ ไม่สร้างคอมเมนต์ซ้ำ', async () => {
  const pr = mkPr(6, 'dave');
  const { github, calls } = fakeRepo({ prs: [pr], files: { 6: [f('a', 120)] } });
  await run({ github, context: ctx(6), core });
  await run({ github, context: ctx(6), core });
  assert.strictEqual(calls.requested.length, 1);
  assert.strictEqual(calls.created.length, 1);
});

test('PR โตหลังเปิด (synchronize): เพิ่ม Reviewer คนที่สอง', async () => {
  const pr = mkPr(7, 'alice');
  const files = { 7: [f('a', 100)] };
  const { github, calls } = fakeRepo({ prs: [pr], files });
  await run({ github, context: ctx(7), core });
  assert.strictEqual(calls.requested.length, 1);
  files[7].push(f('big', 700));
  await run({ github, context: { ...ctx(7), payload: { action: 'synchronize', pull_request: { number: 7 } } }, core });
  assert.strictEqual(calls.requested.length, 2);
  assert.strictEqual(pr.requested_reviewers.length, 2);
});

test('ready_for_review: draft ถูกข้าม, พร้อมแล้วจึง assign และใช้คอมเมนต์เดิมต่อ', async () => {
  const pr = mkPr(9, 'alice', { draft: true });
  const files = { 9: [f('a', 40)] };
  const { github, calls } = fakeRepo({ prs: [pr], files });
  const ev = { ...ctx(9), payload: { action: 'ready_for_review', pull_request: { number: 9 } } };

  await run({ github, context: ev, core });
  assert.strictEqual(calls.requested.length + calls.created.length, 0, 'draft ต้องไม่ทำอะไร');

  pr.draft = false;
  files[9].push(f('b', 60));
  await run({ github, context: ev, core });
  assert.strictEqual(calls.requested.length, 1, 'ready_for_review ต้อง assign');
  assert.strictEqual(calls.created.length, 1);
  assert.match(calls.created[0].body, /100\*\* บรรทัด/);

  // รันซ้ำ event เดิม: ใช้คอมเมนต์ใบเดิม ข้อความถูก normalize 1 ครั้ง (ตัด Score ที่ไม่รู้ค่าออก) แล้วนิ่ง
  await run({ github, context: ev, core });
  assert.strictEqual(calls.created.length, 1, 'ต้องไม่สร้างคอมเมนต์ซ้ำ');
  const afterNormalize = calls.updated.length;
  assert.ok(afterNormalize <= 1, 'เขียนซ้ำได้ไม่เกิน 1 ครั้งเพื่อ normalize ข้อความ');
  await run({ github, context: ev, core });
  assert.strictEqual(calls.updated.length, afterNormalize, 'state นิ่งแล้ว: ต้องไม่เขียนซ้ำอีก');
  assert.strictEqual(calls.requested.length, 1, 'ต้องไม่ request ซ้ำ');
});

test('คอมเมนต์ state ซ้ำ: ใช้ใบล่าสุด ลบใบเก่า และนับภาระจาก state ล่าสุด', async () => {
  const pr11 = mkPr(11, 'carol', { requested_reviewers: [{ login: 'bob' }] });
  const pr12 = mkPr(12, 'alice');
  const body = (total, lines) =>
    `ข้อความ\n\n<!-- auto-review-state: ${JSON.stringify({ v: 1, total, lines, bot: ['bob'], at: '2026-11-01T00:00:00.000Z', needs: null, reminded: false, balanced: false })} -->`;
  const comments = {
    // ใบเก่า (id 10) บอกงานค้าง 4000 บรรทัด ซึ่งเกิน pending_cap 800 -> ถ้าใช้ใบนี้จะข้าม bob
    11: [
      { id: 10, user: { login: 'github-actions[bot]' }, body: body(4000, { bob: 4000 }) },
      { id: 20, user: { login: 'github-actions[bot]' }, body: body(10, { bob: 10 }) },
    ],
  };
  const { github, calls } = fakeRepo({ prs: [pr11, pr12], files: { 11: [f('x', 10)], 12: [f('y', 50)] }, comments });

  await run({ github, context: { repo: { owner: 'o', repo: 'r' }, eventName: 'workflow_dispatch', payload: {} }, core });

  // state ล่าสุด (id 20) -> bob มี P=10 ไม่เกินเพดาน จึงยังถูกเลือกได้ (ถ้าใช้ใบเก่าจะเลือก carol แทน)
  assert.deepStrictEqual(calls.requested.map((c) => c.reviewers.map((r) => r.toLowerCase())), [['bob']]);
  // คอมเมนต์ซ้ำบน #11 ถูกลบเหลือใบเดียว (ใบล่าสุด id 20)
  assert.deepStrictEqual(calls.deleted, [10]);
  const marked = comments[11].filter((c) => c.body.includes('auto-review-state'));
  assert.strictEqual(marked.length, 1);
  assert.strictEqual(marked[0].id, 20);
});

test('ไม่มีคนเลือกได้ (ทุกคนลา): ไม่ assign ใคร ติด label needs-reviewer', async () => {
  const cfgPath = path.join(__dirname, 'tmp-all-away.yml');
  require('fs').writeFileSync(cfgPath, 'team: [alice, bob]\nunavailable: [bob]\nexclude_patterns: []\n');
  process.env.AUTO_REVIEW_CONFIG = path.relative(process.env.GITHUB_WORKSPACE, cfgPath);
  try {
    const pr = mkPr(8, 'alice');
    const { github, calls } = fakeRepo({ prs: [pr], files: { 8: [f('a', 50)] } });
    await run({ github, context: ctx(8), core });
    assert.strictEqual(calls.requested.length, 0);
    assert.ok(calls.labels.some((l) => l.add?.includes('needs-reviewer')));
    assert.match(calls.created[0].body, /ยังไม่มี Reviewer/);
  } finally {
    process.env.AUTO_REVIEW_CONFIG = path.relative(process.env.GITHUB_WORKSPACE, CFG_BASE);
    require('fs').unlinkSync(cfgPath);
  }
});
