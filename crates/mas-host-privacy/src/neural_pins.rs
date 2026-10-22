//! Bind a neural request to the exact operator-approved local detector manifest.

use crate::{
    record::{object, string},
    Error, Limits, Result, Source,
};
use serde_json::{json, Value};

/// Identity pins correlate the worker with a locally verified manifest. They do
/// not establish model quality or grant permission to disclose source content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DetectorPins {
    bundle_sha256: String,
    code_revision: String,
    model_revision: String,
}

impl DetectorPins {
    pub fn new(bundle_sha256: &str, code_revision: &str, model_revision: &str) -> Result<Self> {
        hex(bundle_sha256, 64)?;
        hex(code_revision, 40)?;
        hex(model_revision, 40)?;
        Ok(Self {
            bundle_sha256: bundle_sha256.into(),
            code_revision: code_revision.into(),
            model_revision: model_revision.into(),
        })
    }

    pub fn from_value(value: &Value) -> Result<Self> {
        let record = object(
            value,
            &[
                "bundle_sha256",
                "code_revision",
                "model_revision",
                "backend",
                "adapter",
            ],
        )?;
        if string(record, "backend")? != "opf-cpu-bfloat16/1"
            || string(record, "adapter")? != "mas-privacy-opf/1"
        {
            return Err(Error::DetectorMismatch);
        }
        Self::new(
            string(record, "bundle_sha256")?,
            string(record, "code_revision")?,
            string(record, "model_revision")?,
        )
    }

    pub fn value(&self) -> Value {
        json!({"bundle_sha256":self.bundle_sha256,"code_revision":self.code_revision,
            "model_revision":self.model_revision,"backend":"opf-cpu-bfloat16/1","adapter":"mas-privacy-opf/1"})
    }

    /// The returned value contains raw text and belongs only on the admitted
    /// private local worker channel. It must never enter diagnostics or journals.
    pub fn request(&self, source: &Source, limits: Limits) -> Result<Value> {
        if source.content().len() > limits.max_input_bytes {
            return Err(Error::InputTooLarge);
        }
        let value = json!({"version":"mas-privacy-worker-request/1","source_id":source.id(),
            "source_token":source.token(),"input_bytes":source.content().len(),
            "text":source.content(),"detector":self.value()});
        if value.to_string().len() > limits.max_record_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(value)
    }
}

fn hex(value: &str, count: usize) -> Result<()> {
    if value.len() != count
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Error::InvalidIdentifier);
    }
    Ok(())
}
