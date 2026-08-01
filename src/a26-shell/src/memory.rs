use std::fs;
use std::io;

const MIN_AVAILABLE_BYTES: u64 = 384 * 1024 * 1024;
const MIN_AVAILABLE_FRACTION_DENOMINATOR: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl MemorySnapshot {
    pub fn read() -> io::Result<Self> {
        Self::parse(&fs::read_to_string("/proc/meminfo")?)
    }

    fn parse(value: &str) -> io::Result<Self> {
        let mut total_kib = None;
        let mut available_kib = None;
        for line in value.lines() {
            let mut fields = line.split_whitespace();
            match fields.next() {
                Some("MemTotal:") => total_kib = fields.next().and_then(|v| v.parse().ok()),
                Some("MemAvailable:") => available_kib = fields.next().and_then(|v| v.parse().ok()),
                _ => {}
            }
        }
        let total_kib: u64 = total_kib
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MemTotal is missing"))?;
        let available_kib: u64 = available_kib
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MemAvailable is missing"))?;
        Ok(Self {
            total_bytes: total_kib.saturating_mul(1024),
            available_bytes: available_kib.saturating_mul(1024),
        })
    }

    pub fn under_pressure(self) -> bool {
        let threshold = MIN_AVAILABLE_BYTES.max(
            self.total_bytes
                .saturating_div(MIN_AVAILABLE_FRACTION_DENOMINATOR),
        );
        self.available_bytes < threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kernel_units_and_applies_the_bounded_threshold() {
        let healthy =
            MemorySnapshot::parse("MemTotal:        8000000 kB\nMemAvailable:    2000000 kB\n")
                .unwrap();
        assert_eq!(healthy.total_bytes, 8_192_000_000);
        assert!(!healthy.under_pressure());

        let pressured =
            MemorySnapshot::parse("MemTotal:        8000000 kB\nMemAvailable:     700000 kB\n")
                .unwrap();
        assert!(pressured.under_pressure());
        assert!(MemorySnapshot::parse("MemTotal: 1 kB\n").is_err());
    }
}
