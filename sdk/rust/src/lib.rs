//! Application functions use `Context` for durable requests and admitted effects.
//! The build tool supplies the worker entrypoint; applications do not call `serve`.
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;

pub use serde_json::{json, Value as Json};
pub type Result<T> = std::result::Result<T, String>;
pub type Handler = fn(&mut Context, Json) -> Result<Json>;
const MAX_FRAME: usize = 65_536;

fn object(value: &Json) -> Result<()> {
    if value.is_object() {
        Ok(())
    } else {
        Err("native input and output must be JSON objects".into())
    }
}

// Bound application-created values before serialisation; the MAS parser remains
// the authority for the resulting bytes. This prevents deep values overflowing
// the serializer's stack and large values requiring unbounded output allocation.
fn preflight(value: &Json, depth: usize, budget: &mut usize) -> Result<()> {
    let cost = match value {
        Json::Object(_) | Json::Array(_) => 2,
        Json::String(text) => text.len().saturating_add(2),
        Json::Number(number) => number.as_str().len(),
        _ => 4,
    };
    *budget = budget
        .checked_sub(cost)
        .ok_or("native frame exceeds 65536 bytes")?;
    match value {
        Json::Object(fields) => {
            if depth >= 32 {
                return Err("native JSON exceeds nesting depth".into());
            }
            for (key, child) in fields {
                *budget = budget
                    .checked_sub(key.len().saturating_add(3))
                    .ok_or("native frame exceeds 65536 bytes")?;
                preflight(child, depth + 1, budget)?;
            }
        }
        Json::Array(values) => {
            if depth >= 32 {
                return Err("native JSON exceeds nesting depth".into());
            }
            for child in values {
                preflight(child, depth + 1, budget)?;
            }
        }
        Json::Number(number) => {
            let digits = number.as_str().strip_prefix('-').unwrap_or(number.as_str());
            if digits.is_empty()
                || digits.len() > 1024
                || !digits.bytes().all(|c| c.is_ascii_digit())
            {
                return Err("native JSON requires bounded exact integers".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn read_frame(channel: &mut UnixStream) -> Result<Json> {
    let mut header = [0; 4];
    channel.read_exact(&mut header).map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err("invalid native frame length".into());
    }
    let mut data = vec![0; len];
    channel.read_exact(&mut data).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&data).map_err(|e| e.to_string())?;
    let value =
        mas_protocol_codec::parse_json(text).map_err(|e| format!("invalid native JSON: {e:?}"))?;
    object(&value)?;
    Ok(value)
}

fn write_frame(channel: &mut UnixStream, value: &Json) -> Result<()> {
    object(value)?;
    let mut budget = MAX_FRAME;
    preflight(value, 0, &mut budget)?;
    let data = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if data.is_empty() || data.len() > MAX_FRAME {
        return Err("native frame exceeds 65536 bytes".into());
    }
    // Check user-created values through the protocol authority, including floats and depth.
    mas_protocol_codec::parse_json(std::str::from_utf8(&data).map_err(|e| e.to_string())?)
        .map_err(|e| format!("invalid native JSON: {e:?}"))?;
    channel
        .write_all(&(data.len() as u32).to_be_bytes())
        .map_err(|e| e.to_string())?;
    channel.write_all(&data).map_err(|e| e.to_string())
}

#[derive(Debug, Clone)]
pub struct Request {
    pub key: String,
    pub target: String,
    pub input: Json,
}

impl Request {
    pub fn new(key: impl Into<String>, target: impl Into<String>, input: Json) -> Self {
        Self {
            key: key.into(),
            target: target.into(),
            input,
        }
    }
    fn value(&self) -> Result<Json> {
        object(&self.input)?;
        if self.key.is_empty() || self.target.is_empty() {
            return Err("request key and target must be non-empty".into());
        }
        Ok(json!({"key": self.key, "target": self.target, "input": self.input}))
    }
}

/// One host-owned invocation. Mutable access permits only one outstanding command.
/// The original absolute deadline is informational; the host enforces it.
pub struct Context {
    channel: UnixStream,
    pub actor: String,
    pub invocation_id: String,
    pub deadline_ms: u64,
}

impl Context {
    fn exchange(&mut self, command: Json) -> Result<Json> {
        write_frame(&mut self.channel, &command)?;
        let response = read_frame(&mut self.channel)?;
        if response["ok"] == false {
            return Err(response["error"]
                .as_str()
                .unwrap_or("host rejected native command")
                .to_owned());
        }
        Ok(response)
    }
    pub fn request(&mut self, key: &str, target: &str, input: Json) -> Result<Json> {
        let mut command = Request::new(key, target, input).value()?;
        command["type"] = "request".into();
        let response = self.exchange(command)?;
        successful_response(&response)?;
        let output = response["output"].clone();
        object(&output)?;
        Ok(output)
    }
    /// Results retain request order. Concurrency and replay belong to the host.
    pub fn parallel(&mut self, requests: &[Request]) -> Result<Vec<Json>> {
        let commands = requests
            .iter()
            .map(Request::value)
            .collect::<Result<Vec<_>>>()?;
        let response = self.exchange(json!({"type": "parallel", "requests": commands}))?;
        successful_response(&response)?;
        let outputs = response["outputs"]
            .as_array()
            .ok_or("host omitted parallel outputs")?;
        if outputs.len() != requests.len() {
            return Err("host returned wrong number of parallel outputs".into());
        }
        for output in outputs {
            object(output)?;
        }
        Ok(outputs.clone())
    }
    /// Execute only after durable admission. A cached outcome skips the closure.
    /// Failure after admission remains uncertain until evidence-based resolution.
    pub fn effect<F>(&mut self, key: &str, descriptor: Json, execute: F) -> Result<Json>
    where
        F: FnOnce(&str) -> Result<(Json, Json)>,
    {
        object(&descriptor)?;
        if key.is_empty() {
            return Err("effect key must be non-empty".into());
        }
        let response =
            self.exchange(json!({"type":"effect_begin", "key":key, "descriptor":descriptor}))?;
        if response["type"] != "effect" {
            return Err("invalid host effect response".into());
        }
        if response["state"] == "cached" {
            let output = response["output"].clone();
            object(&output)?;
            return Ok(output);
        }
        if response["state"] != "execute" {
            return Err("invalid host effect state".into());
        }
        let id = response["invocation_id"]
            .as_str()
            .ok_or("host omitted effect identity")?;
        let (output, evidence) = execute(id)?;
        object(&output)?;
        object(&evidence)?;
        if evidence.as_object().is_some_and(|v| v.is_empty()) {
            return Err("effect evidence must be non-empty".into());
        }
        let response = self.exchange(
            json!({"type":"effect_complete", "key":key, "output":output, "evidence":evidence}),
        )?;
        successful_response(&response)?;
        Ok(output)
    }
}

fn successful_response(response: &Json) -> Result<()> {
    if response["type"] == "response" && response["ok"] == true {
        Ok(())
    } else {
        Err("invalid host response".into())
    }
}

fn dispatch(mut channel: UnixStream, expected_handler: &str, handler: Handler) -> Result<()> {
    let start = read_frame(&mut channel)?;
    if start["type"] != "start"
        || start["version"] != "mas-native/1"
        || start["handler"] != expected_handler
    {
        return Err("invalid native start or handler".into());
    }
    object(&start["input"])?;
    let mut context = Context {
        channel,
        actor: start["actor"].as_str().ok_or("start omitted actor")?.into(),
        invocation_id: start["invocation_id"]
            .as_str()
            .ok_or("start omitted invocation identity")?
            .into(),
        deadline_ms: start["deadline_ms"]
            .as_u64()
            .ok_or("start omitted deadline")?,
    };
    match handler(&mut context, start["input"].clone()) {
        Ok(output) => {
            object(&output)?;
            write_frame(
                &mut context.channel,
                &json!({"type":"complete", "output":output}),
            )
        }
        Err(message) => write_frame(
            &mut context.channel,
            &json!({"type":"error", "message":message}),
        ),
    }
}

/// Called by generated build output, not by application source.
pub fn serve(expected_handler: &str, handler: Handler) -> Result<()> {
    let fd: i32 = std::env::var("MAS_WORKER_CHANNEL_FD")
        .map_err(|_| "missing private worker channel")?
        .parse()
        .map_err(|_| "invalid private worker channel")?;
    if fd < 3 {
        return Err("private worker channel must not use stdin/stdout/stderr".into());
    }
    // The guardian gives this worker ownership of the inherited private socket.
    let channel = unsafe { UnixStream::from_raw_fd(fd) };
    dispatch(channel, expected_handler, handler)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    #[test]
    fn framing_rejects_invalid_json_and_preserves_integer_unicode() {
        for source in [
            r#"{"x":1,"x":2}"#,
            r#"{"x":1.0}"#,
            r#"{"x":"\ud800"}"#,
            "[]",
        ] {
            let (mut send, mut recv) = UnixStream::pair().unwrap();
            send.write_all(&(source.len() as u32).to_be_bytes())
                .unwrap();
            send.write_all(source.as_bytes()).unwrap();
            assert!(read_frame(&mut recv).is_err(), "{source}");
        }
        let expected = mas_protocol_codec::parse_json(
            r#"{"n":340282366920938463463374607431768211456,"s":"∀🧠"}"#,
        )
        .unwrap();
        let (mut send, mut recv) = UnixStream::pair().unwrap();
        write_frame(&mut send, &expected).unwrap();
        assert_eq!(read_frame(&mut recv).unwrap(), expected);
        let mut nested = json!({});
        for _ in 0..33 {
            nested = json!({"child":nested});
        }
        assert!(write_frame(&mut send, &nested).is_err());
        assert!(write_frame(&mut send, &json!({"n":1.5})).is_err());
    }
    #[test]
    fn cached_effect_does_not_execute_and_ordered_parallel_is_checked() {
        let (mut host, worker) = UnixStream::pair().unwrap();
        let host_thread = thread::spawn(move || {
            assert_eq!(read_frame(&mut host).unwrap()["type"], "effect_begin");
            write_frame(
                &mut host,
                &json!({"type":"effect", "state":"cached", "output":{"done":true}}),
            )
            .unwrap();
            assert_eq!(
                read_frame(&mut host).unwrap()["requests"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            write_frame(
                &mut host,
                &json!({"type":"response", "ok":true, "outputs":[{"n":1},{"n":2}]}),
            )
            .unwrap();
        });
        let mut ctx = Context {
            channel: worker,
            actor: "a".into(),
            invocation_id: "i".into(),
            deadline_ms: 0,
        };
        assert_eq!(
            ctx.effect("write", json!({"resource":"receipt"}), |_| panic!(
                "cached effect executed"
            ))
            .unwrap()["done"],
            true
        );
        let outputs = ctx
            .parallel(&[
                Request::new("one", "b", json!({})),
                Request::new("two", "b", json!({})),
            ])
            .unwrap();
        assert_eq!(outputs[1]["n"], 2);
        host_thread.join().unwrap();
    }
    #[test]
    fn admitted_effect_reports_output_and_evidence() {
        let (mut host, worker) = UnixStream::pair().unwrap();
        let host_thread = thread::spawn(move || {
            assert_eq!(read_frame(&mut host).unwrap()["type"], "effect_begin");
            write_frame(
                &mut host,
                &json!({"type":"effect", "state":"execute", "invocation_id":"effect-1"}),
            )
            .unwrap();
            let completed = read_frame(&mut host).unwrap();
            assert_eq!(completed["evidence"]["receipt"], "effect-1");
            write_frame(&mut host, &json!({"type":"response", "ok":true})).unwrap();
        });
        let mut ctx = Context {
            channel: worker,
            actor: "a".into(),
            invocation_id: "i".into(),
            deadline_ms: 0,
        };
        assert_eq!(
            ctx.effect("write", json!({}), |id| Ok((
                json!({"done":true}),
                json!({"receipt":id})
            )))
            .unwrap()["done"],
            true
        );
        host_thread.join().unwrap();
    }
}
