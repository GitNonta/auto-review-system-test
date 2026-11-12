# TESTING: ขั้นตอนทดสอบระบบ Auto-assign Reviewer

ทดสอบเป็นลำดับ A → B → C → D ถ้าหมวดก่อนหน้าไม่ผ่าน อย่าไปต่อ

## ข้อกำหนดก่อนเริ่ม
- **Repo ทดสอบแยกต่างหาก** (ห้ามลองกับ repo จริงก่อน) มีบัญชีที่เป็น collaborator อย่างน้อย 4 คน (A, B, C, D) ถ้าไม่มีให้ใช้เพื่อนร่วมทีมช่วยกดเปิด PR/รีวิว
- ไฟล์ทั้งหมดอยู่บน **default branch** แล้ว (`pull_request_target` ใช้ workflow จาก default branch เท่านั้น)
- `team` ใน `.github/auto-review.yml` เป็น username จริงของทั้ง 4 คน
- Settings → Actions → General → Workflow permissions: Read and write
- เครื่องมือบนเครื่อง: Node 20+, git, (ตัวเลือก) `gh` CLI

### สคริปต์ช่วยสร้าง PR ขนาดตามต้องการ
```bash
# ใช้: mkpr <branch> <จำนวนบรรทัด> [ไฟล์]   (รันใน repo ทดสอบ)
mkpr() {
  git checkout -q main && git pull -q && git checkout -q -b "$1"
  f="${3:-src/gen_$1.txt}"; mkdir -p "$(dirname "$f")"
  seq 1 "$2" | sed 's/^/line /' > "$f"
  git add . && git commit -qm "test $1 ($2 lines)" && git push -q -u origin "$1"
  gh pr create --fill --base main --head "$1"
  git checkout -q main
}
# ตัวอย่าง: mkpr t01 100        # PR 100 บรรทัด
#           mkpr t06 800        # PR 800 บรรทัด
# สร้างไฟล์หลายไฟล์: เรียก seq/sed หลายครั้งใน branch เดียวกันก่อน commit
```
ถ้าจะสร้าง PR ในนามผู้เขียนคนอื่น ให้ให้คนนั้น push เอง หรือ login ด้วย `gh auth switch`

## A. Unit และ Integration (ไม่ต้องใช้ GitHub จริง)
```bash
npm install --no-save js-yaml@4.1.0
node --test .github/scripts/auto-review.test.js
node --test .github/scripts/auto-review.integration.test.js
```
ต้องผ่านทั้งหมด (unit 11, integration 8 ข้อ) ถ้ามีข้อล้ม ห้ามไปต่อ
ครอบคลุม: glob ไฟล์ยกเว้น, แบ่งขอบเขตไฟล์, เลือก Score ต่ำสุด, งานค้าง ×2, tie-break ด้วย Exchange Matrix, Round-Robin, เพดานงานค้าง, Gap, marker, จำลอง 80 PR (Gap ≤ 20%), เปิด PR เล็ก/ใหญ่, เลือกเอง, Draft, รันซ้ำ, synchronize, ทุกคนลา

## B. Dry-run บน GitHub จริง
1. ตั้ง `dry_run: true` ใน `.github/auto-review.yml` (merge เข้า default branch)
2. เปิด PR ทดสอบ 1 อัน → ดู Run → Summary ต้องแสดงรายการ "Dry-run: สิ่งที่บอทจะทำ" และ **PR ต้องไม่มี Reviewer และไม่มีคอมเมนต์จากบอท**
3. Actions → Auto-assign Reviewer → Run workflow → `mode=report` → Summary ต้องมีตารางภาระ, ค่า Gap, และ Exchange Matrix
4. ตั้ง `dry_run: false` แล้ว merge กลับ

## C. ทดสอบจริง (ทุกข้อต้องมีหลักฐาน: ลิงก์ PR หรือ Run)
ก่อนเริ่มให้ปิด PR ทดสอบเก่าทั้งหมด (หรือใช้ `history_days` สั้นเพื่อล้างผลกระทบของ Score เดิม) และบันทึก Score เริ่มต้นจากรายงาน

| รหัส | สถานการณ์ | วิธีทำ | ผลที่คาดหวัง |
|---|---|---|---|
| C01 | เปิด PR ปกติ | A เปิด PR 100 บรรทัด | มี Reviewer 1 คน ไม่ใช่ A มีคอมเมนต์บอทพร้อมเหตุผล |
| C02 | Draft | A เปิดเป็น Draft | ไม่มี Reviewer ไม่มีคอมเมนต์ |
| C03 | Draft → Ready | กด Ready for review จาก C02 | บอทเลือก 1 คน ตามขนาด ณ เวลานั้น |
| C04 | Author ไม่ถูกเลือก | ให้ A มี Score ต่ำสุด แล้วให้ A เปิด PR | Reviewer ไม่ใช่ A |
| C05 | ไฟล์ที่ไม่นับ | PR มี `package-lock.json` 5,000 บรรทัด + โค้ด 50 บรรทัด | คอมเมนต์ระบุขนาด 50 บรรทัด |
| C06 | PR ใหญ่ | PR 800 บรรทัด หลายไฟล์ | Reviewer 2 คน ขอบเขตไฟล์ไม่ซ้ำ รวมครบทุกไฟล์ บรรทัดของสองคนใกล้เคียงกัน |
| C07 | PR โตหลังเปิด | เปิด PR 100 บรรทัด แล้ว push เพิ่มเป็น 800 | เพิ่ม Reviewer คนที่สอง คอมเมนต์อัปเดตที่เดิม (ไม่สร้างใหม่) |
| C08 | เลือกเอง ครบแล้ว | A เลือก B เองใน PR เล็ก | บอทไม่เพิ่มใคร B ยังอยู่ คอมเมนต์ระบุ "เลือกเอง" |
| C09 | เลือกเอง ไม่ครบ | A เลือก B เองใน PR 800 บรรทัด | B อยู่ต่อ บอทเพิ่ม 1 คน ขอบเขตแบ่งให้ทั้งคู่ |
| C10 | ภาระนับจากคนเลือกเอง | หลัง C08 เปิด PR ใหม่ | Score ของ B สูงขึ้น (ดูรายงาน) บอทเลือกคนอื่นที่ต่ำกว่า |
| C11 | งานค้างถูกถ่วง ×2 | B ถือ PR ใหญ่ค้างไว้ไม่รีวิว แล้วเปิด PR ใหม่หลายอัน | บอทเลี่ยง B จนกว่า Score ของคนอื่นไล่ทัน |
| C12 | รีวิวเสร็จแล้วยอดย้ายจาก P → C | ให้ B ส่ง review PR ที่ค้าง แล้วรัน report | P ของ B ลด C เพิ่ม ผลรวมไม่เปลี่ยน (ไม่นับซ้ำ) |
| C13 | คนลา | ใส่ B ใน `unavailable` (merge) แล้วเปิด PR | ไม่เลือก B |
| C14 | กลับจากลาอัตโนมัติ | ตั้ง `until` เป็นเมื่อวาน | B กลับมาเป็นตัวเลือก |
| C15 | ไม่มีใครเลือกได้ | ใส่ B, C, D ลา แล้ว A เปิด PR | ไม่มี Reviewer ไม่ assign A มี label `needs-reviewer` คอมเมนต์แจ้ง |
| C16 | กลับมาแล้วจัดให้เอง | เอา B ออกจาก `unavailable` แล้วรัน `workflow_dispatch` mode `sweep` | B ถูก assign label `needs-reviewer` ถูกลบ |
| C17 | ขาดคนที่สอง | ให้ลาจนเหลือผู้เลือกได้ 1 คน แล้วเปิด PR 800 บรรทัด | assign 1 คน label `needs-second-reviewer` |
| C18 | ทุกคนงานค้างเกินเพดาน | ตั้ง `pending_cap` ต่ำ (เช่น 50) แล้วเปิด PR | assign คน Score ต่ำสุด คอมเมนต์เตือนทีมงานล้น |
| C19 | สลับคู่ | ให้ Score ใกล้กัน (ห่าง < 300) และคนหนึ่งเพิ่งรีวิว Author นี้ | บอทเลือกคนที่ไม่ได้รีวิว Author นี้นานสุด |
| C20 | Round-Robin | ตอนที่ทุกคน Score เท่ากันและไม่มีประวัติ เปิด PR ต่อเนื่อง | ผู้รีวิวหมุนเวียน ไม่ซ้ำคนเดียว |
| C21 | ข้ามบอท | ติด label `no-auto-review` ก่อนเปิด/ก่อน Ready | บอทไม่ทำอะไร |
| C22 | ผู้เขียนเป็น Bot/คนนอก | เปิด PR ด้วยบัญชีนอก `team` (หรือ Dependabot) | บอทข้าม |
| C23 | รันซ้ำ | กด Reopen / Draft → Ready ซ้ำ / push เพิ่ม | ไม่มี Reviewer เพิ่ม ไม่มีคอมเมนต์ซ้ำ |
| C24 | เตือนงานเกินเวลา | ตั้ง `overdue_hours: 0.02` ให้ B ค้าง แล้วรัน sweep | บอทคอมเมนต์เตือน 1 ครั้ง เสนอคนต่ำสุด **ไม่เปลี่ยน Reviewer** รัน sweep ซ้ำไม่เตือนซ้ำ |
| C25 | marker ปลอม | ให้คนอื่นคอมเมนต์ข้อความที่มี marker ปลอมใน PR | บอทไม่เชื่อ (ใช้เฉพาะคอมเมนต์ของ github-actions[bot]) |

เกณฑ์ผ่าน: ทุกแถวได้ผลตามที่คาดหวัง และไม่มีครั้งใดที่ Author ถูก assign เป็น Reviewer ของตัวเอง

## D. ทดสอบความเท่าเทียมระยะยาว
1. ใช้งานจริง 2 สัปดาห์ (หรือจำลอง 20-30 PR หลายขนาด 50-400 บรรทัด โดยผู้เขียนหมุนเวียน)
2. รัน `workflow_dispatch` mode `report`
3. ตรวจ
   - Gap ≤ 20% (ถ้าเกินในช่วงแรกที่ PR น้อย ให้บันทึกและวัดใหม่เมื่อมี PR สะสมมากกว่า 20 อัน)
   - Exchange Matrix: ทุกคู่ (ผู้รีวิว × ผู้เขียน) มีค่ามากกว่า 0 อย่างน้อย 1 ครั้งต่อเดือน
   - ไม่มี PR ค้าง label `needs-reviewer` เกิน 1 วันโดยไม่มีเหตุผล
4. เทียบกับสิ่งที่คาดไว้จากการจำลอง (unit test จำลอง 80 PR ได้ Gap ประมาณ 8%)

## รายงานผล (ให้ agent กรอก)
```
สภาพแวดล้อม: GitNonta/auto-review-system-test (private) / 2026-11-12 / Buffy (agent) บน Node v22
A: unit 11/11, integration 8/8
B: ผ่าน — B1 dry_run=true บน main, B2 PR #3 ไม่มี reviewer/คอมเมนต์/label และ log แสดง "[dry-run] request reviewers [Nontawatpkru17]"
   (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43166082276)
   B3 workflow_dispatch mode=report = success (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43166126933)
   B4 ตั้ง dry_run กลับเป็น false บน main แล้ว
C01..C05: ผ่านครบ 5/5 (Nontawatpkru17 และ Themynon2 accept invitation แล้ว)
   C01 PASS — PR #4: reviewer = Nontawatpkru17 (ไม่ใช่ผู้เขียน), คอมเมนต์บอท "บอทเลือก · Score 0", total=100
   C02 PASS — PR #5 ตอน Draft: reviewers/คอมเมนต์/label ว่างทั้งหมด (log "ข้าม (draft...)")
   C03 PASS — PR #8 (rerun หลัง Themynon2 accept): หลัง markPullRequestReadyForReview บอทเลือก Themynon2 1 คน
        คอมเมนต์ total=100, ไม่มี label, ไม่มีคอมเมนต์ซ้ำ
        (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43168541870)
        * รอบแรก PR #5 FAIL เพราะบอทเลือก Themynon2 ตอน invitation ยังค้าง → requestReviewers 422 ทั้งชุด (ดูบั๊กข้อ 2)
        * rerun หลังแก้โค้ด (กรอง collaborator + retry 422): PR #37 → reviewer Themynon2 1 คน, total=100
          (run 43176340357)
   C04 PASS — PR #6: reviewer = Nontawatpkru17, GitNonta (author) ไม่ถูกเลือก
   C05 PASS — PR #7 (rerun ด้วยการปิด/เปิด PR ใหม่): total=50 (package-lock.json 5,000 บรรทัดไม่ถูกนับ)
        reviewer = Nontawatpkru17 1 คน, label needs-reviewer ถูกลบ, คอมเมนต์เดิมถูกอัปเดตไม่สร้างใหม่ (1 -> 1)
        log: "#7: total=50 needed=1 existing=0 added=[nontawatpkru17]"
        (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43168872774)
        * รอบแรก assignment ล้มเพราะบอทเลือกคำเชิญที่ยังค้าง → 422 ทั้งชุด
C06 PASS — PR #9: 800 บรรทัด 4 ไฟล์ → reviewer 2 คน (Nontawatpkru17, Themynon2)
        แบ่งไฟล์ไม่ซ้า ครบทั้ง 4 ไฟล์ คนละ 400 บรรทัด (state.split=true, total=800)
        (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43169132136)
C07 PASS — PR #10: เปิด 100 บรรทัด → reviewer 1 คน (Nontawatpkru17) → push เพิ่มเป็น 800
        บอทเพิ่ม Themynon2 เป็นคนที่สอง, คอมเมนต์เดิมถูกอัปเดต (1 อันเท่าเดิม id ไม่เปลี่ยน) total=800 แบ่ง 334/466
        (run https://github.com/GitNonta/auto-review-system-test/actions/runs/43169304130)
        * push 3 ไฟล์ติดกันทำให้เกิด 3 run ซ้อน (concurrency cancel-in-progress=false) รอบสุดท้ายจัดการ state 800 ถูกต้อง
C08 PASS — PR #11: draft, author ขอ Nontawatpkru17 เอง แล้ว Ready → บอทไม่เพิ่มใคร
        คอมเมนต์: "@Nontawatpkru17 (เลือกเอง/รีวิวแล้ว)", state.bot=[] และ lines = 100 ของ B (run 43169710992)
C09 PASS — PR #12: 800 บรรทัด author เลือก Nontawatpkru17 เอง → บอทเพิ่ม Themynon2 อีก 1 คน
        แบ่งขอบเขต 400/400 คนละ 2 ไฟล์ (ครบ 4 ไฟล์), คอมเมนต์ระบุ B "เลือกเอง" (run 43169852007)
C10 PASS — PR #13: ภาระค้าง B = 1,596 บรรทัด สูงกว่า Themynon2 = 1,234 → บอทเลือก Themynon2 (Score 2468)
        (run 43169959265) — ยืนยันว่าบรรทัดที่ author เลือกเองถูกนับเข้า Score จริง
        * ข้อสังเกต: สะสม PR ทดสอบเยอะ ทำให้ pending_cap (800) ถูกเกินทุกคน บอทจึงขี้น ⚠️ งานค้างเกินเพดาน
C11 PASS — PR #14: B (Nontawatpkru17) ถือ PR ค้าง 500 บรรทัด, P รวม = 2,096 → Score 4,192 vs Themynon2 = 1,334 → 2,668
        เปิด PR ใหม่ 3 อัน (#15,#16,#17) → บอทเลือก Themynon2 ทั้ง 3 ครั้ง (Score 2,668 → 2,868 → 3,068) ไม่เลือก B เลย
        ยืนยันว่างานค้างถูกถ่วง ×2 จริง (run 43170413379, 43170591464, 43170639444, 43170679671)
C12 PASS — Nontawatpkru17 กด APPROVED บน PR #14 เมื่อ 07:39:11 → 500 บรรทัดย้ายจาก P ไป C
        คำนวณจาก API หลังรีวิว: Nontawat C=1,550 / P=2,446 (C+P = 3,996 คงที่, PR #14 นับเป็น C 500 ไม่ใช่ P)
        รัน workflow_dispatch mode=report = success (run 43177096555) — ไม่นับซ้ำ
        * ก่อนหน้านี้จำลองแทนไม่ได้เพราะ token เป็นของ GitNonta (author) และโค้ดตัด review ของ author ออก
C13 PASS — PR #18: commit `unavailable: [{login: Nontawatpkru17, until: 2026-12-31}]` บน main → บอทเลือก Themynon2 ไม่เลือก B
        (run 43170797190) — คืน config กลับเป็น `unavailable: []` แล้วและยืนยันเนื้อหาไฟล์
C14 PASS — PR #19: unavailable มี Nontawatpkru17 แต่ until = 2026-11-11 (เมื่อวาน) และ Themynon2 ลาถึง 2099-12-31
        → บอทเลือก Nontawatpkru17 (ถ้า until ยังไม่หมดอายุ จะไม่มีใครถูกเลือกเลย) (run 43171186403)
C15 PASS — PR #20: unavailable ทั้ง Nontawatpkru17 และ Themynon2 → ไม่มี reviewer, ไม่ assign author,
        label needs-reviewer, คอมเมนต์ "ยังไม่มี Reviewer ... ไม่พร้อม/ลา: ..." (run 43171274922)
C16 PASS — เอา unavailable ออกแล้ว dispatch mode=sweep → PR #20 ได้ Themynon2, label needs-reviewer ถูกลบ, needs=null
        (sweep จัด PR ค้างอื่นต่อด้วย: #21, #5, #3, #1) (run 43171630706)
C17 PASS — PR #22: ให้ Nontawatpkru17 ลา เหลือ Themynon2 คนเดียว + PR 800 บรรทัด → assign 1 คน
        label needs-second-reviewer, คอมเมนต์ "ยังขาด Reviewer คนที่สอง" (run 43171760790)
        * บทเรียนระหว่างทดสอบ: สคริปต์ของ agent แทน YAML ผิด (เหลือบรรทัด - login ค้าง) ทำให้ run #32 ล้มด้วย YAMLException
          — ไม่ใช่บั๊กของ auto-review.js แต่ต้อง validate YAML ทุกครั้งหลังแก้ config (แก้แล้วผ่าน)
C18 PASS — PR #23: ตั้ง pending_cap = 50 → ทุกคนเกินเพดาน บอทเลือก Nontawatpkru17 (Score 4642 = ต่ำสุด)
        และคอมเมนต์ ⚠️ "ทุกคนงานค้างเกินเพดาน" (run 43172262069) — คืน pending_cap = 800 แล้ว
C19 PASS — PR #28: pending_weight = 0 ทำให้ Score = C: Nontawat 550 vs Themynon2 400 (ห่าง 150 < 300) → อยู่กลุ่ม tie
        บอทเลือก Nontawatpkru17 ซึ่งเป็นคนที่รีวิว author นานสุด (06:56:16 เทียบกับ Themynon2 06:57:46)
        (run 43173473532) — คืน pending_weight = 2 แล้ว
        * ทั้งสองคนส่ง APPROVED บน PR #4/#7/#9 เองในช่วง 06:53–06:57 จึงมีประวัติให้ทดสอบ
C20 PASS — รอบแรก PR #29/#30/#31 (Score = 0 ทุกคน): Themynon2, Nontawatpkru17, Nontawatpkru17
        → ไม่สลับ 1:1 เพราะ RR ใช้ index ทั้ง team (3 คน) ทั้งที่ author ถูกตัดออก
        → แก้ RR ให้หมุนตาม candidate pool แล้ว rerun: PR #34/#35/#36 = Nontawatpkru17, Themynon2, Nontawatpkru17
        = สลับ 1:1 จริง (run #53–#55) — คืน config เป็น pending_weight 2 / history_days 30 แล้ว
C21 PASS — PR #27: ติด label no-auto-review ก่อน Ready → ไม่มี reviewer, ไม่มีคอมเมนต์บอท, label คงอยู่
        (run 43172735763)
C22 PASS (proxy) — PR #32: ตัด GitNonta ออกจาก team เพื่อจำลองผู้เขียนนอกทีม (มีบัญชีเดียว)
        log "#32: ข้าม (ผู้เขียนไม่อยู่ในทีม)" — ไม่มี reviewer/คอมเมนต์/label (run 43174431619), คืน team 3 คนแล้ว
C23 PASS — PR #28: reopen → run 47 และ draft→ready → run 48 ต่างก็ได้ `existing=1 added=[]`
        reviewer Nontawatpkru17 อยู่, คอมเมนต์ id 6817261104 ยังอันเดียว ไม่มีซ้ำ
C24 PASS — PR #23: ตั้ง overdue_hours = 0.02 → sweep run 49 เตือน 1 ครั้ง
        "⏰ @Nontawatpkru17 ยังไม่ได้รีวิว PR นี้ ... แนะนำให้ผู้เขียนขอ @Themynon2", reviewer ไม่เปลี่ยน, reminded = true
        sweep run 50 ไม่เตือนซ้ำ (คืน overdue_hours = 48 แล้ว)
        * sweep เก็บ PR ค้างอื่นด้วย: #32 (assign), #27 (ข้ามเพราะ label), #22 (เติมคนที่สอง)
C25 PASS — PR #33: GitNonta โพสต์ marker ปลอม (total 9999, bot ["themynon2"]) ก่อน Ready → บอทไม่เชื่อ
        สร้างคอมเมนต์ของตัวเอง total=100, lines {themynon2:100}, assign Themynon2 (run 43175217898)

สรุปหมวด C: ผ่าน 21/21
D: ยังไม่เริ่ม (ต้องมี C ผ่านก่อน)
ปัญหาที่พบ / ข้อเสนอแก้:
1) dry_run ต้องอยู่บน default branch จริง — commit แรกหลุดไปอยู่ branch ทดสอบ ทำให้ run #2 คอมเมนต์จริง (แก้แล้ว, ถูกต้องเมื่อ dry_run อยู่บน main)
2) **บั๊ก (แก้แล้ว)**: team มีคนที่ยังไม่ใช่ collaborator (invite ค้าง) → chooseReviewer เลือกคนนั้นเพราะ Score 0
   แล้ว requestReviewers ล้มทั้งชุด แม้มี candidate ที่ขอได้จริง
   แก้โดย: กรอง team ด้วย collaborators API + retry requestReviewers ทีละคนเมื่อเจอ 422 + แก้ RR ให้หมุนตาม candidate pool
   ผลหลังแก้: unit 11/11, integration 8/8, rerun C03 (PR #37) และ C20 (PR #34–#36) ผ่าน
3) Node 20 deprecation warning ของ actions/* (2026): ยังใช้ได้ แต่ควรอัปเกรดเมื่อ actions รองรับ
4) Summary ของ B3 ตรวจจาก API ไม่ได้ (Job Summary ไม่มี endpoint) — ตรวจได้แค่ว่า run success และตารางถูกสร้างจากฟังก์ชัน report() ที่ unit-test ผ่าน
```

## ปัญหาที่พบบ่อย
| อาการ | สาเหตุที่น่าจะเป็น | วิธีแก้ |
|---|---|---|
| Workflow ไม่รันกับ PR | ไฟล์ยังไม่อยู่บน default branch | merge เข้า default ก่อน |
| `Resource not accessible by integration` | Workflow permissions เป็น read-only | ตั้ง Read and write |
| `requestReviewers` 422 | username ไม่ใช่ collaborator หรือพิมพ์ผิด | ตรวจ `team` และสิทธิ์ใน repo |
| บอทไม่เลือกคนที่ Score ต่ำสุด | อยู่ในกลุ่ม `tie_threshold` จึงเลือกตามประวัติการแลกเปลี่ยน | ตั้งใจแล้ว ลด `tie_threshold` ถ้าต้องการเข้มขึ้น |
| คอมเมนต์ของบอทซ้ำ | บัญชีที่คอมเมนต์ไม่ใช่ `github-actions[bot]` (เช่นใช้ PAT) | ใช้ `GITHUB_TOKEN` ปกติ หรือปรับ `BOT_LOGIN` |
