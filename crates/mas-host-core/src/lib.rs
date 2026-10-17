//! Local execution above MAS's protocol authority.
//!
//! The protocol repository owns routes, tasks and call reservations. This crate
//! owns application checkpoints and uncertainty. No worker receives database
//! handles, executable selection or permission to choose its authenticated actor.
use mas_protocol_codec::{parse_json, store::Store};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, OpenOptions},
    io::Read,
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{SystemTime, UNIX_EPOCH},
};

mod effects;
mod execution;
mod journal;
mod observation;
mod protocol;
mod request_routes;
mod revision;
use journal::{journal, read_step, Step};
pub use observation::observe;
use protocol::{effect_evidence, finish_effect};

type Result<T> = std::result::Result<T, String>;
const MAX_ACTIVE: usize = 32;
const MAX_DEPTH: usize = 16;
const MAX_STEPS: i64 = 1024;
const CONTRACT: &str = "mas-host-local/1;mas-native/1";

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex
        .lock()
        .map_err(|_| "execution state lock poisoned".into())
}
fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(error)?
        .as_millis() as u64)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn checked_object(value: &Value, label: &str) -> Result<()> {
    if !value.is_object() {
        return Err(format!("{label} must be an object"));
    }
    parse_json(&value.to_string()).map_err(|e| format!("{label}: {}", e.message))?;
    Ok(())
}
fn fields(value: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    let object = value.as_object().ok_or("frame must be an object")?;
    if required.iter().any(|k| !object.contains_key(*k))
        || object
            .keys()
            .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return Err("missing or unknown field".into());
    }
    Ok(())
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("{key} must be a string"))
}
fn step_key(value: &Value) -> Result<&str> {
    let key = text(value, "key")?;
    if key.is_empty() || key.len() > 128 || key.chars().any(char::is_control) {
        return Err("step key must contain 1..128 bytes without control characters".into());
    }
    Ok(key)
}
fn run_directory(state: &Path, run_id: &str) -> Result<PathBuf> {
    if run_id.is_empty()
        || run_id.len() > 96
        || run_id == "."
        || run_id == ".."
        || !run_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(
            "run id must contain 1..96 ASCII letters, digits, dots, underscores or hyphens".into(),
        );
    }
    Ok(state.join(run_id))
}
fn ownership(directory: &Path) -> Result<File> {
    std::fs::create_dir_all(directory).map_err(error)?;
    if std::fs::symlink_metadata(directory)
        .map_err(error)?
        .file_type()
        .is_symlink()
    {
        return Err("run state directory cannot be a symlink".into());
    }
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700)).map_err(error)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("owner.lock"))
        .map_err(error)?;
    // The guardian inherits a descriptor for this same open-file description.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("run is owned by another host or a guardian still cleaning up".into());
    }
    Ok(file)
}

#[derive(Clone)]
struct Agent {
    executable: PathBuf,
    sha256: String,
    handler: Option<String>,
    working_directory: PathBuf,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
    operation: String,
    arguments: Vec<String>,
    configuration: Value,
    environment_names: BTreeSet<String>,
}

fn declared_environment(value: &Value) -> Result<BTreeSet<String>> {
    let names = value
        .as_array()
        .ok_or("environment must be an array of names")?
        .iter()
        .map(|v| {
            v.as_str()
                .ok_or_else(|| "environment names must be strings".to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(mas_host_process::environment_names(names)?
        .into_iter()
        .collect())
}

fn operation_schema(value: &Value, name: &str) -> Result<Option<Value>> {
    let Some(schema) = value.get(name) else {
        return Ok(None);
    };
    mas_host_schema::check(schema)?;
    if schema["type"] != "object" {
        return Err("operation schemas must have object type".into());
    }
    Ok(Some(schema.clone()))
}

impl Agent {
    fn environment(&self) -> Result<BTreeMap<String, String>> {
        let mut environment = BTreeMap::new();
        for name in &self.environment_names {
            match std::env::var(name) {
                Ok(value) => {
                    environment.insert(name.clone(), value);
                }
                Err(std::env::VarError::NotPresent) => {}
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err("declared environment must be Unicode".into())
                }
            }
        }
        Ok(environment)
    }
}

struct Inner {
    run_id: String,
    nonce: String,
    directory: PathBuf,
    agents: BTreeMap<String, Agent>,
    request_routes: BTreeSet<(String, String)>,
    deadline_ms: u64,
    journal: Mutex<Connection>,
    protocol: Mutex<Store>,
    owner: File,
    active: AtomicUsize,
    running: AtomicBool,
}

/// One trusted local owner. Workers inherit no authority beyond their channel.
pub struct Engine {
    inner: Arc<Inner>,
}

impl Engine {
    pub fn open(bundle: &Value, state_dir: &Path, run_id: &str) -> Result<Self> {
        checked_object(bundle, "bundle")?;
        fields(bundle, &["version", "agents", "topology"], &[])?;
        if bundle["version"] != "mas-bundle/3" {
            return Err("unsupported bundle version".into());
        }
        let mut agents = BTreeMap::new();
        let registrations = bundle["agents"]
            .as_array()
            .ok_or("bundle agents must be an array")?;
        if registrations.is_empty() || registrations.len() > 31 {
            return Err("register 1..31 application agents".into());
        }
        let mut verified_executables = BTreeSet::new();
        for value in registrations {
            fields(
                value,
                &[
                    "name",
                    "executable",
                    "sha256",
                    "operation",
                    "environment",
                    "working_directory",
                ],
                &[
                    "handler",
                    "configuration",
                    "arguments",
                    "input_schema",
                    "output_schema",
                ],
            )?;
            let name = text(value, "name")?;
            if name == "client" {
                return Err("client is reserved for host ingress".into());
            }
            let configuration = value
                .get("configuration")
                .cloned()
                .unwrap_or_else(|| json!({}));
            checked_object(&configuration, "agent configuration")?;
            let arguments = match value.get("arguments") {
                None => Vec::new(),
                Some(values) => values
                    .as_array()
                    .ok_or("worker arguments must be an array")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| "worker arguments must be strings".to_owned())
                    })
                    .collect::<Result<Vec<_>>>()?,
            };
            if arguments.len() > 32
                || arguments
                    .iter()
                    .any(|argument| argument.len() > 4096 || argument.contains('\0'))
            {
                return Err("worker arguments exceed supported bounds".into());
            }
            let environment_names = declared_environment(&value["environment"])?;
            let agent = Agent {
                executable: PathBuf::from(text(value, "executable")?),
                sha256: text(value, "sha256")?.to_owned(),
                handler: value
                    .get("handler")
                    .map(|v| {
                        v.as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .ok_or_else(|| "handler must be a nonempty string".to_string())
                    })
                    .transpose()?,
                working_directory: PathBuf::from(text(value, "working_directory")?),
                input_schema: operation_schema(value, "input_schema")?,
                output_schema: operation_schema(value, "output_schema")?,
                operation: text(value, "operation")?.to_owned(),
                configuration,
                arguments,
                environment_names,
            };
            if !agent.executable.is_absolute()
                || !agent.working_directory.is_absolute()
                || !agent.working_directory.is_dir()
                || agent.operation.is_empty()
            {
                return Err(
                    "worker executable and working_directory must be absolute; directory must exist and operation must be nonempty"
                        .into(),
                );
            }
            // Built-in actors share one host executable. Hash each identical
            // path/digest binding once during open; dispatch still rechecks it.
            if verified_executables.insert((agent.executable.clone(), agent.sha256.clone())) {
                verify_executable(&agent)?;
            }
            if agents.insert(name.to_owned(), agent).is_some() {
                return Err("duplicate agent name".into());
            }
        }
        let topology = &bundle["topology"];
        if topology["application"] != "agent-task/1" || !topology["edges"].is_array() {
            return Err("bundle requires explicit topology and agent-task/1".into());
        }
        let declared: BTreeSet<&str> = topology["agents"]
            .as_array()
            .ok_or("topology agents required")?
            .iter()
            .map(|v| v.as_str().ok_or("invalid topology actor"))
            .collect::<std::result::Result<_, _>>()?;
        let expected: BTreeSet<&str> = agents
            .keys()
            .map(String::as_str)
            .chain(std::iter::once("client"))
            .collect();
        if declared != expected {
            return Err("topology actors differ from registered agents".into());
        }
        let directory = run_directory(state_dir, run_id)?;
        let owner = ownership(&directory)?;
        let mut db = journal(&directory)?;
        let encoded = bundle.to_string();
        let host_binary = std::env::current_exe().map_err(error)?;
        let host_digest = digest(
            &std::fs::read(host_binary)
                .map_err(|_| "host executable unavailable for identity binding")?,
        );
        let fingerprint = digest(
            json!({"contract":CONTRACT,"host_sha256":host_digest,"bundle":bundle})
                .to_string()
                .as_bytes(),
        );
        let prior = db
            .query_row(
                "SELECT version,run_id,fingerprint,nonce FROM metadata WHERE singleton=1",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(error)?;
        let nonce = if let Some((version, id, old, nonce)) = prior {
            if version != 1 || id != run_id || old != fingerprint {
                return Err("retained run implementation identity differs".into());
            }
            nonce
        } else {
            let mut random = [0u8; 32];
            File::open("/dev/urandom")
                .and_then(|mut file| file.read_exact(&mut random))
                .map_err(error)?;
            let nonce = digest(&random);
            db.execute("INSERT INTO metadata(singleton,version,run_id,bundle,fingerprint,nonce) VALUES(1,1,?,?,?,?)",
                params![run_id,encoded,fingerprint,nonce]).map_err(error)?;
            nonce
        };
        let mut protocol = Store::open(directory.join("protocol.sqlite"))?;
        let snapshot = protocol.create_run(run_id, topology)?;
        let deadline_ms = (snapshot["deadline"]
            .as_f64()
            .ok_or("invalid protocol deadline")?
            * 1000.0)
            .floor() as u64;
        // Exclusive ownership implies all earlier application processes have
        // stopped. An old execute permission remains uncertain, never retryable.
        let tx = db.transaction().map_err(error)?;
        tx.execute(
            "UPDATE steps SET state='unknown' WHERE kind='effect' AND state='executing'",
            [],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(Self {
            inner: Arc::new(Inner {
                run_id: run_id.into(),
                nonce,
                directory,
                agents,
                request_routes: topology["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|edge| {
                        (
                            edge[0].as_str().unwrap().to_owned(),
                            edge[1].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect(),
                deadline_ms,
                journal: Mutex::new(db),
                protocol: Mutex::new(protocol),
                owner,
                active: AtomicUsize::new(0),
                running: AtomicBool::new(false),
            }),
        })
    }

    pub fn run(&self, target: &str, input: &Value) -> Result<Value> {
        if self.inner.running.swap(true, Ordering::SeqCst) {
            return Err("engine already running".into());
        }
        struct Running<'a>(&'a AtomicBool);
        impl Drop for Running<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _running = Running(&self.inner.running);
        self.inner.check_live()?;
        if self.inner.revision_mode()? {
            return Err("revision run must resume with its original revision plan".into());
        }
        self.inner.reconcile_completed_effects()?;
        if self.inner.has_unknown()? {
            return Err("uncertain effect requires evidence-backed resolution".into());
        }
        let output = self.inner.request("", "root", "client", target, input, 0)?;
        if self.inner.has_unsettled_effects()? {
            return Err("unsettled effect prevents successful completion".into());
        }
        self.inner.reconcile_completed_effects()?;
        self.inner.check_live()?;
        Ok(output)
    }

    pub fn status(&self) -> Result<Value> {
        snapshot(
            &self.inner.journal,
            &self.inner.protocol,
            &self.inner.run_id,
        )
    }
    pub fn state_directory(&self) -> &Path {
        &self.inner.directory
    }
}

fn verify_executable(agent: &Agent) -> Result<()> {
    if agent.sha256.len() != 64
        || !agent
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid executable SHA-256".into());
    }
    let bytes =
        std::fs::read(&agent.executable).map_err(|_| "registered executable unavailable")?;
    if digest(&bytes) != agent.sha256 {
        return Err("registered executable digest differs".into());
    }
    Ok(())
}

impl Inner {
    fn check_live(&self) -> Result<()> {
        let cancelled: i64 = lock(&self.journal)?
            .query_row(
                "SELECT cancelled FROM metadata WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if cancelled != 0 {
            return Err("run cancelled; effect uncertainty is retained".into());
        }
        if now_ms()? >= self.deadline_ms {
            lock(&self.protocol)?.reconcile(&self.run_id)?;
            return Err("original run deadline expired".into());
        }
        Ok(())
    }
    fn has_unknown(&self) -> Result<bool> {
        lock(&self.journal)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM steps WHERE kind='effect' AND state='unknown')",
                [],
                |r| r.get(0),
            )
            .map_err(error)
    }
    fn has_unsettled_effects(&self) -> Result<bool> {
        lock(&self.journal)?.query_row("SELECT EXISTS(SELECT 1 FROM steps WHERE kind='effect' AND state IN ('executing','unknown'))",[],|r|r.get(0)).map_err(error)
    }
    fn has_unsettled_below(&self, id: &str) -> Result<bool> {
        lock(&self.journal)?
            .query_row(
                "WITH RECURSIVE descendants(id) AS (
            SELECT ? UNION ALL SELECT s.id FROM steps s JOIN descendants d ON s.parent=d.id)
            SELECT EXISTS(SELECT 1 FROM steps WHERE id IN (SELECT id FROM descendants)
              AND kind='effect' AND state IN ('executing','unknown'))",
                [id],
                |r| r.get(0),
            )
            .map_err(error)
    }
    fn reconcile_completed_effects(&self) -> Result<()> {
        let effects = {
            let db = lock(&self.journal)?;
            let mut stmt = db.prepare("SELECT id FROM steps WHERE kind='effect' AND state IN ('completed','resolved') ORDER BY id").map_err(error)?;
            let ids = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(error)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(error)?;
            ids.iter()
                .map(|id| read_step(&db, id)?.ok_or("missing effect".into()))
                .collect::<Result<Vec<_>>>()?
        };
        let mut protocol = lock(&self.protocol)?;
        for step in effects {
            finish_effect(&mut protocol, &self.run_id, &step)?;
        }
        Ok(())
    }
    fn step_id(&self, parent: &str, key: &str, kind: &str) -> String {
        format!(
            "{}-{}",
            if kind == "effect" { "eff" } else { "task" },
            digest(
                json!([self.nonce, self.run_id, parent, key])
                    .to_string()
                    .as_bytes()
            )
        )
    }
    fn prepare(
        &self,
        parent: &str,
        key: &str,
        kind: &str,
        actor: &str,
        target: &str,
        input: &Value,
    ) -> Result<Step> {
        checked_object(
            input,
            if kind == "effect" {
                "effect descriptor"
            } else {
                "input"
            },
        )?;
        let id = self.step_id(parent, key, kind);
        let mut db = lock(&self.journal)?;
        let tx = db.transaction().map_err(error)?;
        if let Some(step) = read_step(&tx, &id)? {
            if step.kind != kind
                || step.actor != actor
                || step.target != target
                || step.input != *input
            {
                return Err("step key reused with different kind, actor, target or input".into());
            }
            return Ok(step);
        }
        // A key is shared across request/effect namespaces.
        let existing: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM steps WHERE parent=? AND key=?)",
                params![parent, key],
                |r| r.get(0),
            )
            .map_err(error)?;
        if existing {
            return Err("step key already belongs to another operation".into());
        }
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM steps", [], |r| r.get(0))
            .map_err(error)?;
        if count >= MAX_STEPS {
            return Err("run checkpoint limit exhausted".into());
        }
        tx.execute("INSERT INTO steps(id,parent,key,kind,actor,target,input,state) VALUES(?,?,?,?,?,?,?,'prepared')",
            params![id,parent,key,kind,actor,target,input.to_string()]).map_err(error)?;
        let step = read_step(&tx, &id)?.ok_or("checkpoint missing after insertion")?;
        tx.commit().map_err(error)?;
        Ok(step)
    }
    fn set_state(
        &self,
        id: &str,
        state: &str,
        output: Option<&Value>,
        evidence: Option<&Value>,
    ) -> Result<()> {
        lock(&self.journal)?
            .execute(
                "UPDATE steps SET state=?,output=?,evidence=? WHERE id=?",
                params![
                    state,
                    output.map(Value::to_string),
                    evidence.map(Value::to_string),
                    id
                ],
            )
            .map_err(error)?;
        Ok(())
    }

    // Serialise completion with cancellation intent in the same journal. A
    // cancellation committed before this transaction prevents durable success.
    fn commit_completed(&self, id: &str, output: &Value) -> Result<()> {
        let mut db = lock(&self.journal)?;
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(error)?;
        let cancelled: i64 = tx
            .query_row(
                "SELECT cancelled FROM metadata WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(error)?;
        if cancelled != 0 {
            return Err("run cancelled; effect uncertainty is retained".into());
        }
        if now_ms()? >= self.deadline_ms {
            return Err("original run deadline expired".into());
        }
        tx.execute(
            "UPDATE steps SET state='completed',output=?,evidence=NULL WHERE id=?",
            params![output.to_string(), id],
        )
        .map_err(error)?;
        tx.commit().map_err(error)
    }
}

fn bounded_error(value: &str) -> String {
    if value.trim().is_empty() {
        "worker failed".into()
    } else {
        value.chars().take(1024).collect()
    }
}
fn reply(result: Result<Value>) -> Value {
    match result {
        Ok(output) => json!({"type":"response","ok":true,"output":output}),
        Err(error) => json!({"type":"response","ok":false,"error":bounded_error(&error)}),
    }
}
fn snapshot(journal: &Mutex<Connection>, protocol: &Mutex<Store>, run_id: &str) -> Result<Value> {
    let db = lock(journal)?;
    let mut statement=db.prepare("SELECT id,parent,key,kind,actor,target,state,output,evidence FROM steps ORDER BY rowid").map_err(error)?;
    let steps = statement
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, Option<String>>(8)?,
            ))
        })
        .map_err(error)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(error)?;
    let cancelled: i64 = db
        .query_row(
            "SELECT cancelled FROM metadata WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(error)?;
    let revision = revision::audit(&db)?;
    drop(statement);
    drop(db);
    let steps=steps.into_iter().map(|r|Ok(json!({"id":r.0,"parent":r.1,"key":r.2,"kind":r.3,"actor":r.4,"target":r.5,"state":r.6,
        "output":r.7.map(|v|serde_json::from_str::<Value>(&v)).transpose().map_err(error)?,
        "evidence":r.8.map(|v|serde_json::from_str::<Value>(&v)).transpose().map_err(error)?}))).collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"version":"mas-host-inspection/1","run_id":run_id,"cancelled":cancelled!=0,
        "steps":steps,"protocol":lock(protocol)?.inspect(run_id)?,"revision":revision}),
    )
}

/// Read retained evidence without requiring the original application binaries.
pub fn inspect(state_dir: &Path, run_id: &str) -> Result<Value> {
    let directory = run_directory(state_dir, run_id)?;
    if !directory.join("execution.sqlite").is_file() {
        return Err("unknown run".into());
    }
    snapshot(
        &Mutex::new(journal(&directory)?),
        &Mutex::new(Store::open(directory.join("protocol.sqlite"))?),
        run_id,
    )
}

/// Record an operator assertion establishing an uncertain effect's result.
/// This cannot establish the truth of evidence, refund admission or extend time.
pub fn resolve(
    state_dir: &Path,
    run_id: &str,
    effect_id: &str,
    output: &Value,
    evidence: &Value,
) -> Result<Value> {
    checked_object(output, "resolved output")?;
    checked_object(evidence, "resolution evidence")?;
    if evidence.as_object().unwrap().is_empty() {
        return Err("resolution requires non-empty evidence".into());
    }
    let directory = run_directory(state_dir, run_id)?;
    if !directory.join("execution.sqlite").is_file() {
        return Err("unknown run".into());
    }
    let _owner = ownership(&directory)?;
    let db = journal(&directory)?;
    let mut step = read_step(&db, effect_id)?.ok_or("unknown effect")?;
    if step.kind != "effect" {
        return Err("only effects can be resolved".into());
    }
    if step.state == "resolved" {
        if step.output.as_ref() != Some(output) || step.evidence.as_ref() != Some(evidence) {
            return Err("resolved output cannot be overwritten".into());
        }
    } else {
        if !matches!(step.state.as_str(), "unknown" | "executing") {
            return Err("effect is not uncertain".into());
        }
        step.state = "resolved".into();
        step.output = Some(output.clone());
        step.evidence = Some(evidence.clone());
        effect_evidence(&step)?;
        // The reservation must exist before a resolution becomes durable.
        let snapshot = Store::open(directory.join("protocol.sqlite"))?.inspect(run_id)?;
        if !snapshot["calls"].as_array().is_some_and(|calls| {
            calls
                .iter()
                .any(|c| c["id"] == effect_id && c["actor"] == step.actor)
        }) {
            return Err("effect has no matching protocol reservation".into());
        }
        db.execute(
            "UPDATE steps SET state='resolved',output=?,evidence=? WHERE id=?",
            params![output.to_string(), evidence.to_string(), effect_id],
        )
        .map_err(error)?;
    }
    finish_effect(
        &mut Store::open(directory.join("protocol.sqlite"))?,
        run_id,
        &step,
    )?;
    Ok(json!({"resolved":true,"effect_id":effect_id,"output":output}))
}

/// Persist cancellation intent. A running worker observes this at its next host
/// interaction; the original deadline remains its hard process lifetime bound.
pub fn cancel(state_dir: &Path, run_id: &str) -> Result<Value> {
    let directory = run_directory(state_dir, run_id)?;
    if !directory.join("execution.sqlite").is_file() {
        return Err("unknown run".into());
    }
    let db = journal(&directory)?;
    db.execute("UPDATE metadata SET cancelled=1 WHERE singleton=1", [])
        .map_err(error)?;
    Ok(
        json!({"cancelled":true,"run_id":run_id,"semantics":"intent; worker stops at next interaction or original deadline"}),
    )
}

#[cfg(test)]
mod tests;
