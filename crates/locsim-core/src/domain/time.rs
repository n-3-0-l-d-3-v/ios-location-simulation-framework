/// Nanoseconds since the Unix epoch.
///
/// Integer time keeps tick arithmetic (`start + n × interval`) exact and
/// makes seeded runs bit-reproducible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

const NANOS_PER_SEC: f64 = 1e9;

impl Timestamp {
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    pub const fn as_nanos(&self) -> i64 {
        self.0
    }

    /// `None` if `secs` is non-finite or does not fit in the representable range.
    pub fn from_secs_f64(secs: f64) -> Option<Self> {
        let nanos = (secs * NANOS_PER_SEC).round();
        if !nanos.is_finite() || nanos.abs() >= i64::MAX as f64 {
            return None;
        }
        Some(Self(nanos as i64))
    }

    pub fn as_secs_f64(&self) -> f64 {
        self.0 as f64 / NANOS_PER_SEC
    }

    pub fn checked_add_nanos(&self, nanos: i64) -> Option<Self> {
        self.0.checked_add(nanos).map(Self)
    }

    /// Signed seconds elapsed from `earlier` to `self`.
    pub fn seconds_since(&self, earlier: Timestamp) -> f64 {
        // Subtract in i128 so extreme values cannot overflow.
        (self.0 as i128 - earlier.0 as i128) as f64 / NANOS_PER_SEC
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        assert_eq!(
            Timestamp::from_secs_f64(1.25).unwrap().as_nanos(),
            1_250_000_000
        );
        // An f64 holding present-day epoch seconds resolves only ~240 ns, so
        // float conversion is for I/O boundaries; tick arithmetic stays in i64.
        let t = Timestamp::from_secs_f64(1_700_000_000.25).unwrap();
        assert!((t.as_nanos() - 1_700_000_000_250_000_000).abs() < 512);
        assert!((t.as_secs_f64() - 1_700_000_000.25).abs() < 1e-6);
    }

    #[test]
    fn rejects_unrepresentable() {
        assert!(Timestamp::from_secs_f64(f64::NAN).is_none());
        assert!(Timestamp::from_secs_f64(f64::INFINITY).is_none());
        assert!(Timestamp::from_secs_f64(1e300).is_none());
        assert!(Timestamp::from_nanos(i64::MAX)
            .checked_add_nanos(1)
            .is_none());
    }

    #[test]
    fn ordering_and_differences() {
        let a = Timestamp::from_nanos(1_000_000_000);
        let b = a.checked_add_nanos(500_000_000).unwrap();
        assert!(b > a);
        assert_eq!(b.seconds_since(a), 0.5);
        assert_eq!(a.seconds_since(b), -0.5);
        let extreme =
            Timestamp::from_nanos(i64::MAX).seconds_since(Timestamp::from_nanos(i64::MIN));
        assert!(extreme.is_finite() && extreme > 0.0);
    }
}
