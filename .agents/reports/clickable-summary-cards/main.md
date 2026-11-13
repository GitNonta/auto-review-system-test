# main · clickable-summary-cards

- เวลา: 2026-11-13 12:42
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: ทำให้ element ที่แสดงตัวเลขสรุปคลิกได้ และพาไปยังรายการ/ส่วนข้อมูลที่เกี่ยวข้อง
- สิ่งที่ไม่ทำ: ไม่แก้ backend/API, ไม่เพิ่ม dependency, ไม่เปลี่ยน layout หลัก

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| KPI/stat/legend interaction | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/main.ts`, `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Verify | main (runner) | build + Chromium mock | done | ไฟล์นี้ |

## สิ่งที่ทำ
### KPI cards
- เพิ่ม `MetricAction` + `metricCard()` กลางสำหรับทำการ์ดเป็น `button`/`a`
- KPI ทุกใบในหน้า list / people / mine / history / profile คลิกได้:
  - list: กรอง `all`, `approved`, `changes`, `review`, `needs`, `large`; `บรรทัดรวม` เลื่อนไป `#overview-risk`
  - people/mine/history: เลื่อนไป section ที่เกี่ยวข้อง
  - profile: เปิด GitHub repos/gists/followers/following
- KPI ที่กำลังกรองอยู่มี `aria-pressed="true"` และพื้น accent-soft

### Overview
- การ์ดสถิติทั้ง 8 ใบคลิกได้:
  - อัตราพร้อม merge → กรอง approved
  - ต้องตามต่อ → กรอง changes + needs
  - PR ใหญ่ → กรอง large
  - ค้างเกิน 7 วัน → กรอง stale
  - ผู้เขียน / Reviewer ที่มีงาน → ไปหน้ารายคน
  - ค่าเฉลี่ย/ไฟล์ → เลื่อนไป `#overview-risk`
- Legend สถานะทุกอันเป็นปุ่มกรอง พร้อม `aria-pressed`

### Filters / rows
- เพิ่ม filter ใหม่: `large`, `attention` (changes+needs), `stale` (ไม่แตะเกิน 7 วัน)
- เพิ่ม chip ทั้งสามตัว และ sync สถานะ active ระหว่าง KPI/chip/legend
- การ์ด PR ทั้งใบคลิกเปิดรายละเอียด (ยกเว้นคลิก link/button/ขณะเลือกข้อความ)
- แถวตารางในหน้ารายคนและหน้า mine คลิกเปิด PR รายละเอียด
- แถวภาระงานยังคลิกเปิดโปรไฟล์คนตามเดิม
- ตัวเลขบรรทัด/ไฟล์ใน meta ของ detail เป็นลิงก์ไป `#files-section`

### CSS
- `a.kpi`, `button.kpi`, `a.ov-stat`, `button.ov-stat`: reset เป็นการ์ดที่คลิกได้
- hover ใช้เส้นขอบ accent; active ใช้ accent-soft; ไม่มีเงาสีม่วง
- `button.ov-lg` เป็นปุ่ม legend แบบโปร่งใส
- `article.pr[data-pr-number]`, `tr[data-pr-number]` มี cursor pointer และ hover
- เพิ่ม `scroll-margin-top` ให้ anchor ใต้ appbar

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-vegZoWEj.css` (42.99 kB)
  - `dist/assets/index-CSSA9U1e.js` (90.64 kB)
- Chromium headless + mock API ผ่าน:
  - list: KPI 7/7 เป็น button/a, overview stat 8/8 คลิกได้, legend 5 ปุ่ม, PR card 4 ใบ
  - KPI approved → แสดงเฉพาะ approved และ `aria-pressed=true`
  - chip large → แสดงเฉพาะ PR ที่ `data-large=1`
  - legend changes → แสดงเฉพาะ changes
  - stat attention → แสดง changes + needs
  - KPI all → กลับมาแสดงครบ 4 รายการ
  - คลิก meta ของ PR card → เปิด detail
  - คลิกตัวเลขบรรทัด/ไฟล์ใน detail → hash ไป `#files-section`
  - หน้ารายคน: KPI 4/4 คลิกได้, แถว PR 2 แถว, คลิกช่องตัวเลข → เปิด detail
  - ไม่มี runtime exception / console error
- Server พอร์ต 7790 ยังทำงาน PID `26987`
  - CSS ใหม่ = 200 (42,985 bytes)
  - JS ใหม่ = 200 (90,641 bytes)
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- ยังไม่ commit/push
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- ตัวเลขในข้อความบรรยาย (ไม่ใช่การ์ด/ปุ่ม) ยังไม่ได้ทำให้คลิก เพราะไม่มีปลายทางเฉพาะ
- Filter ใหม่ยังไม่ถูกเก็บใน URL; รีเฟรชแล้วกลับเป็น `all`
