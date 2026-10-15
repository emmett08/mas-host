use crate::{Result, MAX_BYTES, MAX_ITERATIONS, MAX_NODES};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone)]
pub(crate) enum Node {
    Request {
        key: String,
        target: String,
        input: Value,
    },
    Sequence(Vec<Arc<Node>>),
    Parallel(Vec<Arc<Node>>),
    Branch {
        condition: Value,
        yes: Arc<Node>,
        no: Arc<Node>,
    },
    Repeat {
        max: usize,
        body: Arc<Node>,
        until: Value,
    },
    ForEach {
        items: Value,
        body: Arc<Node>,
        parallel: bool,
    },
    Call(String),
    Value(Value),
}

pub(crate) struct Program {
    pub(crate) root: Arc<Node>,
    pub(crate) definitions: BTreeMap<String, Arc<Node>>,
}

pub(crate) fn object<'a>(value: &'a Value, label: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}

fn field_text<'a>(fields: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    fields
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("workflow {key} must be a non-empty string"))
}

fn required<'a>(fields: &'a Map<String, Value>, key: &str) -> Result<&'a Value> {
    fields
        .get(key)
        .ok_or_else(|| format!("workflow omitted {key}"))
}

fn fields_allowed(fields: &Map<String, Value>, allowed: &[&str], root: bool) -> Result<()> {
    if let Some(key) = fields
        .keys()
        .find(|k| !allowed.contains(&k.as_str()) && !(root && k.as_str() == "definitions"))
    {
        return Err(format!("unknown workflow field {key}"));
    }
    Ok(())
}

pub(crate) fn checked_json(value: &Value) -> Result<()> {
    // Bound traversal before serialising a caller-created Value.
    let mut stack = vec![(value, 0usize)];
    let mut values = 0usize;
    while let Some((v, depth)) = stack.pop() {
        values += 1;
        if depth > 32 || values > MAX_BYTES {
            return Err("workflow JSON exceeds structural limits".into());
        }
        match v {
            Value::Array(a) => stack.extend(a.iter().map(|v| (v, depth + 1))),
            Value::Object(o) => stack.extend(o.values().map(|v| (v, depth + 1))),
            _ => {}
        }
    }
    let bytes = serde_json::to_string(value).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("workflow JSON exceeds 65536 bytes".into());
    }
    mas_protocol_codec::parse_json(&bytes).map_err(|e| format!("invalid workflow JSON: {e:?}"))?;
    Ok(())
}

fn pointer_valid(pointer: &str) -> bool {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return false;
    }
    let mut chars = pointer.chars();
    while let Some(c) = chars.next() {
        if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
            return false;
        }
    }
    true
}

fn template_valid(value: &Value) -> Result<()> {
    match value {
        Value::Object(fields) => {
            if let Some((key, pointer)) = fields.iter().find(|(k, _)| k.starts_with('$')) {
                if fields.len() != 1
                    || !matches!(
                        key.as_str(),
                        "$input" | "$previous" | "$item" | "$iteration"
                    )
                {
                    return Err("template bindings must be single-field $input, $previous, $item or $iteration objects".into());
                }
                if !pointer.as_str().is_some_and(pointer_valid) {
                    return Err("template binding requires a valid JSON pointer".into());
                }
                return Ok(());
            }
            for child in fields.values() {
                template_valid(child)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                template_valid(child)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn binding(value: &Value) -> bool {
    value
        .as_object()
        .is_some_and(|o| o.len() == 1 && o.keys().next().is_some_and(|k| k.starts_with('$')))
}

fn boolean_template(value: &Value) -> Result<()> {
    template_valid(value)?;
    if value.is_boolean() || binding(value) {
        Ok(())
    } else {
        Err("workflow condition must be a boolean or a binding".into())
    }
}

fn parse_node(
    value: &Value,
    agents: Option<&BTreeSet<String>>,
    count: &mut usize,
    root: bool,
) -> Result<Arc<Node>> {
    *count += 1;
    if *count > MAX_NODES {
        return Err("workflow exceeds 4096 declared nodes".into());
    }
    let f = object(value, "workflow node")?;
    let kind = field_text(f, "type")?;
    let node = match kind {
        "request" => {
            fields_allowed(f, &["type", "key", "target", "input"], root)?;
            let key = field_text(f, "key")?;
            let target = field_text(f, "target")?;
            if key.len() > 128 || target.len() > 128 {
                return Err("workflow key or target exceeds 128 bytes".into());
            }
            if agents.is_some_and(|a| !a.contains(target)) {
                return Err(format!("unknown workflow target {target}"));
            }
            let input = required(f, "input")?.clone();
            object(&input, "request input template")?;
            template_valid(&input)?;
            Node::Request {
                key: key.into(),
                target: target.into(),
                input,
            }
        }
        "sequence" | "parallel" => {
            fields_allowed(f, &["type", "steps"], root)?;
            let steps = required(f, "steps")?
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or("workflow steps must be a non-empty array")?;
            let children = steps
                .iter()
                .map(|v| parse_node(v, agents, count, false))
                .collect::<Result<Vec<_>>>()?;
            if kind == "sequence" {
                Node::Sequence(children)
            } else {
                Node::Parallel(children)
            }
        }
        "branch" => {
            fields_allowed(f, &["type", "condition", "then", "else"], root)?;
            let condition = required(f, "condition")?.clone();
            boolean_template(&condition)?;
            Node::Branch {
                condition,
                yes: parse_node(required(f, "then")?, agents, count, false)?,
                no: parse_node(required(f, "else")?, agents, count, false)?,
            }
        }
        "repeat" => {
            fields_allowed(f, &["type", "max_iterations", "body", "until"], root)?;
            let max = required(f, "max_iterations")?
                .as_u64()
                .filter(|n| *n > 0 && *n <= MAX_ITERATIONS as u64)
                .ok_or("max_iterations must be an integer from 1 to 1024")?
                as usize;
            let until = required(f, "until")?.clone();
            boolean_template(&until)?;
            Node::Repeat {
                max,
                body: parse_node(required(f, "body")?, agents, count, false)?,
                until,
            }
        }
        "foreach" => {
            fields_allowed(f, &["type", "items", "body", "mode"], root)?;
            let items = required(f, "items")?.clone();
            template_valid(&items)?;
            if !items.is_array() && !binding(&items) {
                return Err("foreach items must be an array or a binding".into());
            }
            let parallel = match field_text(f, "mode")? {
                "parallel" => true,
                "sequence" => false,
                _ => return Err("foreach mode must be parallel or sequence".into()),
            };
            Node::ForEach {
                items,
                body: parse_node(required(f, "body")?, agents, count, false)?,
                parallel,
            }
        }
        "call" => {
            fields_allowed(f, &["type", "name"], root)?;
            Node::Call(field_text(f, "name")?.into())
        }
        "value" => {
            fields_allowed(f, &["type", "output"], root)?;
            let output = required(f, "output")?.clone();
            object(&output, "value output template")?;
            template_valid(&output)?;
            Node::Value(output)
        }
        _ => return Err(format!("unknown workflow type {kind}")),
    };
    Ok(Arc::new(node))
}

fn references(node: &Node, names: &mut BTreeSet<String>) {
    match node {
        Node::Call(name) => {
            names.insert(name.clone());
        }
        Node::Sequence(nodes) | Node::Parallel(nodes) => {
            for n in nodes {
                references(n, names);
            }
        }
        Node::Branch { yes, no, .. } => {
            references(yes, names);
            references(no, names);
        }
        Node::Repeat { body, .. } | Node::ForEach { body, .. } => references(body, names),
        _ => {}
    }
}

fn visit(
    name: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    visiting: &mut BTreeSet<String>,
    done: &mut BTreeSet<String>,
) -> Result<()> {
    if done.contains(name) {
        return Ok(());
    }
    if visiting.len() >= 64 {
        return Err("workflow definition depth exceeds 64".into());
    }
    if !visiting.insert(name.into()) {
        return Err(format!("cyclic workflow definition {name}"));
    }
    for child in graph
        .get(name)
        .ok_or_else(|| format!("unknown workflow definition {name}"))?
    {
        visit(child, graph, visiting, done)?;
    }
    visiting.remove(name);
    done.insert(name.into());
    Ok(())
}

fn expanded_height(
    node: &Node,
    definitions: &BTreeMap<String, Arc<Node>>,
    heights: &mut BTreeMap<String, usize>,
    depth: usize,
) -> Result<usize> {
    if depth > 64 {
        return Err("expanded workflow depth exceeds 64".into());
    }
    let children = match node {
        Node::Request { .. } | Node::Value(_) => 0,
        Node::Call(name) => {
            if let Some(height) = heights.get(name) {
                *height
            } else {
                let height = expanded_height(
                    definitions.get(name).ok_or("unknown workflow definition")?,
                    definitions,
                    heights,
                    depth + 1,
                )?;
                heights.insert(name.clone(), height);
                height
            }
        }
        Node::Sequence(nodes) | Node::Parallel(nodes) => {
            let mut height = 0;
            for child in nodes {
                height = height.max(expanded_height(child, definitions, heights, depth + 1)?);
            }
            height
        }
        Node::Branch { yes, no, .. } => expanded_height(yes, definitions, heights, depth + 1)?
            .max(expanded_height(no, definitions, heights, depth + 1)?),
        Node::Repeat { body, .. } | Node::ForEach { body, .. } => {
            expanded_height(body, definitions, heights, depth + 1)?
        }
    };
    let height = children + 1;
    if height > 64 {
        return Err("expanded workflow depth exceeds 64".into());
    }
    Ok(height)
}

impl Program {
    pub(crate) fn parse(plan: &Value, agents: Option<&BTreeSet<String>>) -> Result<Self> {
        checked_json(plan)?;
        let fields = object(plan, "workflow plan")?;
        let mut count = 0;
        let root = parse_node(plan, agents, &mut count, true)?;
        let mut definitions = BTreeMap::new();
        if let Some(values) = fields.get("definitions") {
            for (name, value) in object(values, "workflow definitions")? {
                if name.is_empty() || name.len() > 128 {
                    return Err("invalid workflow definition name".into());
                }
                definitions.insert(name.clone(), parse_node(value, agents, &mut count, false)?);
            }
        }
        let mut graph = BTreeMap::new();
        for (name, node) in &definitions {
            let mut names = BTreeSet::new();
            references(node, &mut names);
            graph.insert(name.clone(), names);
        }
        let mut root_refs = BTreeSet::new();
        references(&root, &mut root_refs);
        for name in root_refs {
            if !definitions.contains_key(&name) {
                return Err(format!("unknown workflow definition {name}"));
            }
        }
        let mut done = BTreeSet::new();
        for name in graph.keys() {
            visit(name, &graph, &mut BTreeSet::new(), &mut done)?;
        }
        let mut heights = BTreeMap::new();
        expanded_height(&root, &definitions, &mut heights, 0)?;
        for node in definitions.values() {
            expanded_height(node, &definitions, &mut heights, 0)?;
        }
        Ok(Self { root, definitions })
    }
}

/// Validate syntax, bindings, finite definitions and statically declared targets.
/// Conditional targets in unreachable branches are checked as well.
pub fn validate(plan: &Value, agents: &BTreeSet<String>) -> Result<()> {
    Program::parse(plan, Some(agents)).map(|_| ())
}
