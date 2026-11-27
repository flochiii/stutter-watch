#[derive(Debug)]
pub struct Estimate {
    pub bytes_per_hour: Option<f64>,
}
pub fn estimate(written_bytes: u64, observed_seconds: f64) -> Estimate {
    let rate = (observed_seconds >= 10.0 && written_bytes > 0)
        .then(|| written_bytes as f64 / observed_seconds * 3600.0);
    Estimate {
        bytes_per_hour: rate,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_measurements_and_scales_bytes_per_hour() {
        assert!(estimate(0, 120.0).bytes_per_hour.is_none());
        assert!(estimate(1_000_000, 0.0).bytes_per_hour.is_none());
        assert_eq!(
            estimate(50_000_000, 1800.0).bytes_per_hour,
            Some(100_000_000.0)
        );
    }
}
