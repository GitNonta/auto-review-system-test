# main · dashboard-overview (เพิ่มรายละเอียดภาพรวมเชิงวิเคราะห์)

- เวลา: 2026-11-13 07:20
- สถานะ: done (ยังไม่ commit/push)
- ขอบเขต: frontend `dashboard-web/src/main.ts`, `dashboard-web/src/style.css`

## สิ่งที่เพิ่ม
- section **“ภาพรวมเชิงวิเคราะห์”** ใต้ KPI เดิมในหน้า `?view=list`
- สถิติ 8 ช่อง: อัตราพร้อม merge, เฉลี่ย/มัธยฐานบรรทัดต่อ PR, ไฟล์ที่เปลี่ยนแปลง, PR ที่ต้องตามต่อ, PR ใหญ่ + % ของบรรทัด, จำนวนผู้เขียน + คนที่เปิดมากสุด, reviewer ที่มีงาน + บรรทัดค้างรวม, PR ค้างเกิน 7 วัน + อายุมากสุด
- แถบสัดส่วนสถานะ PR (พร้อม merge / รอรีวิว / ขอแก้ไข / ไม่มี reviewer / draft) + legend จำนวนและ %
- บทวิเคราะห์ 4 พารากราฟที่คำนวณสดจากข้อมูล: สุขภาพโดยรวม, คอขวดการรีวิว/อายุ PR, ภาระงานและการกระจายตัวของ reviewer, ความเสี่ยงด้านขนาด/ความสด
- “จุดที่ควรดูก่อน” สูงสุด 5 PR พร้อมเหตุผล (ไม่มี reviewer / ขอแก้ไข / PR ใหญ่ / ไม่ขยับเกิน 7 วัน) ลิงก์เข้าหน้ารายละเอียด PR ได้
- หมายเหตุว่าสถิติคิดจาก PR ทั้งหมด ไม่อิงตัวกรอง
- CSS `.overview`, `.ov-grid`, `.ov-stat`, `.ov-stack`, `.ov-legend`, `.ov-notes`, `.ov-do` + responsive 2 คอลัมน์บนมือถือ

## ผลตรวจ
- `npm run build` (Vite) ผ่าน exit 0 — asset ล่าสุด `index-CyYu1Ohh.js`, `index-C-Pm_hDW.css`
- เสิร์ฟจาก dashboard-rs PID 29230 ที่พอร์ต 7790: `/` = 200, asset ใหม่ = 200
- ตรวจ DOM ด้วย Chromium headless (`--dump-dom`, virtual-time 5s): section เรนเดอร์จริง, มีสถิติ/แถบสัดส่วน/บทวิเคราะห์ครบ, risk list 5 รายการ
- ตรวจถ้อยคำกรณีภาระกระจุกคนเดียว (แก้ไม่ให้ขัดกัน) และกรณี PR อัปเดตวันนี้

## ข้อจำกัด / หมายเหตุ
- ไม่มี `typescript` ใน devDependencies และไม่มี tsconfig → ตรวจด้วย `vite build` + DOM จริงเท่านั้น ไม่ได้รัน `tsc --noEmit`
- ยังไม่ทดสอบ interaction ด้วยมือบนมือถือ/desktop จริง (ไม่ได้ขอ screenshot)
- AGENTS.md §7: session นี้ไม่มี `spawn_agents` → ทำเองและบันทึก exception
- ยังไม่ commit/push
