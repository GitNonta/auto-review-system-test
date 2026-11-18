//! สถิติขนาด PR แบบ nearest-rank (ใช้แสดง p50/p90 ของจำนวนบรรทัดบนแดชบอร์ด)
#![cfg_attr(not(test), allow(dead_code))]

/// percentile แบบ nearest-rank: `p` อยู่ในช่วง 0..=100, คืน `None` เมื่อไม่มีข้อมูล
pub fn percentile(values: &[u32], p: u8) -> Option<u32> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let p = u32::from(p.min(100));
    // rank = ceil(p/100 * n) โดยอย่างน้อยเป็น 1
    let rank = ((p * sorted.len() as u32).div_ceil(100)).max(1) as usize;
    Some(sorted[rank - 1])
}

/// มัธยฐานแบบเฉลี่ยสองค่ากลางเมื่อจำนวนเป็นคู่ (ปัดลง)
pub fn median(values: &[u32]) -> Option<u32> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some(((u64::from(sorted[n / 2 - 1]) + u64::from(sorted[n / 2])) / 2) as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_matches_textbook_example() {
        let v = [15, 20, 35, 40, 50];
        assert_eq!(percentile(&v, 5), Some(15));
        assert_eq!(percentile(&v, 30), Some(20));
        assert_eq!(percentile(&v, 40), Some(20));
        assert_eq!(percentile(&v, 50), Some(35));
        assert_eq!(percentile(&v, 100), Some(50));
    }

    #[test]
    fn percentile_edges() {
        assert_eq!(percentile(&[], 50), None);
        assert_eq!(percentile(&[7], 0), Some(7));
        assert_eq!(percentile(&[3, 1, 2], 255), Some(3), "p เกิน 100 ถูกบีบเป็น 100");
    }

    #[test]
    fn median_odd_even_and_overflow() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[9, 1, 5]), Some(5));
        assert_eq!(median(&[4, 1, 3, 2]), Some(2));
        assert_eq!(median(&[u32::MAX, u32::MAX]), Some(u32::MAX));
    }
}
