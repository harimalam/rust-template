//! Centralised, typed configuration loaded from environment variables.
//!
//! `Settings::from_env()` reads every variable, validates it and reports **all**
//! problems at once so a misconfigured deployment fails fast with a useful message.

use std::{collections::HashMap, fmt, str::FromStr, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppEnv {
    Development,
    Staging,
    Production,
}

impl AppEnv {
    pub fn is_production(self) -> bool {
        self == Self::Production
    }
}

impl FromStr for AppEnv {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "development" | "dev" | "local" => Ok(Self::Development),
            "staging" => Ok(Self::Staging),
            "production" | "prod" => Ok(Self::Production),
            _ => Err("expected development | staging | production".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Pretty,
    Json,
}

impl FromStr for LogFormat {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "pretty" => Ok(Self::Pretty),
            "json" => Ok(Self::Json),
            _ => Err("expected pretty | json".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    None,
    StartTls,
    Tls,
}

impl FromStr for SmtpTls {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "starttls" => Ok(Self::StartTls),
            "tls" => Ok(Self::Tls),
            _ => Err("expected none | starttls | tls".into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct HttpSettings {
    pub host: String,
    pub port: u16,
    pub public_base_url: String,
    pub max_body_bytes: usize,
    pub request_timeout: Duration,
    pub cors_allowed_origins: Vec<String>,
    pub trust_proxy_headers: bool,
    pub shutdown_timeout: Duration,
    /// Serve OpenAPI JSON and the Scalar UI. Defaults to off in production.
    pub docs_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct DatabaseSettings {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub run_migrations_on_start: bool,
}

#[derive(Debug, Clone)]
pub struct CacheSettings {
    pub url: String,
    pub key_prefix: String,
    pub default_ttl: Duration,
    /// Hard bound on any single cache operation, so a dead Valkey can't stall requests.
    pub op_timeout: Duration,
    /// After a connection-level failure, skip the cache entirely for this long.
    pub circuit_open: Duration,
}

#[derive(Debug, Clone)]
pub struct AuthSettings {
    pub jwt_secret: String,
    pub jwt_issuer: String,
    pub access_ttl: Duration,
    pub refresh_ttl: Duration,
    pub email_verification_ttl: Duration,
    pub password_reset_ttl: Duration,
    pub require_email_verification: bool,
}

#[derive(Debug, Clone)]
pub struct RateLimitSettings {
    pub auth_max: u32,
    pub auth_window: Duration,
    pub login_max_failed_attempts: u32,
    pub login_lockout: Duration,
}

#[derive(Debug, Clone)]
pub struct SmtpSettings {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: SmtpTls,
    pub from: String,
}

#[derive(Debug, Clone)]
pub struct S3Settings {
    pub endpoint: String,
    pub public_endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub force_path_style: bool,
    pub auto_create_bucket: bool,
    pub presign_ttl: Duration,
    pub max_upload_bytes: usize,
    pub allowed_mime_types: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct QueueSettings {
    pub url: String,
    pub prefetch: u16,
    pub job_max_attempts: u32,
    pub retry_base_delay: Duration,
}

#[derive(Debug, Clone)]
pub struct SchedulerSettings {
    pub cron_purge_expired_tokens: String,
    pub cron_purge_deleted_notes: String,
    pub soft_delete_retention_days: i64,
    /// Lock TTL per cron tick. Must be shorter than the shortest cron interval and longer than
    /// the clock skew between scheduler replicas.
    pub lock_ttl: Duration,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub env: AppEnv,
    pub log_level: String,
    pub log_format: LogFormat,
    pub http: HttpSettings,
    pub database: DatabaseSettings,
    pub cache: CacheSettings,
    pub auth: AuthSettings,
    pub rate_limit: RateLimitSettings,
    pub smtp: SmtpSettings,
    pub s3: S3Settings,
    pub queue: QueueSettings,
    pub scheduler: SchedulerSettings,
}

/// All configuration problems found while loading, reported together.
#[derive(Debug)]
pub struct ConfigError(pub Vec<String>);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "invalid configuration ({} problem(s)):", self.0.len())?;
        for problem in &self.0 {
            writeln!(f, "  - {problem}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigError {}

/// Reads variables from a map and accumulates errors instead of failing on the first.
struct Reader<'a> {
    vars: &'a HashMap<String, String>,
    errors: Vec<String>,
}

impl<'a> Reader<'a> {
    fn raw(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }

    fn string(&mut self, key: &str) -> String {
        match self.raw(key) {
            Some(v) => v.to_owned(),
            None => {
                self.errors.push(format!("{key} is required"));
                String::new()
            }
        }
    }

    fn string_or(&self, key: &str, default: &str) -> String {
        self.raw(key).unwrap_or(default).to_owned()
    }

    fn optional(&self, key: &str) -> Option<String> {
        self.raw(key).map(str::to_owned)
    }

    fn parse<T: FromStr>(&mut self, key: &str, default: T) -> T
    where
        T::Err: fmt::Display,
    {
        match self.raw(key).map(str::to_owned) {
            None => default,
            Some(v) => v.parse().unwrap_or_else(|e| {
                self.errors.push(format!("{key}={v:?} is invalid: {e}"));
                default
            }),
        }
    }

    fn boolean(&mut self, key: &str, default: bool) -> bool {
        match self.raw(key).map(str::to_ascii_lowercase).as_deref() {
            None => default,
            Some("true" | "1" | "yes" | "on") => true,
            Some("false" | "0" | "no" | "off") => false,
            Some(other) => {
                self.errors
                    .push(format!("{key}={other:?} is invalid: expected true | false"));
                default
            }
        }
    }

    fn secs(&mut self, key: &str, default: u64) -> Duration {
        Duration::from_secs(self.parse(key, default))
    }

    fn list(&self, key: &str, default: &str) -> Vec<String> {
        self.raw(key)
            .unwrap_or(default)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn check(&mut self, ok: bool, message: impl Into<String>) {
        if !ok {
            self.errors.push(message.into());
        }
    }
}

impl Settings {
    /// Loads `.env` (if present, development convenience) and then the process environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();
        Self::from_map(&std::env::vars().collect())
    }

    /// Builds settings from an explicit map. Used by `from_env` and by unit tests.
    pub fn from_map(vars: &HashMap<String, String>) -> Result<Self, ConfigError> {
        let mut r = Reader {
            vars,
            errors: Vec::new(),
        };

        let env: AppEnv = r.parse("APP_ENV", AppEnv::Development);
        let default_log_format = if env == AppEnv::Development {
            LogFormat::Pretty
        } else {
            LogFormat::Json
        };

        let http = HttpSettings {
            host: r.string_or("HTTP_HOST", "0.0.0.0"),
            port: r.parse("HTTP_PORT", 8080),
            public_base_url: r
                .string_or("PUBLIC_BASE_URL", "http://localhost:8080")
                .trim_end_matches('/')
                .to_owned(),
            max_body_bytes: r.parse("MAX_BODY_BYTES", 1024 * 1024),
            request_timeout: r.secs("REQUEST_TIMEOUT_SECS", 30),
            cors_allowed_origins: r.list("CORS_ALLOWED_ORIGINS", ""),
            trust_proxy_headers: r.boolean("TRUST_PROXY_HEADERS", false),
            shutdown_timeout: r.secs("SHUTDOWN_TIMEOUT_SECS", 30),
            docs_enabled: r.boolean("DOCS_ENABLED", !env.is_production()),
        };

        let database = DatabaseSettings {
            url: r.string("DATABASE_URL"),
            max_connections: r.parse("DATABASE_MAX_CONNECTIONS", 10),
            min_connections: r.parse("DATABASE_MIN_CONNECTIONS", 1),
            acquire_timeout: r.secs("DATABASE_ACQUIRE_TIMEOUT_SECS", 5),
            run_migrations_on_start: r.boolean("RUN_MIGRATIONS_ON_START", false),
        };

        let cache = CacheSettings {
            url: r.string("VALKEY_URL"),
            key_prefix: r.string_or("CACHE_KEY_PREFIX", "app"),
            default_ttl: r.secs("CACHE_DEFAULT_TTL_SECS", 300),
            op_timeout: Duration::from_millis(r.parse("CACHE_OP_TIMEOUT_MS", 300)),
            circuit_open: r.secs("CACHE_CIRCUIT_OPEN_SECS", 5),
        };

        let auth = AuthSettings {
            jwt_secret: r.string("JWT_SECRET"),
            jwt_issuer: r.string_or("JWT_ISSUER", "app"),
            access_ttl: r.secs("JWT_ACCESS_TTL_SECS", 900),
            refresh_ttl: r.secs("JWT_REFRESH_TTL_SECS", 30 * 24 * 3600),
            email_verification_ttl: r.secs("EMAIL_VERIFICATION_TTL_SECS", 24 * 3600),
            password_reset_ttl: r.secs("PASSWORD_RESET_TTL_SECS", 3600),
            require_email_verification: r.boolean("REQUIRE_EMAIL_VERIFICATION", true),
        };

        let rate_limit = RateLimitSettings {
            auth_max: r.parse("RATE_LIMIT_AUTH_MAX", 20),
            auth_window: r.secs("RATE_LIMIT_AUTH_WINDOW_SECS", 60),
            login_max_failed_attempts: r.parse("LOGIN_MAX_FAILED_ATTEMPTS", 5),
            login_lockout: r.secs("LOGIN_LOCKOUT_SECS", 900),
        };

        let smtp = SmtpSettings {
            host: r.string("SMTP_HOST"),
            port: r.parse("SMTP_PORT", 587),
            username: r.optional("SMTP_USERNAME"),
            password: r.optional("SMTP_PASSWORD"),
            tls: r.parse("SMTP_TLS", SmtpTls::StartTls),
            from: r.string("SMTP_FROM"),
        };

        let s3_endpoint = r.string("S3_ENDPOINT");
        let s3 = S3Settings {
            public_endpoint: r.string_or("S3_PUBLIC_ENDPOINT", &s3_endpoint),
            endpoint: s3_endpoint,
            region: r.string_or("S3_REGION", "us-east-1"),
            bucket: r.string("S3_BUCKET"),
            access_key: r.string("S3_ACCESS_KEY"),
            secret_key: r.string("S3_SECRET_KEY"),
            force_path_style: r.boolean("S3_FORCE_PATH_STYLE", true),
            auto_create_bucket: r.boolean("S3_AUTO_CREATE_BUCKET", false),
            presign_ttl: r.secs("S3_PRESIGN_TTL_SECS", 300),
            max_upload_bytes: r.parse("MAX_UPLOAD_BYTES", 10 * 1024 * 1024),
            allowed_mime_types: r.list(
                "UPLOAD_ALLOWED_MIME_TYPES",
                "image/png,image/jpeg,image/webp,application/pdf,text/plain",
            ),
        };

        let queue = QueueSettings {
            url: r.string("RABBITMQ_URL"),
            prefetch: r.parse("RABBITMQ_PREFETCH", 10),
            job_max_attempts: r.parse("JOB_MAX_ATTEMPTS", 5),
            retry_base_delay: r.secs("JOB_RETRY_BASE_DELAY_SECS", 5),
        };

        let scheduler = SchedulerSettings {
            cron_purge_expired_tokens: r.string_or("CRON_PURGE_EXPIRED_TOKENS", "0 0 * * * *"),
            cron_purge_deleted_notes: r.string_or("CRON_PURGE_DELETED_NOTES", "0 30 3 * * *"),
            soft_delete_retention_days: r.parse("SOFT_DELETE_RETENTION_DAYS", 30),
            lock_ttl: r.secs("CRON_LOCK_TTL_SECS", 30),
        };

        let log_level = r.string_or("LOG_LEVEL", "info");
        let log_format = r.parse("LOG_FORMAT", default_log_format);

        // Cross-field validation.
        r.check(
            auth.jwt_secret.is_empty() || auth.jwt_secret.len() >= 32,
            "JWT_SECRET must be at least 32 bytes (try: openssl rand -base64 48)",
        );
        r.check(
            !(env.is_production() && auth.jwt_secret.starts_with("change-me")),
            "JWT_SECRET still has the example value; set a real secret in production",
        );
        r.check(
            !(env.is_production() && http.cors_allowed_origins.iter().any(|o| o == "*")),
            "CORS_ALLOWED_ORIGINS must not be \"*\" in production",
        );
        r.check(
            database.min_connections <= database.max_connections,
            "DATABASE_MIN_CONNECTIONS must be <= DATABASE_MAX_CONNECTIONS",
        );
        r.check(queue.job_max_attempts >= 1, "JOB_MAX_ATTEMPTS must be >= 1");
        r.check(
            !cache.op_timeout.is_zero(),
            "CACHE_OP_TIMEOUT_MS must be > 0",
        );
        r.check(http.max_body_bytes > 0, "MAX_BODY_BYTES must be > 0");
        r.check(s3.max_upload_bytes > 0, "MAX_UPLOAD_BYTES must be > 0");
        r.check(
            smtp.username.is_some() == smtp.password.is_some(),
            "SMTP_USERNAME and SMTP_PASSWORD must be set together",
        );

        if r.errors.is_empty() {
            Ok(Self {
                env,
                log_level,
                log_format,
                http,
                database,
                cache,
                auth,
                rate_limit,
                smtp,
                s3,
                queue,
                scheduler,
            })
        } else {
            Err(ConfigError(r.errors))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_vars() -> HashMap<String, String> {
        [
            ("DATABASE_URL", "postgres://u:p@localhost/db"),
            ("VALKEY_URL", "redis://localhost:6379"),
            ("JWT_SECRET", "0123456789abcdef0123456789abcdef"),
            ("SMTP_HOST", "localhost"),
            ("SMTP_FROM", "App <a@example.com>"),
            ("S3_ENDPOINT", "http://localhost:9000"),
            ("S3_BUCKET", "b"),
            ("S3_ACCESS_KEY", "k"),
            ("S3_SECRET_KEY", "s"),
            ("RABBITMQ_URL", "amqp://localhost"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect()
    }

    #[test]
    fn loads_with_defaults() {
        let s = Settings::from_map(&valid_vars()).expect("valid config");
        assert_eq!(s.env, AppEnv::Development);
        assert_eq!(s.http.port, 8080);
        assert_eq!(s.log_format, LogFormat::Pretty);
        assert_eq!(s.s3.public_endpoint, s.s3.endpoint);
    }

    #[test]
    fn reports_all_problems_at_once() {
        let mut vars = valid_vars();
        vars.remove("DATABASE_URL");
        vars.insert("HTTP_PORT".into(), "not-a-port".into());
        vars.insert("JWT_SECRET".into(), "short".into());
        let err = Settings::from_map(&vars).unwrap_err();
        assert_eq!(err.0.len(), 3, "{err}");
    }

    #[test]
    fn production_rejects_example_secret_and_wildcard_cors() {
        let mut vars = valid_vars();
        vars.insert("APP_ENV".into(), "production".into());
        vars.insert(
            "JWT_SECRET".into(),
            "change-me-change-me-change-me-change-me".into(),
        );
        vars.insert("CORS_ALLOWED_ORIGINS".into(), "*".into());
        let err = Settings::from_map(&vars).unwrap_err();
        assert_eq!(err.0.len(), 2, "{err}");
    }

    #[test]
    fn production_defaults_to_json_logs() {
        let mut vars = valid_vars();
        vars.insert("APP_ENV".into(), "production".into());
        let s = Settings::from_map(&vars).unwrap();
        assert_eq!(s.log_format, LogFormat::Json);
    }
}
