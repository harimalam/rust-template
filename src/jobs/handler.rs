use std::sync::Arc;

use async_trait::async_trait;

use super::{Job, JobEnvelope};
use crate::infra::{mail::Mailer, queue::consumer::JobHandler};

/// Dispatches each job type to the code that performs it.
pub struct AppJobHandler {
    pub mailer: Arc<dyn Mailer>,
}

#[async_trait]
impl JobHandler for AppJobHandler {
    async fn handle(&self, envelope: &JobEnvelope) -> anyhow::Result<()> {
        match &envelope.job {
            Job::SendEmail(email) => self.mailer.send(email).await,
        }
    }
}
