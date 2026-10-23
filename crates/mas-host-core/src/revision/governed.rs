//! Preserve source restrictions across the revision controller's root invocations.
use crate::governance::runtime::exposure::{self, Exposure};
use crate::{lock, Inner, Result};
use serde_json::Value;
impl Inner {
    pub(super) fn revision_exposure(&self) -> Result<Exposure> {
        let Some(session) = &self.governance else {
            return Ok(Exposure::default());
        };
        let db = lock(&self.journal)?;
        let mut statement = db
            .prepare("SELECT id FROM steps WHERE kind='request' ORDER BY id")
            .map_err(crate::error)?;
        let ids = statement
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(crate::error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(crate::error)?;
        let mut all = Exposure::default();
        for id in ids {
            all = all.union(&exposure::load_invocation(&db, session, &id)?)?;
        }
        Ok(all)
    }
    pub(super) fn mediate_revision_record(&self, kind: &str, value: &Value) -> Result<()> {
        if let Some(session) = &self.governance {
            session.revision_record(kind, value, &self.revision_exposure()?)?;
        }
        Ok(())
    }
}
