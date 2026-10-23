//! Opt-in exact-boundary custody. This owner stores payloads, never work items.
//! Ciphertext and per-case keys have independent storage/backup ownership.
use super::{
    exposure::{self, Exposure},
    source::Source,
    GovernedSession,
};
use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use std::{
    fs::OpenOptions,
    io::Read,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use zeroize::Zeroizing;
type Result<T> = std::result::Result<T, String>;
const UNAVAILABLE: &str = "governed-source-quarantine-unavailable";
const VERSION: &str = "mas-governed-source-quarantine/1";
// Hex ciphertext plus the complete authenticated envelope must fit the same
// 65,536-byte canonical commitment/wire bound as other governed records.
const MAX_PLAIN: usize = 30_000;
const MAX_ENVELOPE: usize = 65_536;

#[derive(Clone)]
pub(super) struct Configuration {
    payload_file: PathBuf,
    control_file: PathBuf,
    max_cases: usize,
    retention_ms: u64,
}
pub(super) struct Namespace {
    pub directory: PathBuf,
    pub run: String,
}

impl Configuration {
    pub(super) fn parse(value: &Value) -> Result<Self> {
        crate::fields(
            value,
            &[
                "version",
                "payload_file",
                "control_file",
                "max_cases",
                "retention_ms",
            ],
            &[],
        )?;
        if value["version"] != VERSION {
            return Err(UNAVAILABLE.into());
        }
        let path = |name: &str| -> Result<PathBuf> {
            let path = PathBuf::from(
                value[name]
                    .as_str()
                    .filter(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
                    .ok_or(UNAVAILABLE)?,
            );
            if !path.is_absolute()
                || path.file_name().is_none()
                || path
                    .components()
                    .any(|p| matches!(p, std::path::Component::ParentDir))
            {
                return Err(UNAVAILABLE.into());
            }
            Ok(path)
        };
        let payload_file = path("payload_file")?;
        let control_file = path("control_file")?;
        if payload_file.parent() == control_file.parent() {
            return Err("quarantine-key-control-must-have-independent-directory".into());
        }
        let max_cases = value["max_cases"]
            .as_u64()
            .filter(|n| (1..=4096).contains(n))
            .ok_or(UNAVAILABLE)? as usize;
        let retention_ms = value["retention_ms"]
            .as_u64()
            .filter(|n| (1..=604_800_000).contains(n))
            .ok_or(UNAVAILABLE)?;
        Ok(Self {
            payload_file,
            control_file,
            max_cases,
            retention_ms,
        })
    }
    fn independent(&self) -> Result<()> {
        let parent = |path: &Path| -> Result<PathBuf> {
            let dir = path.parent().ok_or(UNAVAILABLE)?;
            let metadata = std::fs::metadata(dir).map_err(|_| UNAVAILABLE)?;
            if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
                return Err("quarantine-directories-require-private-permissions".into());
            }
            std::fs::canonicalize(dir).map_err(|_| UNAVAILABLE.into())
        };
        if parent(&self.payload_file)? == parent(&self.control_file)? {
            return Err(UNAVAILABLE.into());
        }
        Ok(())
    }
}
fn config(session: &GovernedSession) -> Result<&Configuration> {
    session.0.quarantine.as_ref().ok_or(UNAVAILABLE.into())
}
fn sql_time(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| UNAVAILABLE.into())
}
fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| UNAVAILABLE)?;
    Ok(bytes)
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str, maximum: usize) -> Result<Vec<u8>> {
    if text.len() % 2 != 0 || text.len() > maximum * 2 {
        return Err(UNAVAILABLE.into());
    }
    text.as_bytes()
        .chunks_exact(2)
        .map(|p| {
            let digit = |b: u8| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(UNAVAILABLE.to_owned()),
            };
            Ok(digit(p[0])? * 16 + digit(p[1])?)
        })
        .collect()
}
fn open(path: &Path, write: bool) -> Result<Connection> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| UNAVAILABLE)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(UNAVAILABLE.into());
    }
    let db = Connection::open_with_flags(
        path,
        (if write {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        }) | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(|_| UNAVAILABLE)?;
    db.busy_timeout(Duration::from_secs(2))
        .map_err(|_| UNAVAILABLE)?;
    if write {
        db.execute_batch("PRAGMA secure_delete=ON; PRAGMA synchronous=EXTRA;")
            .map_err(|_| UNAVAILABLE)?;
    }
    Ok(db)
}
fn epoch(db: &Connection) -> Result<String> {
    db.query_row(
        "SELECT CASE WHEN length(CAST(epoch AS BLOB))=64 THEN epoch END FROM quarantine_meta WHERE singleton=1 AND version=?",
        [VERSION],
        |r| r.get(0),
    )
    .map_err(|_| UNAVAILABLE.into())
}
fn owner(session: &GovernedSession) -> Result<String> {
    session.identity_commitment(&json!(["quarantine-owner/1"]))
}
fn namespace_scope(session: &GovernedSession, identity: &Value) -> Result<String> {
    session.commitment(&json!(["quarantine-namespace/1", identity]))
}
fn namespace_identity(session: &GovernedSession) -> Result<Value> {
    match &session.3 {
        Some(n) => {
            let identity = super::lifecycle::custody_identity(&n.directory, session)?;
            if identity["run"] != n.run {
                return Err(UNAVAILABLE.into());
            }
            Ok(identity)
        }
        None => Ok(Value::Null),
    }
}
fn admin(session: &GovernedSession, actor: &str, operation: &str, query: &Value) -> Result<()> {
    session.agent(actor)?;
    session
        .authorize(
            actor,
            operation,
            "host-source-quarantine",
            "quarantine-control",
            query,
            query,
        )
        .map(|_| ())
}

/// Explicit fresh-store creation. Missing ledgers are never recreated by reads,
/// inspection or retirement. An interrupted initialisation fails closed.
pub fn initialize_governed_source_quarantine(
    session: &GovernedSession,
    actor: &str,
) -> Result<Value> {
    let cfg = config(session)?;
    cfg.independent()?;
    admin(
        session,
        actor,
        "quarantine-admin",
        &json!({"action":"initialize"}),
    )?;
    for path in [&cfg.control_file, &cfg.payload_file] {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .map_err(|_| UNAVAILABLE)?
            .sync_all()
            .map_err(|_| UNAVAILABLE)?;
    }
    let id = hex(&random::<32>()?);
    for path in [&cfg.control_file, &cfg.payload_file] {
        let db = open(path, true)?;
        db.execute_batch("PRAGMA journal_mode=DELETE; CREATE TABLE quarantine_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),version TEXT NOT NULL,epoch TEXT NOT NULL);").map_err(|_|UNAVAILABLE)?;
        db.execute(
            "INSERT INTO quarantine_meta VALUES(1,?,?)",
            params![VERSION, id],
        )
        .map_err(|_| UNAVAILABLE)?;
    }
    open(&cfg.control_file,true)?.execute_batch("CREATE TABLE quarantine_keys(id TEXT PRIMARY KEY,owner TEXT NOT NULL,profile TEXT NOT NULL,actor TEXT NOT NULL,scope TEXT NOT NULL,expires INTEGER NOT NULL,held INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL,key BLOB,binding TEXT NOT NULL,namespace TEXT NOT NULL); CREATE TABLE quarantine_review_keys(id TEXT PRIMARY KEY,case_id TEXT NOT NULL,key BLOB,binding TEXT NOT NULL);").map_err(|_|UNAVAILABLE)?;
    open(&cfg.payload_file,true)?.execute_batch("CREATE TABLE quarantine_cases(id TEXT PRIMARY KEY,body TEXT NOT NULL); CREATE TABLE quarantine_reviews(id TEXT PRIMARY KEY,case_id TEXT NOT NULL,body TEXT NOT NULL);").map_err(|_|UNAVAILABLE)?;
    session.check_current()?;
    Ok(json!({"version":VERSION,"action":"initialized","epoch":id}))
}

fn encoded_exposure(exposure: &Exposure) -> Value {
    json!({"sources":exposure.sources.values().map(Source::metadata).collect::<Vec<_>>(),"roots":exposure.roots,"pending":exposure.pending})
}
fn decoded_exposure(session: &GovernedSession, value: &Value) -> Result<Exposure> {
    crate::fields(value, &["sources", "roots", "pending"], &[])?;
    let mut out = Exposure::default();
    for value in value["sources"]
        .as_array()
        .filter(|v| v.len() <= session.source_limit())
        .ok_or(UNAVAILABLE)?
    {
        let source = Source::parse(session, value.clone())?;
        if out.sources.insert(source.id().into(), source).is_some() {
            return Err(UNAVAILABLE.into());
        }
    }
    for (field, set) in [("roots", &mut out.roots), ("pending", &mut out.pending)] {
        for id in value[field]
            .as_array()
            .filter(|v| v.len() <= session.source_limit())
            .ok_or(UNAVAILABLE)?
        {
            if !set.insert(id.as_str().ok_or(UNAVAILABLE)?.to_owned()) {
                return Err(UNAVAILABLE.into());
            }
        }
    }
    if !out.roots.iter().all(|id| out.sources.contains_key(id)) {
        return Err(UNAVAILABLE.into());
    }
    out.validate_for(session)?;
    Ok(out)
}
impl GovernedSession {
    pub(crate) fn check_quarantine_directory(&self, directory: &Path) -> Result<()> {
        if let Some(cfg) = &self.0.quarantine {
            cfg.independent()?;
            if self.lifecycle_file().is_none() {
                return Err("quarantine-namespace-requires-lifecycle-ledger".into());
            }
            let directory = std::fs::canonicalize(directory).map_err(|_| UNAVAILABLE)?;
            for path in [&cfg.control_file, &cfg.payload_file] {
                let parent = std::fs::canonicalize(path.parent().ok_or(UNAVAILABLE)?)
                    .map_err(|_| UNAVAILABLE)?;
                if parent.starts_with(&directory) {
                    return Err("quarantine-stores-must-be-independent-of-run-backups".into());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn with_quarantine_namespace(mut self, run: &str, directory: &Path) -> Self {
        self.3 = Some(Arc::new(Namespace {
            directory: directory.to_owned(),
            run: run.to_owned(),
        }));
        self
    }
    pub(super) fn retain_quarantine(
        &self,
        actor: &str,
        original: &Value,
        projection: &Value,
        inspection: &Value,
        exposure: &Exposure,
    ) -> Result<Option<String>> {
        if self.0.quarantine.is_none() {
            return Ok(None);
        }
        let retain = || retain(self, actor, original, projection, inspection, exposure).map(Some);
        match &self.3 {
            Some(namespace) => super::lifecycle::with_admission(&namespace.directory, self, retain),
            None => retain(),
        }
    }
}
fn retain(
    session: &GovernedSession,
    actor: &str,
    original: &Value,
    projection: &Value,
    inspection: &Value,
    exposure: &Exposure,
) -> Result<String> {
    let cfg = config(session)?;
    cfg.independent()?;
    let identity = namespace_identity(session)?;
    let payload = json!({"original":original,"projection":projection,"inspection":inspection,"exposure":encoded_exposure(exposure),"namespace":identity});
    let plain = Zeroizing::new(super::encoding::canonical(&payload, MAX_PLAIN)?);
    // Both permissions apply to the complete exact payload and full inherited
    // source policies. Detector cleanliness supplies neither permission.
    let source_decision = session.authorize_exposure(
        actor,
        "retain-source",
        "host-source-quarantine",
        "quarantine-payload",
        &payload,
        &payload,
        exposure,
    )?;
    let assessment_decision = session.authorize_exposure(
        actor,
        "retain-assessment",
        "host-source-quarantine",
        "quarantine-payload",
        &payload,
        &payload,
        exposure,
    )?;
    let boundary = session.commitment(&json!([original, projection]))?;
    let assessment = session.commitment(inspection)?;
    let scope = match &session.3 {
        Some(_) => namespace_scope(session, &identity)?,
        None => session.commitment(&json!(["quarantine-unadmitted/1", actor, boundary]))?,
    };
    let id = format!(
        "quarantine-{}",
        session.commitment(&json!([VERSION, scope, actor, boundary, assessment]))?
    );
    let mut control = open(&cfg.control_file, true)?;
    let tx = control
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    let control_epoch = epoch(&tx)?;
    let payload_db = open(&cfg.payload_file, true)?;
    if epoch(&payload_db)? != control_epoch {
        return Err(UNAVAILABLE.into());
    }
    let existing: Option<(String, i64, String, String)> = tx
        .query_row(
            "SELECT owner,expires,state,binding FROM quarantine_keys WHERE id=?",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|_| UNAVAILABLE)?;
    if let Some((prior_owner, expires, state, binding)) = existing {
        let expires = u64::try_from(expires).map_err(|_| UNAVAILABLE)?;
        if prior_owner != owner(session)? || state != "available" || expires <= crate::now_ms()? {
            return Err(UNAVAILABLE.into());
        }
        let body = case_body(&payload_db, &id)?;
        if session.commitment(&body)? != binding {
            return Err(UNAVAILABLE.into());
        }
        authorize_retention_bytes(session, actor, &payload, &body, exposure, expires)?;
        session.check_current()?;
        tx.commit().map_err(|_| UNAVAILABLE)?;
        return Ok(id);
    }
    let count: i64 = tx
        .query_row("SELECT COUNT(*) FROM quarantine_keys", [], |r| r.get(0))
        .map_err(|_| UNAVAILABLE)?;
    if count >= cfg.max_cases as i64 {
        return Err("governed-source-quarantine-capacity".into());
    }
    let now = crate::now_ms()?;
    let expires = now
        .checked_add(cfg.retention_ms)
        .ok_or(UNAVAILABLE)?
        .min(GovernedSession::admitted_expiry(&source_decision)?)
        .min(GovernedSession::admitted_expiry(&assessment_decision)?)
        .min(exposure.expiry());
    if expires <= now {
        return Err(UNAVAILABLE.into());
    }
    let aad = json!({"version":VERSION,"id":id,"epoch":control_epoch,"owner":owner(session)?,"profile":session.fingerprint(),"actor":actor,"scope":scope,"context_binding":boundary,"source_binding":session.commitment(original)?,"assessment_binding":assessment,"expires_at_ms":expires});
    let key = Zeroizing::new(random::<32>()?);
    let nonce = random::<12>()?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_| UNAVAILABLE)?;
    let aad_bytes = super::encoding::canonical(&aad, 4096)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &aad_bytes,
            },
        )
        .map_err(|_| UNAVAILABLE)?;
    let body = json!({"record":aad,"nonce":hex(&nonce),"ciphertext":hex(&ciphertext)});
    super::encoding::canonical(&body, MAX_ENVELOPE)?;
    authorize_retention_bytes(session, actor, &payload, &body, exposure, expires)?;
    tx.execute("INSERT INTO quarantine_keys(id,owner,profile,actor,scope,expires,state,key,binding,namespace) VALUES(?,?,?,?,?,?,'available',?,?,?)",params![id,owner(session)?,session.fingerprint(),actor,scope,sql_time(expires)?,key.as_ref() as &[u8],session.commitment(&body)?,identity.to_string()]).map_err(|_|UNAVAILABLE)?;
    // Ciphertext commits first. A crash before key commit leaves inaccessible
    // ciphertext, never a payload whose key retirement can be bypassed.
    payload_db
        .execute(
            "INSERT INTO quarantine_cases VALUES(?,?)",
            params![id, body.to_string()],
        )
        .map_err(|_| UNAVAILABLE)?;
    session.check_current()?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    Ok(id)
}
fn case_body(db: &Connection, id: &str) -> Result<Value> {
    let body:Option<String>=db.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=65536 THEN body END FROM quarantine_cases WHERE id=?",[id],|r|r.get(0)).map_err(|_|UNAVAILABLE)?;
    mas_protocol_codec::parse_json(&body.ok_or(UNAVAILABLE)?).map_err(|_| UNAVAILABLE.into())
}
fn authorize_retention_bytes(
    session: &GovernedSession,
    actor: &str,
    payload: &Value,
    body: &Value,
    exposure: &Exposure,
    expires: u64,
) -> Result<()> {
    let bytes = super::encoding::canonical(body, MAX_ENVELOPE)?.len();
    for operation in ["retain-source", "retain-assessment"] {
        let decision = session.authorize_exposure_sized(
            actor,
            operation,
            "host-source-quarantine",
            "quarantine-payload",
            payload,
            body,
            bytes,
            exposure,
        )?;
        if GovernedSession::admitted_expiry(&decision)? < expires {
            return Err(UNAVAILABLE.into());
        }
    }
    Ok(())
}
struct Case {
    body: Value,
    payload: Value,
    exposure: Exposure,
    expires: u64,
}
fn read_case(
    control: &Connection,
    session: &GovernedSession,
    actor: &str,
    id: &str,
) -> Result<Case> {
    let cfg = config(session)?;
    cfg.independent()?;
    let (key,binding,expires,state):(Option<Vec<u8>>,String,i64,String)=control.query_row("SELECT CASE WHEN length(key)=32 THEN key END,CASE WHEN length(CAST(binding AS BLOB))=64 THEN binding END,expires,CASE WHEN length(CAST(state AS BLOB))<=16 THEN state END FROM quarantine_keys WHERE id=? AND owner=? AND profile=? AND actor=?",params![id,owner(session)?,session.fingerprint(),actor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|UNAVAILABLE)?;
    let expires = u64::try_from(expires).map_err(|_| UNAVAILABLE)?;
    if state != "available" || expires <= crate::now_ms()? {
        return Err(UNAVAILABLE.into());
    }
    let key = Zeroizing::new(key.filter(|k| k.len() == 32).ok_or(UNAVAILABLE)?);
    let payload_db = open(&cfg.payload_file, false)?;
    if epoch(&payload_db)? != epoch(control)? {
        return Err(UNAVAILABLE.into());
    }
    let body = case_body(&payload_db, id)?;
    crate::fields(&body, &["record", "nonce", "ciphertext"], &[])?;
    if session.commitment(&body)? != binding
        || body["record"]["id"] != id
        || body["record"]["expires_at_ms"]
            .as_u64()
            .filter(|n| *n >= expires)
            .is_none()
        || body["record"]["epoch"] != epoch(control)?
        || body["record"]["owner"] != owner(session)?
        || body["record"]["profile"] != session.fingerprint()
        || body["record"]["actor"] != actor
    {
        return Err(UNAVAILABLE.into());
    }
    let nonce = unhex(body["nonce"].as_str().ok_or(UNAVAILABLE)?, 12)?;
    if nonce.len() != 12 {
        return Err(UNAVAILABLE.into());
    }
    let ciphertext = unhex(
        body["ciphertext"].as_str().ok_or(UNAVAILABLE)?,
        MAX_PLAIN + 16,
    )?;
    let aad = super::encoding::canonical(&body["record"], 4096)?;
    let plain = Zeroizing::new(
        Aes256Gcm::new_from_slice(&key)
            .map_err(|_| UNAVAILABLE)?
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| UNAVAILABLE)?,
    );
    if plain.len() > MAX_PLAIN {
        return Err(UNAVAILABLE.into());
    }
    let payload =
        mas_protocol_codec::parse_json(std::str::from_utf8(&plain).map_err(|_| UNAVAILABLE)?)
            .map_err(|_| UNAVAILABLE)?;
    crate::fields(
        &payload,
        &[
            "original",
            "projection",
            "inspection",
            "exposure",
            "namespace",
        ],
        &[],
    )?;
    if session.commitment(&json!([payload["original"], payload["projection"]]))?
        != body["record"]["context_binding"]
        || session.commitment(&payload["original"])? != body["record"]["source_binding"]
        || session.commitment(&payload["inspection"])? != body["record"]["assessment_binding"]
    {
        return Err(UNAVAILABLE.into());
    }
    let namespace: String = control
        .query_row(
            "SELECT CASE WHEN length(CAST(namespace AS BLOB))<=8192 THEN namespace END FROM quarantine_keys WHERE id=?",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| UNAVAILABLE)?;
    if mas_protocol_codec::parse_json(&namespace).map_err(|_| UNAVAILABLE)? != payload["namespace"]
    {
        return Err(UNAVAILABLE.into());
    }
    if !payload["namespace"].is_null()
        && namespace_scope(session, &payload["namespace"])? != body["record"]["scope"]
    {
        return Err(UNAVAILABLE.into());
    }
    let mut exposure = decoded_exposure(session, &payload["exposure"])?;
    // Completion inspection can precede settlement in the existing journal.
    // Resolve exact pending effect IDs without refetching or dropping ancestry.
    if !exposure.pending.is_empty() {
        let directory = Path::new(
            payload["namespace"]["directory"]
                .as_str()
                .ok_or(UNAVAILABLE)?,
        );
        let db = Connection::open_with_flags(
            directory.join("execution.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|_| UNAVAILABLE)?;
        for effect in exposure.pending.clone() {
            if exposure::load_pending(&db, session, &effect)?.is_some() {
                return Err("governed-quarantine-pending-exposure".into());
            }
            let settled = exposure::load_step_source(&db, session, &effect, "effect-output")?;
            exposure.pending.remove(&effect);
            exposure = exposure.union(&settled)?;
        }
    }
    Ok(Case {
        body,
        payload,
        exposure,
        expires,
    })
}
fn authorize_case(
    session: &GovernedSession,
    actor: &str,
    case: &Case,
    operation: &str,
    view: &Value,
) -> Result<()> {
    session
        .authorize_exposure(
            actor,
            operation,
            &session.0.principal,
            "quarantine-review",
            &case.payload,
            view,
            &case.exposure,
        )
        .map(|_| ())
}
fn view(case: &Case) -> Value {
    json!({"version":"mas-governed-quarantine-source-view/1","reference":case.body["record"]["id"],"context_binding":case.body["record"]["context_binding"],"source_binding":case.body["record"]["source_binding"],"assessment_binding":case.body["record"]["assessment_binding"],"expires_at_ms":case.expires,"original":case.payload["original"],"projection":case.payload["projection"],"inspection":case.payload["inspection"],"release_permission":"requires-fresh-policy-decision"})
}

/// Exact raw-source review is a separate, current authorisation. This API does
/// not re-enter the detector or provide worker invocation/release permission.
fn reference_namespace(session: &GovernedSession, actor: &str, id: &str) -> Result<Value> {
    let cfg = config(session)?;
    cfg.independent()?;
    let control = open(&cfg.control_file, false)?;
    let namespace:String=control.query_row("SELECT CASE WHEN length(CAST(namespace AS BLOB))<=8192 THEN namespace END FROM quarantine_keys WHERE id=? AND owner=? AND profile=? AND actor=?",params![id,owner(session)?,session.fingerprint(),actor],|r|r.get(0)).map_err(|_|UNAVAILABLE)?;
    mas_protocol_codec::parse_json(&namespace).map_err(|_| UNAVAILABLE.into())
}
pub fn read_governed_source_quarantine(
    session: &GovernedSession,
    actor: &str,
    id: &str,
) -> Result<Value> {
    (|| {
        admin(session, actor, "resolve-source", &json!({"reference":id}))?;
        let namespace = reference_namespace(session, actor, id)?;
        super::lifecycle::with_custody_admission(&[namespace], session, || {
            let mut control = open(&config(session)?.control_file, true)?;
            let tx = control
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|_| UNAVAILABLE)?;
            let case = read_case(&tx, session, actor, id)?;
            let view = view(&case);
            super::encoding::canonical(&view, 65_536)?;
            authorize_case(session, actor, &case, "resolve-source", &view)?;
            authorize_case(session, actor, &case, "resolve-assessment", &view)?;
            session.check_current()?;
            tx.commit().map_err(|_| UNAVAILABLE)?;
            Ok(view)
        })
    })()
    .map_err(|_: String| UNAVAILABLE.into())
}
/// Discovery is a bounded current snapshot, including failed pre-task ingress.
/// Always follow next_after until complete; an empty page may still continue.
pub fn list_governed_source_quarantine(session: &GovernedSession, actor: &str) -> Result<Value> {
    list_governed_source_quarantine_page(session, actor, None)
}
pub fn list_governed_source_quarantine_page(
    session: &GovernedSession,
    actor: &str,
    after: Option<&str>,
) -> Result<Value> {
    (|| {
        let after=after.unwrap_or("");
        if after.len()>128 || !after.bytes().all(|b|b.is_ascii_alphanumeric() || b"-_".contains(&b)) {return Err(UNAVAILABLE.into());}
        admin(session,actor,"list-source-quarantine",&json!({"action":"list","after":after}))?;
        let control=open(&config(session)?.control_file,false)?;
        let mut ids:Vec<String>=control.prepare("SELECT id FROM quarantine_keys WHERE owner=? AND profile=? AND actor=? AND state='available' AND expires>? AND id>? ORDER BY id LIMIT 65").map_err(|_|UNAVAILABLE)?.query_map(params![owner(session)?,session.fingerprint(),actor,sql_time(crate::now_ms()?)?,after],|r|r.get(0)).map_err(|_|UNAVAILABLE)?.collect::<rusqlite::Result<_>>().map_err(|_|UNAVAILABLE)?;
        let more=ids.len()>64;ids.truncate(64);let next_after=if more {ids.last().cloned()}else{None};drop(control);
        let mut visible=Vec::new();let mut namespaces=Vec::new();
        for id in ids {
            let namespace=reference_namespace(session,actor,&id)?;
            match super::lifecycle::with_custody_admission(std::slice::from_ref(&namespace),session,||Ok(())) {
                Ok(())=>{visible.push(id);namespaces.push(namespace);},
                Err(e) if e=="governed-payload-expired" || e=="governed-payload-deleted"=>{},
                Err(e)=>return Err(e),
            }
        }
        super::lifecycle::with_custody_admission(&namespaces,session,|| {
            let mut control=open(&config(session)?.control_file,true)?;
            let tx=control.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|UNAVAILABLE)?;
            let mut records=Vec::new();
            for id in visible {
                let case=match read_case(&tx,session,actor,&id) {Ok(case)=>case,Err(e) if e=="governed-quarantine-pending-exposure"=>continue,Err(e)=>return Err(e)};let metadata=case.body["record"].clone();
                authorize_case(session,actor,&case,"list-source-quarantine",&metadata)?;
                records.push(json!({"reference":id,"expires_at_ms":case.expires,"context_binding":metadata["context_binding"],"assessment_binding":metadata["assessment_binding"]}));
            }
            let view=json!({"version":"mas-governed-quarantine-list/1","cases":records,"complete":!more,"next_after":next_after});
            super::encoding::canonical(&view,65_536)?;session.check_current()?;tx.commit().map_err(|_|UNAVAILABLE)?;Ok(view)
        })
    })().map_err(|_:String|UNAVAILABLE.into())
}

/// Human verdicts append to the exact encrypted case. They never edit detector
/// truth, permit a new dispatch or change the closed projection policy.
pub fn adjudicate_governed_source_quarantine(
    session: &GovernedSession,
    actor: &str,
    id: &str,
    review: &Value,
) -> Result<Value> {
    (|| {
        crate::fields(review,&["review_key","verdict","reason"],&[])?;
        let review_key=super::super::identifier(review,"review_key")?;
        if !["false-positive","confirmed-finding","inconclusive"].contains(&review["verdict"].as_str().ok_or(UNAVAILABLE)?) || !["authorised-source-review","domain-context","insufficient-evidence"].contains(&review["reason"].as_str().ok_or(UNAVAILABLE)?) || (review["verdict"]=="inconclusive" && review["reason"]!="insufficient-evidence") {return Err(UNAVAILABLE.into());}
        admin(session,actor,"adjudicate-source",&json!({"reference":id}))?;
        let namespace=reference_namespace(session,actor,id)?;
        super::lifecycle::with_custody_admission(&[namespace],session,|| {
            let mut control=open(&config(session)?.control_file,true)?;
            let tx=control.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|_|UNAVAILABLE)?;
            let case=read_case(&tx,session,actor,id)?; let source_view=view(&case);
            for operation in ["resolve-source","resolve-assessment","adjudicate-source","adjudicate-assessment"] {authorize_case(session,actor,&case,operation,&source_view)?;}
            let body=json!({"version":"mas-governed-quarantine-review/1","reference":id,"source_binding":case.body["record"]["source_binding"],"assessment_binding":case.body["record"]["assessment_binding"],"reviewer_ref":owner(session)?,"review":review,"release_permission":"requires-fresh-policy-decision"});
            let review_id=session.commitment(&json!(["quarantine-review/1",id,review_key]))?;
            let record=json!({"review_id":review_id,"binding":session.commitment(&body)?,"record":body});
            let plaintext=Zeroizing::new(super::encoding::canonical(&record,4096)?);
            for operation in ["adjudicate-source","adjudicate-assessment"] {authorize_case(session,actor,&case,operation,&record)?;}
            let payload=open(&config(session)?.payload_file,true)?;
            let prior:Option<(Option<Vec<u8>>,String)>=tx.query_row("SELECT CASE WHEN length(key)=32 THEN key END,CASE WHEN length(CAST(binding AS BLOB))=64 THEN binding END FROM quarantine_review_keys WHERE id=? AND case_id=?",params![review_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|_|UNAVAILABLE)?;
            let encrypted=if let Some((key,binding))=prior {
                let key=Zeroizing::new(key.filter(|k|k.len()==32).ok_or(UNAVAILABLE)?);
                let prior:Option<String>=payload.query_row("SELECT CASE WHEN length(CAST(body AS BLOB))<=12288 THEN body END FROM quarantine_reviews WHERE id=? AND case_id=?",params![review_id,id],|r|r.get(0)).map_err(|_|UNAVAILABLE)?;
                let prior=mas_protocol_codec::parse_json(&prior.ok_or(UNAVAILABLE)?).map_err(|_|UNAVAILABLE)?;
                if session.commitment(&prior)?!=binding {return Err(UNAVAILABLE.into());}
                let nonce=unhex(prior["nonce"].as_str().ok_or(UNAVAILABLE)?,12)?;
                if nonce.len()!=12 {return Err(UNAVAILABLE.into());}
                let aad=super::encoding::canonical(&prior["record"],4096)?;
                let cipher=unhex(prior["ciphertext"].as_str().ok_or(UNAVAILABLE)?,4112)?;
                let plain=Zeroizing::new(Aes256Gcm::new_from_slice(&key).map_err(|_|UNAVAILABLE)?.decrypt(Nonce::from_slice(&nonce),Payload{msg:&cipher,aad:&aad}).map_err(|_|UNAVAILABLE)?);
                if plain.as_slice()!=plaintext.as_slice() {return Err(UNAVAILABLE.into());} prior
            } else {
                let count:i64=tx.query_row("SELECT COUNT(*) FROM quarantine_review_keys WHERE case_id=?",[id],|r|r.get(0)).map_err(|_|UNAVAILABLE)?;
                if count>=128 {return Err(UNAVAILABLE.into());}
                // A distinct fresh key per immutable review means its nonce
                // never shares a key with the source or another review. A
                // crashed ciphertext-before-key commit is replaced with a new
                // key, never re-encrypted under an unknown prior key/nonce.
                let key=Zeroizing::new(random::<32>()?);let nonce=random::<12>()?;
                let aad=json!({"version":"mas-governed-quarantine-review-cipher/1","review_id":review_id,"reference":id,"epoch":epoch(&tx)?});
                let aad_bytes=super::encoding::canonical(&aad,4096)?;
                let cipher=Aes256Gcm::new_from_slice(key.as_ref()).map_err(|_|UNAVAILABLE)?.encrypt(Nonce::from_slice(&nonce),Payload{msg:&plaintext,aad:&aad_bytes}).map_err(|_|UNAVAILABLE)?;
                let encrypted=json!({"record":aad,"nonce":hex(&nonce),"ciphertext":hex(&cipher)});
                tx.execute("INSERT INTO quarantine_review_keys VALUES(?,?,?,?)",params![review_id,id,key.as_ref() as &[u8],session.commitment(&encrypted)?]).map_err(|_|UNAVAILABLE)?;
                encrypted
            };
            let bytes=super::encoding::canonical(&encrypted,12288)?.len();
            for operation in ["retain-source","retain-assessment"] {
                let decision=session.authorize_exposure_sized(actor,operation,"host-source-quarantine","quarantine-payload",&record,&encrypted,bytes,&case.exposure)?;
                if GovernedSession::admitted_expiry(&decision)?<case.expires {return Err(UNAVAILABLE.into());}
            }
            session.check_current()?;
            payload.execute("INSERT OR REPLACE INTO quarantine_reviews VALUES(?,?,?)",params![review_id,id,encrypted.to_string()]).map_err(|_|UNAVAILABLE)?;
            session.check_current()?;tx.commit().map_err(|_|UNAVAILABLE)?;Ok(record)
        })
    })().map_err(|_:String|UNAVAILABLE.into())
}

/// Manage access expiry, hold and irreversible logical/key retirement. A hold
/// prevents deletion; it never extends expiry or current disclosure authority.
pub fn govern_source_quarantine(
    session: &GovernedSession,
    actor: &str,
    id: &str,
    action: &str,
    expires_at_ms: Option<u64>,
) -> Result<Value> {
    admin(
        session,
        actor,
        if action == "hold" || action == "release-hold" {
            "hold-source"
        } else {
            "retire-source"
        },
        &json!({"reference":id,"action":action,"expires_at_ms":expires_at_ms}),
    )?;
    let cfg = config(session)?;
    if action == "retire" || action == "expire" {
        let control = open(&cfg.control_file, false)?;
        let (namespace,state,held,expires):(String,String,bool,i64)=control.query_row("SELECT namespace,state,held,expires FROM quarantine_keys WHERE id=? AND owner=? AND actor=?",params![id,owner(session)?,actor],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|_|UNAVAILABLE)?;
        if action == "retire" && held {
            return Err("governed-source-quarantine-held".into());
        }
        let expiry = if action == "retire" {
            crate::now_ms()?
        } else {
            expires_at_ms
                .filter(|n| *n <= u64::try_from(expires).unwrap_or(0))
                .ok_or(UNAVAILABLE)?
        };
        let namespace = mas_protocol_codec::parse_json(&namespace).map_err(|_| UNAVAILABLE)?;
        if state == "available" && !namespace.is_null() {
            let directory = Path::new(namespace["directory"].as_str().ok_or(UNAVAILABLE)?);
            if super::lifecycle::custody_identity(directory, session)? != namespace {
                return Err(UNAVAILABLE.into());
            }
            super::lifecycle::expire_for_custody(directory, session, expiry)?;
        }
    }
    let mut control = open(&cfg.control_file, true)?;
    let tx = control
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    let (expires, held, state): (i64, bool, String) = tx
        .query_row(
            "SELECT expires,held,state FROM quarantine_keys WHERE id=? AND owner=? AND actor=?",
            params![id, owner(session)?, actor],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| UNAVAILABLE)?;
    let expires = u64::try_from(expires).map_err(|_| UNAVAILABLE)?;
    match action {
        "hold" | "release-hold" if state == "available" => {
            tx.execute(
                "UPDATE quarantine_keys SET held=? WHERE id=?",
                params![action == "hold", id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
        "expire" if state == "available" => {
            let expiry = expires_at_ms.filter(|n| *n <= expires).ok_or(UNAVAILABLE)?;
            tx.execute(
                "UPDATE quarantine_keys SET expires=? WHERE id=?",
                params![sql_time(expiry)?, id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
        "retire" => {
            if held {
                return Err("governed-source-quarantine-held".into());
            }
            tx.execute(
                "UPDATE quarantine_keys SET state='retired',key=NULL WHERE id=?",
                [id],
            )
            .map_err(|_| UNAVAILABLE)?;
            tx.execute(
                "UPDATE quarantine_review_keys SET key=NULL WHERE case_id=?",
                [id],
            )
            .map_err(|_| UNAVAILABLE)?;
        }
        _ => return Err(UNAVAILABLE.into()),
    }
    session.check_current()?;
    tx.commit().map_err(|_| UNAVAILABLE)?;
    if action == "retire" {
        purge(cfg, &[id.to_owned()])?;
    }
    Ok(
        json!({"version":VERSION,"reference":id,"action":action,"payload":if action=="retire" {"deleted"} else {"access-controlled"},"physical_erasure":"not-claimed"}),
    )
}
fn purge(cfg: &Configuration, ids: &[String]) -> Result<()> {
    let mut payload = open(&cfg.payload_file, true)?;
    let tx = payload
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    for id in ids {
        tx.execute("DELETE FROM quarantine_reviews WHERE case_id=?", [id])
            .map_err(|_| UNAVAILABLE)?;
        tx.execute("DELETE FROM quarantine_cases WHERE id=?", [id])
            .map_err(|_| UNAVAILABLE)?;
    }
    tx.commit().map_err(|_| UNAVAILABLE)?;
    payload
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
        .map_err(|_| UNAVAILABLE)?;
    Ok(())
}
pub(super) fn retire_namespace(
    session: &GovernedSession,
    directory: &Path,
    run: &str,
) -> Result<()> {
    let Some(cfg) = &session.0.quarantine else {
        return Ok(());
    };
    if super::lifecycle::custody_identity(directory, session)?["run"] != run {
        return Err(UNAVAILABLE.into());
    }
    let mut control = open(&cfg.control_file, true)?;
    let tx = control
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| UNAVAILABLE)?;
    let rows: Vec<(String, bool)> = tx
        .prepare("SELECT id,held FROM quarantine_keys WHERE owner=? AND scope=?")
        .map_err(|_| UNAVAILABLE)?
        .query_map(
            params![
                owner(session)?,
                namespace_scope(
                    session,
                    &super::lifecycle::custody_identity(directory, session)?
                )?
            ],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| UNAVAILABLE)?
        .collect::<rusqlite::Result<_>>()
        .map_err(|_| UNAVAILABLE)?;
    if rows.iter().any(|(_, held)| *held) {
        return Err("governed-source-quarantine-held".into());
    }
    for (id, _) in &rows {
        tx.execute(
            "UPDATE quarantine_review_keys SET key=NULL WHERE case_id=?",
            [id],
        )
        .map_err(|_| UNAVAILABLE)?;
        tx.execute(
            "UPDATE quarantine_keys SET state='retired',key=NULL WHERE id=?",
            [id],
        )
        .map_err(|_| UNAVAILABLE)?;
    }
    tx.commit().map_err(|_| UNAVAILABLE)?;
    purge(cfg, &rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>())
}

#[cfg(test)]
#[path = "quarantine_tests.rs"]
mod tests;
