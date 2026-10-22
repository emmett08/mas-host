//! Exercise local privacy inspection under the actual guardian with faulty workers.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
}
impl Fixture {
    fn new(mode: &str, text: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "mas-privacy-cli-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let code = format!(
            r#"import json,os,socket,struct,time
s=socket.socket(fileno=int(os.environ['MAS_WORKER_CHANNEL_FD']))
def read(n):
    out=b''
    while len(out)<n:
        chunk=s.recv(n-len(out))
        if not chunk: raise RuntimeError('EOF')
        out+=chunk
    return out
r=json.loads(read(struct.unpack('!I',read(4))[0]))
mode={mode:?}
if mode=='wait':
    time.sleep(20)
if mode=='descendant':
    child=os.fork()
    if child==0:
        time.sleep(20)
        os._exit(0)
    open('child.pid','w').write(str(child))
print(r['text'],flush=True)
os.write(2,r['text'].encode())
v={{'version':'mas-privacy-worker-response/1','source_id':r['source_id'],'source_token':r['source_token'],'input_bytes':r['input_bytes'],'detector':r['detector'],'offset_unit':'unicode-codepoints','completion':'complete','checked_ranges':[{{'start':0,'end':len(r['text'])}}] if r['text'] else [],'omissions':[],'findings':[]}}
if mode=='wrong-source': v['source_token']='0'*64
if mode=='truncated': v['checked_ranges']=[{{'start':0,'end':1}}]
if mode=='bad-units': v['offset_unit']='utf16-code-units'
if mode=='bad-json':
    b=b'{{"raw":"private","raw":"value"}}'
else:
    b=json.dumps(v).encode()
s.sendall(struct.pack('!I',len(b))+b)
"#
        );
        fs::write(root.join("worker.py"), &code).unwrap();
        let hash: String = Sha256::digest(code.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let manifest = json!({"version":"mas-privacy-bundle/1","code_revision":"a".repeat(40),"model_revision":"b".repeat(40),"backend":"opf-cpu-bfloat16/1","adapter":"mas-privacy-opf/1","worker_sha256":hash,"adapter_files":{},"files":{},"runtime":{},"settings":{"max_input_bytes":16384,"max_response_bytes":65536,"max_findings":128}});
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        fs::write(root.join("input.txt"), text).unwrap();
        Self { root }
    }
    fn command(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_mas-host"));
        c.args([
            "privacy-inspect",
            "--python",
            "/usr/bin/python3",
            "--timeout-ms",
            "5000",
        ])
        .arg("--input")
        .arg(self.root.join("input.txt"))
        .arg("--worker")
        .arg(self.root.join("worker.py"))
        .arg("--manifest")
        .arg(self.root.join("manifest.json"))
        .arg("--assets")
        .arg(&self.root);
        c
    }
    fn result(&self) -> Value {
        let out = self.command().output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        assert!(!String::from_utf8_lossy(&out.stdout).contains("alice@example.invalid"));
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn valid_local_scan_never_admits_release_and_hides_worker_output() {
    let f = Fixture::new("complete", "é alice@example.invalid");
    let v = f.result();
    assert_eq!(v["completion"], "complete");
    assert_eq!(v["finding_count"], 1);
    assert_eq!(v["disposition"], "withhold");
    assert_eq!(v["release_permitted"], false);
    assert_eq!(v["quality_qualified"], false);
    assert!(v.get("restricted_assessment").is_none());
    let empty = Fixture::new("complete", "").result();
    assert_eq!(empty["completion"], "complete");
    assert_eq!(empty["disposition"], "hold");
}

#[test]
fn faulty_neural_results_preserve_deterministic_positive_findings() {
    for mode in ["wrong-source", "truncated", "bad-units", "bad-json"] {
        let v = Fixture::new(mode, "alice@example.invalid").result();
        assert_eq!(v["completion"], "failed", "{mode}: {v}");
        assert_eq!(v["finding_count"], 1);
        assert_eq!(v["disposition"], "withhold");
    }
}

#[test]
fn timeout_and_large_encoded_requests_do_not_release_content() {
    let f = Fixture::new("wait", "plain");
    let c = f.command();
    // Replace the test command's fixed timeout rather than append a duplicate.
    let args: Vec<_> = c.get_args().map(|s| s.to_owned()).collect();
    let mut fast = Command::new(env!("CARGO_BIN_EXE_mas-host"));
    let mut args = args;
    let i = args.iter().position(|a| a == "--timeout-ms").unwrap();
    args[i + 1] = "40".into();
    fast.args(args);
    let before = Instant::now();
    let out = fast.output().unwrap();
    assert!(before.elapsed() < Duration::from_secs(3));
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["completion"], "failed");
    let large = Fixture::new("complete", &"\u{1}".repeat(16_384)).result();
    assert_eq!(large["completion"], "failed");
    assert_eq!(large["release_permitted"], false);
}

#[test]
fn guardian_reaps_classifier_descendants_before_returning_assessment() {
    let f = Fixture::new("descendant", "plain");
    let v = f.result();
    assert_eq!(v["completion"], "complete");
    let pid: i32 = fs::read_to_string(f.root.join("child.pid"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}

#[test]
fn changed_worker_is_rejected_before_execution() {
    let f = Fixture::new("complete", "alice@example.invalid");
    fs::write(
        f.root.join("worker.py"),
        "raise RuntimeError('alice@example.invalid')",
    )
    .unwrap();
    let out = f.command().output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("identity mismatch"));
    assert!(!error.contains("alice@example.invalid"));
}

#[test]
fn invalid_encoding_and_oversized_source_have_explicit_safe_outcomes() {
    let f = Fixture::new("complete", "plain");
    fs::write(f.root.join("input.txt"), [0xff, 0xfe]).unwrap();
    let v = f.result();
    assert_eq!(v["version"], "mas-privacy-inspection/1");
    assert_eq!(v["completion"], "unsupported");
    assert_eq!(v["release_permitted"], false);
    fs::write(f.root.join("input.txt"), vec![b'x'; 16_385]).unwrap();
    let out = f.command().output().unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("byte limit"));
}
