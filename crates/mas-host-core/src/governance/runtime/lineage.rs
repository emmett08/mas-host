//! Adapt host-owned invocation/effect journal transactions to source lineage.
use super::super::{
    exposure::{self, Exposure},
    session::is_unavailable,
};
use crate::{lock, read_step, Inner, Result, Step};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeSet;
impl Inner {
    pub(crate) fn governed_recipient_exposure(
        &self,
        recipient: &str,
        source: &Exposure,
    ) -> Result<Exposure> {
        if recipient.is_empty() {
            return Ok(source.clone());
        }
        let session = self
            .governance
            .as_ref()
            .ok_or("governed-session-unavailable")?;
        let db = lock(&self.journal)?;
        exposure::load_invocation(&db, session, recipient)?.union(source)
    }
    pub(crate) fn final_governed_delivery(
        &self,
        actor: &str,
        recipient_actor: &str,
        recipient: &str,
        value: &Value,
        source: &Exposure,
    ) -> Result<()> {
        let session = self
            .governance
            .as_ref()
            .ok_or("governed-session-unavailable")?;
        let current = self.governed_recipient_exposure(recipient, source)?;
        session.authorize_delivery_without_inspection(actor, recipient_actor, value, &current)?;
        // Current-authority callbacks run without a database lock. If one
        // changes recipient exposure, this admission cannot certify that cut.
        let final_context = self.governed_recipient_exposure(recipient, source)?;
        if current.sources.keys().ne(final_context.sources.keys())
            || current.roots != final_context.roots
            || current.pending != final_context.pending
            || crate::now_ms()? >= final_context.expiry()
        {
            return Err("governed-context-changed".into());
        }
        Ok(())
    }

    pub(crate) fn governed_context(
        &self,
        parent: &str,
        id: &str,
        frozen: Option<&Exposure>,
    ) -> Result<(Exposure, Exposure)> {
        let mut context = Exposure::default();
        if let Some(session) = self.governance.as_ref().filter(|s| s.is_v2()) {
            let db = lock(&self.journal)?;
            context = if read_step(&db, id)?.is_some() {
                exposure::load_context(&db, session, id)?
            } else if let Some(frozen) = frozen {
                frozen.clone()
            } else if parent.is_empty() {
                Exposure::default()
            } else {
                exposure::load_invocation(&db, session, parent)?
            };
            if !parent.is_empty() {
                exposure::load_invocation(&db, session, parent)?;
            }
        }
        let permission_context =
            if let Some(session) = self.governance.as_ref().filter(|s| s.is_v2()) {
                if !parent.is_empty() && frozen.is_none() {
                    let db = lock(&self.journal)?;
                    context.union(&exposure::load_invocation(&db, session, parent)?)?
                } else {
                    context.clone()
                }
            } else {
                context.clone()
            };
        Ok((context, permission_context))
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn bind_governed_step(
        &self,
        tx: &Connection,
        id: &str,
        kind: &str,
        target: &str,
        input: &Value,
        context: &Exposure,
        admitted_expiry: u64,
    ) -> Result<()> {
        if let Some(session) = self.governance.as_ref().filter(|s| s.is_v2()) {
            exposure::save_context(tx, session, id, context)?;
            if kind == "request" {
                let initial = exposure::create_source(
                    tx,
                    session,
                    id,
                    "request-input",
                    target,
                    input,
                    context,
                    &BTreeSet::new(),
                    true,
                    admitted_expiry,
                )?;
                exposure::save_invocation(tx, session, id, &initial)?;
            }
        }
        Ok(())
    }
    pub(crate) fn bind_governed_request_output(
        &self,
        tx: &Connection,
        id: &str,
        output: &Value,
        admitted_expiry: u64,
    ) -> Result<()> {
        if let Some(session) = self.governance.as_ref().filter(|s| s.is_v2()) {
            let step = read_step(tx, id)?.ok_or("governed-request-unavailable")?;
            let context = exposure::load_invocation(tx, session, id)?;
            let result = exposure::create_source(
                tx,
                session,
                id,
                "request-output",
                &step.target,
                output,
                &context,
                &BTreeSet::new(),
                !is_unavailable(output),
                admitted_expiry,
            )?;
            exposure::save_invocation(tx, session, id, &result)?;
        }
        Ok(())
    }
    pub(crate) fn begin_governed_effect(&self, invocation: &Step, step: &Step) -> Result<bool> {
        if let Some(session) = self.governance.as_ref().filter(|s| s.is_v2()) {
            let context = {
                let db = lock(&self.journal)?;
                exposure::load_invocation(&db, session, &invocation.id)?
            };
            let (_, decision) =
                session.prepare_effect_with_exposure(&step.actor, &step.input, &context)?;
            let admitted_expiry = super::super::GovernedSession::admitted_expiry(&decision)?;
            let policy = session.effect(
                &step.actor,
                step.input["format"]
                    .as_str()
                    .ok_or("governed-effect-format")?,
            )?;
            let mut db = lock(&self.journal)?;
            let tx = db.transaction().map_err(crate::error)?;
            exposure::begin_pending(
                &tx,
                session,
                &invocation.id,
                &step.id,
                &policy.source_policy_refs,
                &context,
                admitted_expiry,
            )?;
            tx.execute(
                "UPDATE steps SET state='executing',output=NULL,evidence=NULL WHERE id=?",
                [&step.id],
            )
            .map_err(crate::error)?;
            tx.commit().map_err(crate::error)?;
            return Ok(true);
        }
        Ok(false)
    }
    pub(crate) fn bind_governed_effect_output(
        &self,
        tx: &Connection,
        invocation: &Step,
        step: &Step,
        completion: &Value,
    ) -> Result<()> {
        let session = self
            .governance
            .as_ref()
            .ok_or("governed-session-unavailable")?;
        let context = exposure::load_invocation_settling(tx, session, &invocation.id)?;
        let policy = session.effect(
            &step.actor,
            step.input["format"]
                .as_str()
                .ok_or("governed-effect-format")?,
        )?;
        let admitted_expiry = exposure::load_pending(tx, session, &step.id)?
            .ok_or("governed-pending-unavailable")?["expires_at_ms"]
            .as_u64()
            .ok_or("governed-pending-invalid")?;
        let result = exposure::create_source(
            tx,
            session,
            &step.id,
            "effect-output",
            &step.actor,
            step.output.as_ref().ok_or("governed-result-unavailable")?,
            &context,
            &policy.source_policy_refs,
            completion["payload_disposition"] == "available",
            admitted_expiry,
        )?;
        exposure::expose(tx, session, &invocation.id, &result)?;
        exposure::finish_pending(tx, session, &invocation.id, &step.id)
    }
}
