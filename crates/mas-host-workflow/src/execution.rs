use crate::{
    plan::{checked_json, object, Node, Program},
    Result, MAX_BATCH, MAX_EVALUATIONS, MAX_ITERATIONS,
};
use mas_host_sdk::{Context, Request};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// Strategy boundary: applications can test composition independently of MAS.
/// An implementation must return exactly one object per request, in input order.
pub trait Runtime {
    fn requests(&mut self, requests: &[Request]) -> Result<Vec<Value>>;
}

struct SdkRuntime<'a>(&'a mut Context);
impl Runtime for SdkRuntime<'_> {
    fn requests(&mut self, requests: &[Request]) -> Result<Vec<Value>> {
        if requests.len() == 1 {
            let r = &requests[0];
            Ok(vec![self.0.request(&r.key, &r.target, r.input.clone())?])
        } else {
            self.0.parallel(requests)
        }
    }
}

#[derive(Clone)]
struct Environment {
    input: Arc<Value>,
    previous: Arc<Value>,
    item: Arc<Value>,
    iteration: Value,
}
impl Environment {
    fn after(&self, previous: Value) -> Self {
        Self {
            previous: Arc::new(previous),
            ..self.clone()
        }
    }
    fn iteration(&self, index: usize) -> Self {
        Self {
            iteration: json!({"index":index}),
            ..self.clone()
        }
    }
}

fn resolve(template: &Value, env: &Environment) -> Result<Value> {
    match template {
        Value::Object(fields) => {
            if fields.len() == 1 {
                let (key, pointer) = fields.iter().next().unwrap();
                let source = match key.as_str() {
                    "$input" => Some(env.input.as_ref()),
                    "$previous" => Some(env.previous.as_ref()),
                    "$item" => Some(env.item.as_ref()),
                    "$iteration" => Some(&env.iteration),
                    _ => None,
                };
                if let Some(source) = source {
                    let pointer = pointer.as_str().ok_or("invalid template pointer")?;
                    return source
                        .pointer(pointer)
                        .cloned()
                        .ok_or_else(|| format!("unresolved workflow binding {key}:{pointer}"));
                }
            }
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), resolve(v, env)?)))
                .collect::<Result<Map<_, _>>>()
                .map(Value::Object)
        }
        Value::Array(values) => values
            .iter()
            .map(|v| resolve(v, env))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        _ => Ok(template.clone()),
    }
}

enum State {
    Pending(Arc<Node>, Environment, String, usize),
    Waiting(Request),
    Done(Value),
    Sequence {
        nodes: Vec<Arc<Node>>,
        index: usize,
        child: Box<State>,
        env: Environment,
        path: String,
        depth: usize,
    },
    Parallel(Vec<State>),
    Repeat {
        max: usize,
        body: Arc<Node>,
        until: Value,
        index: usize,
        child: Box<State>,
        env: Environment,
        path: String,
        depth: usize,
    },
    ForEach {
        body: Arc<Node>,
        items: Vec<Value>,
        index: usize,
        results: Vec<Value>,
        child: Box<State>,
        env: Environment,
        path: String,
        depth: usize,
    },
}

fn pending(node: Arc<Node>, env: Environment, path: String, depth: usize) -> State {
    State::Pending(node, env, path, depth)
}

impl State {
    fn advance(&mut self, program: &Program, evaluations: &mut usize) -> Result<()> {
        loop {
            match self {
                Self::Waiting(_) | Self::Done(_) => return Ok(()),
                Self::Pending(_, _, _, _) => {
                    let Self::Pending(node, env, path, depth) =
                        std::mem::replace(self, Self::Done(Value::Null))
                    else {
                        unreachable!()
                    };
                    *evaluations += 1;
                    if *evaluations > MAX_EVALUATIONS || depth > 64 {
                        return Err("workflow execution budget exhausted".into());
                    }
                    *self = match node.as_ref() {
                        Node::Request { key, target, input } => {
                            let input = resolve(input, &env)?;
                            object(&input, "resolved request input")?;
                            checked_json(&input)?;
                            let digest = Sha256::digest(format!("{path}/request/{key}"));
                            let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                            Self::Waiting(Request::new(format!("wf-{hex}"), target, input))
                        }
                        Node::Value(output) => {
                            let output = resolve(output, &env)?;
                            object(&output, "resolved workflow output")?;
                            checked_json(&output)?;
                            Self::Done(output)
                        }
                        Node::Sequence(nodes) => Self::Sequence {
                            nodes: nodes.clone(),
                            index: 0,
                            child: Box::new(pending(
                                nodes[0].clone(),
                                env.clone(),
                                format!("{path}/s0"),
                                depth + 1,
                            )),
                            env,
                            path,
                            depth,
                        },
                        Node::Parallel(nodes) => Self::Parallel(
                            nodes
                                .iter()
                                .enumerate()
                                .map(|(i, n)| {
                                    pending(
                                        n.clone(),
                                        env.clone(),
                                        format!("{path}/p{i}"),
                                        depth + 1,
                                    )
                                })
                                .collect(),
                        ),
                        Node::Branch { condition, yes, no } => {
                            let condition = resolve(condition, &env)?
                                .as_bool()
                                .ok_or("branch condition must resolve to a boolean")?;
                            pending(
                                if condition { yes } else { no }.clone(),
                                env,
                                format!("{path}/b{condition}"),
                                depth + 1,
                            )
                        }
                        Node::Call(name) => pending(
                            program
                                .definitions
                                .get(name)
                                .ok_or("unknown workflow definition")?
                                .clone(),
                            env,
                            format!("{path}/call/{name}"),
                            depth + 1,
                        ),
                        Node::Repeat { max, body, until } => Self::Repeat {
                            max: *max,
                            body: body.clone(),
                            until: until.clone(),
                            index: 0,
                            child: Box::new(pending(
                                body.clone(),
                                env.iteration(0),
                                format!("{path}/r0"),
                                depth + 1,
                            )),
                            env,
                            path,
                            depth,
                        },
                        Node::ForEach {
                            items,
                            body,
                            parallel,
                        } => {
                            let items = resolve(items, &env)?.as_array().filter(|a| a.len() <= MAX_ITERATIONS).cloned().ok_or("foreach items must resolve to an array of at most 1024 elements")?;
                            if items.is_empty() {
                                Self::Done(json!({"results":[]}))
                            } else if *parallel {
                                Self::Parallel(
                                    items
                                        .iter()
                                        .enumerate()
                                        .map(|(i, item)| {
                                            let mut e = env.iteration(i);
                                            e.item = Arc::new(item.clone());
                                            pending(
                                                body.clone(),
                                                e,
                                                format!("{path}/f{i}"),
                                                depth + 1,
                                            )
                                        })
                                        .collect(),
                                )
                            } else {
                                let mut first = env.iteration(0);
                                first.item = Arc::new(items[0].clone());
                                Self::ForEach {
                                    body: body.clone(),
                                    items,
                                    index: 0,
                                    results: vec![],
                                    child: Box::new(pending(
                                        body.clone(),
                                        first,
                                        format!("{path}/f0"),
                                        depth + 1,
                                    )),
                                    env,
                                    path,
                                    depth,
                                }
                            }
                        }
                    };
                }
                Self::Sequence {
                    nodes,
                    index,
                    child,
                    env,
                    path,
                    depth,
                } => {
                    child.advance(program, evaluations)?;
                    let Self::Done(output) = child.as_ref() else {
                        return Ok(());
                    };
                    let output = output.clone();
                    *index += 1;
                    if *index == nodes.len() {
                        *self = Self::Done(output);
                    } else {
                        *env = env.after(output);
                        **child = pending(
                            nodes[*index].clone(),
                            env.clone(),
                            format!("{path}/s{index}"),
                            *depth + 1,
                        );
                    }
                }
                Self::Parallel(children) => {
                    for child in children.iter_mut() {
                        child.advance(program, evaluations)?;
                    }
                    if children.iter().all(|c| matches!(c, Self::Done(_))) {
                        let outputs: Vec<_> = children
                            .iter()
                            .map(|c| match c {
                                Self::Done(v) => v.clone(),
                                _ => unreachable!(),
                            })
                            .collect();
                        let output = json!({"results":outputs});
                        checked_json(&output)?;
                        *self = Self::Done(output);
                    } else {
                        return Ok(());
                    }
                }
                Self::Repeat {
                    max,
                    body,
                    until,
                    index,
                    child,
                    env,
                    path,
                    depth,
                } => {
                    child.advance(program, evaluations)?;
                    let Self::Done(output) = child.as_ref() else {
                        return Ok(());
                    };
                    let output = output.clone();
                    *env = env.after(output.clone()).iteration(*index);
                    let done = resolve(until, env)?
                        .as_bool()
                        .ok_or("repeat until must resolve to a boolean")?;
                    if done {
                        *self = Self::Done(output);
                    } else {
                        *index += 1;
                        if *index >= *max {
                            return Err(
                                "repeat reached max_iterations before until became true".into()
                            );
                        }
                        **child = pending(
                            body.clone(),
                            env.iteration(*index),
                            format!("{path}/r{index}"),
                            *depth + 1,
                        );
                    }
                }
                Self::ForEach {
                    body,
                    items,
                    index,
                    results,
                    child,
                    env,
                    path,
                    depth,
                } => {
                    child.advance(program, evaluations)?;
                    let Self::Done(output) = child.as_ref() else {
                        return Ok(());
                    };
                    let output = output.clone();
                    results.push(output.clone());
                    *index += 1;
                    if *index == items.len() {
                        let output = json!({"results":results});
                        checked_json(&output)?;
                        *self = Self::Done(output);
                    } else {
                        *env = env.after(output);
                        let mut next = env.iteration(*index);
                        next.item = Arc::new(items[*index].clone());
                        **child =
                            pending(body.clone(), next, format!("{path}/f{index}"), *depth + 1);
                    }
                }
            }
        }
    }

    fn requests(&self, requests: &mut Vec<Request>) {
        match self {
            Self::Waiting(r) => requests.push(r.clone()),
            Self::Sequence { child, .. }
            | Self::Repeat { child, .. }
            | Self::ForEach { child, .. } => child.requests(requests),
            Self::Parallel(children) => {
                for child in children {
                    child.requests(requests);
                }
            }
            _ => {}
        }
    }

    fn complete(&mut self, outputs: &mut impl Iterator<Item = Value>) -> Result<()> {
        match self {
            Self::Waiting(_) => {
                *self = Self::Done(outputs.next().ok_or("runtime omitted workflow result")?)
            }
            Self::Sequence { child, .. }
            | Self::Repeat { child, .. }
            | Self::ForEach { child, .. } => child.complete(outputs)?,
            Self::Parallel(children) => {
                for child in children {
                    child.complete(outputs)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// Execute a validated finite plan through a host SDK context.
pub fn execute(ctx: &mut Context, plan: &Value, input: Value) -> Result<Value> {
    execute_with(&mut SdkRuntime(ctx), plan, input)
}

/// Independently testable interpreter entrypoint. Remote requests are emitted in
/// deterministic traversal order and bounded batches; results retain plan order.
pub fn execute_with(runtime: &mut impl Runtime, plan: &Value, input: Value) -> Result<Value> {
    let program = Program::parse(plan, None)?;
    object(&input, "workflow input")?;
    checked_json(&input)?;
    let env = Environment {
        input: Arc::new(input.clone()),
        previous: Arc::new(input),
        item: Arc::new(Value::Null),
        iteration: json!({"index":0}),
    };
    let mut state = pending(program.root.clone(), env, "root".into(), 0);
    let mut evaluations = 0;
    loop {
        state.advance(&program, &mut evaluations)?;
        if let State::Done(output) = state {
            checked_json(&output)?;
            return Ok(output);
        }
        let mut requests = vec![];
        state.requests(&mut requests);
        if requests.is_empty() {
            return Err("workflow made no progress".into());
        }
        let mut outputs = vec![];
        for batch in requests.chunks(MAX_BATCH) {
            let result = runtime.requests(batch)?;
            if result.len() != batch.len() {
                return Err("runtime returned wrong number of workflow results".into());
            }
            for output in &result {
                object(output, "runtime result")?;
                checked_json(output)?;
            }
            outputs.extend(result);
        }
        state.complete(&mut outputs.into_iter())?;
    }
}
