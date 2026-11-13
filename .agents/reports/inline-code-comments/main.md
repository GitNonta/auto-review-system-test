# main · inline-code-comments (คอมเมนต์รีวิวโค้ดรายบรรทัด)

- เวลา: 2026-11-13 11:12
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: `dashboard-rs/src/main.rs`, `dashboard-web/src/main.ts`, `dashboard-web/src/style.css`

## สิ่งที่ทำ
### Backend
- เพิ่ม `POST /api/comment`
  - รับ `repo`, `pr`, `path`, `line`, `side` (`LEFT`/`RIGHT`), `body`
  - ต้องเปิด `--allow-write`
  - ต้องมี session เมื่อเปิด OAuth
  - ตรวจ repo อยู่ในรายการ, path/line/side/body ถูกต้อง และจำกัดข้อความ 60,000 ตัวอักษร
  - ดึง head SHA ของ PR แล้วเรียก GitHub REST:
    `POST /repos/{repo}/pulls/{pr}/comments`
  - กันคอมเมนต์บน PR ที่ปิด/merge แล้ว พร้อมข้อความไทย
  - ล้าง cache ของ repo/PR/diff/PR detail แล้ว refresh เบื้องหลัง
  - แปลง error จาก GitHub เป็นคำแนะนำภาษาไทย (422 line ไม่อยู่ใน diff, 403, 404)
- เพิ่ม route `.route("/api/comment", post(api_comment))`

### Frontend
- เพิ่มปุ่มไอคอนคอมเมนต์ที่ทุกบรรทัดใน diff (ทั้งฝั่งใหม่และฝั่งเดิม)
  - บรรทัดเพิ่ม/context ใช้ `side=RIGHT`
  - บรรทัดลบใช้ `side=LEFT`
- คลิกแล้วแทรกฟอร์มใต้บรรทัดนั้นทันที ไม่ reload หน้า
  - textarea, ปุ่มส่ง, ปุ่มยกเลิก, สถานะกำลังส่ง/error
  - โฟกัส textarea อัตโนมัติ
  - `Ctrl/Cmd + Enter` เพื่อส่ง
  - เก็บ draft ใน state เพื่อไม่หายเมื่อ dashboard poll/render ใหม่
- แสดงเฉพาะเมื่อมีสิทธิ์เขียน:
  - ไม่ใช่หน้า readonly/history
  - `state.canWrite`
  - ถ้าเปิด OAuth ต้องล็อกอินแล้ว
  - เจ้าของ PR ยังคอมเมนต์ได้ (ต่างจาก approve/request changes)
- หลังส่งสำเร็จ ล้าง draft, refresh ข้อมูล และแสดงคอมเมนต์ใหม่ใต้บรรทัด
- เพิ่ม CSS ขาว–ดำสำหรับปุ่ม, ฟอร์ม, สถานะ error และ responsive

## ไฟล์ที่แก้
- `dashboard-rs/src/main.rs`
  - `CommentRequest`, `comment_hint`, `api_comment`, route ใหม่
- `dashboard-web/src/main.ts`
  - `LineDraft`, `state.lineDraft`
  - `lineCommentButton`, `lineCommentComposerHtml`, `fileBlockHtml`
  - `submitLineComment`, `wireLineCommentComposer`, `openLineComment`
  - reset draft เมื่อเปลี่ยนหน้า/PR/project
- `dashboard-web/src/style.css`
  - `.line-comment-btn`, `.cmt-compose`, `.line-comment-form`, `.lcf-*`

## ผลตรวจ
- `cd dashboard-web && npm run build` ผ่าน
  - `dist/assets/index-CaEfs4JL.css` (42.45 kB)
  - `dist/assets/index-QMYZhz6J.js` (86.66 kB)
- `cd dashboard-rs && cargo check` ผ่าน
- `cargo test -- --test-threads=1` ผ่าน 11 tests
- `cargo build --release` ผ่าน (50.87s)
- restart server PID `8345` พอร์ต 7790
- `GET /login` = 200
- `POST /api/comment` ไม่มี session = 401 (middleware ทำงาน)
- asset JS/CSS ใหม่ = 200
- ทดสอบ UI ด้วย Chromium + mock API 12/12 ผ่าน:
  - diff 4 บรรทัดมีปุ่มคอมเมนต์ 4 ปุ่ม
  - บรรทัดลบส่ง `side=LEFT` และ line ถูกต้อง
  - ฟอร์มแทรกใต้บรรทัดและโฟกัส textarea
  - payload ที่ส่งคือ `path=src/app.ts`, `line=1`, `side=LEFT`, body ถูกต้อง
  - คอมเมนต์ใหม่แสดงใต้บรรทัดหลัง refresh
  - ปุ่มยกเลิกปิดฟอร์มโดยไม่ส่ง
  - mobile 390px ไม่มี horizontal overflow
  - ไม่มี runtime exception / console error
- `git diff --check` ผ่าน (exit 0)

## ข้อจำกัด / หมายเหตุ
- ไม่ได้ยิงคอมเมนต์จริงไป GitHub เพราะจะสร้างความคิดเห็นและแจ้งเตือนบน PR จริง; ทดสอบด้วย mock API แทน
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → ทำเองทั้งหมดและบันทึก exception
- ยังไม่ commit/push
