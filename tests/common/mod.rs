//! Shared helpers for the integration tests: run the oxslt binary on a case
//! and compare its result with xsltproc's recorded output.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Non-empty lines of a file, or nothing if it doesn't exist.
pub fn lines(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|s| {
            s.lines()
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Runs oxslt in `dir` with `args`. `env` entries are defaults: a variable
/// already set in the test's environment wins. Returns the problems found
/// when comparing with `expected/stdout` and `expected/exit` in `expected`;
/// an empty list means the case passed.
pub fn run_and_compare(
    dir: &Path,
    args: &[String],
    env: &[String],
    expected: &Path,
) -> Vec<String> {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_oxslt"));
    cmd.current_dir(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for kv in env {
        if let Some((k, v)) = kv.split_once('=')
            && std::env::var_os(k).is_none()
        {
            cmd.env(k, v);
        }
    }

    let mut child = cmd.spawn().expect("failed to start oxslt");
    // Read stdout on a thread so a large output can't fill the pipe and stall.
    let mut out = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut out, &mut buf).map(|_| buf)
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait failed") {
            break Some(status);
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let stdout = reader.join().unwrap().unwrap_or_default();
    let Some(status) = status else {
        return vec![format!("timed out after {TIMEOUT:?}")];
    };

    let exit = status
        .code()
        .map_or_else(|| "signal".to_string(), |c| c.to_string());
    let want_exit = fs::read_to_string(expected.join("exit"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let want_stdout = fs::read(expected.join("stdout")).unwrap_or_default();

    let mut problems = Vec::new();
    if exit != want_exit {
        problems.push(format!("exit {exit}, expected {want_exit}"));
    }
    if stdout != want_stdout {
        problems.push(first_difference(&want_stdout, &stdout));
    }
    problems
}

fn first_difference(want: &[u8], got: &[u8]) -> String {
    let at = want
        .iter()
        .zip(got)
        .position(|(a, b)| a != b)
        .unwrap_or(want.len().min(got.len()));
    let line = want[..at].iter().filter(|&&b| b == b'\n').count() + 1;
    format!(
        "stdout differs at byte {at} (line {line}): {} bytes, expected {}",
        got.len(),
        want.len()
    )
}
