//! Resolved authoring choices, before any executable is built or launched.
use mas_host_runtime::command::Specification;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Native {
    pub language: String,
    pub source: String,
    pub handler: String,
}

#[derive(Debug)]
pub enum Implementation {
    Native(Native),
    Llm(Value),
    Workflow(Value),
    Command(Specification),
}

#[derive(Debug)]
pub struct Agent {
    pub name: String,
    pub operation: String,
    pub implementation: Implementation,
    pub input_schema: Option<Value>,
    pub output_schema: Option<Value>,
    pub environment: Vec<String>,
    pub working_directory: PathBuf,
}

impl Agent {
    pub fn native(&self) -> Option<Value> {
        match &self.implementation {
            Implementation::Native(native) => Some(json!({"name":self.name,
                "language":native.language,"source":native.source,"handler":native.handler})),
            _ => None,
        }
    }
}
