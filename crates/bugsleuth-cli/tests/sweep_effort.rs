//! The sweep command must reject a bad effort before it starts a provider.
//!
//! Replaces a source scan that only looked for the string `check_effort` in
//! `run_sweep`: `let _ = check_effort(...)` keeps that substring while throwing
//! the validation error away, so the scan stayed green while a mistyped effort
//! reached the vendor's CLI — either rejected after the sweep was paid for, or
//! ignored, leaving a report that never says the depth asked for was not used.
//!
//! This runs the real binary, with PATH and both home variables pointed at an
//! empty directory so a regressed build cannot discover a provider and spend
//! quota proving the point.

use std::path::Path;
use std::process::Command;

#[test]
fn sweep_rejects_invalid_effort_before_starting_a_provider() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let empty_home =
        std::env::temp_dir().join(format!("bugsleuth-no-provider-{}", std::process::id()));
    std::fs::create_dir_all(&empty_home).expect("create empty provider home");
    let output = Command::new(env!("CARGO_BIN_EXE_bugsleuth"))
        .args(["sweep", "--repo"])
        .arg(&repo)
        .args([
            "--lane",
            "correctness",
            "--model",
            "sonnet",
            "--effort",
            "definitely-invalid",
            "--timeout-secs",
            "1",
        ])
        .env("PATH", "")
        .env("HOME", &empty_home)
        .env("USERPROFILE", &empty_home)
        .output()
        .expect("run bugsleuth");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(
        stderr.contains("model `sonnet` asks for effort `definitely-invalid`"),
        "{stderr}"
    );
    // The banner is printed immediately before the provider is invoked, so its
    // absence is what proves nothing was spent.
    assert!(!stderr.contains("Starting Correctness sweep"), "{stderr}");

    let spaced_kimi = Command::new(env!("CARGO_BIN_EXE_bugsleuth"))
        .args(["sweep", "--repo"])
        .arg(&repo)
        .args([
            "--lane",
            "correctness",
            "--model",
            " kimi:kimi-code/k3 ",
            "--effort",
            "high",
            "--timeout-secs",
            "1",
        ])
        .env("PATH", "")
        .env("HOME", &empty_home)
        .env("USERPROFILE", &empty_home)
        .output()
        .expect("run bugsleuth with spaced Kimi model");
    let stderr = String::from_utf8_lossy(&spaced_kimi.stderr);
    assert!(!spaced_kimi.status.success(), "{stderr}");
    assert!(stderr.contains("which kimi does not accept"), "{stderr}");
    assert!(!stderr.contains("Starting Correctness sweep"), "{stderr}");
    let _ = std::fs::remove_dir_all(&empty_home);
}
