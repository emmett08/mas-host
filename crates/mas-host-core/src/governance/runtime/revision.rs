//! Mediate exact structured revision records before their existing journal sinks.
use super::{exposure::Exposure, schema, GovernedSession};
use serde_json::Value;
type Result<T> = std::result::Result<T, String>;

impl GovernedSession {
    pub(crate) fn revision_record(
        &self,
        kind: &str,
        value: &Value,
        exposure: &Exposure,
    ) -> Result<()> {
        let contract = self
            .0
            .revision
            .as_ref()
            .ok_or("governed-revision-unsupported")?;
        let actor = contract["actor"]
            .as_str()
            .ok_or("governed-revision-invalid")?;
        let schema = contract["schemas"]
            .get(kind)
            .ok_or("governed-revision-schema-unavailable")?;
        let choices = schema
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(std::slice::from_ref(schema));
        if !choices.iter().any(|candidate| {
            schema::project(candidate, value).is_ok_and(|projected| projected == *value)
        }) {
            return Err("governed-revision-record-would-change".into());
        }
        let inspection = self.inspect_boundary_with_exposure(actor, value, value, exposure)?;
        self.authorize_inspected_with_exposure(
            actor,
            "revision-retain",
            &format!("controller:{actor}"),
            &self.agent(actor)?.destination,
            value,
            value,
            inspection,
            exposure,
        )?;
        Ok(())
    }
    pub(crate) fn revision_delivery(&self, value: &Value, exposure: &Exposure) -> Result<()> {
        self.revision_record("delivery", &value["output"], exposure)?;
        let actor = self
            .0
            .revision
            .as_ref()
            .ok_or("governed-revision-unsupported")?["actor"]
            .as_str()
            .ok_or("governed-revision-invalid")?;
        self.authorize_status_with_exposure(actor, value, exposure)
    }
}
