//! Keeping child processes from opening console windows.
//!
//! Every process this project spawns — `git`, the target's test runner, the
//! provider CLIs — is a console program. Windows gives a console program
//! launched from a *windowed* process a brand new console, which appears as a
//! black window on the user's desktop. A single run spawns a dozen of them.
//!
//! Observed during a real run of the desktop app: a `conhost.exe` sitting
//! underneath the CLI child, and console windows flashing over the desktop.
//!
//! `bugsleuth-provider` carries its own copy of this for `tokio::process`.
//! The two cannot share one function because `creation_flags` arrives from a
//! different extension trait for each `Command` type, and a whole crate to hold
//! four lines twice is a worse trade than the four lines.

use std::process::Command;

/// Suppress the console window Windows would otherwise create for a child.
#[cfg(windows)]
pub fn hide(command: &mut Command) -> &mut Command {
    use std::os::windows::process::CommandExt;

    // Named rather than left as a bare hex literal: `0x0800_0000` at a call
    // site tells the next reader nothing.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW)
}

/// Nothing to do: only Windows conjures a console for a child process.
#[cfg(not(windows))]
pub fn hide(command: &mut Command) -> &mut Command {
    command
}
