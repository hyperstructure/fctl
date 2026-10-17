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

pub fn disk_available_mb(path: &Path) -> Option<u64> {
    let o = Command::new("df").args(["-Pk"]).arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&o.stdout);
    let line = text.lines().nth(1)?;
    line.split_whitespace()
        .nth(3)?
        .parse::<u64>()
        .ok()
        .map(|kb| kb / 1024)
}
