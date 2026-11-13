# main · click-line-to-comment

- เวลา: 2026-11-13 11:42
- สถานะรวม: done (ยังไม่ commit/push)
- ขอบเขตงาน: ทำให้คลิกที่ตัวบรรทัดโค้ดใน diff เพื่อเปิดฟอร์มคอมเมนต์ได้ทันที โดยยังคงปุ่มไอคอนเดิมไว้
- สิ่งที่ไม่ทำ: ไม่แก้ backend/API, ไม่เปลี่ยน payload, ไม่ทดสอบสร้างคอมเมนต์บน GitHub จริง

## slice และเจ้าของ
| slice | worker | ไฟล์ | สถานะ | report |
|---|---|---|---|---|
| UI interaction | main (exception: ไม่มี `spawn_agents` ใน session) | `dashboard-web/src/main.ts`, `dashboard-web/src/style.css` | done | ไฟล์นี้ |
| Verify | main (runner) | build + Chromium mock | done | ไฟล์นี้ |

## สิ่งที่ทำ
- `main.ts` — `fileBlockHtml`
  - เพิ่ม `data-line-target`, `data-comment-file`, `data-comment-line`, `data-comment-side` ให้ `<tr>` ของบรรทัด diff ที่คอมเมนต์ได้
  - ยังสร้างปุ่ม `lineCommentButton` เดิมไว้ทุกบรรทัด
- `main.ts` — `openLineComment`
  - รับ `HTMLElement` แทน `HTMLButtonElement` เพื่อรองรับทั้งปุ่มและแถว
  - ถ้าคลิกบรรทัดเดิมขณะฟอร์มเปิดอยู่ ให้โฟกัส textarea เดิมแทนการสร้างใหม่ (ไม่ทำให้ draft หาย)
- `main.ts` — `wire()`
  - เพิ่ม click listener บน `tr[data-line-target]`
  - ไม่เปิดฟอร์มเมื่อคลิกปุ่ม/ลิงก์/input/textarea/select/label
  - ไม่เปิดฟอร์มเมื่อมีข้อความถูกเลือกอยู่ (กันการ select/double-click โค้ด)
- `main.ts` — ข้อความช่วยเหลือในหน้า detail เปลี่ยนเป็น “คลิกที่บรรทัดหรือไอคอนเพื่อคอมเมนต์”
- `style.css`
  - แถวที่คลิกได้ใช้ `cursor:pointer`
  - hover แสดงเส้นขอบซ้ายดำและเน้นเลขบรรทัด

## ผล verify
- `cd dashboard-web && npm run build` → exit 0
  - `dist/assets/index-iuv9aUg9.css` (42.61 kB)
  - `dist/assets/index-xuhgkWWS.js` (87.21 kB)
- Chromium headless + mock API (viewport 1200×900) → ผ่านทุกข้อ:
  - พบ 6 แถว `tr[data-line-target]`, 6 ปุ่ม, `cursor=pointer`
  - คลิกกลาง `td.code` ของบรรทัด context → ฟอร์มแทรกใต้แถวเดียวกัน, heading “คอมเมนต์บรรทัด 1”, textarea โฟกัส
  - ยกเลิกแล้วคลิกแถวบรรทัดลบ → heading “คอมเมนต์บรรทัด 2”, meta “ฝั่งเดิม · src/app.ts”, มีฟอร์มเดียว
  - คลิกปุ่มไอคอนเดิม → ยังเปิดฟอร์มได้และไม่มีฟอร์มซ้ำ
  - ไม่มี runtime exception / console error
- ตรวจ server จริงพอร์ต 7790:
  - `assets/index-xuhgkWWS.js` = 200 (87,208 bytes)
  - `assets/index-iuv9aUg9.css` = 200 (42,611 bytes)
  - `/login` = 200; `/` = 302 ไป `/login` เมื่อไม่มี session (พฤติกรรม auth เดิม)
- `git diff --check` → exit 0

## ปัญหา/defect
- ไม่มี

## ขั้นถัดไป / สิ่งที่คนแก้ต่อควรรู้
- หากต้องการให้คลิกบรรทัดขณะเลือกข้อความได้ด้วย ต้องออกแบบ interaction เพิ่ม เพราะปัจจุบันกันการเปิดฟอร์มเมื่อมี selection อยู่
- ยังไม่ได้ทดสอบคอมเมนต์จริงกับ GitHub เพื่อหลีกเลี่ยงการสร้างความคิดเห็น/แจ้งเตือนบน PR จริง
- AGENTS.md §7 บังคับใช้ subagent แต่ session นี้ไม่มี tool `spawn_agents` → บันทึก exception นี้
- ยังไม่ commit/push
