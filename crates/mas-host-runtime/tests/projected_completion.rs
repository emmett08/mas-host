//! Real worker sockets exercise both fresh and cached command/provider delivery.
#![cfg(unix)]
use mas_host_sdk::{json, Context, Json, Result};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

struct CountingProvider;
impl mas_host_providers::Provider for CountingProvider {
    fn infer(&self, _: &mas_host_providers::Request) -> mas_host_providers::Outcome {
        let path = std::env::var("MAS_RUNTIME_PROJECTION_COUNTER").unwrap();
        let mut counter = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        counter.write_all(b"x").unwrap();
        mas_host_providers::Outcome::Completed {
            output: json!({"private":"private-local-output"}),
            evidence: json!({"receipt":"one"}),
        }
    }
}

fn handler(context: &mut Context, input: Json, configuration: Json) -> Result<Json> {
    if configuration["kind"] == "command" {
        mas_host_runtime::command::execute(context, input, &configuration)
    } else {
        mas_host_runtime::inference::execute(context, input, &configuration, &CountingProvider)
    }
}

#[test]
#[ignore = "private subprocess fixture"]
fn worker_entry() {
    mas_host_sdk::serve_configured(handler).unwrap();
}

fn send(channel: &mut UnixStream, value: &Json) {
    let bytes = serde_json::to_vec(value).unwrap();
    channel
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    channel.write_all(&bytes).unwrap();
}

fn receive(channel: &mut UnixStream) -> Json {
    let mut header = [0; 4];
    channel.read_exact(&mut header).unwrap();
    let length = u32::from_be_bytes(header) as usize;
    assert!((1..=65_536).contains(&length));
    let mut bytes = vec![0; length];
    channel.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn run_worker(
    configuration: &Json,
    counter: &std::path::Path,
    projection: &Json,
    cached: bool,
    malformed: bool,
    expected_reported: Option<&Json>,
) -> Json {
    let (mut host, worker) = UnixStream::pair().unwrap();
    host.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    host.set_write_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let fd = worker.as_raw_fd();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "worker_entry", "--ignored", "--nocapture"])
        .env("MAS_WORKER_CHANNEL_FD", "3")
        .env("MAS_RUNTIME_PROJECTION_COUNTER", counter)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Only the child changes descriptor flags. The SDK revokes inheritance
    // immediately after adopting this fixture's private transport descriptor.
    unsafe {
        command.pre_exec(move || {
            unsafe extern "C" {
                fn dup2(old: i32, new: i32) -> i32;
                fn fcntl(fd: i32, command: i32, value: i32) -> i32;
            }
            if dup2(fd, 3) < 0 || fcntl(3, 2, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(worker);
    send(
        &mut host,
        &json!({"type":"start","version":"mas-native/1","actor":"worker","handler":"Run",
        "input":{},"configuration":configuration,"invocation_id":"invocation-1",
        "deadline_ms":mas_host_process::unix_ms()+10_000}),
    );
    assert_eq!(receive(&mut host)["type"], "effect_begin");
    if cached {
        send(
            &mut host,
            &json!({"type":"effect","state":"cached","output":projection}),
        );
    } else {
        send(
            &mut host,
            &json!({"type":"effect","state":"execute","invocation_id":"effect-1"}),
        );
        let completed = receive(&mut host);
        assert_eq!(completed["type"], "effect_complete");
        if let Some(expected) = expected_reported {
            assert_eq!(&completed["output"], expected);
        } else {
            assert!(completed["output"]
                .to_string()
                .contains("private-local-output"));
        }
        if configuration["kind"] == "command" {
            assert_eq!(completed["evidence"]["execution"], "completed");
            assert_eq!(completed["evidence"]["execution_scope"], "command-process");
            assert_eq!(completed["evidence"]["exit_code"], 0);
        }
        let withheld = projection["version"] == "mas-governed-unavailable/1";
        send(
            &mut host,
            &json!({"type":"response","ok":true,"completion":{
            "version":"mas-governed-effect-completion/1",
            "effect_id":if malformed {"different-effect"} else {"effect-1"},
            "host_settlement":"completed","external_execution":"completed",
            "payload_disposition":if withheld {"withheld"} else {"available"},"output":projection}}),
        );
    }
    let result = receive(&mut host);
    assert!(!result.to_string().contains("private-local-output"));
    assert!(child.wait().unwrap().success());
    result
}

#[test]
fn runtime_uses_safe_fresh_projection_and_never_dispatches_cached_or_withheld_work() {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "mas-runtime-completion-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let counter = root.join("count");
    let schema = json!({"type":"object","properties":{"private":{"type":"string"},"redacted":{"type":"boolean"}},"additionalProperties":false});
    for kind in ["command", "inference"] {
        let configuration = if kind == "command" {
            json!({"kind":"command","executable":"/bin/sh","arguments":[
                {"literal":"-c"},{"literal":"printf x >> \"$1\"; printf '{\"private\":\"private-local-output\"}'"},
                {"literal":"fixture"},{"literal":counter}],"output_schema":schema,"timeout_seconds":5})
        } else {
            json!({"kind":"llm","model":"synthetic","max_turns":3,"output_schema":schema})
        };
        let available = if kind == "command" {
            json!({"status":"completed","output":{"redacted":true}})
        } else {
            json!({"certainty":"completed","output_status":"valid","output":{"redacted":true}})
        };
        let withheld = json!({"version":"mas-governed-unavailable/1","external_execution":"completed","payload_disposition":"withheld"});
        for (projection, malformed) in [(&available, false), (&withheld, false), (&available, true)]
        {
            std::fs::write(&counter, b"").unwrap();
            let fresh = run_worker(&configuration, &counter, projection, false, malformed, None);
            let replay = run_worker(&configuration, &counter, projection, true, false, None);
            assert_eq!(
                std::fs::read(&counter).unwrap(),
                b"x",
                "{kind} dispatched again"
            );
            if malformed {
                assert_eq!(
                    fresh,
                    json!({"type":"error","message":"invalid governed effect completion"})
                );
                assert_eq!(
                    replay,
                    json!({"type":"complete","output":{"redacted":true}})
                );
            } else {
                assert_eq!(fresh, replay);
                if projection == &withheld {
                    assert_eq!(
                        fresh,
                        json!({"type":"error","message":"effect completed; result unavailable under current handling policy"})
                    );
                } else {
                    assert_eq!(fresh, json!({"type":"complete","output":{"redacted":true}}));
                }
            }
        }
    }
    // Observed successful process completion stays known when stdout cannot be
    // interpreted as operation output or its delivery is prohibited.
    let invalid = json!({"kind":"command","executable":"/bin/sh","arguments":[
        {"literal":"-c"},{"literal":"printf x >> \"$1\"; printf private-local-output"},
        {"literal":"fixture"},{"literal":counter}],"output_schema":schema,"timeout_seconds":5});
    let withheld = json!({"version":"mas-governed-unavailable/1","external_execution":"completed","payload_disposition":"withheld"});
    std::fs::write(&counter, b"").unwrap();
    let fresh = run_worker(
        &invalid,
        &counter,
        &withheld,
        false,
        false,
        Some(&json!({"status":"invalid_output"})),
    );
    assert_eq!(
        fresh,
        json!({"type":"error","message":"effect completed; result unavailable under current handling policy"})
    );
    assert_eq!(
        run_worker(&invalid, &counter, &withheld, true, false, None),
        fresh
    );
    assert_eq!(std::fs::read(&counter).unwrap(), b"x");
    std::fs::remove_dir_all(root).unwrap();
}
