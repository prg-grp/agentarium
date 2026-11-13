-- Instances and their configuration revisions. Timestamps are unix milliseconds.

CREATE TABLE instances (
    id         TEXT PRIMARY KEY,                    -- UUIDv7, never reused
    name       TEXT NOT NULL UNIQUE COLLATE NOCASE,
    disk_path  TEXT NOT NULL,                       -- persistent root disk image
    kernel     TEXT NOT NULL,                       -- the guest kernel the disk's modules match (guest.rs)
    base       TEXT NOT NULL,                       -- the base rootfs the disk was cloned from (guest.rs); written once, read by nothing yet
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
    disk_gb           INTEGER NOT NULL CHECK (disk_gb > 0), -- GiB; the disk only grows
    command           TEXT NOT NULL,                -- from the preset, unless custom
    arguments         TEXT NOT NULL CHECK (json_valid(arguments)),     -- ["--flag", …]
    working_directory TEXT NOT NULL,
    environment       BLOB NOT NULL,                -- 12-byte nonce + AES-256-GCM ciphertext of [{name, value}]; key in the keychain (db.rs)
    mounts            TEXT NOT NULL CHECK (json_valid(mounts)),        -- [{host_path, guest_path, writable, enabled}]
    network_default   TEXT NOT NULL CHECK (network_default IN ('deny', 'allow')),
    network_rules     TEXT NOT NULL CHECK (json_valid(network_rules)), -- [{target, action}]
    network_allow_local INTEGER NOT NULL CHECK (network_allow_local IN (0, 1)), -- the Mac and local network
    port_forwards     TEXT NOT NULL CHECK (json_valid(port_forwards)), -- [{guest_port, host_port (null: random per boot), all_interfaces}]
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
-- and the other times are the guest's clock, which the guest sets from the Mac at boot.
-- `observer_run` is the observer process (Hello.run): the supervisor starts it again if it
-- exits, and its execution numbers count from 1 in each run. `execution` names the program
-- run that did something, as executions.execution within the same boot and observer run;
-- 0 if unknown.

-- Programs the agent and its descendants ran, and attempts the kernel refused.
CREATE TABLE executions (
    id             INTEGER PRIMARY KEY,
    boot_id        INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    observer_run   INTEGER NOT NULL,
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
CREATE UNIQUE INDEX executions_by_number ON executions (boot_id, observer_run, execution);

-- Files opened (the first time per process and access), deleted or renamed.
CREATE TABLE file_accesses (
    id             INTEGER PRIMARY KEY,
    boot_id        INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    observer_run   INTEGER NOT NULL,
    execution      INTEGER NOT NULL,
    pid            INTEGER NOT NULL,
    at             INTEGER NOT NULL,
    path           TEXT NOT NULL,
    host_path      TEXT,                            -- the Mac's path, inside a shared folder
    access         TEXT NOT NULL,                   -- e.g. "read", "write,create,truncate", "delete"
    new_path       TEXT,                            -- renames only
    path_truncated INTEGER NOT NULL DEFAULT 0 CHECK (path_truncated IN (0, 1))
) STRICT;
CREATE INDEX file_accesses_by_boot ON file_accesses (boot_id, id);

-- Network destinations as the guest saw them: TCP connections, UDP destinations (DNS
-- too). A TCP "connected" only means the Mac's network stack accepted the connection; what
-- happened next is in network_flows, matched by address, port and local_port.
CREATE TABLE connections (
    id           INTEGER PRIMARY KEY,
    boot_id      INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    observer_run INTEGER NOT NULL,
    execution    INTEGER NOT NULL,
    pid          INTEGER NOT NULL,
    at           INTEGER NOT NULL,
    protocol     TEXT NOT NULL CHECK (protocol IN ('tcp', 'udp', 'unknown')),
    address      TEXT NOT NULL,
    port         INTEGER NOT NULL,
    local_port   INTEGER NOT NULL DEFAULT 0,        -- the guest's source port; TCP only, 0 if unknown
    result       TEXT NOT NULL CHECK (result IN ('connected', 'failed', 'sent', 'unknown'))
) STRICT;
CREATE INDEX connections_by_boot ON connections (boot_id, id);

-- Changes of a process's user or group IDs or capabilities: sudo, su, setuid programs.
CREATE TABLE credential_changes (
    id                  INTEGER PRIMARY KEY,
    boot_id             INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    observer_run        INTEGER NOT NULL,
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
CREATE INDEX credential_changes_by_boot ON credential_changes (boot_id, id);

-- The observer's counters (Health), stored when events were lost, otherwise once a minute.
-- `at` is the app's clock.
CREATE TABLE collector_health (
    id           INTEGER PRIMARY KEY,
    boot_id      INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    observer_run INTEGER NOT NULL,
    at           INTEGER NOT NULL DEFAULT (CAST(unixepoch('subsec') * 1000 AS INTEGER)),
    lost_kernel  INTEGER NOT NULL,                  -- the kernel's buffer was full
    lost_queue   INTEGER NOT NULL,                  -- the observer's queue was full
    filtered     TEXT NOT NULL CHECK (json_valid(filtered)), -- {"repeated_open": 12, …}
    processes    INTEGER NOT NULL,
    sequence     INTEGER NOT NULL
) STRICT;
CREATE INDEX collector_health_by_boot ON collector_health (boot_id, id);

-- What the app's network stack on the Mac saw in agent boots (src/net.rs, src/netlog.rs).
-- Recorded outside the guest; `at` is the Mac's clock.

-- DNS lookups of the guest: every query to port 53 goes to the Mac's nameserver.
CREATE TABLE dns_lookups (
    id      INTEGER PRIMARY KEY,
    boot_id INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    at      INTEGER NOT NULL,
    name    TEXT NOT NULL,                          -- the queried name, lowercase, no trailing dot
    type    TEXT NOT NULL,                          -- "A", "AAAA", "HTTPS", …
    result  TEXT NOT NULL CHECK (result IN ('answered', 'refused')), -- refused: by the policy
    rcode   TEXT,                                   -- the answer's code, e.g. "NoError", "NameError"
    cnames  TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(cnames)) -- names of the CNAME chain
) STRICT;
CREATE INDEX dns_lookups_by_boot ON dns_lookups (boot_id, id);

-- The addresses an answered lookup returned.
CREATE TABLE dns_answers (
    lookup_id INTEGER NOT NULL REFERENCES dns_lookups(id) ON DELETE CASCADE,
    boot_id   INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    address   TEXT NOT NULL,
    ttl       INTEGER NOT NULL                      -- seconds
) STRICT;
CREATE INDEX dns_answers_by_lookup ON dns_answers (lookup_id);

-- TCP connections and UDP flows of the guest, as the Mac's policy decided them and, when
-- allowed, whether the Mac reached the destination. DNS isn't here; see dns_lookups.
CREATE TABLE network_flows (
    id         INTEGER PRIMARY KEY,
    boot_id    INTEGER NOT NULL REFERENCES boots(id) ON DELETE CASCADE,
    at         INTEGER NOT NULL,
    protocol   TEXT NOT NULL CHECK (protocol IN ('tcp', 'udp')),
    local_port INTEGER NOT NULL,                    -- the guest's source port
    address    TEXT NOT NULL,
    port       INTEGER NOT NULL,
    decision   TEXT NOT NULL CHECK (decision IN ('allowed', 'blocked')),
    rule       TEXT,                                -- the deciding rule's target; 'local network' when
                                                    -- the Mac and the local network are closed; NULL: the default action
    outcome    TEXT CHECK (outcome IS NULL OR outcome IN ('connected', 'failed', 'timed out')),
    error      TEXT                                 -- why connecting failed
) STRICT;
CREATE INDEX network_flows_by_boot ON network_flows (boot_id, id);

-- App-wide preferences, such as the update channel (update.rs).
CREATE TABLE preferences (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
