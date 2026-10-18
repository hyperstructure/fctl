//! Sandbox sizes: what a session asks for, what a host gives it, and amounts
//! of memory the way people write and read them.

use fctl_proto::runner::{SandboxLimits, SandboxSize};
use fctl_proto::session::ProjectSettings;

/// The CPUs and memory a session asks for. `None` takes the host's default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Want {
    pub cpus: Option<u32>,
    pub memory_mb: Option<u32>,
}

impl Want {
    /// What a session of this project asks for.
    pub fn of(settings: &ProjectSettings) -> Self {
        Self {
            cpus: settings.cpus,
            memory_mb: settings.memory_mb,
        }
    }

    /// The size a host with these limits gives it.
    pub fn on(self, limits: &SandboxLimits) -> SandboxSize {
        SandboxSize {
            cpus: self.cpus.unwrap_or(limits.default.cpus),
            memory_mb: self.memory_mb.unwrap_or(limits.default.memory_mb),
        }
    }

    /// Whether a host with these limits can give it.
    pub fn fits(self, limits: &SandboxLimits) -> bool {
        let size = self.on(limits);
        size.cpus <= limits.max.cpus && size.memory_mb <= limits.max.memory_mb
    }
}

/// `1 CPU`, `4 CPUs`.
pub fn cpus(n: u32) -> String {
    if n == 1 {
        "1 CPU".into()
    } else {
        format!("{n} CPUs")
    }
}

/// `512 MiB`, `8 GiB`, `15.6 GiB`: GiB to a tenth from 1 GiB up.
pub fn memory(mb: u64) -> String {
    if mb < 1024 {
        return format!("{mb} MiB");
    }
    let tenths = (mb.saturating_mul(10) + 512) / 1024;
    match tenths % 10 {
        0 => format!("{} GiB", tenths / 10),
        t => format!("{}.{t} GiB", tenths / 10),
    }
}

/// `4 CPUs and 8 GiB by default, at most 10 CPUs and 15.6 GiB`.
pub fn limits(l: &SandboxLimits) -> String {
    let mem = |mb: u32| match mb {
        u32::MAX => "unlimited memory".to_owned(),
        mb => memory(mb.into()),
    };
    format!(
        "{} and {} by default, at most {} and {}",
        cpus(l.default.cpus),
        mem(l.default.memory_mb),
        cpus(l.max.cpus),
        mem(l.max.memory_mb)
    )
}

/// Parses an amount of memory into MiB: `8g`, `8 GiB`, `1.5g`, `512m`, or a
/// plain number of MiB (`8192`). Units are binary, as in Docker: `g` is GiB.
pub fn parse_memory_mb(input: &str) -> Result<u32, String> {
    let bad = || format!("`{input}` is not an amount of memory (try 8g, 512m or 8192)");
    let s = input.trim().to_ascii_lowercase();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (number, unit) = s.split_at(end);
    let n: f64 = number.parse().map_err(|_| bad())?;
    let mib = match unit.trim() {
        "" | "m" | "mb" | "mib" => n,
        "g" | "gb" | "gib" => n * 1024.0,
        "t" | "tb" | "tib" => n * 1024.0 * 1024.0,
        _ => return Err(bad()),
    };
    if !(1.0..=f64::from(u32::MAX)).contains(&mib.round()) {
        return Err(bad());
    }
    Ok(mib.round() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(cpus: u32, memory_mb: u32, max_cpus: u32, max_memory_mb: u32) -> SandboxLimits {
        SandboxLimits {
            default: SandboxSize { cpus, memory_mb },
            max: SandboxSize {
                cpus: max_cpus,
                memory_mb: max_memory_mb,
            },
        }
    }

    #[test]
    fn a_session_gets_its_size_or_the_hosts_default() {
        let l = limits(4, 8192, 10, 15_990);
        let want = Want {
            cpus: None,
            memory_mb: Some(12_288),
        };
        assert_eq!(
            want.on(&l),
            SandboxSize {
                cpus: 4,
                memory_mb: 12_288
            }
        );
        assert!(want.fits(&l));
        assert!(Want::default().fits(&l));
        let big = Want {
            cpus: None,
            memory_mb: Some(16_384),
        };
        assert!(!big.fits(&l));
        let many = Want {
            cpus: Some(12),
            memory_mb: None,
        };
        assert!(!many.fits(&l));
    }

    #[test]
    fn formats_amounts() {
        assert_eq!(cpus(1), "1 CPU");
        assert_eq!(cpus(4), "4 CPUs");
        assert_eq!(memory(512), "512 MiB");
        assert_eq!(memory(1024), "1 GiB");
        assert_eq!(memory(1536), "1.5 GiB");
        assert_eq!(memory(8192), "8 GiB");
        assert_eq!(memory(15_990), "15.6 GiB");
        assert_eq!(memory(32_768), "32 GiB");
        assert_eq!(
            super::limits(&limits(4, 8192, 10, 15_990)),
            "4 CPUs and 8 GiB by default, at most 10 CPUs and 15.6 GiB"
        );
    }

    #[test]
    fn parses_amounts() {
        for (input, mb) in [
            ("8g", 8192),
            ("8G", 8192),
            ("8 GiB", 8192),
            ("8gb", 8192),
            ("1.5g", 1536),
            ("512m", 512),
            ("512MiB", 512),
            ("8192", 8192),
            (" 16384 ", 16_384),
            ("1t", 1_048_576),
        ] {
            assert_eq!(parse_memory_mb(input), Ok(mb), "{input}");
        }
        for bad in [
            "",
            "g",
            "8x",
            "eight",
            "-1g",
            "0",
            "0.0001m",
            "1.2.3g",
            "99999999t",
        ] {
            assert!(parse_memory_mb(bad).is_err(), "{bad}");
        }
    }
}
