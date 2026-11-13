# main · color-accent-theme

- เวลา: 2026-11-13 11:55
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: เปลี่ยน chrome ของ dashboard + หน้า `/login` จากขาว–ดำล้วนเป็นธีมที่มีสี accent สำหรับหัวข้อ สถานะ ตัวเลขสำคัญ และปุ่มที่ต้องกด
- สิ่งที่ไม่ทำ: ไม่แก้ API/payload, ไม่เปลี่ยน layout, ไม่เพิ่ม dependency, ไม่แตะสี syntax/diff ที่ใช้ช่วยอ่านโค้ด

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| Color system + dashboard UI | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Login page | main | `dashboard-rs/src/main.rs` (`LOGIN_HTML`) | done | ไฟล์นี้ |
| Verify | main (runner) | build + Chromium computed styles | done | ไฟล์นี้ |

## สีที่ใช้
- Primary (indigo): `--accent:#4f46e5`, `--accent-strong:#3730a3`, `--accent-soft:#eef2ff`, `--accent-2:#7c3aed`
- Success: `--ok:#15803d` / soft `#dcfce7`
- Warning: `--warn:#b45309` / soft `#fef3c7`
- Danger: `--bad:#b91c1c` / soft `#fee2e2`
- Info: `--info:#0369a1` / soft `#e0f2fe`
- Dark mode ใช้เฉดสว่างขึ้น: accent `#818cf8`, strong `#a5b4fc`, ok `#4ade80`, warn `#fbbf24`, bad `#f87171`, info `#38bdf8`
- คงพื้นหลัง/ตัวอักษรหลักเป็นกลาง และคงสี syntax/diff เดิมเพื่อการอ่านโค้ด

## สิ่งที่ทำ
- `style.css` — `:root` / dark media
  - เปลี่ยนตัวแปรสีจาก grayscale เป็น palette ข้างต้น
  - เพิ่ม `--accent-2` สำหรับ gradient ของหัวข้อ/โลโก้
- `style.css` — Color accents layer (แทน Monochrome layer เดิม)
  - `h1`, `.d-title`, `h2` ใช้ `--accent-strong`; `h3` ใช้ `--accent`
  - `h2::before` และ `.brand-logo` เป็น gradient accent → accent-2
  - ลิงก์ใช้ accent; hover เข้มขึ้น
  - `.num`, `.meta b`, `.who`, `.kpi-v`, `.ov-stat b` เน้นข้อมูลสำคัญ
  - KPI แยกสีตามสถานะ: ok=เขียว, warn=amber, bad=แดง, info=ฟ้า
  - badges/tags ใช้สีตามสถานะจากตัวแปรเดิม (เขียว/amber/แดง/ฟ้า)
  - active nav/chip/tab ใช้ accent-soft + accent-strong
  - ปุ่มหลัก `.rf-submit`, `.mf-submit`, `.auth-login` ใช้ accent; ready=เขียว, danger=แดง
  - focus ring ของ input/select/textarea ใช้ accent
  - `file-head` และกล่องคอมเมนต์ใช้ accent-soft อ่อน ๆ
  - progress bar: งานค้าง=amber, งานเสร็จ=accent; spinner/boot icon=accent
  - radio/checkbox ใช้ `accent-color`
- `style.css` — Inline comment
  - hover บรรทัดและปุ่มคอมเมนต์ใช้ accent แทนดำ
  - หัวข้อฟอร์มคอมเมนต์และ focus ของ textarea ใช้ accent
- `main.rs` — `LOGIN_HTML`
  - ใช้ palette เดียวกับ dashboard (indigo/เขียว/amber/แดง/ฟ้า)
  - พื้นหลังมี radial accent-soft, แถบบนการ์ดเป็น gradient accent → accent-2
  - โลโก้และไอคอน perks ใช้ accent; h1 ใช้ accent-strong
  - ปุ่ม GitHub ใช้ accent (light: indigo, dark: periwinkle) และ focus ring accent
  - กล่อง error ใช้พื้นแดงอ่อน/ตัวอักษรแดง; รองรับ dark mode

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-BnF8ObJF.css` (42.40 kB)
  - `dist/assets/index-BLB6DNhf.js` (87.21 kB)
- Chromium headless + mock API ตรวจ computed style ทั้ง light/dark → ผ่านทุก assertion:
  - light: h1/d-title/h2 = `rgb(55, 48, 163)`, nav active bg = `rgb(238, 242, 255)`
  - light: KPI ok = `rgb(21, 128, 61)`, KPI bad = `rgb(185, 28, 28)`
  - light: badge ok/bad ได้เขียว/แดงตามสถานะ
  - light: submit bg = `rgb(79, 70, 229)`, text = ขาว
  - light: tab active bg = `rgb(238, 242, 255)`
  - dark: h1/nav active = `rgb(165, 180, 252)`, nav active bg = `rgb(30, 27, 75)`
  - dark: KPI ok = `rgb(74, 222, 128)`, KPI bad = `rgb(248, 113, 113)`
  - dark: submit bg = `rgb(129, 140, 248)`, text = `rgb(13, 13, 13)`
  - dark: tab active bg = `rgb(30, 27, 75)`
  - ไม่มี runtime exception / console error
- `cd dashboard-rs && cargo check` → exit 0
- `cargo build --release` (LTO=false, codegen-units=16) → exit 0 (49.21s)
- Chromium headless ตรวจ computed style `/login` light/dark → ผ่านทุก assertion:
  - light: h1 `rgb(55, 48, 163)`, ปุ่ม GitHub bg `rgb(79, 70, 229)` / text ขาว
  - light: perk icon bg `rgb(238, 242, 255)` / fg `rgb(55, 48, 163)`
  - light: แถบบนการ์ด gradient indigo → violet
  - dark: h1 `rgb(165, 180, 252)`, ปุ่ม GitHub bg `rgb(129, 140, 248)` / text `rgb(13, 13, 13)`
  - dark: perk icon bg `rgb(30, 27, 75)` / fg `rgb(165, 180, 252)`
- Server จริงพอร์ต 7790 (PID `24068`, restart แล้ว):
  - `/login` = 200 และ HTML มี `--accent:#4f46e5` / dark `--accent:#818cf8`
  - `assets/index-BnF8ObJF.css` = 200 (42,399 bytes)
  - `assets/index-BLB6DNhf.js` = 200 (87,208 bytes)
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- ยังไม่ได้ commit/push
- ไม่ได้ทดสอบคอมเมนต์จริงบน GitHub
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- หากต้องการลด/เพิ่มความจัดจ้านของสี ให้ปรับที่ตัวแปร `--accent*`, `--ok*`, `--warn*`, `--bad*`, `--info*` ใน `:root` และ dark media
