# main · monochrome-git-brand

- เวลา: 2026-11-13 12:08
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: ทำโลโก้ Git/GitHub และปุ่มล็อกอิน GitHub เป็นขาว–ดำ และตัดเงาออกจากโลโก้/ปุ่ม
- สิ่งที่ไม่ทำ: ไม่เปลี่ยนสี accent ส่วนอื่นของ dashboard, ไม่แก้ API, ไม่เปลี่ยน layout

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| Dashboard brand | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Login brand + button | main | `dashboard-rs/src/main.rs` (`LOGIN_HTML`) | done | ไฟล์นี้ |
| Verify | main (runner) | build + Chromium computed styles | done | ไฟล์นี้ |

## สิ่งที่ทำ
- `style.css` — `.brand-logo`
  - เปลี่ยนจาก gradient indigo → violet เป็น `background:var(--fg)` / `color:var(--bg)`
  - ตัด `box-shadow` ออก
  - ผลลัพธ์: light = ดำพื้น/ไอคอนขาว, dark = ขาวพื้น/ไอคอนดำ
- `main.rs` — `LOGIN_HTML`
  - `--btn` light เปลี่ยนเป็น `#18202f`, hover `#000000`, text ขาว
  - `--btn` dark เปลี่ยนเป็น `#e8e8e8`, hover `#ffffff`, text เข้ม
  - `.brand .logo` เปลี่ยนจาก gradient เป็น `background:var(--fg)` / `color:var(--bg)` และ `box-shadow:none`
  - focus ring ของปุ่ม GitHub เปลี่ยนจาก accent เป็น `var(--fg)` ให้เข้ากับขาว–ดำ
- ปุ่ม `.gh` ยังมี `box-shadow:none` ทั้งสถานะปกติและ hover

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-m0EBo19O.css` (42.30 kB)
  - `dist/assets/index-BEItw6iW.js` (87.21 kB)
- `cd dashboard-rs && cargo check && cargo build --release` → exit 0 (43.23s)
- Chromium headless ตรวจ computed style `/login` → ผ่านทุก assertion:
  - light logo bg `rgb(24, 32, 47)`, fg ขาว, shadow `none`
  - light GitHub button bg `rgb(24, 32, 47)`, fg ขาว, shadow `none`
  - dark logo bg `rgb(232, 232, 232)`, fg `rgb(13, 13, 13)`, shadow `none`
  - dark GitHub button bg `rgb(232, 232, 232)`, fg `rgb(13, 13, 13)`, shadow `none`
- ตรวจ compiled CSS ของ dashboard พบ rule สุดท้าย `.brand-logo{background:var(--fg);color:var(--bg);box-shadow:none}` ทับ gradient เดิม
- Server รีสตาร์ท PID `25409` พอร์ต 7790
- `/login` = 200 และ HTML มี `--btn:#18202f` / dark `--btn:#e8e8e8`
- CSS ใหม่ = 200
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- ยังไม่ commit/push
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- หากต้องการให้ปุ่ม GitHub กลับมาใช้สี accent ให้แก้ `--btn`, `--btn-h`, `--btn-fg` ใน `LOGIN_HTML`
