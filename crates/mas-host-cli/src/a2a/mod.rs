//! Optional A2A service boundary. The protocol and worker wire formats are unchanged.
mod configuration;
mod governed;
mod http;
use configuration::Configuration;
use governed::Caller;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::fs::{OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

type Result<T> = std::result::Result<T, String>;
pub(super) const PROFILE: &str = "urn:mas:host-task-service:1";
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn response_id(request: &Value) -> Value {
    request
        .get("id")
        .filter(|v| v.is_string() || v.is_i64() || v.is_u64())
        .cloned()
        .unwrap_or(Value::Null)
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn fields(v: &Value, required: &[&str], optional: &[&str]) -> Result<()> {
    crate::config::fields(v, required, optional, "A2A request")
}
fn text(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 1024)
        .map(str::to_owned)
        .ok_or_else(|| format!("invalid {key}"))
}
#[derive(Debug)]
struct RpcError(i64, &'static str);
impl From<String> for RpcError {
    fn from(_: String) -> Self {
        Self(-32602, "invalid_parameters")
    }
}
type RpcResult<T> = std::result::Result<T, RpcError>;

struct Service {
    configuration: Configuration,
    bundle: Value,
    descriptor: Value,
    state: PathBuf,
    db: Mutex<Connection>,
    active: AtomicUsize,
    _owner: File,
}
impl Service {
    fn open(bundle: Value, state: &Path, config: Value) -> Result<Arc<Self>> {
        let descriptor = mas_host_core::describe(&bundle)?;
        let configuration = Configuration::parse(&config, &descriptor, &bundle)?;
        std::fs::create_dir_all(state).map_err(error)?;
        if std::fs::symlink_metadata(state)
            .map_err(error)?
            .file_type()
            .is_symlink()
        {
            return Err("A2A state must not be a symlink".into());
        }
        std::fs::set_permissions(state, std::fs::Permissions::from_mode(0o700)).map_err(error)?;
        let owner = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(state.join("service.lock"))
            .map_err(error)?;
        if unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("A2A service state already owned".into());
        }
        let db_path = state.join("service.sqlite");
        if std::fs::symlink_metadata(&db_path).is_ok_and(|v| v.file_type().is_symlink()) {
            return Err("A2A database cannot be a symlink".into());
        }
        // Create privately before SQLite opens it; the parent protects WAL/SHM files.
        OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&db_path)
            .map_err(error)?;
        let mut db = Connection::open(db_path).map_err(error)?;
        db.busy_timeout(Duration::from_secs(2)).map_err(error)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS metadata(singleton INTEGER PRIMARY KEY CHECK(singleton=1), fingerprint TEXT NOT NULL); CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY,principal TEXT NOT NULL,message_id TEXT NOT NULL,content TEXT NOT NULL,agent TEXT NOT NULL,context TEXT NOT NULL,state TEXT NOT NULL,output TEXT,error TEXT,cancelled INTEGER NOT NULL DEFAULT 0,UNIQUE(principal,message_id));").map_err(error)?;
        if configuration.governed.is_some() {
            db.execute_batch("CREATE TABLE IF NOT EXISTS governed_tasks(id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL)").map_err(error)?;
        }
        let binary = std::fs::read(std::env::current_exe().map_err(error)?).map_err(error)?;
        let fingerprint = digest(
            json!({"version":config["version"],"config":config,"bundle":bundle,
            "governance":configuration.governed.as_ref().map(|g|g.identity()),
            "host_digest":digest(&binary)})
            .to_string()
            .as_bytes(),
        );
        let tx = db.transaction().map_err(error)?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM metadata WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(error)?;
        if let Some(prior) = prior {
            if prior != fingerprint {
                return Err("retained A2A service identity differs; use an explicit migration, never relabel outstanding work".into());
            }
        } else {
            tx.execute("INSERT INTO metadata VALUES(1,?)", [fingerprint])
                .map_err(error)?;
        }
        // No automatic replay: a crash can precede or follow an external effect.
        tx.execute(
            "UPDATE tasks SET state='unknown' WHERE state IN ('submitted','working')",
            [],
        )
        .map_err(error)?;
        tx.commit().map_err(error)?;
        Ok(Arc::new(Self {
            configuration,
            bundle,
            descriptor,
            state: state.to_owned(),
            db: Mutex::new(db),
            active: AtomicUsize::new(0),
            _owner: owner,
        }))
    }
    fn task(&self, principal: &Caller, id: &str) -> RpcResult<Value> {
        principal
            .current()
            .map_err(|_| RpcError(-32001, "task_not_found"))?;
        let db = self
            .db
            .lock()
            .map_err(|_| RpcError(-32603, "service_unavailable"))?;
        let row = db
            .query_row(
                "SELECT agent,context,state,output,cancelled FROM tasks WHERE id=? AND principal=?",
                params![id, principal.owner],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, bool>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?
            .ok_or(RpcError(-32001, "task_not_found"))?;
        if !principal.agents.contains(&row.0) {
            return Err(RpcError(-32001, "task_not_found"));
        }
        if let Some(session) = &principal.session {
            let fingerprint: String = db
                .query_row(
                    "SELECT fingerprint FROM governed_tasks WHERE id=?",
                    [id],
                    |r| r.get(0),
                )
                .map_err(|_| RpcError(-32001, "task_not_found"))?;
            if fingerprint != session.fingerprint() {
                return Err(RpcError(-32001, "task_not_found"));
            }
        }
        let wire = match row.2.as_str() {
            "submitted" => "TASK_STATE_SUBMITTED",
            "working" => "TASK_STATE_WORKING",
            "completed" => "TASK_STATE_COMPLETED",
            "failed" => "TASK_STATE_FAILED",
            _ => "TASK_STATE_UNSPECIFIED",
        };
        let mut task = json!({"id":id,"contextId":row.1,"status":{"state":wire},"metadata":{"mas":{"state":row.2,"cancelRequested":row.4,"bundleDigest":self.descriptor["bundle_digest"],"agent":row.0}}});
        if let Some(output) = row.3 {
            let output = mas_protocol_codec::parse_json(&output)
                .map_err(|_| RpcError(-32603, "retained_output_invalid"))?;
            principal
                .delivery(&row.0, &output)
                .map_err(|_| RpcError(-32001, "task_not_found"))?;
            task["artifacts"] = json!([{"artifactId":format!("{id}-result"),"name":"result","parts":[{"data":output,"mediaType":"application/json"}]}]);
        }
        if let Some(session) = &principal.session {
            session
                .check_status_value(&row.0, &task)
                .map_err(|_| RpcError(-32001, "task_not_found"))?;
        }
        Ok(task)
    }
    fn submit(self: &Arc<Self>, principal: &Caller, params: &Value) -> RpcResult<Value> {
        fields(params, &["message"], &["configuration"])?;
        let message = &params["message"];
        fields(message, &["messageId", "role", "parts"], &["contextId"])?;
        let message_id = principal
            .message_key(&text(message, "messageId")?)
            .map_err(|_| RpcError(-32603, "handling_unavailable"))?;
        if message["role"] != "ROLE_USER" {
            return Err(RpcError(-32602, "invalid_message_role"));
        }
        let parts = message["parts"]
            .as_array()
            .filter(|p| p.len() == 1)
            .ok_or(RpcError(-32005, "one_json_data_part_required"))?;
        fields(&parts[0], &["data"], &["mediaType"])?;
        if parts[0]
            .get("mediaType")
            .is_some_and(|v| v != "application/json")
        {
            return Err(RpcError(-32005, "content_type_not_supported"));
        }
        let data = &parts[0]["data"];
        fields(data, &["agent", "input", "descriptorDigest"], &[])?;
        let agent = text(data, "agent")?;
        if !principal.agents.contains(&agent) {
            return Err(RpcError(-32001, "export_not_found"));
        }
        if data["descriptorDigest"] != self.descriptor["bundle_digest"] {
            return Err(RpcError(-32602, "descriptor_identity_conflict"));
        }
        if !data["input"].is_object() {
            return Err(RpcError(-32602, "input_object_required"));
        }
        let contract = self.descriptor["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == agent)
            .unwrap();
        let input = principal
            .input(&agent, &data["input"])
            .map_err(|_| RpcError(-32602, "input_withheld"))?;
        mas_host_schema::validate(&contract["input_schema"], &input)
            .map_err(|_| RpcError(-32602, "input_contract_violation"))?;
        if let Some(configuration) = params.get("configuration") {
            fields(
                configuration,
                &[],
                &["returnImmediately", "acceptedOutputModes"],
            )?;
            if configuration
                .get("returnImmediately")
                .is_some_and(|v| !v.is_boolean())
            {
                return Err(RpcError(-32602, "invalid_execution_mode"));
            }
            if configuration.get("acceptedOutputModes").is_some_and(|v| {
                !v.as_array().is_some_and(|a| {
                    a.iter().all(Value::is_string) && a.iter().any(|v| v == "application/json")
                })
            }) {
                return Err(RpcError(-32005, "content_type_not_supported"));
            }
        }
        let content = principal
            .content(params)
            .map_err(|_| RpcError(-32603, "handling_unavailable"))?;
        let id = format!(
            "a2a-{}",
            &digest(json!([principal.owner, message_id]).to_string().as_bytes())[..56]
        );
        let context = match message.get("contextId") {
            None => format!("context-{id}"),
            Some(_) if principal.session.is_none() => text(message, "contextId")?,
            Some(_) => {
                text(message, "contextId")?;
                format!("context-{id}")
            }
        };
        let mut db = self
            .db
            .lock()
            .map_err(|_| RpcError(-32603, "service_unavailable"))?;
        let tx = db
            .transaction()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        let prior: Option<String> = tx
            .query_row(
                "SELECT content FROM tasks WHERE principal=? AND message_id=?",
                params![principal.owner, message_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        if let Some(prior) = prior {
            if prior != content {
                return Err(RpcError(-32602, "request_identity_conflict"));
            }
            drop(tx);
            drop(db);
            return Ok(json!({"task":self.task(principal,&id)?}));
        }
        let count: i64 = tx
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        if count >= self.configuration.max_tasks as i64
            || self.active.load(Ordering::SeqCst) >= self.configuration.max_active
        {
            return Err(RpcError(-32603, "service_capacity_exhausted"));
        }
        principal
            .current()
            .map_err(|_| RpcError(-32603, "authority_unavailable"))?;
        tx.execute("INSERT INTO tasks(id,principal,message_id,content,agent,context,state) VALUES(?,?,?,?,?,?,'submitted')",params![id,principal.owner,message_id,content,agent,context]).map_err(|_|RpcError(-32603,"storage_unavailable"))?;
        if let Some(session) = &principal.session {
            tx.execute(
                "INSERT INTO governed_tasks(id,fingerprint) VALUES(?,?)",
                params![id, session.fingerprint()],
            )
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        }
        tx.commit()
            .map_err(|_| RpcError(-32603, "admission_outcome_unknown_repeat_original_message"))?;
        self.active.fetch_add(1, Ordering::SeqCst);
        drop(db);
        let service = Arc::clone(self);
        let run_id = id.clone();
        let caller = principal.clone();
        // Failure to create a thread retains submitted work. Restart marks it
        // unknown; identical requests never take the dispatch path again.
        if std::thread::Builder::new()
            .name("a2a-task".into())
            .spawn(move || service.execute(&run_id, &agent, &input, &caller))
            .is_err()
        {
            self.active.fetch_sub(1, Ordering::SeqCst);
            if let Ok(db) = self.db.lock() {
                let _ = db.execute("UPDATE tasks SET state='unknown' WHERE id=?", [&id]);
            }
        }
        Ok(json!({"task":self.task(principal,&id)?}))
    }
    fn execute(&self, id: &str, agent: &str, input: &Value, caller: &Caller) {
        struct Active<'a>(&'a AtomicUsize);
        impl Drop for Active<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _active = Active(&self.active);
        let run_state = self.state.join("runs");
        let cancelled = match self.db.lock() {
            Ok(db) => match db.query_row("SELECT cancelled FROM tasks WHERE id=?", [id], |r| {
                r.get::<_, bool>(0)
            }) {
                Ok(v) => v,
                Err(_) => return,
            },
            Err(_) => return,
        };
        if cancelled {
            if let Ok(db) = self.db.lock() {
                let _ = db.execute("UPDATE tasks SET state='failed' WHERE id=?", [id]);
            }
            return;
        }
        if let Ok(db) = self.db.lock() {
            if db
                .execute("UPDATE tasks SET state='working' WHERE id=?", [id])
                .is_err()
            {
                return;
            }
        } else {
            return;
        }
        let engine = match &caller.session {
            Some(session) => {
                mas_host_core::Engine::open_governed(&self.bundle, &run_state, id, session.clone())
            }
            None => mas_host_core::Engine::open(&self.bundle, &run_state, id),
        };
        let outcome = engine.and_then(|engine| {
            caller.current()?;
            // A cancellation may have arrived between the service transaction and
            // journal creation. Transfer the durable intent before execution.
            let cancelled = self
                .db
                .lock()
                .map_err(|_| "service lock poisoned".to_string())?
                .query_row("SELECT cancelled FROM tasks WHERE id=?", [id], |r| {
                    r.get::<_, bool>(0)
                })
                .map_err(error)?;
            if cancelled {
                mas_host_core::cancel(&run_state, id)?;
            }
            engine.run(agent, input)
        });
        // Do not infer no external effect from any error, deadline or lost reply.
        // Unknown remains explicitly operator-reconcilable in the core journal.
        if let Ok(db) = self.db.lock() {
            match outcome {
                Ok(output) => {
                    let _=db.execute("UPDATE tasks SET state=CASE WHEN cancelled=0 THEN 'completed' ELSE 'unknown' END,output=CASE WHEN cancelled=0 THEN ? ELSE NULL END WHERE id=?",params![output.to_string(),id]);
                }
                Err(error) => {
                    let diagnostic: String = if caller.session.is_some() {
                        "governed_execution_unavailable".into()
                    } else {
                        error.chars().take(2048).collect()
                    };
                    let _ = db.execute(
                        "UPDATE tasks SET state='unknown',error=? WHERE id=?",
                        params![diagnostic, id],
                    );
                }
            }
        }
    }
    fn cancel(&self, principal: &Caller, params: &Value) -> RpcResult<Value> {
        principal
            .current()
            .map_err(|_| RpcError(-32001, "task_not_found"))?;
        fields(params, &["id"], &[])?;
        let id = text(params, "id")?;
        self.task(principal, &id)?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        let tx = db
            .transaction()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM tasks WHERE id=? AND principal=?",
                params![id, principal.owner],
                |r| r.get(0),
            )
            .optional()
            .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        let state = state.ok_or(RpcError(-32001, "task_not_found"))?;
        if matches!(state.as_str(), "completed" | "failed") {
            return Err(RpcError(-32002, "task_not_cancelable"));
        }
        tx.execute(
            "UPDATE tasks SET cancelled=1 WHERE id=? AND principal=?",
            params![id, principal.owner],
        )
        .map_err(|_| RpcError(-32603, "storage_unavailable"))?;
        tx.commit()
            .map_err(|_| RpcError(-32603, "cancellation_outcome_unknown"))?;
        drop(db);
        // The task thread repeats this transfer after creating the core journal.
        let _ = mas_host_core::cancel(&self.state.join("runs"), &id);
        Err(RpcError(
            -32002,
            "cancellation_intent_recorded_poll_original_task",
        ))
    }
    fn rpc(self: &Arc<Self>, principal: &Caller, request: &Value, extension: bool) -> Value {
        let id = if principal.session.is_some() && !governed::rpc_id_allowed(&request["id"]) {
            Value::Null
        } else {
            response_id(request)
        };
        let result = (|| -> RpcResult<Value> {
            principal
                .current()
                .map_err(|_| RpcError(-32603, "authority_unavailable"))?;
            fields(request, &["jsonrpc", "id", "method"], &["params"])
                .map_err(|_| RpcError(-32600, "invalid_request"))?;
            if request["jsonrpc"] != "2.0" || !(id.is_string() || id.is_i64() || id.is_u64()) {
                return Err(RpcError(-32600, "invalid_request"));
            }
            let method = request["method"]
                .as_str()
                .ok_or(RpcError(-32600, "invalid_request"))?;
            let empty = json!({});
            let params = request.get("params").unwrap_or(&empty);
            if method == "GetExtendedAgentCard" {
                fields(params, &[], &[])?;
                return Ok(self.configuration.card(&self.descriptor, Some(principal)));
            }
            if !extension {
                return Err(RpcError(-32008, "required_extension_missing"));
            }
            match method {
                "SendMessage" => self.submit(principal, params),
                "GetTask" => {
                    fields(params, &["id"], &["historyLength"])?;
                    if params.get("historyLength").is_some_and(|v| !v.is_u64()) {
                        return Err(RpcError(-32602, "invalid_history_length"));
                    }
                    self.task(principal, &text(params, "id")?)
                }
                "CancelTask" => self.cancel(principal, params),
                "CreateTaskPushNotificationConfig"
                | "GetTaskPushNotificationConfig"
                | "ListTaskPushNotificationConfigs"
                | "DeleteTaskPushNotificationConfig" => {
                    Err(RpcError(-32003, "push_notifications_not_supported"))
                }
                "SendStreamingMessage" | "SubscribeToTask" | "ListTasks" => {
                    Err(RpcError(-32004, "operation_not_supported"))
                }
                _ => Err(RpcError(-32601, "method_not_found")),
            }
        })();
        match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(RpcError(code, message)) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
            }
        }
    }
}
pub fn serve(bundle: Value, state: &Path, config: Value) -> Result<Value> {
    let service = Service::open(bundle, state, config)?;
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(32)
        .enable_all()
        .build()
        .map_err(error)?
        .block_on(http::serve(service))?;
    Ok(json!({"status":"stopped"}))
}
