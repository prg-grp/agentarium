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
    cpus              INTEGER NOT NULL CHECK (cpus > 0),
    memory_mib        INTEGER NOT NULL CHECK (memory_mib > 0),
    command           TEXT NOT NULL,                -- from the preset, unless custom
    arguments         TEXT NOT NULL CHECK (json_valid(arguments)),     -- ["--flag", …]
    working_directory TEXT NOT NULL,
    environment       TEXT NOT NULL CHECK (json_valid(environment)),   -- [{name, value}]
    mounts            TEXT NOT NULL CHECK (json_valid(mounts)),        -- [{host_path, guest_path, writable, enabled}]
    network_default   TEXT NOT NULL CHECK (network_default IN ('deny', 'allow')),
    network_rules     TEXT NOT NULL CHECK (json_valid(network_rules)), -- [{target, action}]
    network_allow_local INTEGER NOT NULL CHECK (network_allow_local IN (0, 1)), -- the Mac and local network
    provision_script  TEXT NOT NULL,                -- bash, run as root in provision mode
    PRIMARY KEY (instance_id, revision)
) STRICT;

-- Every boot of an instance. ended_at stays NULL while it runs, and if the app quit
-- before it noticed the end.
CREATE TABLE boots (
    id          INTEGER PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    revision    INTEGER NOT NULL,                   -- the configuration revision it used
    mode        TEXT NOT NULL CHECK (mode IN ('agent', 'maintenance', 'provision')),
    started_at  INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    ended_at    INTEGER
) STRICT;
CREATE INDEX boots_by_instance ON boots (instance_id);

-- What the guest observer saw in agent boots (data_plane/proto/data_plane.proto). `at`
-- and the other times are the guest's clock. `run` is the observer process
-- (Hello.run): the supervisor starts it again if it exits, and its execution numbers
-- count from 1 in each run. `execution` names the program run that did something, as
-- executions.execution within the same boot and run; 0 if unknown.

-- Programs the agent and its descendants ran, and attempts the kernel refused.
CREATE TABLE executions (
    id             INTEGER PRIMARY KEY,
    boot_id        INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    run            INTEGER NOT NULL,
    execution      INTEGER,                         -- NULL for a refused attempt
    parent         INTEGER NOT NULL,                -- the execution that started it; 0: none
    pid            INTEGER NOT NULL,
    ppid           INTEGER,
    started_at     INTEGER NOT NULL,
    uid            INTEGER,
    euid           INTEGER,
    gid            INTEGER,
    egid           INTEGER,
    path           TEXT NOT NULL,                   -- the program as asked for, e.g. a script
    binary         TEXT,                            -- the file the kernel ran
    argv           TEXT CHECK (argv IS NULL OR json_valid(argv)), -- ["git", "status"]
    argv_truncated INTEGER NOT NULL DEFAULT 0 CHECK (argv_truncated IN (0, 1)),
    cwd            TEXT,
    subcommand     TEXT,                            -- e.g. "status" for git, "pip" for python -m pip
    path_truncated INTEGER NOT NULL DEFAULT 0 CHECK (path_truncated IN (0, 1)),
    error          INTEGER,                         -- errno of a refused attempt
    exited_at      INTEGER,
    exit_code      INTEGER,                         -- -1 if ended by a signal
    exit_signal    INTEGER
) STRICT;
CREATE UNIQUE INDEX executions_by_number ON executions (boot_id, run, execution);

-- Files opened (the first time per process and access), deleted or renamed.
CREATE TABLE file_accesses (
    id             INTEGER PRIMARY KEY,
    boot_id        INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    run            INTEGER NOT NULL,
    execution      INTEGER NOT NULL,
    pid            INTEGER NOT NULL,
    at             INTEGER NOT NULL,
    path           TEXT NOT NULL,
    host_path      TEXT,                            -- the Mac's path, inside a shared folder
    access         TEXT NOT NULL,                   -- e.g. "read", "write,create,truncate", "delete"
    new_path       TEXT,                            -- renames only
    path_truncated INTEGER NOT NULL DEFAULT 0 CHECK (path_truncated IN (0, 1))
) STRICT;
CREATE INDEX file_accesses_by_execution ON file_accesses (boot_id, run, execution);

-- Network destinations: TCP connections with their outcome, UDP destinations (DNS too).
CREATE TABLE connections (
    id        INTEGER PRIMARY KEY,
    boot_id   INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    run       INTEGER NOT NULL,
    execution INTEGER NOT NULL,
    pid       INTEGER NOT NULL,
    at        INTEGER NOT NULL,
    protocol  TEXT NOT NULL CHECK (protocol IN ('tcp', 'udp', 'unknown')),
    address   TEXT NOT NULL,
    port      INTEGER NOT NULL,
    result    TEXT NOT NULL CHECK (result IN ('connected', 'failed', 'sent', 'unknown'))
) STRICT;
CREATE INDEX connections_by_execution ON connections (boot_id, run, execution);

-- Changes of a process's user or group IDs or capabilities: sudo, su, setuid programs.
CREATE TABLE credential_changes (
    id                  INTEGER PRIMARY KEY,
    boot_id             INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    run                 INTEGER NOT NULL,
    execution           INTEGER NOT NULL,
    pid                 INTEGER NOT NULL,
    at                  INTEGER NOT NULL,
    uid_before          INTEGER NOT NULL,
    uid_after           INTEGER NOT NULL,
    euid_before         INTEGER NOT NULL,
    euid_after          INTEGER NOT NULL,
    gid_before          INTEGER NOT NULL,
    gid_after           INTEGER NOT NULL,
    egid_before         INTEGER NOT NULL,
    egid_after          INTEGER NOT NULL,
    capabilities_raised INTEGER NOT NULL CHECK (capabilities_raised IN (0, 1))
) STRICT;
CREATE INDEX credential_changes_by_execution ON credential_changes (boot_id, run, execution);

-- The observer's counters (Health), stored when events were lost, otherwise once a minute. `at` is the app's clock.
CREATE TABLE collector_health (
    id          INTEGER PRIMARY KEY,
    boot_id     INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    run         INTEGER NOT NULL,
    at          INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    lost_kernel INTEGER NOT NULL,                   -- the kernel's buffer was full
    lost_queue  INTEGER NOT NULL,                   -- the observer's queue was full
    filtered    TEXT NOT NULL CHECK (json_valid(filtered)), -- {"repeated_open": 12, …}
    processes   INTEGER NOT NULL,
    sequence    INTEGER NOT NULL
) STRICT;
