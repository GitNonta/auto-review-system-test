# AGENT_SPEC: ระบบ Auto-assign Reviewer แบบ Equal Review Workload

เอกสารนี้สั่งงาน AI agent (หรือคนที่ลงมือสร้าง) ให้สร้างและติดตั้งระบบนี้ใน GitHub repo ให้ครบ ตรงตามสเปก
ชุดไฟล์อ้างอิงที่ทดสอบแล้วอยู่ในโฟลเดอร์เดียวกัน ให้ใช้เป็นฐาน ถ้าจะแก้ ต้องยังผ่านการทดสอบใน `TESTING.md`

## 1. เป้าหมาย
ทีม 3-4 คน ทุกคนต้อง:
1. รีวิวโค้ดในปริมาณ **บรรทัด** ใกล้เคียงกัน (ไม่ใช่จำนวน PR)
2. ได้แลกเปลี่ยนโค้ดกับทุกคน ไม่แยกสายงาน

Acceptance Criteria ของงาน
- [ ] commit ไฟล์คอนฟิกเข้า repo
- [ ] เปิด PR ใหม่แล้วบอทแท็ก Reviewer ให้อัตโนมัติอย่างน้อย 1 คน
- [ ] ผู้เปิด PR ไม่ถูก assign เป็น Reviewer ของตัวเอง
- [ ] เลือกตอน PR เปิด (ไม่ใช่ Draft) หรือเปลี่ยนจาก Draft เป็น Ready for review

## 2. ไฟล์ที่ต้องส่งมอบ (path ต้องตรงตามนี้)
| ไฟล์ | หน้าที่ |
|---|---|
| `.github/workflows/auto-assign-reviewer.yml` | Workflow (trigger, permissions, concurrency) |
| `.github/scripts/auto-review.js` | Logic ทั้งหมด (โหลดผ่าน `actions/github-script`) |
| `.github/auto-review.yml` | Config ทั้งหมด (ทีม, ลา, threshold, ไฟล์ที่ยกเว้น) |
| `.github/pull_request_template.md` | PR Template 3 บรรทัด |
| `.github/scripts/auto-review.test.js` | Unit test ของฟังก์ชันล้วน |
| `.github/scripts/auto-review.integration.test.js` | ทดสอบ flow ด้วย GitHub API จำลอง |
| `TESTING.md` | ขั้นตอนทดสอบ (unit, dry-run, ทดสอบจริง) |

## 3. ข้อกำหนดทางเทคนิค
- ใช้ `actions/github-script@v7`, `actions/checkout@v4`, `actions/setup-node@v4` (Node 20) และ `js-yaml@4.1.0` เท่านั้น ห้ามพึ่ง Action ภายนอกอื่น
- Trigger: `pull_request_target` (`opened`, `reopened`, `ready_for_review`, `synchronize`), `schedule` (ทุกวัน 02:00 UTC), `workflow_dispatch` (mode: sweep/report, dry_run)
- ใช้ `pull_request_target` เพื่อให้ token เขียนได้กับ PR จาก fork **ห้าม checkout โค้ดของ PR head และห้ามรันโค้ดจาก PR** (ความเสี่ยงด้านความปลอดภัย)
- Permissions ขั้นต่ำ: `contents: read`, `pull-requests: write`, `issues: write`
- Concurrency ต่อ PR: `group: auto-review-<pr number>`, `cancel-in-progress: false`
- ไม่ใช้ฐานข้อมูลภายนอก สถานะทั้งหมดคำนวณจาก GitHub API ทุกครั้ง ยกเว้น marker ในคอมเมนต์ของบอท (ข้อ 6)
- การตั้งชื่อ login ไม่สนตัวพิมพ์เล็ก/ใหญ่

## 4. สูตรและกติกา
### 4.1 Score
```
Score = C + pending_weight × P
C = บรรทัดที่ Reviewer รีวิวเสร็จ (มี review ที่ส่งแล้ว) ใน history_days วันล่าสุด
P = บรรทัดที่ยังค้าง (ถูกขอ review แล้วยังไม่ส่ง) ใน PR ที่เปิดอยู่และไม่ใช่ Draft
```
- บรรทัด = additions + deletions ของไฟล์ที่ **ไม่อยู่ใน** `exclude_patterns`
- C กับ P แยกกัน ไม่นับงานเดียวกันซ้ำ เมื่อรีวิวเสร็จ บรรทัดย้ายจาก P ไป C
- PR ที่ปิดโดยที่ Reviewer ไม่ได้รีวิว ไม่นับเข้า C และไม่นับเป็น P
- บรรทัดของ Reviewer แต่ละคนใน PR = ค่าใน marker `lines[login]` ถ้ามี ไม่เช่นนั้นเท่ากับบรรทัดรวมของ PR (นับเต็ม)

### 4.2 เมื่อไรที่ทำงาน
| เหตุการณ์ | การทำงาน |
|---|---|
| `opened` / `reopened` (ไม่ใช่ Draft) | เลือก Reviewer |
| PR เป็น Draft | ไม่ทำอะไร ไม่นับบรรทัด |
| `ready_for_review` | เลือก Reviewer ด้วยขนาด ณ เวลานั้น |
| `synchronize` | รันกระบวนการเดียวกันซ้ำ (idempotent): ปรับขนาด/ขอบเขต และเติมคนที่สองเมื่อ PR โตเกิน threshold |
| schedule รายวัน | sweep: จัด PR ที่ยังไม่มี marker หรือติดป้ายรอ Reviewer, เตือนงานเกินเวลา, รายงานทุกวันจันทร์ |

### 4.3 จำนวน Reviewer ที่ต้องการ
- ขนาดรวม ≤ `large_pr_threshold` → 1 คน
- ขนาดรวม > `large_pr_threshold` → `large_pr_reviewers` (ค่าเริ่มต้น 2) คน

### 4.4 ผู้มีสิทธิ์ถูกเลือก (candidates)
สมาชิกใน `team` ลบด้วย: Author, คนใน `unavailable` (ถ้ามี `until` ต้องเลยวันนั้นแล้วจึงกลับมา), และคนที่เป็น Reviewer ของ PR นี้อยู่แล้ว

### 4.5 การเลือกคน (ทีละคน)
1. ตัดคนที่ P ≥ `pending_cap` (ถ้าทุกคนเกิน ให้ใช้ทุกคน เลือกต่ำสุด และเตือนในคอมเมนต์)
2. หา Score ต่ำสุด แล้วเอาทุกคนที่ Score ห่างจากต่ำสุดไม่เกิน `tie_threshold`
3. ในกลุ่มนั้นเลือกคนที่ **ไม่ได้รีวิวโค้ดของ Author นี้มานานที่สุด** (Review Exchange Matrix, ไม่เคยเลย = ลำดับแรก)
4. ถ้ายังเท่ากัน ใช้ Round-Robin แบบไม่ต้องเก็บสถานะ (หมุนตามเลข PR)
5. ก่อนเลือกคนถัดไป ให้บวกบรรทัดที่เพิ่ง assign (ส่วนแบ่ง) เข้า P ชั่วคราว

### 4.6 PR ขนาดใหญ่
- `split_large_pr: true`: แบ่งไฟล์ให้ Reviewer แต่ละคนให้บรรทัดใกล้เคียงกัน (greedy ไฟล์ใหญ่ก่อน) ภาระนับตามบรรทัดของไฟล์ที่ได้จริง **ไม่หารครึ่งอัตโนมัติ** และบอทคอมเมนต์ระบุไฟล์ของแต่ละคน
- `split_large_pr: false`: ทุกคนอ่านทั้ง PR และนับเต็มทุกคน
- ถ้ามี Reviewer ที่เลือกเองอยู่แล้ว ให้นับรวมในการแบ่งขอบเขต

### 4.7 มี Reviewer ที่ผู้เขียนเลือกเองอยู่แล้ว
- **ห้ามลบหรือทับ** คนที่เลือกเอง
- นับเป็น Reviewer ของ PR และบรรทัดเข้า Score ของเขาผ่านการคำนวณจาก API เสมอ
- เติมคนให้ครบตามข้อ 4.3 (เช่น เลือกเอง 1 คน แต่ PR ใหญ่ต้องการ 2 → เติม 1 คน)
- คนที่เลือกเองและคนนอกทีมไม่นับเข้าจำนวนที่ครบ เฉพาะสมาชิกใน `team` เท่านั้นที่นับ
- ออปชัน `balance_extra_reviewer` (ค่าเริ่มต้น false): ถ้าเปิด และคนที่เลือกเองมี Score สูงกว่าคนต่ำสุดเกิน `balance_extra_gap` ให้เพิ่มคนต่ำสุดอีก 1 คน (ทำครั้งแรกครั้งเดียว ไม่เกิน `max_reviewers`)

### 4.8 ไม่มีคนที่เลือกได้
| กรณี | การทำงาน |
|---|---|
| ไม่เหลือ candidate เลย | ไม่ assign ใคร (ห้าม assign Author หรือคนลา) ติด label `needs-reviewer` คอมเมนต์แจ้งสาเหตุ sweep รายวันลองใหม่ |
| เหลือน้อยกว่าที่ต้องการ (เช่น ต้องการ 2 ได้ 1) | assign เท่าที่ได้ ติด label `needs-second-reviewer` คอมเมนต์แจ้ง |
| ทุกคนเกิน `pending_cap` | assign คน Score ต่ำสุด และเตือนว่าทีมงานล้น |
| `requestReviewers` ล้มเหลว | ไม่ล้มทั้ง job คอมเมนต์แจ้งให้เลือกเอง และติด label ตามจริง |

เมื่อมี Reviewer ครบ label ต้องถูกลบเอง

### 4.9 กรณีอื่น
- ผู้เขียนเป็น Bot หรือไม่อยู่ใน `team` (และ `allow_external_authors: false`) → ข้าม
- PR ที่ติด label ใน `skip_labels` → ข้าม
- Reviewer ค้างเกิน `overdue_hours` → คอมเมนต์เตือนครั้งเดียวและเสนอคนที่ Score ต่ำสุดแทน **ห้ามเปลี่ยน Reviewer เอง**
- สมาชิกที่ลาไม่ถูกลงโทษ ยอดไม่เพิ่ม กลับมาแล้วได้งานต่อ

## 5. Config (`.github/auto-review.yml`)
Key ทั้งหมด (ต้องมีเอกสารอธิบายในไฟล์):
`timezone`, `team`, `unavailable`, `history_days`, `pending_weight`, `pending_cap`, `tie_threshold`, `large_pr_threshold`, `large_pr_reviewers`, `split_large_pr`, `balance_extra_reviewer`, `balance_extra_gap`, `max_reviewers`, `overdue_hours`, `allow_external_authors`, `skip_labels`, `exclude_patterns`, `labels`, `report`, `dry_run`

- `team` ต้องมีอย่างน้อย 2 คน ถ้าไม่ถึง job ต้องล้มพร้อมข้อความชัดเจน
- `dry_run: true` (หรือ input `dry_run` ของ workflow_dispatch) ต้องไม่ทำการเปลี่ยนแปลงใด ๆ ใน repo และเขียนสิ่งที่จะทำลง Job Summary

## 6. คอมเมนต์ของบอทและ marker
- บอทมีคอมเมนต์เดียวต่อ PR แก้ไขในที่เดิม (ไม่สร้างใหม่ทุกครั้ง)
- ท้ายคอมเมนต์เป็น marker: `<!-- auto-review-state: {JSON} -->`
- JSON ต้องมี: `v`, `total`, `split`, `lines` (login → บรรทัด), `bot` (คนที่บอทเลือก), `at`, `needs`, `reminded`, `balanced`
- ยอมรับ marker เฉพาะคอมเมนต์ของ `github-actions[bot]` (กันคนปลอม)
- ข้อความอธิบายเป็นภาษาไทย ระบุ: ขนาด PR, Reviewer และที่มา (บอท/เลือกเอง), Score ของคนที่บอทเลือก, ขอบเขตไฟล์ (ถ้าแบ่ง), สถานะรอ Reviewer

## 7. รายงาน
ทุกวันจันทร์ (ตาม `timezone`) หรือเรียกเอง (`workflow_dispatch` mode `report`) ให้เขียน Job Summary:
- ตาราง C, P, รวม ต่อคน
- `Gap = (Max − Min) ÷ Average × 100` ของ (C+P) เป้าหมาย ≤ 20% ใน 30 วัน
- Review Exchange Matrix (ผู้รีวิว × ผู้เขียน)
- ถ้า `report.issue: true` สร้าง Issue ด้วย

## 8. ข้อห้าม
- ห้าม assign Author เป็น Reviewer ของตัวเองทุกกรณี
- ห้าม assign คนที่ลา/ไม่พร้อม แม้ไม่มีใครเหลือ
- ห้ามลบ Reviewer ที่ผู้เขียนเลือกเอง
- ห้ามเปลี่ยน Reviewer เองระหว่างรีวิว
- ห้าม checkout หรือรันโค้ดจาก PR head ใน workflow `pull_request_target`
- ห้ามเก็บ secret หรือ token ในไฟล์

## 9. ขั้นตอนที่ agent ต้องทำ
1. สร้างหรือตรวจไฟล์ตามข้อ 2 (ใช้ไฟล์อ้างอิงที่ให้มา)
2. แก้ `team` ใน `.github/auto-review.yml` เป็น username จริง และยืนยันว่าทุกคนเป็น collaborator
3. รัน `node --test .github/scripts/auto-review.test.js` และ `npm install --no-save js-yaml@4.1.0 && node --test .github/scripts/auto-review.integration.test.js` ต้องผ่านทั้งหมด
4. เปิด PR เพิ่มไฟล์เข้า default branch (**workflow ต้องอยู่บน default branch ก่อนจึงจะทำงานกับ PR อื่น**)
5. สร้าง label ที่ใช้ (บอทสร้างให้เองถ้ายังไม่มี) และตรวจ Settings → Actions → General → Workflow permissions ว่าอนุญาต read/write และ "Allow GitHub Actions to create and approve pull requests" ถ้าองค์กรบังคับ
6. ทดสอบตาม `TESTING.md` (เริ่มจาก dry-run) แล้วรายงานผลทุกข้อ ผ่าน/ไม่ผ่าน พร้อมหลักฐาน (ลิงก์ PR/Run)
7. ถ้า Branch protection ใช้ CODEOWNERS ให้ตรวจว่าไม่ขัดกับบอท (CODEOWNERS ที่ถูกขอ review อัตโนมัติจะนับเป็น Reviewer ที่เลือกเองตามข้อ 4.7)

## 10. Definition of Done
- Acceptance Criteria ข้อ 1 ครบ
- Test ทุกข้อใน `TESTING.md` หมวด A, B และ C ผ่าน (หมวด D ผ่านหรือบันทึกเหตุผลที่ข้าม)
- ไม่มี warning ที่เกี่ยวกับสิทธิ์ใน Run ล่าสุด
- ทีมได้รับคำอธิบายสั้น ๆ: วิธีข้ามบอท (label `no-auto-review`), วิธีแจ้งลา (`unavailable`), วิธีอ่านคอมเมนต์ของบอท

## 11. ข้อจำกัดที่รู้อยู่แล้ว (ระบุในเอกสารส่งมอบ)
- บรรทัดเท่ากันไม่ได้แปลว่าความยากเท่ากัน
- PR สองรายการที่เปิดห่างกันไม่กี่วินาทีอาจเลือกคนเดียวกัน (workflow รันแยกกัน) ระบบจะถ่วงดุลใน PR ถัดไป
- ถ้า Reviewer ไม่ได้ส่ง review (PR ถูกปิดไปเอง) งานนั้นไม่ถูกนับ
- Workflow เป็น `pull_request_target` จึงต้องอยู่บน default branch ก่อน และการแก้ workflow ใน PR จะไม่มีผลกับ PR นั้น
