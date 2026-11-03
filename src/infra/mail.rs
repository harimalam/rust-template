//! Outbound email behind a small trait so services and tests don't depend on SMTP.

use anyhow::Context;
use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, MultiPart},
    transport::smtp::authentication::Credentials,
};
use serde::{Deserialize, Serialize};

use crate::config::{SmtpSettings, SmtpTls};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub text: String,
    pub html: String,
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, email: &Email) -> anyhow::Result<()>;
}

pub struct SmtpMailer {
    transport: AsyncSmtpTransport<Tokio1Executor>,
    from: Mailbox,
}

impl SmtpMailer {
    pub fn new(settings: &SmtpSettings) -> anyhow::Result<Self> {
        let builder = match settings.tls {
            SmtpTls::None => {
                AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.host)
            }
            SmtpTls::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
                    .context("invalid SMTP_HOST for STARTTLS")?
            }
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
                .context("invalid SMTP_HOST for TLS")?,
        };
        let mut builder = builder.port(settings.port);
        if let (Some(user), Some(pass)) = (&settings.username, &settings.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        let from = settings
            .from
            .parse()
            .context("SMTP_FROM is not a valid mailbox")?;
        Ok(Self {
            transport: builder.build(),
            from,
        })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, email: &Email) -> anyhow::Result<()> {
        let message = Message::builder()
            .from(self.from.clone())
            .to(email.to.parse().context("invalid recipient address")?)
            .subject(&email.subject)
            .multipart(MultiPart::alternative_plain_html(
                email.text.clone(),
                email.html.clone(),
            ))
            .context("failed to build email")?;
        self.transport
            .send(message)
            .await
            .context("SMTP delivery failed")?;
        Ok(())
    }
}
