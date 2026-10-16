//! Bounded process transport. The enclosing host guardian owns cleanup for
//! foreground tools whose descendants retain the inherited process group.
//! Adapters never create a new session or escape that group.
use std::{
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const MAX_BYTES: usize = 1_048_576;
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
}
pub enum Failure {
    NotStarted,
    Incomplete,
}

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
pub fn remaining(deadline_ms: u64) -> Option<Duration> {
    deadline_ms
        .checked_sub(unix_ms())
        .filter(|n| *n > 0 && *n <= 86_400_000)
        .map(Duration::from_millis)
}

pub fn run(command: Command, input: Vec<u8>, deadline_ms: u64) -> Result<Output, Failure> {
    run_bounded(command, input, deadline_ms, MAX_BYTES)
}

/// Both output streams are drained concurrently and limited independently.
/// The host guardian owns cleanup of descendants in the inherited process group,
/// including inherited pipes. Daemonising tools are outside this profile.
pub fn run_bounded(
    mut command: Command,
    input: Vec<u8>,
    deadline_ms: u64,
    max_bytes: usize,
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
        let (ok, bytes) = read_bounded(stdout, true, max_bytes);
        let _ = reader.send((1, ok, bytes));
    });
    thread::spawn(move || {
        let (ok, _) = read_bounded(stderr, false, max_bytes);
        let _ = sender.send((2, ok, Vec::new()));
    });
    let mut pending = 3;
    let mut output = None;
    let status = loop {
        while let Ok((kind, ok, bytes)) = receiver.try_recv() {
            pending -= 1;
            if !ok {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Failure::Incomplete);
            }
            if kind == 1 {
                output = Some(bytes);
            }
        }
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
    for _ in 0..pending {
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

fn read_bounded(mut stream: impl Read, retain: bool, max_bytes: usize) -> (bool, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0; 8192];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return (total <= max_bytes, bytes),
            Ok(n) => {
                total = total.saturating_add(n);
                if total > max_bytes {
                    return (false, bytes);
                }
                if retain {
                    bytes
                        .extend_from_slice(&buffer[..n.min(max_bytes.saturating_sub(bytes.len()))]);
                }
            }
            Err(_) => return (false, bytes),
        }
    }
}

pub fn scoped_environment(command: &mut Command, names: &[String]) {
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

/// Launch environment names are independent of the implementation kind.
/// Reserved names belong to the worker transport and fixed executable search path.
pub fn environment_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, String> {
    let mut result = std::collections::BTreeSet::new();
    for name in names {
        if name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .enumerate()
                .all(|(i, b)| b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit()))
            || matches!(name, "PATH" | "MAS_WORKER_CHANNEL_FD")
        {
            return Err("invalid or reserved environment name".into());
        }
        result.insert(name.to_owned());
    }
    Ok(result.into_iter().collect())
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

    #[test]
    fn output_overflow_stops_a_running_process_before_its_deadline() {
        let mut command = Command::new("/usr/bin/yes");
        command.arg("bounded");
        let before = Instant::now();
        assert!(matches!(
            run_bounded(command, Vec::new(), unix_ms() + 5000, 128),
            Err(Failure::Incomplete)
        ));
        assert!(before.elapsed() < Duration::from_secs(1));
    }
}
