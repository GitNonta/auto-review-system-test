# main · move-auth-to-navbar

- เวลา: 2026-11-13 12:57
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: ย้ายกลุ่มบัญชี `@username` + ปุ่มออกจากระบบ จากหัวข้อหน้าไปไว้ขวาสุดของ navbar
- สิ่งที่ไม่ทำ: ไม่แก้ backend/auth flow, ไม่เปลี่ยนหน้า login, ไม่เพิ่ม dependency

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| Navbar account group | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/main.ts`, `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Verify | main (runner) | build + Chromium mock | done | ไฟล์นี้ |

## สิ่งที่ทำ
- `main.ts` — `navHtml()`
  - เพิ่ม `${authHtml()}` หลัง `<nav>` ภายใน `<header class="appbar">`
  - ลบเมนู `โปรไฟล์` ออกจาก nav กลาง เพราะบัญชี `@username` ที่ขวาสุดทำหน้าที่เป็นทางเข้าโปรไฟล์แล้ว
- `main.ts` — `authHtml()`
  - เพิ่ม `aria-current="page"` ให้ปุ่มบัญชีเมื่ออยู่หน้าโปรไฟล์
- `main.ts` — ลบ `${authHtml()}` ออกจาก `.top` ของทุกหน้า:
  - list, person, detail, mine (ทั้งกรณีไม่มีตัวตนและมีตัวตน), history, people, profile
  - ตอนนี้เหลือ `authHtml()` จุดเดียวคือใน navbar
- `style.css` — `.auth`
  - เพิ่ม `order:2; flex:none` เพื่อให้อยู่หลัง nav และชิดขวาสุด
  - มือถือ: auth อยู่แถวบนขวา, nav อยู่แถวถัดไป (`order:3`)
  - เพิ่มสไตล์ `.auth-user[aria-current=page]` เป็น accent-soft
  - คงปุ่มออกจากระบบทำงานผ่าน handler `#logout` เดิม

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-sLaAehZq.css` (43.13 kB)
  - `dist/assets/index-CVG-4ap2.js` (90.59 kB)
- Chromium headless + mock auth → ผ่าน:
  - desktop 1280px: `.auth` เป็นลูกสุดท้ายของ `.appbar`, ชิดขวา gap 0px
  - `.top .auth` = 0 (ไม่มีตัวซ้ำ)
  - ข้อความในกลุ่มบัญชี = `@tester` + `ออก`
  - mobile 390px: auth อยู่แถวบน (`top=11`) เหนือ nav (`top=58`)
  - `order` ของ auth = 2, nav = 3
  - ไม่มี runtime exception / console error
- Server พอร์ต 7790 ยังทำงาน PID `26987` (ไม่ต้อง restart เพราะเป็น static frontend)
  - CSS ใหม่ = 200 (43,133 bytes)
  - JS ใหม่ = 200 (90,590 bytes)
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- ยังไม่ commit/push
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- การย้ายไม่มีผลกับ `/login` หรือ OAuth callback
