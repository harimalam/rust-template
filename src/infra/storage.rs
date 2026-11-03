//! S3-compatible object storage (RustFS, MinIO, AWS S3, ...).

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use aws_sdk_s3::{
    Client,
    config::{
        BehaviorVersion, Credentials, Region, RequestChecksumCalculation,
        ResponseChecksumValidation,
    },
    presigning::PresigningConfig,
    primitives::ByteStream,
};

use crate::config::S3Settings;

#[async_trait]
pub trait ObjectStorage: Send + Sync {
    async fn put(&self, key: &str, content_type: &str, data: Vec<u8>) -> anyhow::Result<()>;
    async fn delete(&self, key: &str) -> anyhow::Result<()>;
    /// Time-limited download URL that clients can reach (built from `S3_PUBLIC_ENDPOINT`).
    async fn presigned_get_url(
        &self,
        key: &str,
        download_filename: &str,
        ttl: Duration,
    ) -> anyhow::Result<String>;
    async fn ping(&self) -> anyhow::Result<()>;
}

pub struct S3Storage {
    /// Talks to `S3_ENDPOINT` (in-network address inside containers).
    client: Client,
    /// Only used to sign URLs for `S3_PUBLIC_ENDPOINT`, which clients outside the network can reach.
    signer: Client,
    bucket: String,
}

fn build_client(settings: &S3Settings, endpoint: &str) -> Client {
    let credentials = Credentials::new(
        &settings.access_key,
        &settings.secret_key,
        None,
        None,
        "static-env",
    );
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new(settings.region.clone()))
        .endpoint_url(endpoint)
        .credentials_provider(credentials)
        .force_path_style(settings.force_path_style)
        // S3-compatible stores don't all accept the SDK's default flexible checksums.
        .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
        .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
        .build();
    Client::from_conf(config)
}

impl S3Storage {
    pub async fn connect(settings: &S3Settings) -> anyhow::Result<Self> {
        let storage = Self {
            client: build_client(settings, &settings.endpoint),
            signer: build_client(settings, &settings.public_endpoint),
            bucket: settings.bucket.clone(),
        };
        if settings.auto_create_bucket {
            storage.ensure_bucket().await?;
        }
        Ok(storage)
    }

    async fn ensure_bucket(&self) -> anyhow::Result<()> {
        if self
            .client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .is_ok()
        {
            return Ok(());
        }
        self.client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .with_context(|| format!("failed to create bucket {:?}", self.bucket))?;
        tracing::info!(bucket = %self.bucket, "created S3 bucket");
        Ok(())
    }
}

#[async_trait]
impl ObjectStorage for S3Storage {
    async fn put(&self, key: &str, content_type: &str, data: Vec<u8>) -> anyhow::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(data))
            .send()
            .await
            .context("S3 put_object failed")?;
        Ok(())
    }

    async fn delete(&self, key: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .context("S3 delete_object failed")?;
        Ok(())
    }

    async fn presigned_get_url(
        &self,
        key: &str,
        download_filename: &str,
        ttl: Duration,
    ) -> anyhow::Result<String> {
        // Strip characters that could break out of the quoted header value.
        let safe_name: String = download_filename
            .chars()
            .filter(|c| !c.is_control() && *c != '"')
            .collect();
        let request = self
            .signer
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .response_content_disposition(format!("attachment; filename=\"{safe_name}\""))
            .presigned(PresigningConfig::expires_in(ttl).context("invalid presign TTL")?)
            .await
            .context("failed to presign URL")?;
        Ok(request.uri().to_string())
    }

    async fn ping(&self) -> anyhow::Result<()> {
        self.client
            .head_bucket()
            .bucket(&self.bucket)
            .send()
            .await
            .context("S3 head_bucket failed")?;
        Ok(())
    }
}
