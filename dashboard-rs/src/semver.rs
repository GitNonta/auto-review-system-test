//! อ่านและเปรียบเทียบเวอร์ชันแบบ SemVer 2.0 (ใช้เทียบเวอร์ชันของ dashboard กับ release ล่าสุด)
//!
//! รองรับ `MAJOR.MINOR.PATCH`, prerelease (`-rc.1`) และ build metadata (`+sha.abc`)
//! ลำดับความสำคัญตาม spec §11: build metadata ไม่มีผลต่อการเรียง
#![cfg_attr(not(test), allow(dead_code))]

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ident {
    Num(u64),
    Alpha(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Vec<Ident>,
    pub build: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SemverError {
    Empty,
    BadCore(String),
    LeadingZero(String),
    BadIdent(String),
}

impl fmt::Display for SemverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SemverError::Empty => write!(f, "เวอร์ชันว่าง"),
            SemverError::BadCore(s) => write!(f, "ต้องเป็น MAJOR.MINOR.PATCH: `{s}`"),
            SemverError::LeadingZero(s) => write!(f, "ตัวเลขห้ามขึ้นต้นด้วย 0: `{s}`"),
            SemverError::BadIdent(s) => write!(f, "identifier ไม่ถูกต้อง: `{s}`"),
        }
    }
}

fn num(part: &str) -> Result<u64, SemverError> {
    if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SemverError::BadCore(part.to_string()));
    }
    if part.len() > 1 && part.starts_with('0') {
        return Err(SemverError::LeadingZero(part.to_string()));
    }
    part.parse().map_err(|_| SemverError::BadCore(part.to_string()))
}

fn ident_ok(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

impl Version {
    /// รับ `v1.2.3` ได้ด้วย (แท็กของ git มักมี `v` นำหน้า)
    pub fn parse(input: &str) -> Result<Version, SemverError> {
        let s = input.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        if s.is_empty() {
            return Err(SemverError::Empty);
        }
        let (rest, build) = match s.split_once('+') {
            Some((r, b)) => (r, b.split('.').map(str::to_string).collect::<Vec<_>>()),
            None => (s, Vec::new()),
        };
        if build.iter().any(|b| !ident_ok(b)) {
            return Err(SemverError::BadIdent(input.to_string()));
        }
        let (core, pre_raw) = match rest.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (rest, None),
        };
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() != 3 {
            return Err(SemverError::BadCore(core.to_string()));
        }
        let mut pre = Vec::new();
        for id in pre_raw.map(|p| p.split('.').collect::<Vec<_>>()).unwrap_or_default() {
            if !ident_ok(id) {
                return Err(SemverError::BadIdent(input.to_string()));
            }
            if id.bytes().all(|b| b.is_ascii_digit()) {
                pre.push(Ident::Num(num(id)?));
            } else {
                pre.push(Ident::Alpha(id.to_string()));
            }
        }
        Ok(Version { major: num(parts[0])?, minor: num(parts[1])?, patch: num(parts[2])?, pre, build })
    }

    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let core = (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch));
        if core != Ordering::Equal {
            return core;
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a, b) {
                        (Ident::Num(x), Ident::Num(y)) => x.cmp(y),
                        (Ident::Num(_), Ident::Alpha(_)) => Ordering::Less,
                        (Ident::Alpha(_), Ident::Num(_)) => Ordering::Greater,
                        (Ident::Alpha(x), Ident::Alpha(y)) => x.cmp(y),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_core_pre_and_build() {
        let x = v("v1.20.3-rc.1+sha.5114f85");
        assert_eq!((x.major, x.minor, x.patch), (1, 20, 3));
        assert_eq!(x.pre, vec![Ident::Alpha("rc".into()), Ident::Num(1)]);
        assert_eq!(x.build, vec!["sha", "5114f85"]);
        assert!(x.is_prerelease());
    }

    #[test]
    fn spec_precedence_chain() {
        let chain = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta", "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0"];
        for w in chain.windows(2) {
            assert!(v(w[0]) < v(w[1]), "{} < {}", w[0], w[1]);
        }
        assert!(v("2.0.0") > v("1.99.99"));
        assert_eq!(v("1.0.0+a").cmp(&v("1.0.0+b")), Ordering::Equal);
    }

    #[test]
    fn rejects_invalid() {
        assert_eq!(Version::parse(" "), Err(SemverError::Empty));
        assert!(matches!(Version::parse("1.2"), Err(SemverError::BadCore(_))));
        assert!(matches!(Version::parse("01.2.3"), Err(SemverError::LeadingZero(_))));
        assert!(matches!(Version::parse("1.2.3-rc..1"), Err(SemverError::BadIdent(_))));
        assert!(matches!(Version::parse("1.2.3-01"), Err(SemverError::LeadingZero(_))));
        assert!(SemverError::BadCore("x".into()).to_string().contains("MAJOR"));
    }
}
