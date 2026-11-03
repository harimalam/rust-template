-- Example CRUD resource: owner-scoped, audited, soft-deleted.
CREATE TABLE notes (
    id         UUID PRIMARY KEY,
    owner_id   UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title      TEXT NOT NULL,
    body       TEXT NOT NULL DEFAULT '',
    pinned     BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_by UUID NOT NULL REFERENCES users (id),
    updated_by UUID NOT NULL REFERENCES users (id),
    deleted_at TIMESTAMPTZ
);
-- Listing hot path: a user's live notes, newest first.
CREATE INDEX notes_owner_created_live_idx ON notes (owner_id, created_at DESC) WHERE deleted_at IS NULL;
-- Cron purge of old soft-deleted rows.
CREATE INDEX notes_deleted_at_idx ON notes (deleted_at) WHERE deleted_at IS NOT NULL;
