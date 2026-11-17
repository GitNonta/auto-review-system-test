//! สถิติพื้นฐานสำหรับภาระงานรีวิว (บรรทัดต่อคน)
//!
//! ใช้ประเมินว่าระบบ Equal Review Workload กระจายงานเท่ากันแค่ไหน:
//! ค่าเฉลี่ย มัธยฐาน เปอร์เซ็นไทล์ ส่วนเบี่ยงเบนมาตรฐาน Gini และ Gap
//! ทุกฟังก์ชันรับ slice ของ `f64` และไม่แก้ข้อมูลต้นฉบับ

/// สรุปสถิติของชุดข้อมูลหนึ่งชุด
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub p90: f64,
    pub stddev: f64,
    pub gini: f64,
    pub gap: f64,
}

/// ค่าเฉลี่ยเลขคณิต (ว่าง → None)
pub fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

fn sorted(values: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite values are comparable"));
    v
}

/// เปอร์เซ็นไทล์แบบ linear interpolation (เหมือน numpy `linear`), `p` อยู่ในช่วง 0..=100
pub fn percentile(values: &[f64], p: f64) -> Option<f64> {
    let v = sorted(values);
    if v.is_empty() || !(0.0..=100.0).contains(&p) {
        return None;
    }
    if v.len() == 1 {
        return Some(v[0]);
    }
    let rank = p / 100.0 * (v.len() - 1) as f64;
    let lo = rank.floor() as usize;
    let hi = rank.ceil() as usize;
    let frac = rank - lo as f64;
    Some(v[lo] + (v[hi] - v[lo]) * frac)
}

/// มัธยฐาน = เปอร์เซ็นไทล์ที่ 50
pub fn median(values: &[f64]) -> Option<f64> {
    percentile(values, 50.0)
}

/// ส่วนเบี่ยงเบนมาตรฐานของประชากร (หารด้วย n)
pub fn stddev(values: &[f64]) -> Option<f64> {
    let m = mean(values)?;
    let var = values.iter().map(|x| (x - m).powi(2)).sum::<f64>() / values.len() as f64;
    Some(var.sqrt())
}

/// สัมประสิทธิ์ Gini (0 = เท่ากันทุกคน, เข้าใกล้ 1 = งานกองอยู่คนเดียว)
///
/// ค่าติดลบถูกตัดเป็น 0 เพราะภาระงานติดลบไม่มีความหมาย
pub fn gini(values: &[f64]) -> Option<f64> {
    let v: Vec<f64> = sorted(values).into_iter().map(|x| x.max(0.0)).collect();
    if v.is_empty() {
        return None;
    }
    let total: f64 = v.iter().sum();
    if total == 0.0 {
        return Some(0.0);
    }
    let n = v.len() as f64;
    let weighted: f64 = v.iter().enumerate().map(|(i, x)| (i as f64 + 1.0) * x).sum();
    Some((2.0 * weighted) / (n * total) - (n + 1.0) / n)
}

/// Gap = (max - min) / mean ใช้เทียบกับเกณฑ์ "Gap ≤ 20%" ในเอกสารทดสอบ
pub fn gap_ratio(values: &[f64]) -> Option<f64> {
    let v = sorted(values);
    let m = mean(&v)?;
    if m == 0.0 {
        return Some(0.0);
    }
    Some((v[v.len() - 1] - v[0]) / m)
}

/// คำนวณทุกค่าในครั้งเดียว
pub fn summarize(values: &[f64]) -> Option<Summary> {
    let v = sorted(values);
    if v.is_empty() {
        return None;
    }
    Some(Summary {
        count: v.len(),
        min: v[0],
        max: v[v.len() - 1],
        mean: mean(&v)?,
        median: median(&v)?,
        p90: percentile(&v, 90.0)?,
        stddev: stddev(&v)?,
        gini: gini(&v)?,
        gap: gap_ratio(&v)?,
    })
}

/// จัดค่าเข้าถังความกว้าง `width` เริ่มที่ 0 คืน (ขอบล่าง, จำนวน) เรียงจากน้อยไปมาก
/// ถังว่างตรงกลางถูกใส่ไว้ด้วยเพื่อให้วาดกราฟต่อเนื่อง
pub fn histogram(values: &[f64], width: f64) -> Vec<(f64, usize)> {
    let v: Vec<f64> = sorted(values).into_iter().filter(|x| *x >= 0.0).collect();
    if v.is_empty() || width <= 0.0 {
        return Vec::new();
    }
    let buckets = (v[v.len() - 1] / width).floor() as usize + 1;
    let mut counts = vec![0usize; buckets];
    for x in v {
        counts[(x / width).floor() as usize] += 1;
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, c)| (i as f64 * width, c))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn mean_and_median() {
        assert_eq!(mean(&[]), None);
        assert!(close(mean(&[1.0, 2.0, 3.0, 4.0]).unwrap(), 2.5));
        assert!(close(median(&[5.0, 1.0, 3.0]).unwrap(), 3.0));
        assert!(close(median(&[4.0, 1.0, 3.0, 2.0]).unwrap(), 2.5));
    }

    #[test]
    fn percentile_interpolates_and_validates() {
        let v = [10.0, 20.0, 30.0, 40.0, 50.0];
        assert!(close(percentile(&v, 0.0).unwrap(), 10.0));
        assert!(close(percentile(&v, 100.0).unwrap(), 50.0));
        assert!(close(percentile(&v, 90.0).unwrap(), 46.0));
        assert!(close(percentile(&[7.0], 33.0).unwrap(), 7.0));
        assert_eq!(percentile(&v, 101.0), None);
        assert_eq!(percentile(&[f64::NAN], 50.0), None);
    }

    #[test]
    fn stddev_population() {
        assert!(close(stddev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap(), 2.0));
        assert!(close(stddev(&[3.0, 3.0]).unwrap(), 0.0));
    }

    #[test]
    fn gini_extremes() {
        assert!(close(gini(&[100.0, 100.0, 100.0]).unwrap(), 0.0));
        // คนเดียวทำหมด จาก 4 คน → (n-1)/n
        assert!(close(gini(&[0.0, 0.0, 0.0, 400.0]).unwrap(), 0.75));
        assert!(close(gini(&[0.0, 0.0]).unwrap(), 0.0));
        assert!(close(gini(&[-5.0, 10.0]).unwrap(), 0.5));
        assert_eq!(gini(&[]), None);
    }

    #[test]
    fn gap_ratio_matches_doc_rule() {
        let g = gap_ratio(&[900.0, 1000.0, 1100.0]).unwrap();
        assert!(close(g, 0.2));
        assert!(close(gap_ratio(&[0.0, 0.0]).unwrap(), 0.0));
    }

    #[test]
    fn summarize_all() {
        let s = summarize(&[300.0, 100.0, 200.0]).unwrap();
        assert_eq!(s.count, 3);
        assert!(close(s.min, 100.0) && close(s.max, 300.0));
        assert!(close(s.mean, 200.0) && close(s.median, 200.0));
        assert!(close(s.p90, 280.0));
        assert!(s.gini > 0.0 && s.gini < 1.0);
        assert!(close(s.gap, 1.0));
        assert_eq!(summarize(&[]), None);
    }

    #[test]
    fn histogram_fills_empty_buckets() {
        let h = histogram(&[0.0, 50.0, 99.0, 250.0], 100.0);
        assert_eq!(h, vec![(0.0, 3), (100.0, 0), (200.0, 1)]);
        assert!(histogram(&[1.0], 0.0).is_empty());
        assert!(histogram(&[], 10.0).is_empty());
    }
}
