//! Bounded process transport. The enclosing host guardian owns descendant
//! cleanup; adapters never create a new session or escape its process group.
use std::{
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) const MAX_BYTES: usize = 1_048_576;
pub(crate) struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
}
pub(crate) enum Failure {
    NotStarted,
    Incomplete,
}

pub(crate) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
pub(crate) fn remaining(deadline_ms: u64) -> Option<Duration> {
    deadline_ms
        .checked_sub(unix_ms())
        .filter(|n| *n > 0 && *n <= 86_400_000)
        .map(Duration::from_millis)
}

pub(crate) fn run(
    mut command: Command,
    input: Vec<u8>,
    deadline_ms: u64,
) -> Result<Output, Failure> {
    let duration = remaining(deadline_ms).ok_or(Failure::NotStarted)?;
    let at = Instant::now() + duration;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| Failure::NotStarted)?;
    let mut stdin = child.stdin.take().ok_or(Failure::Incomplete)?;
    let stdout = child.stdout.take().ok_or(Failure::Incomplete)?;
    let stderr = child.stderr.take().ok_or(Failure::Incomplete)?;
    let (sender, receiver) = mpsc::channel();
    let writer = sender.clone();
    thread::spawn(move || {
        let result = stdin.write_all(&input).is_ok();
        drop(stdin);
        let _ = writer.send((0, result, Vec::new()));
    });
    let reader = sender.clone();
    thread::spawn(move || {
        let (ok, bytes) = read_bounded(stdout, true);
        let _ = reader.send((1, ok, bytes));
    });
    thread::spawn(move || {
        let (ok, _) = read_bounded(stderr, false);
        let _ = sender.send((2, ok, Vec::new()));
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < at && unix_ms() < deadline_ms => {
                thread::sleep(Duration::from_millis(5))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Incomplete);
            }
        }
    };
    let mut output = None;
    for _ in 0..3 {
        let wait = at
            .checked_duration_since(Instant::now())
            .ok_or(Failure::Incomplete)?;
        match receiver.recv_timeout(wait) {
            Ok((kind, true, bytes)) => {
                if kind == 1 {
                    output = Some(bytes);
                }
            }
            _ => return Err(Failure::Incomplete),
        }
    }
    Ok(Output {
        status,
        stdout: output.ok_or(Failure::Incomplete)?,
    })
}

fn read_bounded(mut stream: impl Read, retain: bool) -> (bool, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return (total <= MAX_BYTES, bytes),
            Ok(n) => {
                total = total.saturating_add(n);
                if retain {
                    bytes
                        .extend_from_slice(&buffer[..n.min(MAX_BYTES.saturating_sub(bytes.len()))]);
                }
            }
            Err(_) => return (false, bytes),
        }
    }
}

pub(crate) fn scoped_environment(command: &mut Command, names: &[String]) {
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    for name in names {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeout_returns_without_waiting_for_open_inherited_streams() {
        let mut command = Command::new("/bin/sh");
        // Plain sleep does not create grandchildren that would need the host guardian.
        command.args(["-c", "exec sleep 2"]);
        let before = Instant::now();
        assert!(matches!(
            run(command, Vec::new(), unix_ms() + 40),
            Err(Failure::Incomplete)
        ));
        assert!(before.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn output_limit_rejects_a_completed_oversized_process() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 1048577 /dev/zero"]);
        assert!(matches!(
            run(command, Vec::new(), unix_ms() + 5000),
            Err(Failure::Incomplete)
        ));
    }
}
