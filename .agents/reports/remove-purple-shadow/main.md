# main · remove-purple-shadow

- เวลา: 2026-11-13 12:18
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: ปิดเงา/glow สีม่วงที่มาจาก accent ทั้งใน dashboard และหน้า `/login`
- สิ่งที่ไม่ทำ: ไม่ปิดเงาการ์ดสีเทากลาง ๆ, ไม่เปลี่ยน layout, ไม่แก้ API

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| Dashboard shadow cleanup | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Login shadow cleanup | main | `dashboard-rs/src/main.rs` (`LOGIN_HTML`) | done | ไฟล์นี้ |
| Verify | main (runner) | build + grep + health check | done | ไฟล์นี้ |

## สิ่งที่ทำ
- ลบ radial gradient สีม่วง/accent-soft ออกจาก `body` ทั้ง dashboard และหน้า login → ใช้พื้น `var(--bg)` เรียบ
- เปลี่ยน focus glow ของ `input[type=search]`, `select`, textarea และช่องคอมเมนต์จาก `box-shadow:0 0 0 3px var(--accent-soft)` เป็น `box-shadow:none`
- ลบ `box-shadow:inset 3px 0 0 var(--accent)` ตอน hover บรรทัด diff
- ลบ `box-shadow` เก่าของ `.brand-logo` ที่ยังเหลืออยู่ใน rule ก่อนหน้า (final rule ยังเป็น `box-shadow:none`)
- คงเส้นขอบ accent และสีตัวอักษร/ไอคอนไว้ เพื่อให้ยังเห็น focus/hover โดยไม่ใช้เงา
- เงาการ์ดสีกลาง (`var(--shadow)`) ไม่ได้แตะ เพราะไม่ใช่เงาสีม่วง

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-B5hLMpA5.css` (41.90 kB)
  - `dist/assets/index-DpvnjcpP.js` (87.21 kB)
- ตรวจ compiled CSS:
  - `box-shadow` ที่อ้าง `accent-soft` = 0
  - `radial-gradient` = 0
  - `.brand-logo` rule สุดท้าย = `box-shadow:none`
- `cd dashboard-rs && cargo check && cargo build --release` → exit 0 (1m 03s)
- Server รีสตาร์ท PID `26987` พอร์ต 7790
- `/login` = 200 และ HTML ไม่มี `radial-gradient`
- CSS ใหม่ = 200
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- ยังไม่ commit/push
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- หากต้องการเอาเงาการ์ดสีกลางออกด้วย ให้แก้ `--shadow` ใน `:root` และ dark media
