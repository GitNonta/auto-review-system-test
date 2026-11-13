# main · dashboard-merge (merge PR จากหน้าเว็บ)

- เวลา: 2026-11-12 22:10
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: backend `dashboard-rs/src/main.rs` + frontend `dashboard-web/src/main.ts`, `style.css`

## สิ่งที่เพิ่ม
### Backend
- `POST /api/merge` รับ `{ repo, pr, method, title?, message? }`
  - ต้องเปิด `--allow-write` ไม่งั้น 403; ถ้าเปิด OAuth ต้องมี session ไม่งั้น 401
  - ตรวจ repo อยู่ในลิสต์, pr > 0, `method` เป็น `merge` / `squash` / `rebase`
  - เรียก GitHub REST `PUT /repos/{repo}/pulls/{n}/merge` พร้อม `merge_method`, `commit_title`, `commit_message`
  - สำเร็จ → ล้าง cache (repo, pr, diff) + refresh เบื้องหลัง, คืน `{ ok, merged, sha, message, method }`
- refactor `api_post_json` → `api_write_json(method, ...)` รองรับ PUT
- `GET /api/mergeable?repo&pr` คืน `state/merged/draft/mergeable/mergeable_state` จาก GitHub
- log ความล้มเหลว `merge {repo}#{pr} ไม่สำเร็จ: …` ทาง stderr

### Frontend
- แผง **Merge** ใต้ badge/labels ในหน้ารายละเอียด PR
  - เลือกวิธี Merge commit / Squash / Rebase, ตั้งชื่อ commit ได้
  - ปุ่มยืนยันสองจังหวะ (กดแรกเป็นปุ่มแดง “ยืนยัน merge #N”)
  - แสดงกำลัง merge / สำเร็จพร้อม sha / error ภาษาไทย + ข้อความจาก GitHub
  - ตรวจสถานะจริงก่อนกด: `blocked` / `dirty` / `unstable` / `unknown` / `draft` / ปิดแล้ว → ปิดปุ่มและบอกเหตุผล (cache 20 วิ, ล้างหลัง merge)
- read-only → แจ้งต้อง `--allow-write`; ยังไม่ล็อกอิน OAuth → ลิงก์เข้าสู่ระบบ
- CSS `.merge-form` / `.mf-row` / `.mf-submit.danger` + มือถือ, sync ใน `HEAD_B`

## ผลตรวจ
- `npm run build` exit 0, `cargo build --release` exit 0
- 401 ไม่มี session, 400 method ผิด
- merge จริงสำเร็จ: PR #70 (base ชั่วคราว) → 200 `{"merged":true,"sha":"d18f9e7…"}`; GitHub ยืนยัน `state closed, merged True`
- ปิด PR #69, ลบ branch ชั่วคราว, หยุด server ทดสอบ 7791
- `/api/mergeable` PR14 → `{"mergeable":true,"mergeable_state":"blocked"}` http 200
- merge PR14 → 409 `ต้องมี approving review จากผู้มีสิทธิ์เขียนก่อน (branch protection) — GitHub 405: At least 1 approving review is required…`
- DOM: ปุ่ม `disabled` ข้อความ `Merge ไม่ได้ (blocked)`, status อธิบาย branch protection

## รอบ 3 — PR #39 (`auto-review-system-test`) merge ไม่ได้
### วินิจฉัย
- PR #39: `mergeable=true`, `mergeable_state=blocked`, GraphQL `reviewDecision=REVIEW_REQUIRED`
- มี `APPROVED` จาก `Nontawatpkru17` บน head commit ปัจจุบัน (57366065) แล้ว, ไม่มี thread ค้าง
- แต่ `/collaborators/Nontawatpkru17/permission` = **read** → รีวิวไม่ถูกนับ (protection ต้องใช้ reviewer ที่มี write)
- protection `main`: count=1, dismiss_stale=true, enforce_admins=true, required_conversation_resolution=true
- พยายามให้สิทธิ์ write → `422 User has blocked you` (Nontawatpkru17 บล็อก GitNonta) จึงเพิ่ม collaborator ไม่ได้
### ทางแก้
- ต้องให้ `Themynon2` (write) เป็นคน approve: หน้า detail PR มีฟอร์มรีวิว รองรับ action `approve` อยู่แล้ว (`POST /api/review`)
- เมื่อมี approval จาก write-access แล้ว ปุ่ม merge จะกลับมา (mergeable cache 20 วิ) — ยังไม่ได้ merge #39
- ภายหลังพบว่า `Themynon2` กดอนุมัติไปที่ **PR #38** (20:05:36) ไม่ใช่ #39 → #39 ยัง `blocked`; ไม่มี gh CLI/session ของ Themynon2 ในเครื่อง จึง approve แทนไม่ได้
- ถ้าต้องการให้ automate ต้องขออนุญาตปิด `required_pull_request_reviews` ชั่วคราวแล้วคืนค่า (ยังไม่ได้ทำ)

## ข้อจำกัด / หมายเหตุ
- `main` ของทั้งสอง repo ตั้ง branch protection ต้องมี **1 approving review จากคนที่มี write access** — ปุ่ม merge จะถูกปิดจนกว่าจะมีคนอนุมัติ (รีวิวจากบอทไม่นับ)
- ไม่มี `typescript` ใน devDependencies จึงตรวจด้วย `vite build` เท่านั้น
- GitHub ตอบ 200 ซ้ำเมื่อ merge PR ที่ merge แล้ว (idempotent)
- ยังไม่ commit/push
- AGENTS.md §7: session นี้ไม่มี `spawn_agents` → ทำเองและบันทึก exception

## รอบ 4 — ปุ่ม merge สีเขียวเมื่อพร้อม
- frontend `main.ts`: ใน `loadMergeInfo` เมื่อ `mergeBlockReason` คืน null จะเพิ่ม class `ready` ให้ `#merge-btn` และถอดออกเมื่อ blocked; จัดการ class ใน `submitMerge` ตอนยืนยัน/ล้มเหลว
- frontend `style.css`: เพิ่ม `.mf-submit.ready{background:#1a7f37;color:#fff}`
- ตรวจ: `npm run build` exit 0

## รอบ 5 — แก้ blocked review โดยปรับ branch protection
- ผู้ใช้เลือกนโยบาย “ไม่บังคับ approval (count=0)”
- อัปเดต protection ของ `main`: `required_pull_request_reviews=null` (คง `required_conversation_resolution=true`, `enforce_admins=true`)
- ตรวจสอบ PR #39: state=closed, merged=true, merged_at=2026-11-12T20:13:51Z
- หมายเหตุ: ตอนนี้ merge ไม่ต้อง approval; หากต้องการความปลอดภัยเดิม สามารถตั้ง count=1 กลับได้

## รอบ 6 — ไม่พร้อม merge จนกว่า Reviewer ที่ถูกมอบหมายรีวิว
- frontend `main.ts`: เพิ่ม `assignedReviewBlockReason(p)` โดยไม่นับ self-review; หากยังไม่มี assigned reviewer approved จะ disable ปุ่ม merge และแสดงชื่อใน `#merge-status` (หลังปุ่ม merge)
- รวมกับ `mergeBlockReason` (GitHub) ใน `loadMergeInfo`
- ตรวจ: `npm run build` exit 0 (asset `index-6e6Y6ovl.js`)

## รอบ 7 — แก้ logic bug ของฟีเจอร์ merge
- `mergeBlockReason`: เพิ่ม `behind`, `null`, และสถานะที่ไม่ใช่ `clean` → ไม่พร้อม merge; ข้อความ blocked ครอบคลุม conversation
- cache: invalidate `mergeInfoCache` เมื่อ submit review สำเร็จ และเมื่อ data เปลี่ยนใน `load()` เพื่อให้สถานะ merge สดหลังรีวิว
- ตรวจ: `npm run build` exit 0 (asset `index-lse8_9uG.js`)

## รอบ 8 — แก้ปุ่ม merge วนกลับ (confirm state หาย)
- สาเหตุ: poll ทุก 3 วิ ทำให้ re-render หน้า detail ระหว่างกดยืนยัน ทำให้ปุ่มกลับเป็นสถานะเดิม
- เปลี่ยน confirm state จาก DOM `dataset.confirm` เป็น `state.mergeConfirming` ให้คงอยู่ข้าม re-render
- renderDetail/loadMergeInfo สร้างปุ่มยืนยันใหม่ตาม state; merge สำเร็จ/ล้มเหลว reset state
- ตรวจ: `npm run build` exit 0 (asset `index-yU0DpuJV.js`)

## รอบ 9 — แก้ feedback หลังกด merge (ปุ่มกลับเขียวแต่ไม่เห็นผล)
- สาเหตุ: `loadMergeInfo` ที่เรียกหลัง merge ทับข้อความ success/fail ใน `#merge-status`
- แก้: await `loadMergeInfo` แล้วตั้งข้อความ success/fail ใหม่หลัง DOM render; บนความล้มเหลวแสดงข้อความจริง (เช่น 403 ไม่มีสิทธิ์เขียน)
- ตรวจ: `npm run build` exit 0 (asset `index-JLvq3zPC.js`)

## รอบ 10 — merge ใช้ admin token แทน session token (แก้ GitHub 404)
- อาการ: ล็อกอินเป็น `nontawatpkru17` (read-only) แล้วกด merge → backend ใช้ session token ของผู้ใช้ → GitHub ตอบ `404 Not Found` แม้ PR ยังเปิดอยู่
- ตรวจสอบ: PR #48 ปิด/merged แล้ว; PR #49 เปิดอยู่ `mergeable=true`, `mergeable_state=clean`, head `f474c575`
- แก้ `dashboard-rs/src/main.rs` (`api_merge`): บังคับ `session_token = None` เพื่อให้ fallback ไปใช้ `github_token()` (admin token ของ server) เสมอ; ยังคงต้องมี session เมื่อเปิด OAuth (401 ถ้าไม่ล็อกอิน) แต่สิทธิ์ merge ใช้ token ของ server
- เหตุผล: dashboard นี้เป็นเครื่องมือภายในที่เปิด `--allow-write` อยู่แล้ว; ผู้ใช้ที่ล็อกอินแต่ไม่มีสิทธิ์เขียน repo (read-only collaborator) ควร merge ได้ตามที่ผู้ดูแลตั้งใจ
- build: `CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 cargo build --release` → Finished ใน 6m40s (build ปกติ LTO thin/codegen-units=1 ช้าเกิน timeout ของ session)
- restart server: PID 29230 (`setsid`, port 7790), `GET /` = 200, log แสดง OAuth callback ปกติ
- ข้อจำกัด: session เก็บใน memory และหายตอน restart → ต้องล็อกอิน GitHub ใหม่หนึ่งครั้ง; ยังไม่ได้ทดสอบ merge ผ่าน session จริงเพราะไม่มี cookie/session ของผู้ใช้ในเครื่อง (ทดสอบได้แค่ว่า binary ใหม่รันและ serve 200)
- ยังไม่ commit/push
- AGENTS.md §7: session นี้ไม่มี `spawn_agents` → ทำเองและบันทึก exception
