-- Refresh tokens (opaque, stored hashed, rotated on every use, grouped into families).
CREATE TABLE refresh_tokens (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    family_id   UUID NOT NULL,                      -- all tokens descended from one login
    token_hash  BYTEA NOT NULL UNIQUE,              -- SHA-256 of the opaque token
    expires_at  TIMESTAMPTZ NOT NULL,
    revoked_at  TIMESTAMPTZ,
    replaced_by UUID,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    user_agent  TEXT,
    ip_address  TEXT
);
CREATE INDEX refresh_tokens_user_idx ON refresh_tokens (user_id);
CREATE INDEX refresh_tokens_family_idx ON refresh_tokens (family_id);
CREATE INDEX refresh_tokens_expires_idx ON refresh_tokens (expires_at);

-- Single-use tokens for email verification and password reset.
CREATE TYPE one_time_token_kind AS ENUM ('email_verification', 'password_reset');

CREATE TABLE one_time_tokens (
    id          UUID PRIMARY KEY,
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        one_time_token_kind NOT NULL,
    token_hash  BYTEA NOT NULL UNIQUE,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX one_time_tokens_user_kind_idx ON one_time_tokens (user_id, kind);
CREATE INDEX one_time_tokens_expires_idx ON one_time_tokens (expires_at);
