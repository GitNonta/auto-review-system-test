# main · monochrome-theme (กลับไปใช้ธีมขาว–ดำ)

- เวลา: 2026-11-13 10:58
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: `dashboard-web/src/style.css`, `dashboard-rs/src/main.rs` (หน้า `/login`)

## สิ่งที่ทำ
- เปลี่ยน palette หลักกลับเป็นขาว–ดำ/เทา รองรับ light/dark
  - light: bg/card ขาว, ตัวอักษรดำ, พื้นเทาอ่อน, accent เป็นดำ
  - dark: bg/card ดำ, ตัวอักษรขาว, พื้นเทาเข้ม, accent เป็นขาว
- เอา gradient ม่วง/น้ำเงินออกจาก chrome ของหน้า:
  - body background
  - โลโก้ใน app bar
  - แถบหัวข้อ `h2`
  - profile hero
  - การ์ด login และแถบด้านบน
- ปรับ active nav, filter chip, KPI icon, badge/tag, overview segment, error notice และปุ่ม merge/review ให้เป็นขาว–ดำ
- คงไอคอน SVG, card layout, empty/loading state และ responsive เดิมไว้ทั้งหมด
- เปลี่ยนลิงก์เนื้อหาเป็นดำพร้อมขีดเส้นใต้ เพื่อให้แยกออกจากข้อความได้ในธีมไม่มีสี
- หน้า `/login`: เปลี่ยนปุ่ม GitHub เป็นดำในโหมดสว่าง/ขาวในโหมดมืด, โลโก้ขาว–ดำ, เอา gradient และเงาสีออก
- สีใน diff (`add`/`del` + syntax) ยังคงไว้เพื่อให้อ่านโค้ดง่าย

## ไฟล์ที่แก้
- `dashboard-web/src/style.css`
  - เปลี่ยน CSS variables ที่ `:root` และ dark mode
  - เพิ่ม “Monochrome theme” override layer ท้ายไฟล์
- `dashboard-rs/src/main.rs`
  - เปลี่ยน palette และสีปุ่ม/การ์ดใน `LOGIN_HTML`

## ผลตรวจ
- `cd dashboard-web && npm run build` ผ่าน
  - `dist/assets/index-DNAswE1K.css` (40.71 kB)
  - `dist/assets/index-DXU_14Cg.js` (82.83 kB)
- `cd dashboard-rs && cargo check` ผ่าน
- `cargo build --release` ผ่าน (45.83s)
- restart server PID `29818` พอร์ต 7790
- `GET /login` = 200, `GET /` = 302
- asset JS = 200, CSS = 200
- ตรวจ computed style ด้วย Chromium:
  - light: body `rgb(255,255,255)`, โลโก้ `rgb(26,26,26)` + ตัวอักษรขาว, nav active ดำ/ขาว, profile ไม่มี background-image, `h2::before` ไม่มี gradient
  - dark: body `rgb(13,13,13)`, โลโก้ `rgb(232,232,232)` + ตัวอักษรดำ, ปุ่ม GitHub ขาว/ดำ, ไม่มี background-image
- `git diff --check` ผ่าน (exit 0)

## หมายเหตุ
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → ทำเองทั้งหมดและบันทึก exception
- การตรวจ SPA ด้วย mock data ก่อนหน้านี้ไม่พบ runtime/console error; เคสที่ขึ้น FAIL ในการทดสอบเป็นเพราะ fixture ให้เจ้าของ PR รีวิว PR ตัวเอง ซึ่งแอปบล็อกอย่างถูกต้อง ไม่ใช่บัค
- ยังไม่ได้ทดสอบล็อกอิน GitHub จริง end-to-end เพราะไม่มี session ในเบราว์เซอร์อัตโนมัติ
- ยังไม่ commit/push
