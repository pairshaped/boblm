#[path = "usage.generated.rs"]
pub(crate) mod sql;
use crate::{Reader, Result};

#[derive(Debug)]
pub struct UsageSamples {
    pub context: Option<(String, i64)>,
}
impl Reader {
    /// Context belongs to this chat. Account allowances are read from `account_usage`.
    pub fn usage_samples(&self, session: &str) -> Result<UsageSamples> {
        let context = sql::context(&self.conn, sql::ContextParams { session })?
            .pop()
            .and_then(|row| row.observed_at.map(|at| (row.payload, at)));
        Ok(UsageSamples { context })
    }
}
