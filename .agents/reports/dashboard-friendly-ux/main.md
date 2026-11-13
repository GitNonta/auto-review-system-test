# main · dashboard-friendly-ux (ปรับ UX/UI ให้เป็นมิตร + SVG)

- เวลา: 2026-11-13 10:25
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: `dashboard-web/src/main.ts`, `dashboard-web/src/style.css`, `dashboard-web/index.html`, `dashboard-rs/src/main.rs` (หน้า login)

## สิ่งที่ทำ
### ไอคอน SVG
- เพิ่มชุดไอคอน inline SVG ใน `main.ts` (`ICONS`, `icon()`, `emptyState()`) ใช้ `currentColor` และ `aria-hidden`
- ใช้ไอคอนใน:
  - app bar / nav ทั้ง 5 เมนู + โลโก้
  - KPI ทุกการ์ด (auto-map ตามชื่อ KPI)
  - ชื่อ reviewer (`who()`), ปุ่มค้นหา, stamp เวลาอัปเดต
  - แท็บ Changes / Status / Conversation
  - ปุ่มย้อนกลับ, ปุ่ม retry, footer “เปิดบน GitHub”
  - ลิงก์โปรไฟล์ GitHub / เว็บ / Twitter
- ไม่เพิ่ม dependency หรือฟอนต์ไอคอน

### โครง UX/UI
- เปลี่ยน nav เป็น `appbar` แบบ sticky + โลโก้ + pill navigation + สถานะ active ชัดขึ้น
- เปลี่ยน KPI จาก strip ติดกันเป็น card grid พร้อมไอคอนและสีตามความหมาย
- เปลี่ยนรายการ PR เป็น card ที่มี hover/focus state และเพิ่ม chevron บอกว่าคลิกได้
- ปรับ search + filter chips เป็น pill/segmented control, เพิ่ม focus ring และขนาดแตะ
- ปรับตารางเป็น card with sticky header, hover row และ wrapper ที่เลื่อนแนวนอนได้
- ปรับ tabs ของ PR detail เป็น segmented control
- ปรับ overview/analysis เป็น card, stat card และแถบสัดส่วนที่อ่านง่ายขึ้น
- ปรับ profile hero เป็น gradient card, avatar ring, profile field card และ org card
- เพิ่ม empty state พร้อมไอคอนสำหรับรายการว่าง/ตัวกรองไม่พบ/ไม่มี org/email/social/ประวัติ
- เพิ่ม loading/error state ที่มีไอคอนและปุ่ม retry
- ปรับ responsive: nav เลื่อนแนวนอนบนจอเล็ก, KPI 2 คอลัมน์, profile 1 คอลัมน์, ปุ่ม/search เต็มความกว้าง
- เพิ่ม `prefers-reduced-motion`, `focus-visible`, `::selection`
- หน้า `/login` (server-rendered): เพิ่ม gradient card, โลโก้, รายการสิทธิประโยชน์พร้อม SVG, ปุ่ม GitHub เด่นขึ้น, กล่องแจ้งสิทธิ์ และหมายเหตุความปลอดภัย

## ไฟล์ที่แก้
- `dashboard-web/src/main.ts`
  - `ICONS`, `IconName`, `icon()`, `emptyState()` หลัง `esc()`
  - `KPI_ICONS`, `kpi()`
  - `who()`, `pickerHtml()`, `authHtml()`, `navHtml()`
  - toolbar/search, `renderList()`, `workloadRowsHtml()`, `renderPerson()`, `renderMine()`
  - `DETAIL_TABS`, `detailTabsHtml()`
  - `renderHistory()`, `renderHistoryPr()`, `historyBodyHtml()`, `renderProfile()`, `profileBodyHtml()`
- `dashboard-web/src/style.css`
  - เปลี่ยน palette/typography/radius/shadow เป็นระบบเดียว รองรับ light/dark
  - เพิ่ม “Friendly UX layer” ท้ายไฟล์สำหรับ appbar, KPI card, PR card, table card, tabs, profile, empty/loading state และ responsive
- `dashboard-web/index.html`
  - boot/loading state พร้อม SVG spinner และ `role=status`
- `dashboard-rs/src/main.rs`
  - `LOGIN_HTML` CSS + body ใหม่ (gradient, logo, perks, lock note)

## ผลตรวจ
- `cd dashboard-web && npm run build` ผ่าน
  - ล่าสุด: `dist/assets/index-Bua9QXEO.js` (82.83 kB), `dist/assets/index-QPGs39Kd.css` (37.54 kB)
- `cd dashboard-rs && cargo check` ผ่าน
- `cargo test -- --test-threads=1` ผ่าน 11 tests
- `cargo build --release` ผ่านใน 1m 05s
- restart server PID `15777` พอร์ต 7790
- `GET /login` = 200
- `GET /` = 302 → `/login?next=%2F`
- asset ใหม่ JS `index-Bua9QXEO.js` = 200, CSS `index-QPGs39Kd.css` = 200
- Chromium headless `--dump-dom /login` พบ `<ul class="perks">` 1 รายการ, `<li><span class="perk-ico">` 3 รายการ, `Continue with GitHub` 1 ปุ่ม; DOM parse ครบ
- `git diff --check` ผ่าน (exit 0)

## ข้อจำกัด / หมายเหตุ
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → ทำเองทั้งหมดและบันทึก exception นี้
- ยังไม่ได้ตรวจหน้า SPA แบบล็อกอินจริง end-to-end เพราะไม่มี GitHub session ในเบราว์เซอร์อัตโนมัติ; ตรวจด้วย build, static asset และหน้า login ใน Chromium แล้ว
- ไม่ได้รัน `tsc --noEmit` เพราะโปรเจกต์ไม่มี TypeScript compiler; ใช้ `vite build` เป็นตัวตรวจ syntax/type ของ frontend
- หน้า `/login` เป็น HTML server-rendered จึงต้อง build release ใหม่เมื่อแก้ (`cargo build --release`)
- ยังไม่ commit/push
