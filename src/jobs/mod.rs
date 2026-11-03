//! Background job definitions. Add a variant here, handle it in [`handler`], publish it with
//! [`crate::infra::queue::JobPublisher`].

pub mod handler;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::infra::mail::Email;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Job {
    SendEmail(Email),
}

impl Job {
    pub fn name(&self) -> &'static str {
        match self {
            Self::SendEmail(_) => "send_email",
        }
    }
}

/// What actually travels through RabbitMQ.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobEnvelope {
    pub id: Uuid,
    /// Request/correlation ID of the HTTP request that enqueued the job, for log correlation.
    pub request_id: Option<String>,
    pub enqueued_at: DateTime<Utc>,
    pub job: Job,
}

impl JobEnvelope {
    pub fn new(job: Job, request_id: Option<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            request_id,
            enqueued_at: Utc::now(),
            job,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_as_tagged_json() {
        let job = Job::SendEmail(Email {
            to: "a@example.com".into(),
            subject: "s".into(),
            text: "t".into(),
            html: "<p>t</p>".into(),
        });
        let json = serde_json::to_value(JobEnvelope::new(job.clone(), Some("rid".into()))).unwrap();
        assert_eq!(json["job"]["type"], "send_email");
        let back: JobEnvelope = serde_json::from_value(json).unwrap();
        assert_eq!(back.job, job);
        assert_eq!(back.request_id.as_deref(), Some("rid"));
    }
}
