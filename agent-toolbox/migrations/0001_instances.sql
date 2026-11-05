-- Instances and their configuration revisions. Timestamps are unix milliseconds.

CREATE TABLE instances (
    id         TEXT PRIMARY KEY,                    -- UUIDv7, never reused
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    disk_path  TEXT NOT NULL,                       -- persistent root disk image
    provisioned_at INTEGER,                         -- NULL until a provisioning run succeeded
    created_at INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER))
) STRICT;

-- A revision is pending, and still editable, while booted_at is NULL.
-- The first boot that uses it freezes it.
CREATE TABLE instance_revisions (
    instance_id       TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    revision          INTEGER NOT NULL,             -- 1, 2, … per instance
    created_at        INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    booted_at         INTEGER,
    agent             TEXT NOT NULL CHECK (agent IN ('claude-code', 'custom')),
    cpus              INTEGER NOT NULL CHECK (cpus > 0),
    memory_mib        INTEGER NOT NULL CHECK (memory_mib > 0),
    command           TEXT NOT NULL,
    arguments         TEXT NOT NULL CHECK (json_valid(arguments)),     -- ["--flag", …]
    working_directory TEXT NOT NULL,
    environment       TEXT NOT NULL CHECK (json_valid(environment)),   -- [{name, value}]
    mounts            TEXT NOT NULL CHECK (json_valid(mounts)),        -- [{host_path, guest_path, writable, enabled}]
    network_default   TEXT NOT NULL CHECK (network_default IN ('deny', 'allow')),
    network_rules     TEXT NOT NULL CHECK (json_valid(network_rules)), -- [{target, action}]
    provision_script  TEXT NOT NULL,                -- bash, run as root in provision mode
    PRIMARY KEY (instance_id, revision)
) STRICT;
