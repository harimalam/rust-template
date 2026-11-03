-- Users, roles and permissions (RBAC).
CREATE EXTENSION IF NOT EXISTS citext;

CREATE TABLE users (
    id                UUID PRIMARY KEY,
    email             CITEXT NOT NULL,
    password_hash     TEXT,                         -- NULL for social-login-only accounts (see docs/architecture.md)
    full_name         TEXT NOT NULL,
    email_verified_at TIMESTAMPTZ,
    is_active         BOOLEAN NOT NULL DEFAULT TRUE,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at        TIMESTAMPTZ
);
-- Unique among live accounts only, so an email can be re-used after deletion.
CREATE UNIQUE INDEX users_email_live_idx ON users (email) WHERE deleted_at IS NULL;

CREATE TABLE roles (
    id          SMALLSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE permissions (
    id          SMALLSERIAL PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,               -- "<resource>:<action>", e.g. notes:write
    description TEXT NOT NULL DEFAULT ''
);

CREATE TABLE role_permissions (
    role_id       SMALLINT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    permission_id SMALLINT NOT NULL REFERENCES permissions (id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE user_roles (
    user_id UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id SMALLINT NOT NULL REFERENCES roles (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
);
CREATE INDEX user_roles_role_idx ON user_roles (role_id);

INSERT INTO roles (name, description) VALUES
    ('admin', 'Full access'),
    ('user',  'Regular authenticated user');

INSERT INTO permissions (name, description) VALUES
    ('notes:read',   'Read own notes'),
    ('notes:write',  'Create, update and delete own notes'),
    ('files:read',   'Read own files'),
    ('files:write',  'Upload and delete own files'),
    ('users:read',   'List and view all users'),
    ('users:manage', 'Assign roles and deactivate users');

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r JOIN permissions p ON
    (r.name = 'admin') OR
    (r.name = 'user' AND p.name IN ('notes:read', 'notes:write', 'files:read', 'files:write'));
