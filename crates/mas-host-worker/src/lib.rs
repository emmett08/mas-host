//! Private native-worker transport and Linux lifetime supervision.
//!
//! The executable calling this crate must forward its hidden `__supervise`
//! command to [`mas_protocol_codec::worker_supervisor::supervise`]. The guardian
//! holds a duplicate of the caller's ownership lock until it has reaped the
//! trusted worker's process group. Workers are not security-sandboxed: the
//! existing MAS guardian supervises cooperative application processes.
//!
//! This initial profile clears the inherited environment, uses a 2 GiB address
//! space ceiling (including Go's virtual-memory reservation), a CPU limit equal
//! to the remaining wall-time ceiling, and a 64 MiB file-size ceiling. Standard
//! output and error are discarded by the guardian; they never carry frames.

#![cfg(target_os = "linux")]

use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

pub const MAX_FRAME_BYTES: usize = 65_536;
const MEMORY_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const FILE_BYTES: u64 = 64 * 1024 * 1024;

/// One trusted native worker and its guardian. Call `stop` and check its result
/// before recording successful application completion. `Drop` also revokes the
/// worker lifetime and waits, but cannot report a failed cleanup to the caller.
pub struct Worker {
    stream: UnixStream,
    lifetime: Option<OwnedFd>,
    guardian: Option<Child>,
    deadline: Deadline,
    stopped: Option<Result<(), String>>,
}

impl Worker {
    /// Start a registered absolute executable. `deadline_ms` is the original
    /// absolute UNIX deadline, never a new timeout for this invocation.
    /// `ownership_fd` remains owned by the caller and is never closed here.
    pub fn spawn(
        executable: &Path,
        start: &Value,
        deadline_ms: u64,
        ownership_fd: RawFd,
    ) -> Result<Self, String> {
        Self::spawn_scoped(
            executable,
            start,
            deadline_ms,
            ownership_fd,
            &[],
            &BTreeMap::new(),
        )
    }

    /// Start an explicitly configured worker with only its declared environment.
    /// Secret values are passed to the process, never into its start frame.
    pub fn spawn_scoped(
        executable: &Path,
        start: &Value,
        deadline_ms: u64,
        ownership_fd: RawFd,
        arguments: &[String],
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        Self::spawn_scoped_with_launcher(
            executable,
            start,
            deadline_ms,
            ownership_fd,
            arguments,
            environment,
            |command, args| {
                command.arg("__supervise").args(args);
            },
        )
    }

    #[cfg(test)]
    fn spawn_with_launcher(
        executable: &Path,
        start: &Value,
        deadline_ms: u64,
        ownership_fd: RawFd,
        configure: impl FnOnce(&mut Command, &[String]),
    ) -> Result<Self, String> {
        Self::spawn_scoped_with_launcher(
            executable,
            start,
            deadline_ms,
            ownership_fd,
            &[],
            &BTreeMap::new(),
            configure,
        )
    }

    fn spawn_scoped_with_launcher(
        executable: &Path,
        start: &Value,
        deadline_ms: u64,
        ownership_fd: RawFd,
        arguments: &[String],
        environment: &BTreeMap<String, String>,
        configure: impl FnOnce(&mut Command, &[String]),
    ) -> Result<Self, String> {
        if !executable.is_absolute() {
            return Err("worker executable must be an absolute registered path".into());
        }
        // Validate before creating a process or handing it a lifetime capability.
        let encoded_start = encode_frame(start)?;
        let deadline = Deadline::new(deadline_ms)?;
        let ownership = duplicate(ownership_fd)?;
        let (stream, child_channel) = UnixStream::pair().map_err(io_error)?;
        let channel = duplicate(child_channel.as_raw_fd())?;
        drop(child_channel);
        let (lifetime_read, lifetime_write) = lifetime_pipe()?;
        let inherited = [
            lifetime_read.as_raw_fd(),
            channel.as_raw_fd(),
            ownership.as_raw_fd(),
        ];
        let cpu_seconds = deadline.remaining()?.as_millis().div_ceil(1000).max(1);
        let mut args = vec![
            inherited[0].to_string(),
            inherited[1].to_string(),
            deadline_ms.to_string(),
            MEMORY_BYTES.to_string(),
            cpu_seconds.to_string(),
            FILE_BYTES.to_string(),
            inherited[2].to_string(),
            "--".into(),
            executable
                .to_str()
                .ok_or("worker executable path is not UTF-8")?
                .into(),
        ];
        args.extend_from_slice(arguments);
        let mut command = Command::new(std::env::current_exe().map_err(io_error)?);
        command
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .envs(environment)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure(&mut command, &args);
        // Change descriptor flags only in the forked child. Clearing CLOEXEC in
        // the multithreaded parent would leak descriptors to concurrent launches.
        unsafe {
            command.pre_exec(move || {
                // A caller may supply an inheritable lock descriptor. Only its
                // dedicated guardian duplicate may survive this exec; the
                // caller's descriptor and flags remain untouched in the parent.
                let flags = libc::fcntl(ownership_fd, libc::F_GETFD);
                if flags < 0
                    || libc::fcntl(ownership_fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0
                {
                    return Err(io::Error::last_os_error());
                }
                for fd in inherited {
                    if libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let guardian = command.spawn().map_err(io_error)?;
        drop((lifetime_read, channel, ownership));
        let mut worker = Self {
            stream,
            lifetime: Some(lifetime_write),
            guardian: Some(guardian),
            deadline,
            stopped: None,
        };
        if let Err(error) = write_bytes(&mut worker.stream, &worker.deadline, &encoded_start) {
            return Err(match worker.stop() {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; {cleanup}"),
            });
        }
        Ok(worker)
    }

    /// Read one bounded, strictly validated JSON object.
    pub fn read(&mut self) -> Result<Value, String> {
        self.ensure_running()?;
        read_frame(&mut self.stream, &self.deadline)
    }

    /// Read one frame while checking cancellation at most every 50 ms.
    /// Partial headers and bodies remain in place between checks; all reads
    /// retain the original monotonic and wall-clock deadline.
    pub fn read_interruptible(
        &mut self,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Value, String> {
        self.ensure_running()?;
        read_frame_interruptible(&mut self.stream, &self.deadline, &mut check)
    }

    /// Write one bounded JSON object, using the same strict MAS JSON rules as
    /// inbound frames. Every partial write shares the original deadline.
    pub fn write(&mut self, value: &Value) -> Result<(), String> {
        self.ensure_running()?;
        let bytes = encode_frame(value)?;
        write_bytes(&mut self.stream, &self.deadline, &bytes)
    }

    /// Revoke lifetime and confirm that the guardian finished cleanup. This
    /// method never kills the guardian, which must retain its ownership lock
    /// while terminating and reaping descendants. Repeated calls are idempotent.
    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(result) = &self.stopped {
            return result.clone();
        }
        drop(self.lifetime.take());
        let _ = self.stream.shutdown(Shutdown::Both);
        let result = match self.guardian.as_mut() {
            Some(guardian) => match guardian.wait() {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => Err(format!("worker guardian did not confirm cleanup: {status}")),
                Err(error) => Err(format!("cannot confirm worker cleanup: {error}")),
            },
            None => Err("worker guardian is unavailable".into()),
        };
        // Keep a failed wait's Child so Drop can make one more wait attempt.
        if result.is_ok() {
            self.guardian.take();
        }
        self.stopped = Some(result.clone());
        result
    }

    fn ensure_running(&self) -> Result<(), String> {
        if self.stopped.is_some() {
            Err("worker has stopped".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.stop();
        if let Some(guardian) = self.guardian.as_mut() {
            let _ = guardian.wait();
        }
    }
}

struct Deadline {
    at: Instant,
    unix_ms: u64,
}

impl Deadline {
    fn new(unix_ms: u64) -> Result<Self, String> {
        let remaining = unix_ms
            .checked_sub(unix_ms_now())
            .filter(|n| *n > 0)
            .ok_or("worker deadline has elapsed")?;
        if remaining > 86_400_000 {
            return Err("worker deadline exceeds supervisor maximum".into());
        }
        Ok(Self {
            at: Instant::now() + Duration::from_millis(remaining),
            unix_ms,
        })
    }

    fn remaining(&self) -> Result<Duration, String> {
        // Monotonic time prevents wall-clock rollback from extending execution;
        // the wall deadline also honours forward clock adjustments immediately.
        let duration =
            self.at
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(
                    self.unix_ms.saturating_sub(unix_ms_now()),
                ));
        if duration.is_zero() {
            Err("worker deadline has elapsed".into())
        } else {
            Ok(duration)
        }
    }
}

fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn io_error(error: io::Error) -> String {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => "worker deadline has elapsed".into(),
        io::ErrorKind::UnexpectedEof => "worker closed its channel before a complete frame".into(),
        _ => format!("worker transport: {error}"),
    }
}

fn duplicate(fd: RawFd) -> Result<OwnedFd, String> {
    // F_DUPFD_CLOEXEC also keeps internal descriptors clear of stdin/out/err.
    let copy = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 3) };
    if copy < 0 {
        return Err(format!(
            "cannot duplicate worker ownership/transport descriptor: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(copy) })
}

fn lifetime_pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [-1; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io_error(io::Error::last_os_error()));
    }
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((duplicate(read.as_raw_fd())?, duplicate(write.as_raw_fd())?))
}

fn encode_frame(value: &Value) -> Result<Vec<u8>, String> {
    if !value.is_object() {
        return Err("worker frame must be a JSON object".into());
    }
    let body = serde_json::to_vec(value).map_err(|e| format!("invalid worker frame: {e}"))?;
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err("worker frame exceeds size limit".into());
    }
    mas_protocol_codec::parse_json(std::str::from_utf8(&body).map_err(|_| "invalid worker UTF-8")?)
        .map_err(|e| format!("invalid worker JSON: {}", e.message))?;
    let mut bytes = Vec::with_capacity(body.len() + 4);
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

fn read_frame(stream: &mut UnixStream, deadline: &Deadline) -> Result<Value, String> {
    read_frame_interruptible(stream, deadline, &mut || Ok(()))
}

fn read_frame_interruptible(
    stream: &mut UnixStream,
    deadline: &Deadline,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<Value, String> {
    let mut header = [0; 4];
    read_bytes_interruptible(stream, deadline, &mut header, check)?;
    let length = u32::from_be_bytes(header) as usize;
    if !(1..=MAX_FRAME_BYTES).contains(&length) {
        return Err("invalid worker frame length".into());
    }
    let mut body = vec![0; length];
    read_bytes_interruptible(stream, deadline, &mut body, check)?;
    let source = std::str::from_utf8(&body).map_err(|_| "invalid worker UTF-8")?;
    let value = mas_protocol_codec::parse_json(source)
        .map_err(|e| format!("invalid worker JSON: {}", e.message))?;
    if !value.is_object() {
        return Err("worker frame must be a JSON object".into());
    }
    Ok(value)
}

fn read_bytes_interruptible(
    stream: &mut UnixStream,
    deadline: &Deadline,
    mut bytes: &mut [u8],
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<(), String> {
    while !bytes.is_empty() {
        check()?;
        stream
            .set_read_timeout(Some(deadline.remaining()?.min(Duration::from_millis(50))))
            .map_err(io_error)?;
        match stream.read(bytes) {
            Ok(0) => return Err(io_error(io::Error::from(io::ErrorKind::UnexpectedEof))),
            Ok(count) => bytes = &mut bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                continue
            }
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}

fn write_bytes(
    stream: &mut UnixStream,
    deadline: &Deadline,
    mut bytes: &[u8],
) -> Result<(), String> {
    while !bytes.is_empty() {
        stream
            .set_write_timeout(Some(deadline.remaining()?))
            .map_err(io_error)?;
        match stream.write(bytes) {
            Ok(0) => return Err("worker channel stopped accepting data".into()),
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::{self, OpenOptions},
        os::unix::fs::PermissionsExt,
        sync::Mutex,
        thread,
    };
    // The descriptor-preservation test deliberately removes CLOEXEC from a
    // process-global descriptor. Other test launches must not inherit it while
    // that test is asserting the dedicated guardian's ownership lifetime.
    static GUARDIAN_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn receive(body: &[u8]) -> Result<Value, String> {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_all(&(body.len() as u32).to_be_bytes())
            .unwrap();
        writer.write_all(body).unwrap();
        drop(writer);
        read_frame(&mut reader, &Deadline::new(unix_ms_now() + 1_000).unwrap())
    }

    #[test]
    fn protocol_rejects_invalid_json_and_frames() {
        for body in [
            &b"{\"n\":1.5}"[..],
            &b"{\"n\":1,\"n\":2}"[..],
            &b"{\"s\":\"\\ud800\"}"[..],
            &b"[]"[..],
            &b"\xff"[..],
            &b""[..],
        ] {
            assert!(receive(body).is_err(), "accepted {body:?}");
        }
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer
            .write_all(&((MAX_FRAME_BYTES + 1) as u32).to_be_bytes())
            .unwrap();
        assert!(
            read_frame(&mut reader, &Deadline::new(unix_ms_now() + 1_000).unwrap())
                .unwrap_err()
                .contains("length")
        );
        assert!(encode_frame(&serde_json::json!({"n": 1.5})).is_err());
        assert!(encode_frame(&serde_json::json!({"s": "x".repeat(MAX_FRAME_BYTES)})).is_err());
    }

    #[test]
    fn exact_integer_and_unicode_round_trip() {
        let expected = mas_protocol_codec::parse_json(
            r#"{"n":340282366920938463463374607431768211456,"s":"∀x:🧠"}"#,
        )
        .unwrap();
        let bytes = encode_frame(&expected).unwrap();
        assert_eq!(receive(&bytes[4..]).unwrap(), expected);
    }

    #[test]
    fn partial_frame_uses_original_deadline_and_reports_eof() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(&20_u32.to_be_bytes()).unwrap();
        writer.write_all(b"{").unwrap();
        let deadline = Deadline::new(unix_ms_now() + 40).unwrap();
        let before = Instant::now();
        assert!(read_frame(&mut reader, &deadline)
            .unwrap_err()
            .contains("deadline"));
        assert!(before.elapsed() < Duration::from_secs(1));
        drop(writer);
        assert!(
            read_frame(&mut reader, &Deadline::new(unix_ms_now() + 1000).unwrap())
                .unwrap_err()
                .contains("complete frame")
        );
    }

    #[test]
    fn interruptible_read_retains_partial_header_and_body() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let expected = serde_json::json!({"complete":true});
        let encoded = encode_frame(&expected).unwrap();
        let writer_thread = thread::spawn(move || {
            writer.write_all(&encoded[..2]).unwrap();
            thread::sleep(Duration::from_millis(120));
            writer.write_all(&encoded[2..7]).unwrap();
            thread::sleep(Duration::from_millis(120));
            writer.write_all(&encoded[7..]).unwrap();
        });
        let mut checks = 0;
        let result = read_frame_interruptible(
            &mut reader,
            &Deadline::new(unix_ms_now() + 2_000).unwrap(),
            &mut || {
                checks += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(result, expected);
        assert!(checks >= 4, "partial frame did not survive periodic checks");
        writer_thread.join().unwrap();
    }

    #[test]
    fn interruptible_read_cancels_without_waiting_for_deadline() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        // Stop in the middle of a header, where starting a fresh frame on every
        // timeout would either corrupt the stream or postpone cancellation.
        writer.write_all(&[0, 0]).unwrap();
        let before = Instant::now();
        let failure = read_frame_interruptible(
            &mut reader,
            &Deadline::new(unix_ms_now() + 5_000).unwrap(),
            &mut || {
                if before.elapsed() >= Duration::from_millis(70) {
                    Err("run cancelled".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(failure, "run cancelled");
        assert!(before.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn scoped_worker_receives_arguments_and_only_declared_environment() {
        let _launch_guard = GUARDIAN_TEST_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "mas-worker-scope-{}-{}",
            std::process::id(),
            unix_ms_now()
        ));
        fs::create_dir(&directory).unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(directory.join("owner.lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        let result_path = directory.join("observed");
        let executable = directory.join("worker");
        fs::write(&executable, format!(
            "#!/bin/sh\nprintf '%s\\n%s\\n%s\\n%s\\n' \"$1\" \"$MAS_TEST_DECLARED\" \"${{MAS_TEST_UNDECLARED-unset}}\" \"$PATH\" > '{}'\n/bin/sleep 5\n",
            result_path.display()
        )).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        std::env::set_var("MAS_TEST_UNDECLARED", "must-not-inherit");
        let environment = BTreeMap::from([("MAS_TEST_DECLARED".into(), "declared-value".into())]);
        let mut worker = Worker::spawn_scoped_with_launcher(
            &executable,
            &serde_json::json!({"type":"start"}),
            unix_ms_now() + 5_000,
            lock.as_raw_fd(),
            &["configured-argument".into()],
            &environment,
            |command, args| {
                command
                    .args([
                        "--exact",
                        "tests::guardian_entry",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(
                        "MAS_TEST_GUARDIAN_ARGS",
                        serde_json::to_string(args).unwrap(),
                    );
            },
        )
        .unwrap();
        let observed = (0..200)
            .find_map(|_| {
                let value = fs::read_to_string(&result_path).ok();
                if value.as_ref().is_some_and(|text| text.lines().count() == 4) {
                    return value;
                }
                thread::sleep(Duration::from_millis(5));
                None
            })
            .expect("scoped worker did not report its environment");
        worker.stop().unwrap();
        std::env::remove_var("MAS_TEST_UNDECLARED");
        assert_eq!(
            observed,
            "configured-argument\ndeclared-value\nunset\n/usr/local/bin:/usr/bin:/bin\n"
        );
        drop(lock);
        fs::remove_dir_all(directory).unwrap();
    }

    // Test-only command entrypoint: a fresh subprocess runs the real MAS
    // guardian without requiring the application's CLI binary to be built.
    #[test]
    #[ignore = "subprocess entrypoint invoked by supervisor cleanup test"]
    fn guardian_entry() {
        let args: Vec<String> =
            serde_json::from_str(&std::env::var("MAS_TEST_GUARDIAN_ARGS").unwrap()).unwrap();
        mas_protocol_codec::worker_supervisor::supervise(&args).unwrap();
    }

    #[test]
    fn guardian_reaps_descendants_retains_lock_and_preserves_parent_fd() {
        let _launch_guard = GUARDIAN_TEST_LOCK.lock().unwrap();
        let directory = std::env::temp_dir().join(format!(
            "mas-worker-test-{}-{}",
            std::process::id(),
            unix_ms_now()
        ));
        fs::create_dir(&directory).unwrap();
        let lock_path = directory.join("ownership.lock");
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        // Exercise the caller-supplied non-CLOEXEC case explicitly.
        assert_eq!(
            unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
        let pid_path = directory.join("descendant.pid");
        let executable = directory.join("worker");
        let source = format!(
            "#!/bin/sh\n/bin/sleep 30 &\nprintf '%s\\n' \"$!\" > '{}'\nwait\n",
            pid_path.display()
        );
        fs::write(&executable, source).unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let mut worker = Worker::spawn_with_launcher(
            &executable,
            &serde_json::json!({"type":"start"}),
            unix_ms_now() + 5_000,
            lock.as_raw_fd(),
            |command, args| {
                command
                    .args([
                        "--exact",
                        "tests::guardian_entry",
                        "--ignored",
                        "--nocapture",
                    ])
                    .env(
                        "MAS_TEST_GUARDIAN_ARGS",
                        serde_json::to_string(args).unwrap(),
                    );
            },
        )
        .unwrap();
        assert_eq!(unsafe { libc::fcntl(lock.as_raw_fd(), libc::F_GETFD) }, 0);
        let pid = (0..200)
            .find_map(|_| {
                if let Ok(contents) = fs::read_to_string(&pid_path) {
                    if let Ok(pid) = contents.trim().parse::<i32>() {
                        return Some(pid);
                    }
                }
                thread::sleep(Duration::from_millis(5));
                None
            })
            .expect("worker started descendant");
        // Parent no longer owns this open-file description. Only the guardian
        // may retain the lock, and the application must not inherit it.
        drop(lock);
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            -1
        );
        worker.stop().unwrap();
        worker.stop().unwrap();
        assert!(worker.read().unwrap_err().contains("stopped"));
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
        assert_eq!(
            unsafe { libc::flock(contender.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        drop(contender);
        fs::remove_dir_all(directory).unwrap();
    }
}
