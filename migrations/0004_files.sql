-- Metadata for objects stored in S3 / RustFS.
CREATE TABLE files (
    id           UUID PRIMARY KEY,
    owner_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    object_key   TEXT NOT NULL UNIQUE,
    filename     TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes   BIGINT NOT NULL CHECK (size_bytes >= 0),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at   TIMESTAMPTZ
);
CREATE INDEX files_owner_created_live_idx ON files (owner_id, created_at DESC) WHERE deleted_at IS NULL;
