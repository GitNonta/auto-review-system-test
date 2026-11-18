//! ระยะรอก่อนลองเรียก GitHub API ใหม่ (exponential backoff + เคารพ Retry-After)
//!
//! ไม่สุ่ม jitter เพื่อให้ทดสอบได้แน่นอน: ใช้ "decorrelation" จากเลขรอบแทน
#![cfg_attr(not(test), allow(dead_code))]

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    pub base: Duration,
    pub max: Duration,
    pub max_attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff { base: Duration::from_millis(500), max: Duration::from_secs(60), max_attempts: 6 }
    }
}

impl Backoff {
    /// ระยะรอก่อนครั้งที่ `attempt` (เริ่ม 1) หรือ `None` เมื่อครบจำนวนครั้งแล้ว
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }
        let factor = 1u32.checked_shl(attempt - 1).unwrap_or(u32::MAX);
        let raw = self.base.saturating_mul(factor);
        // กระจายจังหวะเล็กน้อยแบบกำหนดได้: +0%, +10%, +20% วนตามเลขรอบ
        let spread = raw.saturating_mul(attempt % 3) / 10;
        Some(raw.saturating_add(spread).min(self.max))
    }

    /// ใช้ค่า `Retry-After` (วินาที) จาก response ถ้ามี โดยไม่เกิน `max`
    pub fn delay_with_hint(&self, attempt: u32, retry_after: Option<&str>) -> Option<Duration> {
        let fallback = self.delay(attempt)?;
        let hinted = retry_after.and_then(|s| s.trim().parse::<u64>().ok()).map(Duration::from_secs);
        Some(hinted.map(|d| d.min(self.max)).unwrap_or(fallback))
    }

    /// เวลารอรวมทั้งหมดถ้าล้มทุกครั้ง (ใช้ตั้ง timeout ของงานทั้งงาน)
    pub fn total_budget(&self) -> Duration {
        (1..=self.max_attempts).filter_map(|a| self.delay(a)).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn grows_exponentially_with_deterministic_spread() {
        let b = Backoff::default();
        assert_eq!(b.delay(1), Some(ms(550)));
        assert_eq!(b.delay(2), Some(ms(1200)));
        assert_eq!(b.delay(3), Some(ms(2000)));
        assert_eq!(b.delay(4), Some(ms(4400)));
    }

    #[test]
    fn caps_and_stops() {
        let b = Backoff { base: Duration::from_secs(10), max: Duration::from_secs(30), max_attempts: 40 };
        assert_eq!(b.delay(3), Some(Duration::from_secs(30)));
        assert_eq!(b.delay(40), Some(Duration::from_secs(30)), "shift ใหญ่ต้องไม่ overflow");
        assert_eq!(b.delay(0), None);
        assert_eq!(b.delay(41), None);
    }

    #[test]
    fn retry_after_hint_wins_but_is_capped() {
        let b = Backoff::default();
        assert_eq!(b.delay_with_hint(1, Some(" 7 ")), Some(Duration::from_secs(7)));
        assert_eq!(b.delay_with_hint(1, Some("3600")), Some(Duration::from_secs(60)));
        assert_eq!(b.delay_with_hint(2, Some("soon")), Some(ms(1200)));
        assert_eq!(b.delay_with_hint(9, Some("1")), None);
    }

    #[test]
    fn total_budget_sums_every_attempt() {
        let b = Backoff { base: ms(100), max: Duration::from_secs(1), max_attempts: 3 };
        assert_eq!(b.total_budget(), ms(110 + 240 + 400));
    }
}
