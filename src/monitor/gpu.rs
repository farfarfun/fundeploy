//! GPU 利用率由各平台的驱动提供。缺失的计数器不当成 0%。

type Samples = Vec<(String, f32)>;

fn percent(s: &str) -> Option<f32> {
    s.trim().parse().ok().filter(|n| (0.0..=100.0).contains(n))
}

#[cfg(any(target_os = "linux", windows))]
fn rows(text: &str) -> Samples {
    text.lines()
        .filter_map(|line| {
            let (name, value) = line.rsplit_once(',')?;
            Some((name.trim().to_owned(), percent(value)?))
        })
        .collect()
}

#[cfg(unix)]
fn query(args: &[&str]) -> String {
    crate::common::cmd::try_run_timeout(args, std::time::Duration::from_secs(2))
        .ok()
        .flatten()
        .filter(|out| out.success())
        .map(|out| out.stdout)
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
pub(super) fn sample() -> Samples {
    static HAS_NVIDIA_SMI: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let mut result = if *HAS_NVIDIA_SMI.get_or_init(|| crate::common::cmd::has("nvidia-smi")) {
        rows(&query(&[
            "nvidia-smi",
            "--query-gpu=name,utilization.gpu",
            "--format=csv,noheader,nounits",
        ]))
    } else {
        Samples::new()
    };
    if let Ok(entries) = std::fs::read_dir("/sys/class/drm") {
        for entry in entries.flatten() {
            let card = entry.file_name().to_string_lossy().into_owned();
            if !card
                .strip_prefix("card")
                .is_some_and(|n| n.parse::<u32>().is_ok())
            {
                continue;
            }
            let device = entry.path().join("device");
            if let Some(value) = std::fs::read_to_string(device.join("gpu_busy_percent"))
                .ok()
                .and_then(|s| percent(&s))
            {
                let driver = std::fs::read_link(device.join("driver")).ok();
                let label = driver
                    .as_deref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("GPU");
                result.push((format!("{label} ({card})"), value));
            }
        }
    }
    result
}

#[cfg(any(target_os = "macos", test))]
fn ioreg(text: &str) -> Samples {
    let mut result = Vec::new();
    let mut name = "GPU";
    for line in text.lines() {
        if let Some((_, entry)) = line.split_once("+-o ") {
            name = entry.split("  <").next().unwrap_or("GPU").trim();
        }
        if let Some((_, value)) = line.split_once("\"Device Utilization %\"=") {
            let digits = value
                .trim_start()
                .split(|c: char| !c.is_ascii_digit() && c != '.')
                .next()
                .unwrap_or("");
            if let Some(n) = percent(digits) {
                result.push((name.to_owned(), n));
            }
        }
    }
    result
}

#[cfg(target_os = "macos")]
pub(super) fn sample() -> Samples {
    let mut result = Samples::new();
    for class in ["IOGPU", "IOAccelerator"] {
        for gpu in ioreg(&query(&[
            "/usr/sbin/ioreg",
            "-r",
            "-d",
            "1",
            "-c",
            class,
            "-l",
            "-w",
            "0",
        ])) {
            if !result.iter().any(|(name, _)| name == &gpu.0) {
                result.push(gpu);
            }
        }
    }
    result
}

#[cfg(windows)]
pub(super) fn sample() -> Samples {
    // PowerShell 启动 + CIM 查询耗时数百毫秒，monitor 2s 刷一次时做 6s 短缓存以降低额外开销。
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    const CACHE_TTL: Duration = Duration::from_secs(6);
    static CACHE: OnceLock<Mutex<Option<(Instant, Samples)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, ref cached)) = *guard {
            if at.elapsed() < CACHE_TTL {
                return cached.clone();
            }
        }
    }

    // WDDM 按进程、引擎分别计数。同一引擎的进程值相加，每张卡取最忙的引擎。
    let script = r#"
$engines = @{}
Get-CimInstance Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine -OperationTimeoutSec 2 -ErrorAction Stop | ForEach-Object {
    if ($_.Name -match 'luid_(.+?_phys_\d+)_eng_(\d+)') {
        $gpu = $Matches[1]
        $engine = $Matches[2]
        if (!$engines.ContainsKey($gpu)) { $engines[$gpu] = @{} }
        $engines[$gpu][$engine] += [int]$_.UtilizationPercentage
    }
}
foreach ($gpu in ($engines.Keys | Sort-Object)) {
    $busy = ($engines[$gpu].Values | Measure-Object -Maximum).Maximum
    "GPU $gpu,$([Math]::Min(100, [int]$busy))"
}
"#;
    let fresh = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| rows(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), fresh.clone()));
    }
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_is_distinct_from_idle() {
        assert_eq!(percent("0"), Some(0.0));
        assert_eq!(percent("N/A"), None);
        assert_eq!(percent("NaN"), None);
    }
    #[test]
    fn apple_registry_load() {
        let text = "+-o AGX  <class AGXAccelerator>\n \"PerformanceStatistics\" = {\"Device Utilization %\"=23,\"other\"=1}";
        assert_eq!(ioreg(text), [("AGX".into(), 23.0)]);
    }
}
