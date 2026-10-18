//! Machine facts for the runner's hello and heartbeats.

use std::path::Path;
use std::process::Command;

pub fn os() -> &'static str {
    std::env::consts::OS
}

pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

pub fn cpus() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn sysctl(name: &str) -> Option<String> {
    let o = Command::new("sysctl").args(["-n", name]).output().ok()?;
    o.status
        .success()
        .then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

pub fn memory_mb() -> u64 {
    if cfg!(target_os = "macos") {
        return sysctl("hw.memsize")
            .and_then(|s| s.parse::<u64>().ok())
            .map_or(0, |b| b / 1024 / 1024);
    }
    meminfo("MemTotal").unwrap_or(0)
}

pub fn memory_available_mb() -> Option<u64> {
    if cfg!(target_os = "macos") {
        // Free + inactive pages, as vm_stat reports them.
        let o = Command::new("vm_stat").output().ok()?;
        let text = String::from_utf8_lossy(&o.stdout);
        let page = text
            .lines()
            .next()
            .and_then(|l| l.split("page size of ").nth(1))
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(16384);
        let pages = |key: &str| -> u64 {
            text.lines()
                .find(|l| l.starts_with(key))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().trim_end_matches('.').parse().ok())
                .unwrap_or(0)
        };
        return Some((pages("Pages free") + pages("Pages inactive")) * page / 1024 / 1024);
    }
    meminfo("MemAvailable")
}

fn meminfo(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    text.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb / 1024)
}

pub fn load1() -> Option<f32> {
    if cfg!(target_os = "macos") {
        // "{ 1.23 1.10 0.98 }"
        return sysctl("vm.loadavg")?
            .trim_matches(|c| c == '{' || c == '}' || c == ' ')
            .split_whitespace()
            .next()?
            .parse()
            .ok();
    }
    std::fs::read_to_string("/proc/loadavg")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Free MiB on the filesystem that holds `path`, as `df` says. `None` when
/// it can't say, or doesn't answer within a minute (a hung mount).
pub async fn disk_available_mb(path: &Path) -> Option<u64> {
    let out = crate::driver::run_cmd("df", &["-Pk", path.to_str()?])
        .await
        .ok()?;
    df_available_mb(&out)
}

/// The free MiB in `df -Pk`'s output for one filesystem: the column before
/// the share used, since a filesystem's name may have spaces in it
/// (`map auto_home` on macOS).
pub fn df_available_mb(out: &str) -> Option<u64> {
    let fields: Vec<&str> = out.lines().nth(1)?.split_whitespace().collect();
    let used = fields.iter().position(|f| {
        f.strip_suffix('%')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })?;
    let kb: u64 = fields.get(used.checked_sub(1)?)?.parse().ok()?;
    Some(kb / 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_free_space_from_df() {
        let linux = "Filesystem     1024-blocks    Used Available Capacity Mounted on\n\
                     /dev/vdb         528310064 2051236 504762220       1% /\n";
        assert_eq!(df_available_mb(linux), Some(492_931));
        let mac = "Filesystem     1024-blocks      Used Available Capacity  Mounted on\n\
                   /dev/disk3s5     482797652 401733996  21059400    96%    /System/Volumes/Data\n";
        assert_eq!(df_available_mb(mac), Some(20_565));
        let spaces = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                      map auto_home 0 0 0 100% /System/Volumes/Data/home\n";
        assert_eq!(df_available_mb(spaces), Some(0));
        assert_eq!(
            df_available_mb("df: /nowhere: No such file or directory\n"),
            None
        );
        assert_eq!(df_available_mb(""), None);
    }

    #[tokio::test]
    async fn measures_the_disk_a_directory_is_on() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        assert!(disk_available_mb(dir.path()).await.is_some());
        assert_eq!(disk_available_mb(&dir.path().join("missing")).await, None);
        Ok(())
    }
}
