# main · dashboard-history (commit / merge history + ผู้กระทำ)

- เวลา: 2026-11-13 07:45
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: `dashboard-rs/src/main.rs`, `dashboard-web/src/main.ts`, `dashboard-web/src/style.css`, `.gitignore`

## สิ่งที่เพิ่ม
### Backend
- `GET /api/commits?repo=&pr=` — commit ทั้งหมดของ PR จาก REST `/pulls/{n}/commits` (sha, ข้อความ, author/committer login+name, เวลา, verified, parents, URL) cache 60 วิ
- `GET /api/history?repo=` — GraphQL ครั้งเดียว:
  - commit ล่าสุด 20 รายการบน default branch พร้อม author/committer
  - PR ที่ merge แล้ว 20 รายการ พร้อม `merged_by`, `merged_at`, `merge_commit_sha`, base/head
  - cache 60 วิ
- audit log `dashboard-audit.jsonl` (เปลี่ยน path ได้ด้วย `DASHBOARD_AUDIT_LOG`):
  - ตอน merge สำเร็จ บันทึก `at/repo/pr/action/actor/method/sha/title`
  - actor มาจาก session ที่ล็อกอิน > `viewer` ของ token > `server-token`
  - `/api/history` ใช้ audit ระบุคนที่สั่ง merge ผ่าน dashboard (เพราะ GitHub เห็นเป็น token ของ server)
- `/api/mergeable` เพิ่ม `merged_by`, `merged_at`, `merge_commit_sha`, `commits`, `changed_files`, `additions`, `deletions`, `base`, `head`, `head_sha`
- merge response เพิ่ม `actor`; ล้าง history cache หลัง merge
- `.gitignore` เพิ่ม `dashboard-audit.jsonl`

### Frontend
- nav ใหม่ **“ประวัติ”** (`?view=history`):
  - KPI: PR merge แล้ว / commit บน branch หลัก / merge ผ่าน dashboard / merge บน GitHub
  - ตารางประวัติ merge: PR, ผู้ merge + ที่มา (ผ่าน dashboard / บน GitHub), สาขา, commit SHA, เวลา
  - ตารางประวัติ commit: SHA, ข้อความ, ผู้เขียน/ผู้ commit, เวลา
  - ถ้าสั่งผ่าน dashboard จะแสดง `@actor` และหมายเหตุว่า GitHub เห็นเป็น `@merged_by` (token ของ server)
- หน้ารายละเอียด PR:
  - แผง **“สถานะ merge”** แสดงสถานะจริง, mergeable, จำนวน commit, ไฟล์, +/−, base/head, head SHA, ผู้ merge, merge commit — แสดงได้แม้ยังไม่ล็อกอิน/โหมดอ่านอย่างเดียว
  - section **Commits** โหลดจาก `/api/commits` แสดง author + committer + เวลา + merge commit/verified badge + ลิงก์ GitHub
  - ข้อความปุ่ม merge แก้เป็น “merge ด้วย token ของ server · ระบบบันทึกชื่อผู้สั่งในประวัติ”
  - ข้อความสำเร็จระบุ `โดย @actor`

## ผลตรวจ
- `cargo check` ผ่าน
- `cargo build --release` (LTO=false, codegen-units=16) ผ่านใน 56.55s
- restart server PID `19042` พอร์ต 7790; `GET /` = 200, asset JS = 200, log ไม่มี error
- `/api/history` repo `uni-activity` → commits 20, merges 8, `merged_by` ถูกต้อง (เช่น #16 = Themynon2)
- `/api/history` repo `auto-review-system-test` → commits 20, merges 16
- `/api/commits?pr=20` → 2 commits พร้อม author/committer
- `/api/mergeable?pr=20` → title/state/merged/mergeable_state/commits/changed_files/additions/deletions/base/head/head_sha ครบ
- `/api/history?repo=bad/repo` → 400; `/api/commits?pr=999999` → 502 พร้อม error จาก GitHub
- Chromium headless ตรวจ DOM:
  - `?view=history` แสดง 2 ตาราง, 8 merge, 20 commit, ไม่ค้าง loading
  - `?pr=20` แสดงสถานะ merge + facts (blocked, 2 commits, +106/−6) + commit 2 รายการ
- ทดสอบ audit attribution: ใส่ audit จำลอง PR #42 actor `nontawatpkru17` → API คืน `merged_by=GitNonta, actor=nontawatpkru17, via_dashboard=true`; UI แสดง “ผ่าน dashboard” + “GitHub เห็นเป็น @GitNonta” แล้วลบไฟล์ทดสอบและ restart ให้สะอาด

## รอบ 2 — เปิด PR จากประวัติในหน้าเว็บ (ไม่เด้ง GitHub)
### สิ่งที่เพิ่ม
- `GET /api/pr?repo=&pr=` ดึง PR เดียว + ไฟล์/patch จาก GitHub รองรับ PR ที่ปิด/merge แล้ว; cache 60 วิ
- หน้าประวัติ: ลิงก์ `#N` และชื่อ PR เปลี่ยนเป็น `?view=history&pr=N` พร้อม `data-history-pr` (คลิกแล้วเปิดในหน้าเดิม)
- URL ตรง `?view=history&pr=16` เปิดหน้ารายละเอียดได้ทันที
- หน้ารายละเอียดโหมดประวัติ (readonly):
  - badge “merge แล้ว” / “ปิดอยู่”, สถานะ merge + ผู้ merge + merge commit + base/head + commit count
  - ตาราง Commits และ diff ไฟล์จาก `/api/pr` (ถ้า patch ถูกตัดจะ fallback `/api/diff`)
  - ซ่อนฟอร์ม merge/รีวิว และปุ่มกลับเปลี่ยนเป็น “กลับไปประวัติ”
- cache ของ PR ประวัติ key เป็น `repo#pr` กันข้อมูลข้าม repo

### ผลตรวจ
- `cargo check` ผ่าน; `cargo build --release` ผ่านใน 43.66s
- restart server PID `23256` พอร์ต 7790; `/`=200, `/api/history`=200, `/api/pr`=200
- `/api/pr?pr=16` คืน merged=true, merged_by=Themynon2, files=1 พร้อม patch, commits=1
- Chromium headless `?view=history&pr=16` และ `pr=19`: แสดง badge merge แล้ว, สถานะ merge, commits, ไฟล์, ปุ่มกลับไปประวัติ; ไม่มี `#merge-form`/`#review-form`
- หน้าประวัติมีลิงก์ภายใน 16 ลิงก์ (8 merge × 2) และไม่มีลิงก์ PR แบบ `target="_blank"`

## รอบ 3 — แท็บ Changes / Status / Conversation
### สิ่งที่เพิ่ม
- `/api/pr` เพิ่มข้อมูล:
  - `reviews[]` (author/state/label/body/at/url)
  - `conversation[]` รวม review + issue comment + inline review comment เรียงตามเวลา พร้อม `path`/`line`
  - `checks` จาก combined status + check-runs; เลือกเฉพาะ run ล่าสุดต่อชื่อ check (กัน run เก่าที่ fail ทำให้สรุปผิด)
- หน้ารายละเอียดที่เปิดจากประวัติมีแท็บ `Changes` / `Status` / `Conversation` พร้อมตัวเลขกำกับ
  - Changes: commits + ไฟล์/diff เดิม
  - Status: สถานะ merge + checks/CI + สรุปรีวิวรายคน
  - Conversation: timeline คอมเมนต์/รีวิว/inline พร้อมผู้เขียน เวลา และ path:line
- คลิกแท็บสลับได้โดยไม่ reload; คงแท็บไว้เมื่อ re-render หลังโหลด diff/commits
- หน้ารายละเอียด PR ปกติ (ไม่ใช่ประวัติ) ยังเป็น layout เดิม ไม่มีแท็บ

### ผลตรวจ
- `cargo check` ผ่าน; `cargo build --release` ผ่านใน 1m 04s (รอบสุดท้าย)
- `npm run build` ผ่าน asset `index-34FP_vGX.js`, CSS `index-CnBkjsiX.css`
- restart server PID `29123` พอร์ต 7790; `/`=200
- `/api/pr` repo `uni-activity` PR 16: reviews=1, conversation=2, checks=success total=1
- Chromium headless + CDP คลิกจริง:
  - เริ่มที่ `changes` (aria-selected=true), panel อื่น `display:none`
  - คลิก `status` → แสดงเฉพาะ status panel: merge แล้ว, checks ผ่าน 1 รายการ, สรุปรีวิวอนุมัติ 1
  - คลิก `conversation` → แสดงเฉพาะ conversation panel: 2 รายการ พร้อมผู้เขียน/เวลา
- ตรวจ dedupe check runs: PR 16 เคยมี run `assign` success+failure จากคนละรอบ → API เหลือ run ล่าสุด 1 รายการ สรุป `success`

## รอบ 4 — บังคับล็อกอินก่อนดูข้อมูลทั้งหมด
### สิ่งที่เพิ่ม
- middleware `require_auth` ครอบทุก route (รวม fallback/SPA):
  - ไม่มี session → หน้าเว็บ 302 ไปหน้า login (รอบ 5 เปลี่ยนเป็น `/login`), API ตอบ `401 {"error":"ต้องเข้าสู่ระบบด้วย GitHub ก่อน"}`
  - ยกเว้นเฉพาะ `/auth/*`, `/assets/*`, `/favicon.ico` เพื่อให้ flow ล็อกอินและไฟล์ static ทำงานได้
  - ถ้าไม่ได้ตั้งค่า OAuth เลย → fail closed ตอบ 503 ไม่เปิดเผยข้อมูล
- frontend ครอบ `window.fetch`: เจอ 401 ระหว่างใช้งาน (session หมดอายุ) → `location.assign('/auth/login')`
- logout ยังใช้งานได้โดยไม่ต้องมี session และล้าง cookie `dash_session`

### ผลตรวจ
- `cargo check` ผ่าน; `cargo build --release` ผ่านใน 1m 16s
- `npm run build` ผ่าน asset `index-BpfPs12C.js`
- restart server PID `11258` พอร์ต 7790
- ไม่มี cookie: `/`=302 → `/auth/login`, `/api/projects`=401, `/api/data.json`=401, `/api/history`=401, `/api/pr`=401
- `/auth/login`=302 ไป GitHub authorize พร้อม callback `http://192.168.1.118:7790/auth/callback`
- `/assets/index-BpfPs12C.js`=200 (ไฟล์ static ไม่มีข้อมูล)
- cookie ปลอม `dash_session=fake` → `/` ยัง 302 ไปล็อกอิน
- `POST /auth/logout`=200 และ `Set-Cookie: dash_session=; Max-Age=0`

## รอบ 5 — หน้า login ของ dashboard + หน้าโปรไฟล์ GitHub
### สิ่งที่เพิ่ม
- `GET /login` เป็นหน้า login ของ dashboard เอง (HTML server-rendered, CSS inline, ไม่ต้องโหลด asset)
  - ปุ่ม Continue with GitHub → `/auth/login?next=...`
  - ถ้ายังไม่ตั้งค่า OAuth แสดงกล่องแจ้งเตือนและปุ่ม disabled (ไม่เปิดข้อมูล)
  - ถ้ามี session อยู่แล้ว → เด้งกลับ `next` หรือ `/`
- เก็บ `next` เฉพาะ path ภายในเว็บ (`safe_next`) กัน open redirect; middleware เด้ง `/login?next=<path+query>` และ callback กลับหน้าต้นทางหลังล็อกอิน
- OAuth scope เพิ่ม `user:email` และ `read:org` → `repo read:user user:email read:org`
- `GET /api/profile` ใช้ token ของ session เรียก GitHub:
  - `/user` (ข้อมูลโปรไฟล์ทั้งหมด), `/user/orgs`, `/user/emails`, `/user/social_accounts`
  - endpoint เสริมที่ถูกปฏิเสธสิทธิ์จะคืน `null`/`[]` โดยไม่ทำให้หน้าล้ม
- SPA มีเมนู `โปรไฟล์` (`?view=profile`) แสดง:
  - รูป avatar, ชื่อ, login, bio, ลิงก์ GitHub/เว็บ/Twitter
  - สถิติ public repos/gists/followers/following
  - ข้อมูลโปรไฟล์หลัก + องค์กร + อีเมล + บัญชีโซเชียล
  - ตาราง “ข้อมูลดิบทั้งหมดจาก GitHub” ครบทุกฟิลด์ที่ `/user` คืนมา
- fetch wrapper เมื่อเจอ 401 → `/login?next=<หน้าปัจจุบัน>`

### ผลตรวจ
- `cargo check` ผ่าน
- `cargo test -- --test-threads=1` ผ่าน 11 tests (รวม `safe_next_allows_only_local_paths`)
- `cargo build --release` ผ่านใน 1m 17s
- `npm run build` ผ่าน asset `index-BVKDXJ-I.js`, CSS `index-erECk7XL.css`
- restart server PID `27358` พอร์ต 7790
- `/login` = 200, มี `<title>เข้าสู่ระบบ</title>` และปุ่ม `Continue with GitHub`
- `/login?next=%2F%3Fview%3Dhistory%26pr%3D16` → ปุ่มส่ง next ครบ
- `/` = 302 → `/login?next=%2F`; deep link `/?view=history&pr=16` → `/login?next=%2F%3Fview%3Dhistory%26pr%3D16`
- `/auth/login` = 302 ไป GitHub authorize พร้อม scope ใหม่
- `/api/profile` ไม่มี session = 401
- ทดสอบ GitHub endpoints ด้วย token จริง: `/user`=200, `/user/orgs`=200, `/user/emails`=200, `/user/social_accounts`=200

## ข้อจำกัด / หมายเหตุ
- ประวัติจาก GitHub จำกัด 20 commits / 20 merged PR ล่าสุด
- ยังไม่ได้ทดสอบหน้าโปรไฟล์แบบล็อกอินจริงแบบ end-to-end เพราะไม่มี GitHub session ในเบราว์เซอร์อัตโนมัติ; ตรวจฝั่ง API ด้วย token จริงและตรวจ build/หน้า login แล้ว
- session ที่ล็อกอินก่อนเพิ่ม scope ต้อง logout/login ใหม่เพื่อให้ `/user/emails` และ orgs ส่วนตัวใช้งานได้ครบ
- audit เริ่มบันทึกตั้งแต่เวอร์ชันนี้; merge เก่าจะแสดง `merged_by` ตามที่ GitHub รู้
- ไม่มี `typescript`/tsconfig → ตรวจ frontend ด้วย `vite build` + DOM จริง ไม่ได้รัน `tsc --noEmit`
- AGENTS.md §7: session นี้ไม่มี `spawn_agents` → ทำเองและบันทึก exception
- การบังคับล็อกอินเปิดให้บัญชี GitHub ใดก็ได้ที่ล็อกอินสำเร็จ (ยังไม่มี allowlist/org check) — ถ้าต้องการจำกัดเฉพาะทีม ต้องเพิ่มรายชื่อหรือตรวจ org membership
- ถ้าไม่ตั้งค่า OAuth ระบบจะ fail closed (503) ไม่เปิดข้อมูลเลย (ยกเว้นหน้า `/login` ที่แสดงคำอธิบาย)
- ยังไม่ commit/push
