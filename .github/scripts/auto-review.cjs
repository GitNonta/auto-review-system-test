'use strict';
/**
 * Auto-assign Reviewer (Equal Review Workload)
 * Score = C + pending_weight * P   (C = บรรทัดที่รีวิวเสร็จใน N วัน, P = บรรทัดที่ยังค้าง)
 * ดูรายละเอียดใน AGENT_SPEC.md
 */
const fs = require('fs');
const path = require('path');

const MARK = '<!-- auto-review-state:';
const MARK_END = '-->';
const BOT_LOGIN = 'github-actions[bot]';

const DEFAULTS = {
  timezone: 'Asia/Bangkok',
  team: [],
  unavailable: [],
  history_days: 30,
  pending_weight: 2,
  pending_cap: 800,
  tie_threshold: 300,
  large_pr_threshold: 600,
  large_pr_reviewers: 2,
  split_large_pr: true,
  balance_extra_reviewer: false,
  balance_extra_gap: 300,
  max_reviewers: 3,
  overdue_hours: 48,
  allow_external_authors: false,
  skip_labels: ['no-auto-review'],
  exclude_patterns: [],
  labels: { needs_reviewer: 'needs-reviewer', needs_second_reviewer: 'needs-second-reviewer' },
  report: { enabled: true, issue: false, weekday: 'Mon' },
  dry_run: false,
};

/* ------------------------------ pure helpers ------------------------------ */

const norm = (s) => String(s).toLowerCase();

function globToRegex(glob) {
  let re = '';
  for (let i = 0; i < glob.length; i++) {
    const c = glob[i];
    if (c === '*') {
      if (glob[i + 1] === '*') {
        re += '.*';
        i++;
        if (glob[i + 1] === '/') i++;
      } else re += '[^/]*';
    } else if (c === '?') re += '[^/]';
    else re += c.replace(/[.+^${}()|[\]\\]/g, '\\$&');
  }
  return new RegExp('^' + re + '$');
}

const sumLines = (files) => files.reduce((s, f) => s + f.lines, 0);

/** แบ่งไฟล์ให้ reviewer หลายคนให้จำนวนบรรทัดใกล้เคียงกัน (greedy, ไฟล์ใหญ่ก่อน) */
function splitFiles(files, logins) {
  const buckets = logins.map((login) => ({ login, lines: 0, files: [] }));
  for (const f of [...files].sort((a, b) => b.lines - a.lines || a.filename.localeCompare(b.filename))) {
    buckets.sort((a, b) => a.lines - b.lines || logins.indexOf(a.login) - logins.indexOf(b.login));
    buckets[0].files.push(f.filename);
    buckets[0].lines += f.lines;
  }
  return buckets;
}

/**
 * เลือก reviewer 1 คนจาก candidates
 * 1) ตัดคนที่งานค้าง (P) เกินเพดาน (ถ้าทุกคนเกิน ใช้ทุกคนและตั้ง overloaded)
 * 2) หา Score ต่ำสุด แล้วเอาทุกคนที่ห่างไม่เกิน tie_threshold
 * 3) ในกลุ่มนั้นเลือกคนที่ไม่ได้รีวิว author นี้นานที่สุด (Exchange Matrix)
 * 4) ถ้ายังเท่ากัน ใช้ Round-Robin แบบไม่ต้องเก็บสถานะ (หมุนตามเลข PR)
 */
function chooseReviewer({ candidates, stats, ex, author, teamIdx, prNumber, cfg, extra = {} }) {
  if (!candidates.length) return null;
  const P = (l) => (stats[l]?.P || 0) + (extra[l] || 0);
  const score = (l) => (stats[l]?.C || 0) + cfg.pending_weight * P(l);
  const under = candidates.filter((l) => P(l) < cfg.pending_cap);
  const pool = under.length ? under : candidates;
  const min = Math.min(...pool.map(score));
  const group = pool.filter((l) => score(l) - min <= cfg.tie_threshold);
  const last = (l) => ex[l]?.[author]?.last || 0;
  const oldest = Math.min(...group.map(last));
  const g2 = group.filter((l) => last(l) === oldest);
  // Round-Robin หมุนตาม candidate pool จริง (ไม่ใช่ index ทั้ง team) เพื่อไม่ให้คนที่ไม่ได้เป็น candidate มากินสล็อต
  const n = Math.max(candidates.length, 1);
  const rr = (l) => (((candidates.indexOf(l) - (prNumber % n)) % n) + n) % n;
  g2.sort((a, b) => rr(a) - rr(b));
  return { login: g2[0], score: score(g2[0]), overloaded: under.length === 0 };
}

function gap(loads) {
  if (!loads.length) return 0;
  const avg = loads.reduce((a, b) => a + b, 0) / loads.length;
  if (avg === 0) return 0;
  return ((Math.max(...loads) - Math.min(...loads)) / avg) * 100;
}

function parseState(body) {
  const i = body.indexOf(MARK);
  if (i < 0) return null;
  const j = body.indexOf(MARK_END, i);
  if (j < 0) return null;
  try {
    return JSON.parse(body.slice(i + MARK.length, j).trim());
  } catch {
    return null;
  }
}
const stripState = (body) => {
  const i = body.indexOf(MARK);
  return i < 0 ? body : body.slice(0, i).trimEnd();
};
const renderBody = (text, state) => `${text.trimEnd()}\n\n${MARK} ${JSON.stringify(state)} ${MARK_END}`;

function toDateStr(v) {
  if (!v) return null;
  if (v instanceof Date) return v.toISOString().slice(0, 10);
  return String(v);
}

function loadConfig() {
  const yaml = require('js-yaml');
  const file = process.env.AUTO_REVIEW_CONFIG || '.github/auto-review.yml';
  const full = path.resolve(process.env.GITHUB_WORKSPACE || '.', file);
  const user = yaml.load(fs.readFileSync(full, 'utf8')) || {};
  return {
    ...DEFAULTS,
    ...user,
    labels: { ...DEFAULTS.labels, ...(user.labels || {}) },
    report: { ...DEFAULTS.report, ...(user.report || {}) },
  };
}

/* --------------------------------- main ---------------------------------- */

async function run({ github, context, core }) {
  const { owner, repo } = context.repo;
  const cfg = loadConfig();
  const teamRaw = cfg.team.map((t) => (typeof t === 'string' ? t : t.login));
  const teamName = Object.fromEntries(teamRaw.map((l) => [norm(l), l]));
  const nameOf = (l) => teamName[l] || l;
  // กรองให้เหลือเฉพาะ collaborator จริง: คำเชิญที่ยังไม่ accept ทำให้ requestReviewers ล้มทั้งชุด (422)
  // อ่านรายชื่อไม่ได้ (เช่น token ไม่มีสิทธิ์) ให้ใช้ team ตาม config ต่อ
  let collabSet = null;
  try {
    const collabs = await github.paginate(github.rest.repos.listCollaborators, { owner, repo, per_page: 100 });
    collabSet = new Set(collabs.map((u) => norm(u.login)));
  } catch (e) {
    core.warning(`อ่านรายชื่อ collaborator ไม่ได้ (ใช้ team ตาม config): ${e.message}`);
  }
  const dropped = [];
  const team = [...new Set(teamRaw.map(norm))].filter((l) => {
    if (!collabSet || collabSet.has(l)) return true;
    dropped.push(l);
    return false;
  });
  if (dropped.length) core.warning(`ข้ามสมาชิกที่ยังไม่ใช่ collaborator: ${dropped.map(nameOf).join(', ')}`);
  if (team.length === 0) {
    core.setFailed('auto-review.yml: ไม่มีสมาชิกใน team ที่เป็น collaborator ของ repo');
    return;
  }
  if (team.length < 2) {
    // โปรเจคจริงมักมีคำเชิญที่ยังไม่ถูกตอบรับ: ไม่ควรทำให้ workflow แดงทุก PR
    // ปล่อยให้ไหลไปทาง 'ไม่มีคนว่าง' แล้วติด label ให้คนมาเลือก reviewer เอง
    core.warning(`team หลังกรอง collaborator เหลือ ${team.length} คน (ต้องมีอย่างน้อย 2) — จะติด label ${cfg.labels.needs_reviewer} แทนการล้ม workflow`);
  }
  const teamIdx = Object.fromEntries(team.map((l, i) => [l, i]));
  const inTeam = (l) => Object.prototype.hasOwnProperty.call(teamIdx, l);
  const today = new Date().toLocaleDateString('en-CA', { timeZone: cfg.timezone });
  const unavailable = new Set(
    cfg.unavailable
      .map((u) => (typeof u === 'string' ? { login: u } : { login: u.login, until: toDateStr(u.until) }))
      .filter((u) => !u.until || u.until >= today)
      .map((u) => norm(u.login))
  );
  const dry = cfg.dry_run === true || process.env.DRY_RUN === 'true';
  const dryLog = [];
  const mutate = async (desc, fn) => {
    if (dry) {
      core.info(`[dry-run] ${desc}`);
      dryLog.push(desc);
      return null;
    }
    return fn();
  };
  const listAll = (fn, params) => github.paginate(fn, { owner, repo, per_page: 100, ...params });
  const exRegex = cfg.exclude_patterns.map(globToRegex);

  async function changedFiles(n) {
    const files = await listAll(github.rest.pulls.listFiles, { pull_number: n });
    return files
      .filter((f) => !exRegex.some((r) => r.test(f.filename)))
      .map((f) => ({ filename: f.filename, lines: (f.additions || 0) + (f.deletions || 0) }));
  }

  async function botComment(n) {
    const cs = await listAll(github.rest.issues.listComments, { issue_number: n });
    // อาจมีคอมเมนต์ state หลายอัน (รันชนกัน/ของสคริปต์เวอร์ชันเก่า/ถูกก๊อปมา)
    // ต้องใช้ "ล่าสุด" เท่านั้น: ถ้าใช้ find ตัวแรกจะได้ state เก่า ทำให้นับบรรทัดผิดและ assign ซ้ำ
    const mine = cs.filter((x) => x.user?.login === BOT_LOGIN && x.body?.includes(MARK));
    if (!mine.length) return null;
    const sorted = [...mine].sort((a, b) => Number(b.id) - Number(a.id));
    const [latest, ...dupes] = sorted;
    if (dupes.length) {
      core.warning(`#${n}: พบคอมเมนต์ state ของบอทซ้ำ ${dupes.length} อัน — ใช้ #${latest.id} เป็นหลัก และลบอันเก่า`);
    }
    return { id: latest.id, body: latest.body, state: parseState(latest.body), dupes: dupes.map((c) => c.id) };
  }

  async function reviewData(pr) {
    const reviews = await listAll(github.rest.pulls.listReviews, { pull_number: pr.number });
    const author = norm(pr.user.login);
    const reviewed = {};
    for (const r of reviews) {
      const l = r.user?.login;
      if (!l || r.state === 'PENDING' || !r.submitted_at) continue;
      const k = norm(l);
      if (k === author) continue;
      reviewed[k] = Math.max(reviewed[k] || 0, new Date(r.submitted_at).getTime());
    }
    const requested = (pr.requested_reviewers || []).map((u) => norm(u.login));
    return { reviewed, requested };
  }

  /** คำนวณ C, P, Exchange Matrix จาก API ทุกครั้ง ไม่เก็บสถานะแยก */
  async function collectWorkload() {
    const cutoff = Date.now() - cfg.history_days * 864e5;
    const prs = [];
    for await (const r of github.paginate.iterator(github.rest.pulls.list, { owner, repo, state: 'open', per_page: 100 })) prs.push(...r.data);
    outer: for await (const r of github.paginate.iterator(github.rest.pulls.list, { owner, repo, state: 'closed', sort: 'updated', direction: 'desc', per_page: 100 })) {
      for (const p of r.data) {
        if (new Date(p.updated_at).getTime() < cutoff) break outer;
        prs.push(p);
      }
    }
    const stats = Object.fromEntries(team.map((l) => [l, { C: 0, P: 0 }]));
    const ex = {};
    const open = [];
    for (const pr of prs) {
      if (pr.draft) continue;
      const author = norm(pr.user.login);
      const rd = await reviewData(pr);
      const involved = [...new Set([...rd.requested, ...Object.keys(rd.reviewed)])].filter((l) => inTeam(l) && l !== author);
      const cm = pr.state === 'open' || involved.length ? await botComment(pr.number) : null;
      const state = cm?.state || null;
      if (pr.state === 'open') open.push({ pr, cm, state, rd });
      if (!involved.length) continue;
      let total = state?.total;
      if (total === undefined) total = sumLines(await changedFiles(pr.number));
      for (const l of involved) {
        const lines = state?.lines?.[l] ?? total;
        const t = rd.reviewed[l];
        if (t) {
          if (t >= cutoff) {
            stats[l].C += lines;
            ex[l] ??= {};
            const e = (ex[l][author] ??= { count: 0, last: 0 });
            e.count++;
            e.last = Math.max(e.last, t);
          }
        } else if (pr.state === 'open' && rd.requested.includes(l)) {
          stats[l].P += lines;
        }
      }
    }
    return { stats, ex, open };
  }

  async function upsertComment(n, existing, text, state) {
    const body = renderBody(text, state);
    if (existing) {
      if (existing.body !== body) {
        await mutate(`update bot comment on #${n}`, () => github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body }));
      }
    } else {
      await mutate(`create bot comment on #${n}`, () => github.rest.issues.createComment({ owner, repo, issue_number: n, body }));
    }
    // ลบคอมเมนต์ state ที่ซ้ำ/เก่ากว่า เพื่อให้เหลือใบเดียวและ state ต่อเนื่องไม่กำกวม
    for (const id of existing?.dupes || []) {
      await mutate(`delete stale bot comment ${id} on #${n}`, async () => {
        try {
          await github.rest.issues.deleteComment({ owner, repo, comment_id: id });
        } catch (e) {
          if (e.status !== 404) throw e;
        }
      });
    }
  }

  async function syncLabel(pr, name, on, color, description) {
    if (!name) return;
    const has = (pr.labels || []).some((l) => l.name === name);
    if (on && !has) {
      await mutate(`add label ${name} to #${pr.number}`, async () => {
        try {
          await github.rest.issues.createLabel({ owner, repo, name, color, description });
        } catch (e) {
          if (e.status !== 422) throw e;
        }
        await github.rest.issues.addLabels({ owner, repo, issue_number: pr.number, labels: [name] });
      });
    } else if (!on && has) {
      await mutate(`remove label ${name} from #${pr.number}`, async () => {
        try {
          await github.rest.issues.removeLabel({ owner, repo, issue_number: pr.number, name });
        } catch (e) {
          if (e.status !== 404) throw e;
        }
      });
    }
  }

  async function handleAssign(pr, wl) {
    const author = norm(pr.user.login);
    const n = pr.number;
    if (pr.user.type === 'Bot') return core.info(`#${n}: ข้าม (ผู้เขียนเป็น Bot)`);
    if (!inTeam(author) && !cfg.allow_external_authors) return core.info(`#${n}: ข้าม (ผู้เขียนไม่อยู่ในทีม)`);
    if ((pr.labels || []).some((l) => cfg.skip_labels.includes(l.name))) return core.info(`#${n}: ข้าม (มี skip label)`);

    const files = await changedFiles(n);
    const total = sumLines(files);
    const large = total > cfg.large_pr_threshold;
    const needed = large ? cfg.large_pr_reviewers : 1;
    const rd = await reviewData(pr);
    const existingAll = [...new Set([...rd.requested, ...Object.keys(rd.reviewed)])].filter((l) => l !== author);
    const existingTeam = existingAll.filter(inTeam);
    const cm = await botComment(n);
    const prev = cm?.state || null;
    const missing = Math.max(0, needed - existingTeam.length);
    const share = large && cfg.split_large_pr ? Math.ceil(total / needed) : total;
    const candidates = team.filter((l) => l !== author && !unavailable.has(l) && !existingAll.includes(l));
    const extra = {};
    const picks = [];
    const pickOne = () => {
      const pool = candidates.filter((l) => !picks.some((p) => p.login === l));
      const c = chooseReviewer({ candidates: pool, stats: wl.stats, ex: wl.ex, author, teamIdx, prNumber: n, cfg, extra });
      if (!c) return null;
      picks.push(c);
      extra[c.login] = (extra[c.login] || 0) + share;
      return c;
    };
    for (let i = 0; i < missing; i++) if (!pickOne()) break;

    // ออปชัน: คนที่ผู้เขียนเลือกเองมี Score สูงกว่าคนต่ำสุดมาก -> เพิ่มคนต่ำสุดอีก 1 คน (ทำครั้งแรกเท่านั้น)
    let balanced = prev?.balanced || false;
    if (cfg.balance_extra_reviewer && !prev && missing === 0 && existingTeam.length && existingAll.length < cfg.max_reviewers) {
      const sc = (l) => wl.stats[l].C + cfg.pending_weight * wl.stats[l].P;
      const worst = Math.max(...existingTeam.map(sc));
      const best = chooseReviewer({ candidates, stats: wl.stats, ex: wl.ex, author, teamIdx, prNumber: n, cfg, extra });
      if (best && worst - best.score > cfg.balance_extra_gap) {
        picks.push(best);
        extra[best.login] = share;
        balanced = true;
      }
    }

    let requestFailed = false;
    const requested = [];
    if (picks.length) {
      const names = picks.map((p) => nameOf(p.login));
      try {
        await mutate(`request reviewers [${names.join(', ')}] on #${n}`, () =>
          github.rest.pulls.requestReviewers({ owner, repo, pull_number: n, reviewers: names })
        );
        requested.push(...picks);
      } catch (e) {
        // fallback: ขอทีละคน เพื่อไม่ให้คนเดียวล้มแล้วเสียทั้งชุด (เช่น username ที่ยังไม่ใช่ collaborator)
        core.warning(`#${n}: requestReviewers ทั้งชุดล้มเหลว (${e.message}) — ลองทีละคน`);
        for (const p of picks) {
          try {
            await mutate(`request reviewer [${nameOf(p.login)}] on #${n}`, () =>
              github.rest.pulls.requestReviewers({ owner, repo, pull_number: n, reviewers: [nameOf(p.login)] })
            );
            requested.push(p);
          } catch (e2) {
            requestFailed = true;
            core.warning(`#${n}: ขอ ${nameOf(p.login)} ไม่สำเร็จ: ${e2.message}`);
          }
        }
      }
    }
    const added = requested;
    const finalTeam = [...existingTeam, ...added.map((p) => p.login)];
    const shortBy = needed - finalTeam.length;
    const needsKey = shortBy > 0 ? (finalTeam.length === 0 ? 'needs_reviewer' : 'needs_second_reviewer') : null;

    const lines = {};
    let scope = null;
    if (large && cfg.split_large_pr && finalTeam.length >= 2) {
      scope = splitFiles(files, finalTeam);
      scope.forEach((b) => (lines[b.login] = b.lines));
    } else finalTeam.forEach((l) => (lines[l] = total));
    for (const p of added) wl.stats[p.login].P += lines[p.login] ?? total;

    const botList = [...new Set([...(prev?.bot || []), ...added.map((p) => p.login)])];
    const state = {
      v: 1,
      total,
      split: !!scope,
      lines,
      bot: botList,
      at: added.length ? new Date().toISOString() : prev?.at || null,
      needs: needsKey,
      reminded: added.length ? false : prev?.reminded || false,
      balanced,
    };

    // ---- ข้อความอธิบาย ----
    const T = [];
    T.push('### 🤖 Auto-assign Reviewer');
    T.push(`- ขนาด PR: **${total}** บรรทัด (ไม่นับไฟล์ที่ยกเว้น)${large ? ` · เกิน ${cfg.large_pr_threshold} บรรทัด จึงใช้ Reviewer ${needed} คน` : ''}`);
    if (finalTeam.length) {
      const who = finalTeam.map((l) => {
        const p = added.find((x) => x.login === l);
        if (p) return `@${nameOf(l)} (บอทเลือก · Score ${p.score}${p.overloaded ? ' · ⚠️ ทุกคนงานค้างเกินเพดาน' : ''})`;
        return `@${nameOf(l)} (${botList.includes(l) ? 'บอทเลือก' : 'เลือกเอง/รีวิวแล้ว'})`;
      });
      T.push(`- Reviewer: ${who.join(', ')}`);
    }
    if (balanced && !prev) T.push('- บอทเพิ่ม Reviewer อีก 1 คนเพื่อถ่วงภาระ เพราะคนที่เลือกเองมีภาระสะสมสูงกว่าคนต่ำสุดมาก');
    if (scope) {
      T.push('- ขอบเขตการรีวิว (แบ่งตามไฟล์ ภาระนับตามบรรทัดของไฟล์ที่ได้รับ):');
      for (const b of scope) {
        const shown = b.files.slice(0, 8).map((f) => `\`${f}\``).join(', ');
        T.push(`  - @${nameOf(b.login)} ≈ ${b.lines} บรรทัด: ${shown}${b.files.length > 8 ? ` และอีก ${b.files.length - 8} ไฟล์` : ''}`);
      }
    }
    if (requestFailed) T.push('- ⚠️ บอทขอ Reviewer ไม่สำเร็จ (ตรวจสอบสิทธิ์ของสมาชิกใน repo) กรุณาเลือกเอง');
    if (needsKey) {
      const why = [`ผู้เขียน @${nameOf(author)} ถูกตัดออก`];
      if (dropped.length) why.push(`ยังไม่ใช่ collaborator (ยังไม่ได้รับคำเชิญ): ${dropped.map(nameOf).join(', ')}`);
      if (unavailable.size) why.push(`ไม่พร้อม/ลา: ${[...unavailable].map(nameOf).join(', ')}`);
      T.push(`- ⏳ ${needsKey === 'needs_reviewer' ? 'ยังไม่มี Reviewer' : 'ยังขาด Reviewer คนที่สอง'}: ไม่มีสมาชิกที่พร้อมรับงาน (${why.join(' · ')}) บอทจะลองใหม่ทุกวัน หรือเลือก Reviewer เอง`);
    }
    T.push('\n<sub>ภาระงานนับจากบรรทัดที่ได้รับมอบหมายจริงทุกกรณี · ข้ามบอทด้วย label `' + cfg.skip_labels[0] + '`</sub>');

    await upsertComment(n, cm, T.join('\n'), state);
    await syncLabel(pr, cfg.labels.needs_reviewer, needsKey === 'needs_reviewer', 'd93f0b', 'ยังไม่มี Reviewer ที่พร้อมรับงาน');
    await syncLabel(pr, cfg.labels.needs_second_reviewer, needsKey === 'needs_second_reviewer', 'fbca04', 'ยังขาด Reviewer คนที่สอง');
    core.info(`#${n}: total=${total} needed=${needed} existing=${existingTeam.length} added=[${added.map((p) => p.login)}] needs=${needsKey}`);
  }

  async function sweep(wl) {
    const now = Date.now();
    for (const item of wl.open) {
      const { pr, cm, state, rd } = item;
      if (!state || state.needs) {
        await handleAssign(pr, wl);
        continue;
      }
      const author = norm(pr.user.login);
      const pending = rd.requested.filter((l) => inTeam(l) && !rd.reviewed[l]);
      const since = new Date(state.at || pr.created_at).getTime();
      if (!pending.length || state.reminded || now - since < cfg.overdue_hours * 36e5) continue;
      const candidates = team.filter((l) => l !== author && !unavailable.has(l) && !pending.includes(l) && !rd.reviewed[l]);
      const sug = chooseReviewer({ candidates, stats: wl.stats, ex: wl.ex, author, teamIdx, prNumber: pr.number, cfg });
      const hours = Math.round((now - since) / 36e5);
      const text =
        `⏰ ${pending.map((l) => '@' + nameOf(l)).join(', ')} ยังไม่ได้รีวิว PR นี้มา ${hours} ชม. (เกณฑ์ ${cfg.overdue_hours} ชม.)` +
        (sug ? `\n\nถ้าไม่สะดวก แนะนำให้ผู้เขียนขอ @${nameOf(sug.login)} (Score ต่ำสุดตอนนี้) แทน บอทจะไม่เปลี่ยนคนเองระหว่างรีวิว` : '');
      await mutate(`reminder on #${pr.number}`, () => github.rest.issues.createComment({ owner, repo, issue_number: pr.number, body: text }));
      await upsertComment(pr.number, cm, stripState(cm.body), { ...state, reminded: true });
    }
  }

  async function report(wl) {
    const rows = team.map((l) => ({ l, C: wl.stats[l].C, P: wl.stats[l].P }));
    const g = gap(rows.map((r) => r.C + r.P));
    const md = [];
    md.push(`## รายงานภาระการรีวิว ${cfg.history_days} วันล่าสุด`);
    md.push('', '| สมาชิก | รีวิวเสร็จ (C) | ค้าง (P) | รวม |', '| --- | ---: | ---: | ---: |');
    for (const r of rows) md.push(`| ${nameOf(r.l)} | ${r.C} | ${r.P} | ${r.C + r.P} |`);
    md.push('', `**Gap = (Max − Min) ÷ Average × 100 = ${g.toFixed(1)}%** (เป้าหมาย ≤ 20%)`);
    md.push('', '### Review Exchange Matrix (แถว = ผู้รีวิว, คอลัมน์ = ผู้เขียน, ค่า = จำนวน PR ที่รีวิว)', '');
    md.push(`| | ${team.map(nameOf).join(' | ')} |`, `| --- |${team.map(() => ' ---:').join(' |')} |`);
    for (const r of team) md.push(`| ${nameOf(r)} | ${team.map((a) => (r === a ? '-' : wl.ex[r]?.[a]?.count || 0)).join(' | ')} |`);
    const text = md.join('\n');
    await core.summary.addRaw(text, true).write();
    if (cfg.report.issue) {
      await mutate('create weekly report issue', () =>
        github.rest.issues.create({ owner, repo, title: `Review workload report ${today}`, body: text })
      );
    }
  }

  const isReportDay = () => new Date().toLocaleDateString('en-US', { weekday: 'short', timeZone: cfg.timezone }) === cfg.report.weekday;

  /* -------------------------------- dispatch -------------------------------- */
  const ev = context.eventName;
  if (ev === 'pull_request_target' || ev === 'pull_request') {
    const { data: pr } = await github.rest.pulls.get({ owner, repo, pull_number: context.payload.pull_request.number });
    if (pr.draft || pr.state !== 'open') return core.info(`#${pr.number}: ข้าม (draft หรือไม่ได้เปิดอยู่)`);
    await handleAssign(pr, await collectWorkload());
  } else {
    const mode = ev === 'workflow_dispatch' ? process.env.MODE || 'sweep' : 'sweep';
    const wl = await collectWorkload();
    if (mode === 'sweep') await sweep(wl);
    if (cfg.report.enabled && (mode === 'report' || (ev === 'schedule' && isReportDay()))) await report(wl);
  }
  if (dry && dryLog.length) await core.summary.addHeading('Dry-run: สิ่งที่บอทจะทำ', 3).addList(dryLog).write();
}

module.exports = run;
module.exports.pure = { globToRegex, splitFiles, chooseReviewer, gap, parseState, stripState, renderBody, sumLines, DEFAULTS };
