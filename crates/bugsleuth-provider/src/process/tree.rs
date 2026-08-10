//! Process-tree ownership for every spawned CLI and network Git command.

use std::process::Stdio;

use tokio::process::Command;

/// Give a Unix child its own process group so one signal reaches descendants.
#[cfg(unix)]
pub(super) fn prepare(command: &mut Command, isolate: bool) {
    use std::os::unix::process::CommandExt;

    if isolate {
        command.as_std_mut().process_group(0);
    }
}

#[cfg(not(unix))]
pub(super) fn prepare(_command: &mut Command, _isolate: bool) {}

/// Kills a spawned process and everything it started when its owner unwinds.
/// Armed only until the direct child is reaped, before its pid can be reused.
pub(super) struct KillTree {
    pid: Option<u32>,
    isolated: bool,
}

impl KillTree {
    pub(super) fn new(pid: Option<u32>, isolated: bool) -> Self {
        Self { pid, isolated }
    }

    pub(super) fn fire(&mut self) {
        if let Some(pid) = self.pid.take() {
            kill_tree(pid, self.isolated);
        }
    }

    pub(super) fn disarm(&mut self) {
        self.pid = None;
    }
}

impl Drop for KillTree {
    fn drop(&mut self) {
        self.fire();
    }
}

/// `taskkill /T` must run while the direct child still anchors the tree.
#[cfg(windows)]
fn kill_tree(pid: u32, _isolated: bool) {
    use std::os::windows::process::CommandExt;

    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(super::CREATE_NO_WINDOW)
        .status();
}

/// The direct child is the process-group leader, so its negative pid addresses
/// the whole group. Processes that deliberately leave the group are outside the
/// operating system boundary available here without unsafe platform APIs.
#[cfg(unix)]
fn kill_tree(pid: u32, isolated: bool) {
    if !isolated {
        return;
    }
    let group = format!("-{pid}");
    let _ = std::process::Command::new("kill")
        .args(["-KILL", "--", &group])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
