//! Hyper parses HTTP; bounds apply before JSON, identity or execution admission.
use super::{Result, Service, PROFILE};
use http_body_util::{BodyExt, Full, Limited};
use hyper::{
    body::{Bytes, Incoming},
    service::service_fn,
    Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use std::{convert::Infallible, sync::Arc, time::Duration};

type Body = Full<Bytes>;
fn response(status: StatusCode, value: Value) -> Response<Body> {
    let encoded = value.to_string();
    let mut result = if encoded.len() <= 262_144 {
        Response::new(Full::new(Bytes::from(encoded)))
    } else {
        Response::new(Full::new(Bytes::from(
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"response_limit"}}"#,
        )))
    };
    *result.status_mut() = status;
    result.headers_mut().insert(
        "content-type",
        hyper::header::HeaderValue::from_static("application/json"),
    );
    result.headers_mut().insert(
        "cache-control",
        hyper::header::HeaderValue::from_static("no-store"),
    );
    result.headers_mut().insert(
        "x-content-type-options",
        hyper::header::HeaderValue::from_static("nosniff"),
    );
    result.headers_mut().insert(
        "a2a-version",
        hyper::header::HeaderValue::from_static("1.0"),
    );
    result
}
fn rpc_error(request: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":super::response_id(request),"error":{"code":code,"message":message}})
}
async fn handle(service: Arc<Service>, request: Request<Incoming>) -> Response<Body> {
    let path = request.uri().path();
    if request.uri().query().is_some() {
        return response(StatusCode::NOT_FOUND, json!({"error":"not_found"}));
    }
    if request.method() == hyper::Method::GET && path == "/.well-known/agent-card.json" {
        return response(
            StatusCode::OK,
            service.configuration.card(&service.descriptor, None),
        );
    }
    if request.method() != hyper::Method::POST || path != "/a2a" {
        return response(StatusCode::NOT_FOUND, json!({"error":"not_found"}));
    }
    for key in [
        "authorization",
        "a2a-version",
        "a2a-extensions",
        "content-type",
    ] {
        if request.headers().get_all(key).iter().count() > 1 {
            return response(StatusCode::BAD_REQUEST, json!({"error":"duplicate_header"}));
        }
    }
    // This local service is not a browser API. A TLS reverse proxy must preserve
    // the bearer header and may impose stronger origin and client policies.
    if request.headers().contains_key("origin") {
        return response(
            StatusCode::FORBIDDEN,
            json!({"error":"browser_origin_not_supported"}),
        );
    }
    let bearer = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(principal) = bearer.and_then(|v| service.configuration.authenticate(v)) else {
        let mut r = response(StatusCode::UNAUTHORIZED, json!({"error":"unauthorised"}));
        r.headers_mut().insert(
            "www-authenticate",
            hyper::header::HeaderValue::from_static("Bearer"),
        );
        return r;
    };
    if request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim())
        != Some("application/json")
    {
        return response(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            json!({"error":"application_json_required"}),
        );
    }
    let version = request
        .headers()
        .get("a2a-version")
        .and_then(|v| v.to_str().ok())
        == Some("1.0");
    let extension = request
        .headers()
        .get("a2a-extensions")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|x| x.trim() == PROFILE));
    let effect_extension = service.configuration.governed.is_some()
        && request
            .headers()
            .get("a2a-extensions")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(',')
                    .any(|x| x.trim() == super::retained_effect::PROFILE)
            });
    let source_extension = service.configuration.source_reads_supported()
        && request
            .headers()
            .get("a2a-extensions")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(',')
                    .any(|x| x.trim() == super::retained_source::PROFILE)
            });
    let collected = tokio::time::timeout(
        Duration::from_secs(3),
        Limited::new(request.into_body(), 65_536).collect(),
    )
    .await;
    let bytes = match collected {
        Ok(Ok(v)) => v.to_bytes(),
        Ok(Err(_)) => {
            return response(
                StatusCode::PAYLOAD_TOO_LARGE,
                json!({"error":"request_body_limit"}),
            )
        }
        Err(_) => {
            return response(
                StatusCode::REQUEST_TIMEOUT,
                json!({"error":"request_body_timeout"}),
            )
        }
    };
    let mut request = match std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| mas_protocol_codec::parse_json(s).ok())
    {
        Some(v) => v,
        None => {
            return response(
                StatusCode::OK,
                rpc_error(&Value::Null, -32700, "invalid_exact_integer_json"),
            )
        }
    };
    if service.configuration.governed.is_some()
        && request.is_object()
        && !super::governed::rpc_id_allowed(&request["id"])
    {
        request["id"] = Value::Null;
    }
    if !version {
        return response(
            StatusCode::OK,
            rpc_error(&request, -32009, "version_not_supported"),
        );
    }
    let cloned = Arc::clone(&service);
    let caller = principal.clone();
    let body = request.clone();
    let mut result = match tokio::task::spawn_blocking(move || {
        cloned.rpc(
            &caller,
            &body,
            extension,
            effect_extension,
            source_extension,
        )
    })
    .await
    {
        Ok(v) => v,
        Err(_) => {
            return response(
                StatusCode::OK,
                rpc_error(&request, -32603, "service_unavailable"),
            )
        }
    };
    if request["method"] == "SendMessage"
        && request["params"]["configuration"]["returnImmediately"] != true
        && result["result"]["task"].is_object()
    {
        let id = result["result"]["task"]["id"].as_str().unwrap().to_owned();
        let context_id = result["result"]["task"]["contextId"]
            .as_str()
            .unwrap()
            .to_owned();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let state = result["result"]["task"]["metadata"]["mas"]["state"]
                .as_str()
                .unwrap_or("unknown");
            if matches!(state, "completed" | "failed") {
                break;
            }
            if state == "unknown" {
                result = rpc_error(&request, -32603, "outcome_unknown_poll_original_task");
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                result = rpc_error(
                    &request,
                    -32603,
                    "response_deadline_repeat_original_message",
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
            let cloned = Arc::clone(&service);
            let caller = principal.clone();
            let task_id = id.clone();
            match tokio::task::spawn_blocking(move || cloned.task(&caller, &task_id)).await {
                Ok(Ok(task)) => result["result"]["task"] = task,
                _ => {
                    result = rpc_error(&request, -32603, "task_observation_unavailable");
                    break;
                }
            }
        }
        if result.get("error").is_some() {
            let cloned = Arc::clone(&service);
            let caller = principal.clone();
            let task_id = id.clone();
            let information = json!([{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"RETAINED_TASK_REQUIRES_OBSERVATION","domain":"mas-host","metadata":{"taskId":id,"contextId":context_id}}]);
            let mut projection = result["error"].clone();
            projection["data"] = information.clone();
            if matches!(
                tokio::task::spawn_blocking(move || {
                    let task = cloned.task(&caller, &task_id)?;
                    if let Some(session) = &caller.session {
                        let actor = task["metadata"]["mas"]["agent"]
                            .as_str()
                            .ok_or(super::RpcError(-32603, "status_unavailable"))?;
                        if session.is_v2() {
                            mas_host_core::governance::read_governed_root_delivery(
                                &cloned.state.join("runs").join(&task_id),
                                session,
                                actor,
                                None,
                                &projection,
                            )
                        } else {
                            session.check_status_value(actor, &projection)
                        }
                        .map_err(|_| super::RpcError(-32603, "status_unavailable"))?;
                    }
                    Ok::<(), super::RpcError>(())
                })
                .await,
                Ok(Ok(_))
            ) {
                result["error"]["data"] = information;
            }
        }
    }
    let mut response = response(StatusCode::OK, result);
    if extension {
        let mut extensions = vec![PROFILE];
        if effect_extension {
            extensions.push(super::retained_effect::PROFILE);
        }
        if source_extension {
            extensions.push(super::retained_source::PROFILE);
        }
        response.headers_mut().insert(
            "a2a-extensions",
            hyper::header::HeaderValue::from_str(&extensions.join(", ")).unwrap(),
        );
    }
    response
}
pub(super) async fn serve(service: Arc<Service>) -> Result<()> {
    let listener = tokio::net::TcpListener::bind(service.configuration.listen)
        .await
        .map_err(super::error)?;
    let slots = Arc::new(tokio::sync::Semaphore::new(32));
    let connection_timeout = service.configuration.connection_timeout();
    eprintln!(
        "Nehemiah A2A service listening on {}",
        service.configuration.listen
    );
    loop {
        let (socket, _) = listener.accept().await.map_err(super::error)?;
        let Ok(permit) = Arc::clone(&slots).try_acquire_owned() else {
            drop(socket);
            continue;
        };
        let cloned = Arc::clone(&service);
        tokio::spawn(async move {
            let _permit = permit;
            let handler = service_fn(move |request| {
                let service = Arc::clone(&cloned);
                async move { Ok::<_, Infallible>(handle(service, request).await) }
            });
            let mut http = hyper::server::conn::http1::Builder::new();
            http.keep_alive(false)
                .max_headers(32)
                .max_buf_size(16_384)
                .timer(TokioTimer::new())
                .header_read_timeout(Duration::from_secs(3));
            // Includes response-body writing, preventing unread responses from
            // holding the finite connection set indefinitely.
            let _ = tokio::time::timeout(
                connection_timeout,
                http.serve_connection(TokioIo::new(socket), handler),
            )
            .await;
        });
    }
}
