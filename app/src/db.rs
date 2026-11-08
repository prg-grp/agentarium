//! The local SQLite database: instances and their configuration revisions.
//!
//! It lives in the user's Application Support folder and is used on the main
//! thread; the queries here are small and rare.
//!
//! Each saved configuration is a revision. The latest revision stays editable
//! while it is pending (never booted); the first boot that uses it freezes it,
//! so the configuration of every run stays on record.

use std::error::Error;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use objc2_foundation::{NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Type, ValueRef};
use rusqlite::{Connection, OptionalExtension, Row, params};
use rusqlite_migration::{M, Migrations, SchemaVersion};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// The app identifier; names the app's Application Support folder.
pub const APP_ID: &str = "com.programming-group.agentarium";
const FILE_NAME: &str = "agentarium.sqlite";

// Users' databases are upgraded in place when the app starts, so migrations are
// append-only: never edit, reorder or delete one that has shipped. Every schema
// change is a new file. A database newer than the app is refused, not touched.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!(
        "../migrations/0001_instances.sql"
    ))])
}

pub struct Db {
    conn: Connection,
}

pub struct Instance {
    pub id: String,
    pub name: String,
    /// The persistent root disk image.
    pub disk_path: PathBuf,
    /// The launch command of the latest revision.
    pub command: String,
    /// Whether a provisioning run succeeded on the disk; until then, starting the
    /// instance provisions it first.
    pub provisioned: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkAction {
    Deny,
    Allow,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct Mount {
    pub host_path: PathBuf,
    pub guest_path: String,
    pub writable: bool,
    pub enabled: bool,
}

/// A network rule: what happens to connections to its target.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NetworkRule {
    pub target: RuleTarget,
    pub action: NetworkAction,
}

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum RuleTarget {
    /// A domain name, matched through DNS results.
    Domain {
        name: String,
        include_subdomains: bool,
    },
    /// An address range in `address/prefix` form with the host bits cleared; a single
    /// address is a /32 (IPv4) or /128 (IPv6).
    Cidr { network: String },
}

impl RuleTarget {
    /// Whether this rule's target includes some of `other`: the same domain, a subdomain
    /// when this rule includes subdomains, or an overlapping address range. Domains and
    /// addresses aren't compared: which addresses a domain has is only known from DNS.
    pub fn covers(&self, other: &RuleTarget) -> bool {
        match (self, other) {
            (
                RuleTarget::Domain {
                    name: rule,
                    include_subdomains,
                },
                RuleTarget::Domain { name, .. },
            ) => name == rule || (*include_subdomains && name.ends_with(&format!(".{rule}"))),
            (RuleTarget::Cidr { network: a }, RuleTarget::Cidr { network: b }) => {
                match (parse_network(a), parse_network(b)) {
                    (Some((IpAddr::V4(a), pa)), Some((IpAddr::V4(b), pb))) => {
                        (u32::from(a) ^ u32::from(b)) & mask32(pa.min(pb)) == 0
                    }
                    (Some((IpAddr::V6(a), pa)), Some((IpAddr::V6(b), pb))) => {
                        (u128::from(a) ^ u128::from(b)) & mask128(pa.min(pb)) == 0
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

pub fn parse_network(network: &str) -> Option<(IpAddr, u8)> {
    let (address, prefix) = network.split_once('/')?;
    Some((address.parse().ok()?, prefix.parse().ok()?))
}

pub fn max_prefix(address: IpAddr) -> u8 {
    if address.is_ipv4() { 32 } else { 128 }
}

pub fn mask32(prefix: u8) -> u32 {
    u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0)
}

pub fn mask128(prefix: u8) -> u128 {
    u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0)
}

/// An instance's configuration, as saved in one revision.
#[derive(Clone, PartialEq, Debug)]
pub struct Config {
    pub cpus: u32,
    pub memory_mib: u32,
    pub command: String,
    pub arguments: Vec<String>,
    pub working_directory: String,
    pub environment: Vec<EnvVar>,
    pub mounts: Vec<Mount>,
    /// What happens to destinations without a rule.
    pub network_default: NetworkAction,
    pub network_rules: Vec<NetworkRule>,
    /// Bash script run as root in provision mode, before the first agent boot.
    pub provision_script: String,
}

/// Everything but the agent: its command, provisioning script and network rules come
/// from the preset chosen at creation.
impl Default for Config {
    fn default() -> Self {
        Config {
            // Claude Code is a Node process, and builds inside the guest need memory too.
            // The VM only takes memory from the Mac as the guest uses it.
            cpus: 2,
            memory_mib: 4096,
            command: String::new(),
            arguments: Vec::new(),
            // The agent user's ~/workspace, which the preset scripts create and trust.
            working_directory: "/home/ubuntu/workspace".into(),
            environment: Vec::new(),
            mounts: Vec::new(),
            network_default: NetworkAction::Deny,
            network_rules: Vec::new(),
            provision_script: String::new(),
        }
    }
}

pub struct Revision {
    pub number: i64,
    pub config: Config,
}

/// The app's folder in Application Support, created if needed. It holds the database
/// and, under `instances/<id>`, each instance's files.
pub fn app_dir() -> Result<PathBuf> {
    let dir = NSFileManager::defaultManager()
        .URLsForDirectory_inDomains(
            NSSearchPathDirectory::ApplicationSupportDirectory,
            NSSearchPathDomainMask::UserDomainMask,
        )
        .firstObject()
        .and_then(|url| url.to_file_path())
        .ok_or("Couldn't find the Application Support folder.")?
        .join(APP_ID);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// An instance's folder, holding its root disk (`root.ext4`).
pub fn instance_dir(id: &str) -> Result<PathBuf> {
    Ok(app_dir()?.join("instances").join(id))
}

impl Db {
    /// Opens the database and brings its schema up to date.
    pub fn open() -> Result<Db> {
        let dir = app_dir()?;
        let mut conn = Connection::open(dir.join(FILE_NAME))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", true)?;
        conn.busy_timeout(Duration::from_secs(5))?;

        // Keep a copy before upgrading an existing database, so a failed upgrade can be undone.
        let migrations = migrations();
        if let SchemaVersion::Inside(version) = migrations.current_version(&conn)?
            && migrations.pending_migrations(&conn)? > 0
        {
            let backup = dir.join(format!("{FILE_NAME}.v{version}.bak"));
            if !backup.exists() {
                conn.execute("VACUUM INTO ?1", [backup.to_string_lossy()])?;
            }
        }
        migrations.to_latest(&mut conn)?;
        Ok(Db { conn })
    }

    pub fn instances(&self) -> Result<Vec<Instance>> {
        let mut statement = self
            .conn
            .prepare(&format!("{INSTANCE_QUERY} ORDER BY i.name"))?;
        let instances = statement
            .query_map([], instance_from_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(instances)
    }

    pub fn instance(&self, id: &str) -> Result<Instance> {
        Ok(self.conn.query_row(
            &format!("{INSTANCE_QUERY} WHERE i.id = ?1"),
            [id],
            instance_from_row,
        )?)
    }

    /// Creates an instance with `config` as its first revision. The caller picks the
    /// id (a UUIDv7), so the disk can be created in `instance_dir(id)` beforehand.
    pub fn create_instance(
        &self,
        id: &str,
        name: &str,
        disk_path: &Path,
        config: &Config,
    ) -> Result<Instance> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO instances (id, name, disk_path) VALUES (?1, ?2, ?3)",
            params![id, name, disk_path.to_string_lossy()],
        )?;
        put_revision(&tx, id, 1, config)?;
        tx.commit()?;
        Ok(Instance {
            id: id.into(),
            name: name.into(),
            disk_path: disk_path.into(),
            command: config.command.clone(),
            provisioned: false,
        })
    }

    pub fn latest_revision(&self, instance_id: &str) -> Result<Revision> {
        Ok(self.conn.query_row(
            &format!("{REVISION_QUERY} ORDER BY revision DESC LIMIT 1"),
            [instance_id],
            revision_from_row,
        )?)
    }

    /// The newest revision that was booted: the one the instance runs, or ran last.
    pub fn active_revision(&self, instance_id: &str) -> Result<Option<Revision>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "{REVISION_QUERY} AND booted_at IS NOT NULL ORDER BY revision DESC LIMIT 1"
                ),
                [instance_id],
                revision_from_row,
            )
            .optional()?)
    }

    pub fn rename_instance(&self, id: &str, name: &str) -> Result<()> {
        self.conn
            .execute("UPDATE instances SET name = ?2 WHERE id = ?1", [id, name])?;
        Ok(())
    }

    /// Saves `config` for the next boot: into the pending revision if there is
    /// one, otherwise into a new revision. Returns the revision number.
    pub fn save_config(&self, instance_id: &str, config: &Config) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let (latest, booted_at): (i64, Option<i64>) = tx.query_row(
            "SELECT revision, booted_at FROM instance_revisions WHERE instance_id = ?1
             ORDER BY revision DESC LIMIT 1",
            [instance_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let number = if booted_at.is_none() {
            latest
        } else {
            latest + 1
        };
        put_revision(&tx, instance_id, number, config)?;
        tx.commit()?;
        Ok(number)
    }

    /// Records that `revision` was booted, which freezes it.
    pub fn mark_booted(&self, instance_id: &str, revision: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE instance_revisions SET booted_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE instance_id = ?1 AND revision = ?2 AND booted_at IS NULL",
            params![instance_id, revision],
        )?;
        Ok(())
    }

    /// Deletes the instance with its revisions; its files are the caller's.
    pub fn delete_instance(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM instances WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn revision_count(&self, instance_id: &str) -> Result<i64> {
        Ok(self.conn.query_row(
            "SELECT count(*) FROM instance_revisions WHERE instance_id = ?1",
            [instance_id],
            |row| row.get(0),
        )?)
    }

    /// Records that a provisioning run succeeded on the instance's disk.
    pub fn mark_provisioned(&self, instance_id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE instances SET provisioned_at = CAST(unixepoch('subsec') * 1000 AS INTEGER)
             WHERE id = ?1",
            [instance_id],
        )?;
        Ok(())
    }
}

const INSTANCE_QUERY: &str = "SELECT i.id, i.name, i.disk_path,
    (SELECT r.command FROM instance_revisions r WHERE r.instance_id = i.id
     ORDER BY r.revision DESC LIMIT 1),
    i.provisioned_at IS NOT NULL
    FROM instances i";

const REVISION_QUERY: &str = "SELECT revision, cpus, memory_mib, command, arguments,
    working_directory, environment, mounts, network_default, network_rules, provision_script
    FROM instance_revisions WHERE instance_id = ?1";

fn revision_from_row(row: &Row) -> rusqlite::Result<Revision> {
    Ok(Revision {
        number: row.get(0)?,
        config: Config {
            cpus: row.get(1)?,
            memory_mib: row.get(2)?,
            command: row.get(3)?,
            arguments: json(row, 4)?,
            working_directory: row.get(5)?,
            environment: json(row, 6)?,
            mounts: json(row, 7)?,
            network_default: row.get(8)?,
            network_rules: json(row, 9)?,
            provision_script: row.get(10)?,
        },
    })
}

fn instance_from_row(row: &Row) -> rusqlite::Result<Instance> {
    Ok(Instance {
        id: row.get(0)?,
        name: row.get(1)?,
        disk_path: PathBuf::from(row.get::<_, String>(2)?),
        command: row.get(3)?,
        provisioned: row.get(4)?,
    })
}

/// Inserts a revision, or updates it if it exists and is still pending.
fn put_revision(conn: &Connection, instance_id: &str, number: i64, config: &Config) -> Result<()> {
    conn.execute(
        "INSERT INTO instance_revisions (instance_id, revision, cpus, memory_mib, command,
             arguments, working_directory, environment, mounts, network_default, network_rules,
             provision_script)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT (instance_id, revision) DO UPDATE SET
             cpus = excluded.cpus, memory_mib = excluded.memory_mib,
             command = excluded.command, arguments = excluded.arguments,
             working_directory = excluded.working_directory, environment = excluded.environment,
             mounts = excluded.mounts, network_default = excluded.network_default,
             network_rules = excluded.network_rules, provision_script = excluded.provision_script
         WHERE booted_at IS NULL",
        params![
            instance_id,
            number,
            config.cpus,
            config.memory_mib,
            config.command,
            serde_json::to_string(&config.arguments)?,
            config.working_directory,
            serde_json::to_string(&config.environment)?,
            serde_json::to_string(&config.mounts)?,
            config.network_default,
            serde_json::to_string(&config.network_rules)?,
            config.provision_script,
        ],
    )?;
    Ok(())
}

/// Reads a JSON column.
fn json<T: DeserializeOwned>(row: &Row, index: usize) -> rusqlite::Result<T> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(index, Type::Text, error.into()))
}

impl ToSql for NetworkAction {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            NetworkAction::Deny => "deny",
            NetworkAction::Allow => "allow",
        }
        .into())
    }
}

impl FromSql for NetworkAction {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value.as_str()? {
            "deny" => Ok(NetworkAction::Deny),
            "allow" => Ok(NetworkAction::Allow),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}
