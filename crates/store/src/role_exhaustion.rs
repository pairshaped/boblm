//! Provider quota or billing marks for role routing steps (ADR 0043).
#[path = "role_exhaustion.generated.rs"]
mod sql;

use crate::{Reader, Result, Store};
use boblm_tuning::{ExhaustedStep, StepId};

impl Reader {
    /// Read every mark that names a step this release still knows.
    pub fn role_exhaustions(&self) -> Result<Vec<ExhaustedStep>> {
        Ok(sql::all(&self.conn)?
            .into_iter()
            .filter_map(|row| {
                Some(ExhaustedStep {
                    step: StepId::parse(&row.step)?,
                    until: row.until,
                })
            })
            .collect())
    }
}

impl Store {
    /// Record or replace the exhaustion mark for one routing step.
    pub fn mark_role_step_exhausted(
        &self,
        step: StepId,
        until: i64,
        marked_at: i64,
        reason: &str,
    ) -> Result<()> {
        let mut conn = self.writer.lock().map_err(|_| crate::Error::Poisoned)?;
        let tx = conn.transaction()?;
        sql::mark(
            &tx,
            sql::MarkParams {
                step: step.as_str(),
                until,
                marked_at,
                reason,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
}
