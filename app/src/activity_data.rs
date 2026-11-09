//! The Activity tab's data: what one agent run did, shaped into the tab's views on a
//! thread of its own (see activity.rs for the UI side).
//!
//! The loader keeps the run's rows in memory and reads only what is new on each request;
//! executions are read in full, as their exits update them. Then it builds the requested
//! view from scratch: a few thousand rows take milliseconds.
//!
//! Nothing here knows the agent's harness. The agent process is the program the supervisor
//! started (`parent = 0`) and whatever it ran in its place; every process it started is a
//! command, and everything that command started belongs to it. A process that replaced
//! itself with another program (same pid) stays one process ("dpkg → dpkg-query").
//!
//! Network evidence comes from two sides: the guest observer says which process connected
//! where; the Mac's network log (netlog.rs) says what the policy decided and whether the
//! destination was reached. A guest TCP connection and its flow on the Mac share the
//! destination and the guest's source port. Domain names come from the guest's own DNS
//! lookups that returned the address, never from reverse lookups.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::mpsc::{self, Sender};

use rusqlite::{Connection, params};

use crate::db;
use crate::policy::{DNS_GRACE, LOCAL, MAX_TTL};

/// Local time of a stored `at` (unix ms), with milliseconds.
const TIME: &str = "strftime('%H:%M:%f', at / 1000.0, 'unixepoch', 'localtime')";
/// How far apart in time a guest connection and its flow on the Mac may be.
const MATCH_WINDOW_MS: i64 = 5_000;
/// The most rows the All events view lists.
const EVENT_LIMIT: usize = 2_000;
/// The most file changes and read folders listed under one process.
const LIST_LIMIT: usize = 30;
/// The most entries in the detail pane.
const ENTRY_LIMIT: usize = 300;
/// The most changed files a detail page lists.
const DETAIL_LIMIT: usize = 50;
const SHELLS: &[&str] = &["sh", "bash", "dash", "zsh", "fish", "ash", "ksh"];

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Commands,
    Files,
    Network,
    Events,
}

/// What the tab shows: one run, one view, with its filters.
pub struct Request {
    /// Counts up with each request; the UI keeps only the answer to the latest.
    pub generation: u64,
    pub boot: i64,
    /// The run is still going: a process without an exit is running.
    pub live: bool,
    pub view: View,
    pub search: String,
    pub show_reads: bool,
    pub failed_only: bool,
    /// All events only: processes, files, network.
    pub kinds: [bool; 3],
    /// Rows whose expansion the user flipped. Commands start collapsed, everything else
    /// expanded.
    pub toggled: HashSet<String>,
    /// The row whose details are shown.
    pub selected: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tone {
    Neutral,
    Muted,
    Warn,
    Bad,
}

/// One row of a view. `key` is unique within the view; the part before `#` names what the
/// detail pane shows for it.
pub struct Row {
    pub key: String,
    pub depth: i32,
    pub heading: bool,
    pub expandable: bool,
    pub expanded: bool,
    pub time: String,
    pub label: String,
    pub title: String,
    pub subtitle: String,
    pub subtitle_tone: Tone,
    pub meta: String,
    pub result: String,
    pub tone: Tone,
    pub dim: bool,
}

pub struct Field {
    pub label: String,
    pub value: String,
    pub mono: bool,
}

pub struct Entry {
    pub time: String,
    pub text: String,
    pub tone: Tone,
}

/// A detail page: a summary in sections first, the full record last.
#[derive(Default)]
pub struct Detail {
    /// "Command", "Process", "Destination", "DNS name", "File", …
    pub kind: String,
    /// One line: the command, the domain or address, the path.
    pub headline: String,
    pub facts: Vec<Field>,
    /// Shells: the script they ran; other programs: the command line. "" otherwise.
    pub script: String,
    pub argv: Vec<String>,
    pub argv_note: String,
    pub sections: Vec<Section>,
    /// Everything in order; the UI shows it collapsed.
    pub record: Vec<Entry>,
    pub note: String,
}

pub struct Section {
    pub title: String,
    /// Shown under the title, e.g. how sure an association is.
    pub note: String,
    pub items: Vec<Item>,
}

/// One line of a section; `key` names the page it opens, or is "".
pub struct Item {
    pub depth: i32,
    pub text: String,
    pub detail: String,
    pub tone: Tone,
    pub key: String,
    pub mono: bool,
}

pub struct Loaded {
    pub generation: u64,
    /// For the counts bar, e.g. "12 commands".
    pub counts: Vec<String>,
    /// "2 blocked", or "" when nothing was.
    pub blocked: String,
    pub monitoring: String,
    pub monitoring_tone: Tone,
    pub rows: Vec<Row>,
    /// Shown under the list, e.g. "Showing the newest 2,000 of 5,312 events."
    pub note: String,
    /// Shown instead of the list when it is empty.
    pub empty: String,
    pub detail: Option<Detail>,
}

/// Builds the views on a thread of its own. Only the newest waiting request is answered.
pub struct Loader {
    requests: Sender<Request>,
}

impl Loader {
    pub fn new(done: impl Fn(Loaded) + Send + 'static) -> Loader {
        let (requests, waiting) = mpsc::channel::<Request>();
        let spawned = std::thread::Builder::new()
            .name("activity".into())
            .spawn(move || {
                let conn = match db::open_writer() {
                    Ok(conn) => conn,
                    Err(error) => return eprintln!("The Activity tab can't read: {error}"),
                };
                let mut cache = Cache::default();
                while let Ok(mut request) = waiting.recv() {
                    while let Ok(newer) = waiting.try_recv() {
                        request = newer;
                    }
                    if cache.boot != request.boot {
                        cache = Cache {
                            boot: request.boot,
                            ..Cache::default()
                        };
                    }
                    match cache.refresh(&conn) {
                        Ok(()) => done(build(&cache, &request)),
                        Err(error) => eprintln!("The Activity tab couldn't read a run: {error}"),
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("The Activity tab couldn't start its loader: {error}");
        }
        Loader { requests }
    }

    pub fn load(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

// What the database holds for a run.

struct Exec {
    run: i64,
    execution: Option<i64>,
    parent: i64,
    pid: i64,
    started_at: i64,
    time: String,
    uid: Option<i64>,
    euid: Option<i64>,
    path: String,
    binary: Option<String>,
    argv: Vec<String>,
    argv_truncated: bool,
    cwd: Option<String>,
    error: Option<i64>,
    exited_at: Option<i64>,
    exit_time: Option<String>,
    exit_code: Option<i64>,
    exit_signal: Option<i64>,
}

struct FileAccess {
    run: i64,
    execution: i64,
    at: i64,
    time: String,
    path: String,
    host_path: Option<String>,
    access: String,
    new_path: Option<String>,
}

struct GuestConn {
    run: i64,
    execution: i64,
    at: i64,
    time: String,
    protocol: String,
    address: String,
    port: i64,
    local_port: i64,
}

struct Cred {
    run: i64,
    execution: i64,
    at: i64,
    time: String,
    uid: (i64, i64),
    euid: (i64, i64),
    gid: (i64, i64),
    egid: (i64, i64),
    caps: bool,
}

struct Lookup {
    at: i64,
    time: String,
    name: String,
    kind: String,
    refused: bool,
    rcode: String,
    cnames: Vec<String>,
    answers: Vec<(String, i64)>,
}

struct Flow {
    at: i64,
    time: String,
    protocol: String,
    local_port: i64,
    address: String,
    port: i64,
    blocked: bool,
    rule: Option<String>,
    outcome: Option<String>,
    error: Option<String>,
}

#[derive(Default)]
struct Cache {
    boot: i64,
    execs: Vec<Exec>,
    files: Vec<FileAccess>,
    conns: Vec<GuestConn>,
    creds: Vec<Cred>,
    lookups: Vec<Lookup>,
    lookup_ids: HashMap<i64, usize>,
    flows: Vec<Flow>,
    /// The newest id read of files, connections, credential changes, lookups and flows.
    last: [i64; 5],
    /// Events lost, summed over observer runs; how many observer runs reported; whether
    /// any reported at all.
    lost: i64,
    observer_runs: i64,
    reported: bool,
}

impl Cache {
    fn refresh(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        let boot = self.boot;
        self.execs = conn
            .prepare(
                "SELECT observer_run, execution, parent, pid, started_at,
                 strftime('%H:%M:%f', started_at / 1000.0, 'unixepoch', 'localtime'), uid, euid,
                 path, binary, argv, argv_truncated, cwd, error,
                 strftime('%H:%M:%f', exited_at / 1000.0, 'unixepoch', 'localtime'),
                 exit_code, exit_signal, exited_at
                 FROM executions WHERE boot_id = ?1 ORDER BY observer_run, started_at, id",
            )?
            .query_map([boot], |row| {
                let argv: Option<String> = row.get(10)?;
                Ok(Exec {
                    run: row.get(0)?,
                    execution: row.get(1)?,
                    parent: row.get(2)?,
                    pid: row.get(3)?,
                    started_at: row.get(4)?,
                    time: row.get(5)?,
                    uid: row.get(6)?,
                    euid: row.get(7)?,
                    path: row.get(8)?,
                    binary: row.get(9)?,
                    argv: argv
                        .and_then(|argv| serde_json::from_str(&argv).ok())
                        .unwrap_or_default(),
                    argv_truncated: row.get(11)?,
                    cwd: row.get(12)?,
                    error: row.get(13)?,
                    exit_time: row.get(14)?,
                    exit_code: row.get(15)?,
                    exit_signal: row.get(16)?,
                    exited_at: row.get(17)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;

        let mut rows = conn.prepare(&format!(
            "SELECT id, observer_run, execution, at, {TIME}, path, host_path, access, new_path
             FROM file_accesses WHERE boot_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let mut query = rows.query(params![boot, self.last[0]])?;
        while let Some(row) = query.next()? {
            self.last[0] = row.get(0)?;
            self.files.push(FileAccess {
                run: row.get(1)?,
                execution: row.get(2)?,
                at: row.get(3)?,
                time: row.get(4)?,
                path: row.get(5)?,
                host_path: row.get(6)?,
                access: row.get(7)?,
                new_path: row.get(8)?,
            });
        }

        let mut rows = conn.prepare(&format!(
            "SELECT id, observer_run, execution, at, {TIME}, protocol, address, port, local_port
             FROM connections WHERE boot_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let mut query = rows.query(params![boot, self.last[1]])?;
        while let Some(row) = query.next()? {
            self.last[1] = row.get(0)?;
            self.conns.push(GuestConn {
                run: row.get(1)?,
                execution: row.get(2)?,
                at: row.get(3)?,
                time: row.get(4)?,
                protocol: row.get(5)?,
                address: row.get(6)?,
                port: row.get(7)?,
                local_port: row.get(8)?,
            });
        }

        let mut rows = conn.prepare(&format!(
            "SELECT id, observer_run, execution, at, {TIME}, uid_before, uid_after, euid_before,
             euid_after, gid_before, gid_after, egid_before, egid_after, capabilities_raised
             FROM credential_changes WHERE boot_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let mut query = rows.query(params![boot, self.last[2]])?;
        while let Some(row) = query.next()? {
            self.last[2] = row.get(0)?;
            self.creds.push(Cred {
                run: row.get(1)?,
                execution: row.get(2)?,
                at: row.get(3)?,
                time: row.get(4)?,
                uid: (row.get(5)?, row.get(6)?),
                euid: (row.get(7)?, row.get(8)?),
                gid: (row.get(9)?, row.get(10)?),
                egid: (row.get(11)?, row.get(12)?),
                caps: row.get(13)?,
            });
        }

        let first_new_lookup = self.last[3];
        let mut rows = conn.prepare(&format!(
            "SELECT id, at, {TIME}, name, type, result, rcode, cnames
             FROM dns_lookups WHERE boot_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let mut query = rows.query(params![boot, first_new_lookup])?;
        while let Some(row) = query.next()? {
            let id: i64 = row.get(0)?;
            self.last[3] = id;
            let result: String = row.get(5)?;
            let cnames: String = row.get(7)?;
            self.lookup_ids.insert(id, self.lookups.len());
            self.lookups.push(Lookup {
                at: row.get(1)?,
                time: row.get(2)?,
                name: row.get(3)?,
                kind: row.get(4)?,
                refused: result == "refused",
                rcode: row.get::<_, Option<String>>(6)?.unwrap_or_default(),
                cnames: serde_json::from_str(&cnames).unwrap_or_default(),
                answers: Vec::new(),
            });
        }
        let mut rows = conn.prepare(
            "SELECT lookup_id, address, ttl FROM dns_answers
             WHERE boot_id = ?1 AND lookup_id > ?2 AND lookup_id <= ?3",
        )?;
        let mut query = rows.query(params![boot, first_new_lookup, self.last[3]])?;
        while let Some(row) = query.next()? {
            if let Some(&index) = self.lookup_ids.get(&row.get(0)?) {
                self.lookups[index].answers.push((row.get(1)?, row.get(2)?));
            }
        }

        let mut rows = conn.prepare(&format!(
            "SELECT id, at, {TIME}, protocol, local_port, address, port, decision, rule, outcome,
             error FROM network_flows WHERE boot_id = ?1 AND id > ?2 ORDER BY id"
        ))?;
        let mut query = rows.query(params![boot, self.last[4]])?;
        while let Some(row) = query.next()? {
            self.last[4] = row.get(0)?;
            let decision: String = row.get(7)?;
            self.flows.push(Flow {
                at: row.get(1)?,
                time: row.get(2)?,
                protocol: row.get(3)?,
                local_port: row.get(4)?,
                address: row.get(5)?,
                port: row.get(6)?,
                blocked: decision == "blocked",
                rule: row.get(8)?,
                outcome: row.get(9)?,
                error: row.get(10)?,
            });
        }

        // Counters restart with each observer run, so each run's highest counts.
        (self.lost, self.observer_runs, self.reported) = conn.query_row(
            "SELECT coalesce(sum(lost), 0), count(*), count(*) > 0 FROM (
                 SELECT max(lost_kernel + lost_queue) AS lost FROM collector_health
                 WHERE boot_id = ?1 GROUP BY observer_run)",
            [boot],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        Ok(())
    }
}

// The run as processes, with what each did.

/// A process: one or more executions with the same pid, each replacing the one before.
struct Group {
    execs: Vec<usize>,
    parent: Option<usize>,
    children: Vec<usize>,
    agent: bool,
    /// Programs it tried to run that the kernel refused.
    failed: Vec<usize>,
    files: Vec<usize>,
    conns: Vec<usize>,
    creds: Vec<usize>,
}

/// What a set of processes did, in short.
#[derive(Default)]
struct Tally<'a> {
    changed: BTreeSet<&'a str>,
    read: BTreeSet<&'a str>,
    conns: usize,
    blocked: usize,
    failed_conns: usize,
    root: bool,
    user_change: bool,
    caps: bool,
    failed_starts: Vec<&'a str>,
    programs: Vec<&'a str>,
    failed_exit: bool,
}

struct Model<'a> {
    c: &'a Cache,
    request: &'a Request,
    groups: Vec<Group>,
    by_exec: HashMap<(i64, i64), usize>,
    commands: Vec<usize>,
    agents: Vec<usize>,
    /// Activity whose process isn't known: files, connections and credential changes.
    loose: (Vec<usize>, Vec<usize>, Vec<usize>),
    /// The Mac-side flow of each guest connection, if one matched.
    conn_flow: Vec<Option<usize>>,
    flow_matched: Vec<bool>,
    /// Lookups (index, TTL) that returned an address.
    answered: HashMap<&'a str, Vec<(usize, i64)>>,
}

impl<'a> Model<'a> {
    fn new(c: &'a Cache, request: &'a Request) -> Model<'a> {
        let mut groups: Vec<Group> = Vec::new();
        let mut by_exec: HashMap<(i64, i64), usize> = HashMap::new();
        let mut order: Vec<usize> = (0..c.execs.len())
            .filter(|&i| c.execs[i].execution.is_some())
            .collect();
        order.sort_by_key(|&i| (c.execs[i].run, c.execs[i].execution));
        for i in order {
            let exec = &c.execs[i];
            let parent = by_exec.get(&(exec.run, exec.parent)).copied();
            let group = match parent {
                // The parent's process ran another program.
                Some(p) if c.execs[*groups[p].execs.last().unwrap_or(&i)].pid == exec.pid => {
                    groups[p].execs.push(i);
                    p
                }
                _ => {
                    groups.push(Group {
                        execs: vec![i],
                        parent,
                        children: Vec::new(),
                        agent: exec.parent == 0,
                        failed: Vec::new(),
                        files: Vec::new(),
                        conns: Vec::new(),
                        creds: Vec::new(),
                    });
                    let g = groups.len() - 1;
                    if let Some(p) = parent {
                        groups[p].children.push(g);
                    }
                    g
                }
            };
            by_exec.insert((exec.run, exec.execution.unwrap_or(0)), group);
        }
        for (i, exec) in c.execs.iter().enumerate() {
            if exec.execution.is_none()
                && let Some(&g) = by_exec.get(&(exec.run, exec.parent))
            {
                groups[g].failed.push(i);
            }
        }

        let mut loose = (Vec::new(), Vec::new(), Vec::new());
        for (i, file) in c.files.iter().enumerate() {
            match by_exec.get(&(file.run, file.execution)) {
                Some(&g) => groups[g].files.push(i),
                None => loose.0.push(i),
            }
        }
        for (i, conn) in c.conns.iter().enumerate() {
            match by_exec.get(&(conn.run, conn.execution)) {
                Some(&g) => groups[g].conns.push(i),
                None => loose.1.push(i),
            }
        }
        for (i, cred) in c.creds.iter().enumerate() {
            match by_exec.get(&(cred.run, cred.execution)) {
                Some(&g) => groups[g].creds.push(i),
                None => loose.2.push(i),
            }
        }

        let agents: Vec<usize> = (0..groups.len()).filter(|&g| groups[g].agent).collect();
        // Started by the agent process, or by a process whose start wasn't recorded.
        let commands = (0..groups.len())
            .filter(|&g| match groups[g].parent {
                Some(p) => groups[p].agent,
                None => !groups[g].agent,
            })
            .collect();

        let mut flows_at: HashMap<(&str, i64), Vec<usize>> = HashMap::new();
        for (i, flow) in c.flows.iter().enumerate() {
            flows_at
                .entry((flow.address.as_str(), flow.port))
                .or_default()
                .push(i);
        }
        let mut flow_matched = vec![false; c.flows.len()];
        let conn_flow = c
            .conns
            .iter()
            .map(|conn| {
                let candidates: Vec<usize> = flows_at
                    .get(&(conn.address.as_str(), conn.port))?
                    .iter()
                    .copied()
                    .filter(|&f| {
                        let flow = &c.flows[f];
                        flow.protocol == conn.protocol
                            && (flow.at - conn.at).abs() <= MATCH_WINDOW_MS
                            && (conn.protocol != "tcp" || flow.local_port == conn.local_port)
                    })
                    .collect();
                let found = if conn.protocol == "tcp" {
                    candidates
                        .into_iter()
                        .min_by_key(|&f| (c.flows[f].at - conn.at).abs())
                } else if candidates.len() == 1 {
                    Some(candidates[0])
                } else {
                    None
                };
                if let Some(f) = found {
                    flow_matched[f] = true;
                }
                found
            })
            .collect();

        let mut answered: HashMap<&str, Vec<(usize, i64)>> = HashMap::new();
        for (i, lookup) in c.lookups.iter().enumerate() {
            for (address, ttl) in &lookup.answers {
                answered
                    .entry(address.as_str())
                    .or_default()
                    .push((i, *ttl));
            }
        }

        Model {
            c,
            request,
            groups,
            by_exec,
            commands,
            agents,
            loose,
            conn_flow,
            flow_matched,
            answered,
        }
    }

    fn first(&self, g: usize) -> &'a Exec {
        &self.c.execs[self.groups[g].execs[0]]
    }

    fn last(&self, g: usize) -> &'a Exec {
        &self.c.execs[*self.groups[g].execs.last().unwrap_or(&0)]
    }

    fn key(&self, g: usize) -> String {
        let exec = self.first(g);
        format!("proc:{}:{}", exec.run, exec.execution.unwrap_or(0))
    }

    /// "curl 392"
    fn process(&self, g: usize) -> String {
        let exec = self.last(g);
        format!("{} {}", program(exec), exec.pid)
    }

    /// The process and everything it started.
    fn subtree(&self, g: usize) -> Vec<usize> {
        let mut all = vec![g];
        let mut i = 0;
        while i < all.len() {
            all.extend(self.groups[all[i]].children.iter().copied());
            i += 1;
        }
        all
    }

    fn tally(&self, groups: &[usize]) -> Tally<'a> {
        let c = self.c;
        let mut tally = Tally::default();
        for &g in groups {
            let group = &self.groups[g];
            for &f in &group.files {
                let file = &c.files[f];
                if changes(&file.access) {
                    tally.changed.insert(&file.path);
                } else {
                    tally.read.insert(&file.path);
                }
            }
            for &i in &group.conns {
                tally.conns += 1;
                if let Some(f) = self.conn_flow[i] {
                    let flow = &c.flows[f];
                    if flow.blocked {
                        tally.blocked += 1;
                    } else if matches!(flow.outcome.as_deref(), Some("failed" | "timed out")) {
                        tally.failed_conns += 1;
                    }
                }
            }
            for &i in &group.creds {
                let cred = &c.creds[i];
                if (cred.uid.1 == 0 && cred.uid.0 != 0) || (cred.euid.1 == 0 && cred.euid.0 != 0) {
                    tally.root = true;
                } else if cred.uid.0 != cred.uid.1
                    || cred.euid.0 != cred.euid.1
                    || cred.gid.0 != cred.gid.1
                    || cred.egid.0 != cred.egid.1
                {
                    tally.user_change = true;
                }
                tally.caps |= cred.caps;
            }
            for &i in &group.failed {
                tally.failed_starts.push(&c.execs[i].path);
            }
            for &e in &group.execs {
                let name = program(&c.execs[e]);
                if !tally.programs.contains(&name) {
                    tally.programs.push(name);
                }
            }
            let last = self.last(g);
            if last.exit_signal.is_some_and(|signal| signal != 0)
                || last.exit_code.is_some_and(|code| code != 0)
            {
                tally.failed_exit = true;
            }
        }
        tally
    }

    /// The short list of what processes did, and its tone.
    fn effects(&self, tally: &Tally, ran: bool) -> (Vec<String>, Tone) {
        let mut parts = Vec::new();
        let mut tone = Tone::Muted;
        if ran && tally.programs.len() > 1 {
            let shown: Vec<&str> = tally.programs.iter().skip(1).take(5).copied().collect();
            let more = tally.programs.len().saturating_sub(6);
            parts.push(if more > 0 {
                format!("ran {} (+{more})", shown.join(", "))
            } else {
                format!("ran {}", shown.join(", "))
            });
        }
        if tally.root {
            parts.push("became root".into());
            tone = Tone::Warn;
        } else if tally.user_change {
            parts.push("changed user".into());
            tone = Tone::Warn;
        } else if tally.caps {
            parts.push("gained capabilities".into());
            tone = Tone::Warn;
        }
        if let Some(first) = tally.failed_starts.first() {
            parts.push(match tally.failed_starts.len() {
                1 => format!("couldn't run {first}"),
                n => format!("couldn't run {first} and {} more", n - 1),
            });
            tone = Tone::Bad;
        }
        if !tally.changed.is_empty() {
            parts.push(plural(tally.changed.len(), "file changed", "files changed"));
        }
        if self.request.show_reads && !tally.read.is_empty() {
            parts.push(format!("{} read", tally.read.len()));
        }
        if tally.conns > 0 {
            parts.push(plural(tally.conns, "connection", "connections"));
        }
        if tally.blocked > 0 {
            parts.push(format!("{} blocked", tally.blocked));
            tone = Tone::Bad;
        }
        if tally.failed_conns > 0 {
            parts.push(format!("{} failed to connect", tally.failed_conns));
            tone = Tone::Bad;
        }
        (parts, tone)
    }

    /// The exit of a process: its last program's.
    fn result(&self, g: usize) -> (String, Tone) {
        let exec = self.last(g);
        match (exec.exit_code, exec.exit_signal) {
            (Some(_), Some(signal)) if signal != 0 => (format!("signal {signal}"), Tone::Bad),
            (Some(0), _) => ("exit 0".into(), Tone::Neutral),
            (Some(code), _) => (format!("exit {code}"), Tone::Bad),
            _ if self.request.live => ("running".into(), Tone::Muted),
            _ => ("no exit recorded".into(), Tone::Muted),
        }
    }

    /// Everything in the processes the search can find, lowercase.
    fn haystack(&self, groups: &[usize]) -> String {
        let c = self.c;
        let mut text = String::new();
        for &g in groups {
            let group = &self.groups[g];
            for &e in group.execs.iter().chain(&group.failed) {
                text.push_str(&c.execs[e].argv.join(" "));
                text.push(' ');
                text.push_str(&c.execs[e].path);
                text.push('\n');
            }
            for &f in &group.files {
                text.push_str(&c.files[f].path);
                text.push('\n');
            }
            for &i in &group.conns {
                let conn = &c.conns[i];
                text.push_str(&format!("{}:{}\n", conn.address, conn.port));
                text.push_str(&self.domains(&conn.address, conn.at).join(" "));
                text.push('\n');
            }
        }
        text.to_lowercase()
    }

    fn matches(&self, groups: &[usize]) -> bool {
        let request = self.request;
        if request.failed_only {
            let tally = self.tally(groups);
            if !(tally.failed_exit
                || !tally.failed_starts.is_empty()
                || tally.blocked > 0
                || tally.failed_conns > 0)
            {
                return false;
            }
        }
        request.search.is_empty() || self.haystack(groups).contains(&request.search)
    }

    /// The names whose DNS answers pointed to `address` at `at`, newest lookup first.
    fn domains(&self, address: &str, at: i64) -> Vec<String> {
        let mut names: Vec<(i64, &str)> = Vec::new();
        for &(l, ttl) in self.answered.get(address).into_iter().flatten() {
            let lookup = &self.c.lookups[l];
            let until = lookup.at
                + i64::from(ttl.clamp(0, i64::from(MAX_TTL)) as u32) * 1000
                + DNS_GRACE.as_millis() as i64;
            // The guest's clock may run a little apart from the Mac's.
            if lookup.at <= at + 2_000
                && at <= until
                && !names.iter().any(|(_, name)| *name == lookup.name)
            {
                names.push((lookup.at, &lookup.name));
            }
        }
        names.sort_by_key(|(at, _)| -at);
        names.into_iter().map(|(_, name)| name.to_owned()).collect()
    }

    /// The domain line of a destination, and its tone.
    fn domain_text(&self, address: &str, port: i64, at: i64) -> (String, Tone) {
        if port == 53 {
            return ("DNS server".into(), Tone::Muted);
        }
        match self.domains(address, at).as_slice() {
            [] => ("No domain recorded".into(), Tone::Muted),
            [name] => (name.clone(), Tone::Neutral),
            names => (
                format!("Possible domains: {}", names.join(", ")),
                Tone::Warn,
            ),
        }
    }

    /// The outcome of a guest connection: from the Mac's side when its flow is known.
    fn conn_outcome(&self, i: usize) -> (String, Tone) {
        match self.conn_flow[i] {
            Some(f) => flow_outcome(&self.c.flows[f]),
            None if self.c.conns[i].protocol == "udp" => ("Sent".into(), Tone::Neutral),
            None => ("Recorded in the guest".into(), Tone::Muted),
        }
    }
}

fn build(c: &Cache, request: &Request) -> Loaded {
    let model = Model::new(c, request);
    let (rows, note) = match request.view {
        View::Commands => (commands_view(&model), String::new()),
        View::Files => (files_view(&model), String::new()),
        View::Network => (network_view(&model), String::new()),
        View::Events => events_view(&model),
    };

    let changed: HashSet<&str> = c
        .files
        .iter()
        .filter(|file| changes(&file.access))
        .map(|file| file.path.as_str())
        .collect();
    let read: HashSet<&str> = c
        .files
        .iter()
        .filter(|file| !changes(&file.access))
        .map(|file| file.path.as_str())
        .collect();
    let connections = c.conns.iter().filter(|conn| conn.port != 53).count()
        + model
            .flow_matched
            .iter()
            .filter(|matched| !**matched)
            .count();
    let blocked = c.flows.iter().filter(|flow| flow.blocked).count()
        + c.lookups.iter().filter(|lookup| lookup.refused).count();
    let counts = vec![
        plural(model.commands.len(), "command", "commands"),
        plural(changed.len(), "file change", "file changes"),
        plural(read.len(), "file read", "files read"),
        plural(connections, "connection", "connections"),
    ];
    let (monitoring, monitoring_tone) = if !c.reported && c.execs.is_empty() {
        (
            "The observer didn't report; activity in this run is unknown".into(),
            Tone::Warn,
        )
    } else if c.lost > 0 {
        (
            format!(
                "{} lost; activity is incomplete",
                plural(c.lost as usize, "event", "events")
            ),
            Tone::Warn,
        )
    } else if c.observer_runs > 1 {
        (
            format!(
                "Monitoring restarted {}; activity may be incomplete",
                plural(c.observer_runs as usize - 1, "time", "times")
            ),
            Tone::Warn,
        )
    } else {
        ("Monitoring complete for this run".into(), Tone::Muted)
    };
    let nothing = c.execs.is_empty() && c.files.is_empty() && c.conns.is_empty();
    let empty = if nothing && c.lookups.is_empty() && c.flows.is_empty() {
        "No activity recorded in this run.".into()
    } else {
        "No matching activity in this run.".into()
    };
    let detail = request
        .selected
        .as_deref()
        .and_then(|key| detail(&model, key.split('#').next().unwrap_or(key)));
    Loaded {
        generation: request.generation,
        counts,
        blocked: if blocked > 0 {
            format!("{blocked} blocked")
        } else {
            String::new()
        },
        monitoring,
        monitoring_tone,
        rows,
        note,
        empty,
        detail,
    }
}

// Views.

fn commands_view(m: &Model) -> Vec<Row> {
    let mut rows = Vec::new();
    let request = m.request;

    // The agent process, pinned: what it did itself.
    if !m.agents.is_empty() && m.matches(&m.agents) {
        let g = m.agents[0];
        let tally = m.tally(&m.agents);
        let (parts, tone) = m.effects(&tally, false);
        let expanded = !request.toggled.contains("agent");
        let (result, result_tone) = m.result(g);
        rows.push(Row {
            key: "agent".into(),
            label: "Agent".into(),
            title: title(m.first(g)).0,
            subtitle: parts.join(" · "),
            subtitle_tone: tone,
            time: m.first(g).time.clone(),
            result,
            tone: result_tone,
            expandable: true,
            expanded,
            ..Row::default()
        });
        if expanded {
            for &g in &m.agents {
                effect_rows(m, g, 1, &mut rows);
            }
        }
    }

    let mut commands = m.commands.clone();
    commands.sort_by_key(|&g| std::cmp::Reverse(m.first(g).started_at));
    for g in commands {
        let subtree = m.subtree(g);
        if !m.matches(&subtree) {
            continue;
        }
        process_row(m, g, &subtree, 0, &mut rows);
    }

    let (files, conns, creds) = &m.loose;
    if !files.is_empty() || !conns.is_empty() || !creds.is_empty() {
        rows.push(Row {
            key: "loose".into(),
            title: "Activity without a known process".into(),
            subtitle: "The observer couldn't link it to a program run, e.g. after it restarted."
                .into(),
            subtitle_tone: Tone::Muted,
            ..Row::default()
        });
    }
    rows
}

/// A process row and, when expanded, what it did and the processes it started.
fn process_row(m: &Model, g: usize, subtree: &[usize], depth: i32, rows: &mut Vec<Row>) {
    let key = m.key(g);
    let (text, shell) = title(m.first(g));
    let chain: Vec<&str> = m.groups[g].execs[1..]
        .iter()
        .map(|&e| program(&m.c.execs[e]))
        .collect();
    let mut title = text;
    for name in chain {
        title.push_str(" → ");
        title.push_str(name);
    }
    let tally = m.tally(if depth == 0 { subtree } else { &subtree[..1] });
    let (mut parts, tone) = m.effects(&tally, depth == 0);
    if let Some(shell) = shell {
        parts.insert(0, format!("{shell} -c"));
    }
    let (result, result_tone) = m.result(g);
    let has_more = m.groups[g].children.len()
        + m.groups[g].files.len()
        + m.groups[g].conns.len()
        + m.groups[g].creds.len()
        + m.groups[g].failed.len()
        > 0;
    // Commands start collapsed; the processes inside them expanded.
    let expanded = has_more && ((depth == 0) == m.request.toggled.contains(&key));
    rows.push(Row {
        key,
        depth,
        time: m.first(g).time.clone(),
        title,
        subtitle: parts.join(" · "),
        subtitle_tone: tone,
        result,
        tone: result_tone,
        expandable: has_more,
        expanded,
        ..Row::default()
    });
    if expanded {
        effect_rows(m, g, depth + 1, rows);
        let mut children = m.groups[g].children.clone();
        children.sort_by_key(|&c| m.first(c).started_at);
        for child in children {
            process_row(m, child, &m.subtree(child), depth + 1, rows);
        }
    }
}

/// What one process did itself: refused starts, privilege changes, file changes, reads by
/// folder, and connections by destination.
fn effect_rows(m: &Model, g: usize, depth: i32, rows: &mut Vec<Row>) {
    let c = m.c;
    let group = &m.groups[g];
    let key = m.key(g);
    for &e in &group.failed {
        let exec = &c.execs[e];
        rows.push(Row {
            key: format!("{key}#failed{e}"),
            depth,
            time: exec.time.clone(),
            title: format!("couldn't run {}", exec.path),
            subtitle: errno_text(exec.error.unwrap_or(0)),
            subtitle_tone: Tone::Bad,
            tone: Tone::Bad,
            ..Row::default()
        });
    }
    if !group.creds.is_empty() {
        let tally = m.tally(&[g]);
        let first = &c.creds[group.creds[0]];
        let text = if tally.root {
            "became root"
        } else if tally.user_change {
            "changed user or group"
        } else {
            "gained capabilities"
        };
        rows.push(Row {
            key: format!("{key}#creds"),
            depth,
            time: first.time.clone(),
            title: text.into(),
            subtitle: plural(group.creds.len(), "privilege change", "privilege changes"),
            subtitle_tone: Tone::Muted,
            tone: Tone::Warn,
            result: "Privilege".into(),
            ..Row::default()
        });
    }

    // Changes, one row per file, newest first.
    let mut changed: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut read: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &f in &group.files {
        let file = &c.files[f];
        if changes(&file.access) {
            changed.entry(&file.path).or_default().push(f);
        } else {
            read.entry(folder(&file.path)).or_default().push(f);
        }
    }
    let mut changed: Vec<(&str, Vec<usize>)> = changed.into_iter().collect();
    changed.sort_by_key(|(_, files)| std::cmp::Reverse(files.iter().map(|&f| c.files[f].at).max()));
    let total = changed.len();
    for (path, files) in changed.into_iter().take(LIST_LIMIT) {
        let last = &c.files[*files.last().unwrap_or(&0)];
        rows.push(Row {
            key: format!("file:{path}#{g}"),
            depth,
            time: last.time.clone(),
            title: path.into(),
            subtitle: operations(c, &files),
            subtitle_tone: Tone::Muted,
            ..Row::default()
        });
    }
    if total > LIST_LIMIT {
        rows.push(more_row(
            &key,
            depth,
            total - LIST_LIMIT,
            "file change",
            "file changes",
        ));
    }

    if m.request.show_reads {
        let mut read: Vec<(String, Vec<usize>)> = read.into_iter().collect();
        read.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
        let total = read.len();
        for (dir, files) in read.into_iter().take(LIST_LIMIT) {
            let paths: HashSet<&str> = files.iter().map(|&f| c.files[f].path.as_str()).collect();
            rows.push(Row {
                key: format!("reads:{key}:{dir}"),
                depth,
                time: c.files[files[0]].time.clone(),
                title: format!("{dir}/"),
                subtitle: format!("{} read", plural(paths.len(), "file", "files")),
                subtitle_tone: Tone::Muted,
                dim: true,
                ..Row::default()
            });
        }
        if total > LIST_LIMIT {
            rows.push(more_row(
                &key,
                depth,
                total - LIST_LIMIT,
                "folder read",
                "folders read",
            ));
        }
    }

    // Connections, one row per destination.
    let mut destinations: BTreeMap<(&str, &str, i64), Vec<usize>> = BTreeMap::new();
    for &i in &group.conns {
        let conn = &c.conns[i];
        destinations
            .entry((&conn.protocol, &conn.address, conn.port))
            .or_default()
            .push(i);
    }
    for ((protocol, address, port), conns) in destinations {
        let first = &c.conns[conns[0]];
        let (domain, domain_tone) = m.domain_text(address, port, first.at);
        let (outcome, tone) = summarize(conns.iter().map(|&i| m.conn_outcome(i)));
        rows.push(Row {
            key: format!("dest:{protocol}:{address}:{port}#{g}"),
            depth,
            time: first.time.clone(),
            title: format!("{} {protocol}", endpoint(address, port)),
            subtitle: if conns.len() > 1 {
                format!("{domain} · {} attempts", conns.len())
            } else {
                domain
            },
            subtitle_tone: domain_tone,
            result: outcome,
            tone,
            ..Row::default()
        });
    }
}

fn files_view(m: &Model) -> Vec<Row> {
    let c = m.c;
    let request = m.request;
    let failed_only = request.failed_only;
    // Per area: the changed files by path, and the reads by folder.
    type Area<'a> = (BTreeMap<&'a str, Vec<usize>>, BTreeMap<String, Vec<usize>>);
    let mut areas: BTreeMap<usize, Area> = BTreeMap::new();
    for (f, file) in c.files.iter().enumerate() {
        let haystack = format!("{} {}", file.path, file.new_path.as_deref().unwrap_or(""));
        if failed_only || !haystack.to_lowercase().contains(&request.search) {
            continue;
        }
        let (changed, read) = areas.entry(area(file)).or_default();
        if changes(&file.access) {
            changed.entry(&file.path).or_default().push(f);
        } else if request.show_reads {
            read.entry(folder(&file.path)).or_default().push(f);
        }
    }

    let mut rows = Vec::new();
    for (area_index, (changed, read)) in areas {
        if changed.is_empty() && read.is_empty() {
            continue;
        }
        let key = format!("area:{area_index}");
        let expanded = !request.toggled.contains(&key);
        let read_count: HashSet<&str> = read
            .values()
            .flatten()
            .map(|&f| c.files[f].path.as_str())
            .collect();
        let mut summary = vec![plural(changed.len(), "file changed", "files changed")];
        if request.show_reads {
            summary.push(format!("{} read", read_count.len()));
        }
        rows.push(Row {
            key,
            heading: true,
            expandable: true,
            expanded,
            title: AREAS[area_index].into(),
            subtitle: summary.join(" · "),
            subtitle_tone: Tone::Muted,
            ..Row::default()
        });
        if !expanded {
            continue;
        }
        let mut changed: Vec<(&str, Vec<usize>)> = changed.into_iter().collect();
        changed.sort_by_key(|(_, files)| {
            std::cmp::Reverse(files.iter().map(|&f| c.files[f].at).max())
        });
        for (path, files) in changed {
            let last = &c.files[*files.last().unwrap_or(&0)];
            let mut subtitle = operations(c, &files);
            if let Some(host) = &last.host_path {
                subtitle.push_str(&format!(" · on your Mac: {host}"));
            }
            rows.push(Row {
                key: format!("file:{path}"),
                depth: 1,
                time: last.time.clone(),
                title: path.into(),
                subtitle,
                subtitle_tone: Tone::Muted,
                meta: processes(
                    m,
                    files
                        .iter()
                        .map(|&f| (c.files[f].run, c.files[f].execution)),
                ),
                ..Row::default()
            });
        }
        let mut read: Vec<(String, Vec<usize>)> = read.into_iter().collect();
        read.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
        for (dir, files) in read {
            let paths: HashSet<&str> = files.iter().map(|&f| c.files[f].path.as_str()).collect();
            rows.push(Row {
                key: format!("readdir:{dir}"),
                depth: 1,
                time: c.files[files[0]].time.clone(),
                title: format!("{dir}/"),
                subtitle: format!("{} read", plural(paths.len(), "file", "files")),
                subtitle_tone: Tone::Muted,
                meta: processes(
                    m,
                    files
                        .iter()
                        .map(|&f| (c.files[f].run, c.files[f].execution)),
                ),
                dim: true,
                ..Row::default()
            });
        }
    }
    rows
}

fn network_view(m: &Model) -> Vec<Row> {
    let c = m.c;
    let request = m.request;
    let mut rows = Vec::new();

    // Destinations: guest connections and flows nothing in the guest matched.
    // Per protocol, address and port: the guest's connections and the unmatched flows.
    type Attempts = (Vec<usize>, Vec<usize>);
    let mut destinations: BTreeMap<(&str, &str, i64), Attempts> = BTreeMap::new();
    for (i, conn) in c.conns.iter().enumerate() {
        destinations
            .entry((&conn.protocol, &conn.address, conn.port))
            .or_default()
            .0
            .push(i);
    }
    for (f, flow) in c.flows.iter().enumerate() {
        if !m.flow_matched[f] {
            destinations
                .entry((&flow.protocol, &flow.address, flow.port))
                .or_default()
                .1
                .push(f);
        }
    }
    let mut destination_rows = Vec::new();
    for ((protocol, address, port), (conns, flows)) in destinations {
        let first_at = conns
            .iter()
            .map(|&i| c.conns[i].at)
            .chain(flows.iter().map(|&f| c.flows[f].at))
            .min()
            .unwrap_or(0);
        let last = conns
            .iter()
            .map(|&i| (c.conns[i].at, c.conns[i].time.as_str()))
            .chain(
                flows
                    .iter()
                    .map(|&f| (c.flows[f].at, c.flows[f].time.as_str())),
            )
            .max()
            .unwrap_or((0, ""));
        let (domain, domain_tone) = m.domain_text(address, port, first_at);
        let outcomes: Vec<(String, Tone)> = conns
            .iter()
            .map(|&i| m.conn_outcome(i))
            .chain(flows.iter().map(|&f| flow_outcome(&c.flows[f])))
            .collect();
        let failed = outcomes.iter().any(|(_, tone)| *tone == Tone::Bad);
        let who = if conns.is_empty() {
            "seen on the Mac only".to_owned()
        } else {
            processes(
                m,
                conns
                    .iter()
                    .map(|&i| (c.conns[i].run, c.conns[i].execution)),
            )
        };
        let haystack = format!("{address}:{port} {domain} {who}").to_lowercase();
        if (request.failed_only && !failed) || !haystack.contains(&request.search) {
            continue;
        }
        let (outcome, tone) = summarize(outcomes.into_iter());
        let attempts = conns.len() + flows.len();
        destination_rows.push((
            last.0,
            Row {
                key: format!("dest:{protocol}:{address}:{port}"),
                depth: 1,
                time: last.1.to_owned(),
                title: format!("{} {protocol}", endpoint(address, port)),
                subtitle: if attempts > 1 {
                    format!("{domain} · {attempts} attempts")
                } else {
                    domain
                },
                subtitle_tone: domain_tone,
                meta: who,
                result: outcome,
                tone,
                ..Row::default()
            },
        ));
    }
    destination_rows.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let key = "area:connections".to_owned();
    let expanded = !request.toggled.contains(&key);
    if !destination_rows.is_empty() {
        rows.push(Row {
            key,
            heading: true,
            expandable: true,
            expanded,
            title: "Destinations".into(),
            subtitle: plural(destination_rows.len(), "address", "addresses"),
            subtitle_tone: Tone::Muted,
            ..Row::default()
        });
        if expanded {
            rows.extend(destination_rows.into_iter().map(|(_, row)| row));
        }
    }

    // Lookups, one row per name.
    let mut names: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (l, lookup) in c.lookups.iter().enumerate() {
        names.entry(&lookup.name).or_default().push(l);
    }
    let connected: HashSet<&str> = c.conns.iter().map(|conn| conn.address.as_str()).collect();
    let mut lookup_rows = Vec::new();
    for (name, lookups) in names {
        let refused = lookups.iter().any(|&l| c.lookups[l].refused);
        let addresses: BTreeSet<&str> = lookups
            .iter()
            .flat_map(|&l| {
                c.lookups[l]
                    .answers
                    .iter()
                    .map(|(address, _)| address.as_str())
            })
            .collect();
        let kinds: BTreeSet<&str> = lookups
            .iter()
            .map(|&l| c.lookups[l].kind.as_str())
            .collect();
        let (result, tone) = if refused {
            ("Refused by policy".to_owned(), Tone::Bad)
        } else if addresses.iter().any(|address| connected.contains(address)) {
            ("Answered; connected".to_owned(), Tone::Neutral)
        } else if lookups.iter().any(|&l| c.lookups[l].rcode == "NameError") {
            ("No such domain".to_owned(), Tone::Muted)
        } else {
            (
                "DNS lookup recorded; no related connection recorded".to_owned(),
                Tone::Muted,
            )
        };
        let haystack = format!(
            "{name} {}",
            addresses.iter().copied().collect::<Vec<_>>().join(" ")
        );
        if (request.failed_only && !refused) || !haystack.to_lowercase().contains(&request.search) {
            continue;
        }
        let last = &c.lookups[*lookups.last().unwrap_or(&0)];
        let mut subtitle = vec![kinds.into_iter().collect::<Vec<_>>().join(", ")];
        if lookups.len() > 1 {
            subtitle.push(format!("{} lookups", lookups.len()));
        }
        if !addresses.is_empty() {
            subtitle.push(addresses.into_iter().take(4).collect::<Vec<_>>().join(", "));
        }
        lookup_rows.push((
            last.at,
            Row {
                key: format!("dns:{name}"),
                depth: 1,
                time: last.time.clone(),
                title: name.into(),
                subtitle: subtitle.join(" · "),
                subtitle_tone: Tone::Muted,
                result,
                tone,
                ..Row::default()
            },
        ));
    }
    lookup_rows.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let key = "area:lookups".to_owned();
    let expanded = !request.toggled.contains(&key);
    if !lookup_rows.is_empty() {
        rows.push(Row {
            key,
            heading: true,
            expandable: true,
            expanded,
            title: "DNS lookups".into(),
            subtitle: plural(lookup_rows.len(), "name", "names"),
            subtitle_tone: Tone::Muted,
            ..Row::default()
        });
        if expanded {
            rows.extend(lookup_rows.into_iter().map(|(_, row)| row));
        }
    }
    rows
}

fn events_view(m: &Model) -> (Vec<Row>, String) {
    let c = m.c;
    let request = m.request;
    let [processes, files, network] = request.kinds;
    let mut events: Vec<(i64, Row)> = Vec::new();
    let process_of = |run: i64, execution: i64| {
        m.by_exec
            .get(&(run, execution))
            .map_or_else(String::new, |&g| m.process(g))
    };

    if processes {
        for (e, exec) in c.execs.iter().enumerate() {
            let (result, tone, failed) = match exec.error {
                Some(errno) => (errno_text(errno), Tone::Bad, true),
                None => match m.by_exec.get(&(exec.run, exec.execution.unwrap_or(0))) {
                    Some(&g) if m.groups[g].execs.last() == Some(&e) => {
                        let (result, tone) = m.result(g);
                        let failed = tone == Tone::Bad;
                        (result, tone, failed)
                    }
                    _ => ("replaced by another program".into(), Tone::Muted, false),
                },
            };
            let key = match exec.execution {
                Some(execution) => match m.by_exec.get(&(exec.run, execution)) {
                    Some(&g) => m.key(g),
                    None => String::new(),
                },
                None => match m.by_exec.get(&(exec.run, exec.parent)) {
                    Some(&g) => m.key(g),
                    None => String::new(),
                },
            };
            events.push((
                exec.started_at,
                event_row(
                    format!("{key}#ev{e}"),
                    &exec.time,
                    "Process",
                    if exec.error.is_some() {
                        "Start failed"
                    } else {
                        "Start"
                    },
                    title(exec).0,
                    format!("{} {}", program(exec), exec.pid),
                    (result, tone),
                    failed,
                ),
            ));
        }
        for (i, cred) in c.creds.iter().enumerate() {
            let mut changes = Vec::new();
            for (name, (before, after)) in [
                ("uid", cred.uid),
                ("euid", cred.euid),
                ("gid", cred.gid),
                ("egid", cred.egid),
            ] {
                if before != after {
                    changes.push(format!("{name} {before} → {after}"));
                }
            }
            if cred.caps {
                changes.push("capabilities raised".into());
            }
            let key = m
                .by_exec
                .get(&(cred.run, cred.execution))
                .map_or_else(String::new, |&g| m.key(g));
            events.push((
                cred.at,
                event_row(
                    format!("{key}#cred{i}"),
                    &cred.time,
                    "Privilege",
                    "Change",
                    changes.join(", "),
                    process_of(cred.run, cred.execution),
                    ("".into(), Tone::Warn),
                    false,
                ),
            ));
        }
    }
    if files {
        for (f, file) in c.files.iter().enumerate() {
            if !request.show_reads && !changes(&file.access) {
                continue;
            }
            let target = match &file.new_path {
                Some(new) => format!("{} → {new}", file.path),
                None => file.path.clone(),
            };
            events.push((
                file.at,
                event_row(
                    format!("file:{}#ev{f}", file.path),
                    &file.time,
                    "File",
                    &file.access.replace(',', ", "),
                    target,
                    process_of(file.run, file.execution),
                    (String::new(), Tone::Neutral),
                    false,
                ),
            ));
        }
    }
    if network {
        for (i, conn) in c.conns.iter().enumerate() {
            let (domain, _) = m.domain_text(&conn.address, conn.port, conn.at);
            let (result, tone) = m.conn_outcome(i);
            events.push((
                conn.at,
                event_row(
                    format!(
                        "dest:{}:{}:{}#ev{i}",
                        conn.protocol, conn.address, conn.port
                    ),
                    &conn.time,
                    "Network",
                    if conn.protocol == "udp" {
                        "Send"
                    } else {
                        "Connect"
                    },
                    format!(
                        "{} {} ({domain})",
                        endpoint(&conn.address, conn.port),
                        conn.protocol
                    ),
                    process_of(conn.run, conn.execution),
                    (result, tone),
                    tone == Tone::Bad,
                ),
            ));
        }
        for (f, flow) in c.flows.iter().enumerate() {
            if m.flow_matched[f] {
                continue;
            }
            let (result, tone) = flow_outcome(flow);
            events.push((
                flow.at,
                event_row(
                    format!(
                        "dest:{}:{}:{}#flow{f}",
                        flow.protocol, flow.address, flow.port
                    ),
                    &flow.time,
                    "Network",
                    "Connect",
                    format!("{} {}", endpoint(&flow.address, flow.port), flow.protocol),
                    "seen on the Mac only".into(),
                    (result, tone),
                    tone == Tone::Bad,
                ),
            ));
        }
        for (l, lookup) in c.lookups.iter().enumerate() {
            let result = if lookup.refused {
                ("Refused by policy".to_owned(), Tone::Bad)
            } else if lookup.answers.is_empty() {
                (format!("No addresses ({})", lookup.rcode), Tone::Muted)
            } else {
                (
                    lookup
                        .answers
                        .iter()
                        .map(|(address, _)| address.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    Tone::Neutral,
                )
            };
            events.push((
                lookup.at,
                event_row(
                    format!("dns:{}#ev{l}", lookup.name),
                    &lookup.time,
                    "DNS",
                    &format!("Lookup {}", lookup.kind),
                    lookup.name.clone(),
                    String::new(),
                    result,
                    lookup.refused,
                ),
            ));
        }
    }

    events.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    let matching: Vec<Row> = events
        .into_iter()
        .map(|(_, row)| row)
        .filter(|row| {
            (!request.failed_only || row.tone == Tone::Bad)
                && (request.search.is_empty()
                    || format!("{} {} {}", row.title, row.meta, row.result)
                        .to_lowercase()
                        .contains(&request.search))
        })
        .collect();
    let total = matching.len();
    let note = if total > EVENT_LIMIT {
        format!(
            "Showing the newest {} of {} events.",
            thousands(EVENT_LIMIT),
            thousands(total)
        )
    } else {
        String::new()
    };
    (matching.into_iter().take(EVENT_LIMIT).collect(), note)
}

#[allow(clippy::too_many_arguments)]
fn event_row(
    key: String,
    time: &str,
    label: &str,
    action: &str,
    target: String,
    process: String,
    (result, tone): (String, Tone),
    failed: bool,
) -> Row {
    Row {
        key,
        time: time.to_owned(),
        label: label.to_owned(),
        title: target,
        subtitle: action.to_owned(),
        subtitle_tone: Tone::Muted,
        meta: process,
        result,
        tone: if failed { Tone::Bad } else { tone },
        ..Row::default()
    }
}

// The detail page.

fn detail(m: &Model, key: &str) -> Option<Detail> {
    let c = m.c;
    if key == "agent" {
        return m.agents.first().map(|&g| process_detail(m, g));
    }
    if key == "loose" {
        let (files, conns, creds) = &m.loose;
        let mut items: Vec<(i64, Item)> = Vec::new();
        for &f in files {
            let file = &c.files[f];
            let what = capitalize(&file.access.replace(',', ", "));
            items.push((
                file.at,
                item(
                    &file.path,
                    &format!("{what} · {}", file.time),
                    Tone::Muted,
                    String::new(),
                    true,
                ),
            ));
        }
        for &i in conns {
            let conn = &c.conns[i];
            let at = format!("{} · {}", conn.protocol.to_uppercase(), conn.time);
            items.push((
                conn.at,
                item(
                    &endpoint(&conn.address, conn.port),
                    &at,
                    Tone::Muted,
                    String::new(),
                    true,
                ),
            ));
        }
        for &i in creds {
            let cred = &c.creds[i];
            items.push((
                cred.at,
                item(
                    &cred_text(cred),
                    &cred.time,
                    Tone::Warn,
                    String::new(),
                    false,
                ),
            ));
        }
        items.sort_by_key(|(at, _)| *at);
        return Some(Detail {
            kind: "Activity".into(),
            headline: "Activity without a known process".into(),
            sections: vec![section(
                "Recorded",
                "",
                items.into_iter().map(|(_, item)| item).collect(),
            )],
            note: "The observer recorded this activity but couldn't link it to a program run, for \
                   example because it restarted while the process ran."
                .into(),
            ..Detail::default()
        });
    }
    if let Some(rest) = key.strip_prefix("proc:") {
        let (run, execution) = rest.split_once(':')?;
        let g = *m
            .by_exec
            .get(&(run.parse().ok()?, execution.parse().ok()?))?;
        return Some(process_detail(m, g));
    }
    if let Some(path) = key.strip_prefix("file:") {
        return Some(file_detail(m, path));
    }
    if let Some(dir) = key.strip_prefix("readdir:") {
        return Some(reads_detail(m, dir, 0..c.files.len()));
    }
    if let Some(rest) = key.strip_prefix("reads:") {
        // reads:proc:<run>:<execution>:<folder>
        let rest = rest.strip_prefix("proc:")?;
        let mut parts = rest.splitn(3, ':');
        let run = parts.next()?.parse().ok()?;
        let execution = parts.next()?.parse().ok()?;
        let dir = parts.next()?;
        let g = *m.by_exec.get(&(run, execution))?;
        return Some(reads_detail(m, dir, m.groups[g].files.iter().copied()));
    }
    if let Some(rest) = key.strip_prefix("dest:") {
        let (protocol, rest) = rest.split_once(':')?;
        let (address, port) = rest.rsplit_once(':')?;
        return Some(destination_detail(m, protocol, address, port.parse().ok()?));
    }
    if let Some(name) = key.strip_prefix("dns:") {
        return Some(dns_detail(m, name));
    }
    None
}

/// A process: what it ran, and what it and everything it started did.
fn process_detail(m: &Model, g: usize) -> Detail {
    let c = m.c;
    let group = &m.groups[g];
    let first = m.first(g);
    let last = m.last(g);
    // The agent's commands have pages of their own.
    let scope: Vec<usize> = if group.agent { vec![g] } else { m.subtree(g) };

    let (result, _) = m.result(g);
    let mut facts = vec![field(
        "Started",
        &format!("{} guest clock", first.time),
        false,
    )];
    if let (Some(end), Some(time)) = (last.exited_at, &last.exit_time) {
        facts.push(field("Ended", &format!("{time} guest clock"), false));
        facts.push(field("Duration", &duration(end - first.started_at), false));
    }
    facts.push(field("Result", &result, false));
    facts.push(field(
        "Process",
        &format!("{}, pid {}", program(last), last.pid),
        false,
    ));
    let mut ancestors = Vec::new();
    let mut parent = group.parent;
    while let Some(p) = parent {
        ancestors.push(m.process(p));
        parent = m.groups[p].parent;
    }
    if !ancestors.is_empty() {
        facts.push(field("Started by", &ancestors.join(", from "), false));
    }
    if group.execs.len() > 1 {
        let chain: Vec<&str> = group.execs[1..]
            .iter()
            .map(|&e| program(&c.execs[e]))
            .collect();
        facts.push(field("Then ran", &chain.join(" → "), false));
    }
    if let Some(cwd) = &first.cwd {
        facts.push(field("Working directory", cwd, true));
    }
    if let (Some(uid), Some(euid)) = (first.uid, first.euid) {
        let user = if uid == euid {
            format!("uid {uid}")
        } else {
            format!("uid {uid}, euid {euid}")
        };
        facts.push(field("User", &user, false));
    }
    if let Some(binary) = &first.binary {
        facts.push(field("Program", binary, true));
    }

    let mut sections = Vec::new();

    let mut refused = Vec::new();
    for &s in &scope {
        for &e in &m.groups[s].failed {
            let exec = &c.execs[e];
            let reason = format!("{} · {}", errno_text(exec.error.unwrap_or(0)), exec.time);
            refused.push(item(&exec.path, &reason, Tone::Bad, m.key(s), true));
        }
    }
    sections.push(section("Couldn't run", "", refused));

    let mut privilege = Vec::new();
    for &s in &scope {
        let creds = &m.groups[s].creds;
        if creds.is_empty() {
            continue;
        }
        let tally = m.tally(&[s]);
        let what = if tally.root {
            "became root"
        } else if tally.user_change {
            "changed user or group"
        } else {
            "gained capabilities"
        };
        // The change that made it so, e.g. sudo's switch to euid 0.
        let telling = creds
            .iter()
            .map(|&i| &c.creds[i])
            .find(|cred| cred.euid.1 == 0 && cred.euid.0 != 0 || cred.uid.1 == 0 && cred.uid.0 != 0)
            .unwrap_or(&c.creds[creds[0]]);
        privilege.push(item(
            &format!("{} {what} ({})", m.process(s), cred_text(telling)),
            &plural(creds.len(), "privilege change", "privilege changes"),
            Tone::Warn,
            m.key(s),
            false,
        ));
    }
    sections.push(section("Privilege", "", privilege));

    let mut changed: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    let mut read: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for &s in &scope {
        for &f in &m.groups[s].files {
            let file = &c.files[f];
            if changes(&file.access) {
                changed.entry(&file.path).or_default().push(f);
            } else {
                read.entry(folder(&file.path)).or_default().push(f);
            }
        }
    }
    let mut changed: Vec<(&str, Vec<usize>)> = changed.into_iter().collect();
    changed.sort_by_key(|(_, files)| files.iter().map(|&f| c.files[f].at).min());
    let total = changed.len();
    let mut items: Vec<Item> = changed
        .into_iter()
        .take(DETAIL_LIMIT)
        .map(|(path, files)| {
            let by = processes(
                m,
                files
                    .iter()
                    .map(|&f| (c.files[f].run, c.files[f].execution)),
            );
            let what = format!("{} · {by}", operations(c, &files));
            item(path, &what, Tone::Muted, format!("file:{path}"), true)
        })
        .collect();
    if total > DETAIL_LIMIT {
        items.push(more_item(total - DETAIL_LIMIT, "file", "files"));
    }
    sections.push(section("Changed files", "", items));

    // Destinations, in the order first contacted.
    let mut destinations: BTreeMap<(&str, &str, i64), Vec<usize>> = BTreeMap::new();
    for &s in &scope {
        for &i in &m.groups[s].conns {
            let conn = &c.conns[i];
            destinations
                .entry((&conn.protocol, &conn.address, conn.port))
                .or_default()
                .push(i);
        }
    }
    let mut destinations: Vec<_> = destinations.into_iter().collect();
    destinations.sort_by_key(|(_, conns)| conns.iter().map(|&i| c.conns[i].at).min());
    let sent_dns = destinations.iter().any(|((_, _, port), _)| *port == 53);
    let mut items = Vec::new();
    for ((protocol, address, port), conns) in destinations {
        let first_at = conns.iter().map(|&i| c.conns[i].at).min().unwrap_or(0);
        let text = if port == 53 {
            format!("DNS queries to {address}")
        } else {
            match m.domains(address, first_at).as_slice() {
                [] => endpoint(address, port),
                [name] => name.clone(),
                names => format!("Possible domains: {}", names.join(", ")),
            }
        };
        let (outcome, tone) = summarize(conns.iter().map(|&i| m.conn_outcome(i)));
        let detail = format!(
            "{} {} · {} · {outcome} · {}",
            endpoint(address, port),
            protocol.to_uppercase(),
            plural(conns.len(), "connection", "connections"),
            span(
                conns
                    .iter()
                    .map(|&i| (c.conns[i].at, c.conns[i].time.as_str()))
            ),
        );
        let key = format!("dest:{protocol}:{address}:{port}");
        items.push(item(&text, &detail, tone, key, true));
    }
    sections.push(section("Network", "", items));

    // The Mac sees lookups, the guest sees which process sent DNS queries; they can only be
    // put side by side in time.
    if sent_dns {
        let start = scope
            .iter()
            .map(|&s| m.first(s).started_at)
            .min()
            .unwrap_or(0);
        let end = scope
            .iter()
            .map(|&s| m.last(s).exited_at.unwrap_or(i64::MAX))
            .max()
            .unwrap_or(i64::MAX)
            .saturating_add(1_000);
        let mut names: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (l, lookup) in c.lookups.iter().enumerate() {
            if lookup.at + 2_000 >= start && lookup.at <= end {
                names.entry(&lookup.name).or_default().push(l);
            }
        }
        let items = names
            .into_iter()
            .map(|(name, lookups)| {
                let (result, tone) = lookup_result(c, &lookups);
                let detail = format!("{result} · {}", plural(lookups.len(), "lookup", "lookups"));
                item(name, &detail, tone, format!("dns:{name}"), true)
            })
            .collect();
        sections.push(section(
            "DNS lookups while it ran",
            "Lookups can't be tied to a process exactly; these happened while it ran.",
            items,
        ));
    }

    if m.request.show_reads {
        let mut read: Vec<(String, Vec<usize>)> = read.into_iter().collect();
        read.sort_by_key(|(_, files)| std::cmp::Reverse(files.len()));
        let total = read.len();
        let mut items: Vec<Item> = read
            .into_iter()
            .take(LIST_LIMIT)
            .map(|(dir, files)| {
                let paths: HashSet<&str> =
                    files.iter().map(|&f| c.files[f].path.as_str()).collect();
                let by = processes(
                    m,
                    files
                        .iter()
                        .map(|&f| (c.files[f].run, c.files[f].execution)),
                );
                let what = format!("{} · {by}", plural(paths.len(), "file", "files"));
                item(
                    &format!("{dir}/"),
                    &what,
                    Tone::Muted,
                    format!("readdir:{dir}"),
                    true,
                )
            })
            .collect();
        if total > LIST_LIMIT {
            items.push(more_item(total - LIST_LIMIT, "folder", "folders"));
        }
        sections.push(section("Files read", "", items));
    }

    if !group.agent {
        let mut items = Vec::new();
        started(m, g, 0, &mut items);
        sections.push(section("Processes started", "", items));
    }
    sections.retain(|section| !section.items.is_empty());

    Detail {
        kind: if group.agent {
            "Agent process".into()
        } else if m.commands.contains(&g) {
            "Command".into()
        } else {
            "Process".into()
        },
        headline: {
            let (text, shell) = title(first);
            match shell {
                Some(shell) => format!("{shell} -c {}", one_line(&text, 120)),
                None => one_line(&text, 160),
            }
        },
        facts,
        script: script(first),
        argv: if first.argv.len() > 1 {
            first.argv.clone()
        } else {
            Vec::new()
        },
        argv_note: if first.argv_truncated {
            "Truncated after 4 KB".into()
        } else {
            String::new()
        },
        sections,
        record: process_record(m, g),
        note: "Environment variables and output aren't recorded.".into(),
    }
}

/// The processes below `g`, each indented under the one that started it.
fn started(m: &Model, g: usize, depth: i32, items: &mut Vec<Item>) {
    let mut children = m.groups[g].children.clone();
    children.sort_by_key(|&child| m.first(child).started_at);
    for child in children {
        let (result, result_tone) = m.result(child);
        let (mut parts, tone) = m.effects(&m.tally(&[child]), false);
        parts.insert(0, result);
        let tone = if result_tone == Tone::Bad {
            Tone::Bad
        } else {
            tone
        };
        let mut text = title(m.first(child)).0;
        for &e in &m.groups[child].execs[1..] {
            text.push_str(" → ");
            text.push_str(program(&m.c.execs[e]));
        }
        items.push(Item {
            depth,
            text: one_line(&text, 200),
            detail: parts.join(" · "),
            tone,
            key: m.key(child),
            mono: true,
        });
        started(m, child, depth + 1, items);
    }
}

/// Everything one process did, in order.
fn process_record(m: &Model, g: usize) -> Vec<Entry> {
    let c = m.c;
    let group = &m.groups[g];
    let mut entries: Vec<(i64, Entry)> = Vec::new();
    for &e in &group.failed {
        let exec = &c.execs[e];
        let text = format!(
            "Couldn't run {}: {}",
            exec.path,
            errno_text(exec.error.unwrap_or(0))
        );
        entries.push((exec.started_at, toned(&exec.time, text, Tone::Bad)));
    }
    for &e in &group.execs[1..] {
        let exec = &c.execs[e];
        entries.push((
            exec.started_at,
            entry(&exec.time, format!("Ran {}", title(exec).0)),
        ));
    }
    for &i in &group.creds {
        let cred = &c.creds[i];
        let text = format!("Privilege change: {}", cred_text(cred));
        entries.push((cred.at, toned(&cred.time, text, Tone::Warn)));
    }
    for &f in &group.files {
        let file = &c.files[f];
        if !changes(&file.access) && !m.request.show_reads {
            continue;
        }
        let text = match &file.new_path {
            Some(new) => format!("Renamed {} to {new}", file.path),
            None => format!(
                "{}: {}",
                capitalize(&file.access.replace(',', ", ")),
                file.path
            ),
        };
        entries.push((file.at, entry(&file.time, text)));
    }
    for &i in &group.conns {
        let conn = &c.conns[i];
        let (outcome, tone) = m.conn_outcome(i);
        let text = format!(
            "{} {}: {outcome}",
            endpoint(&conn.address, conn.port),
            conn.protocol.to_uppercase()
        );
        entries.push((conn.at, toned(&conn.time, text, tone)));
    }
    entries.sort_by_key(|(at, _)| *at);
    entries
        .into_iter()
        .map(|(_, entry)| entry)
        .take(ENTRY_LIMIT)
        .collect()
}

fn file_detail(m: &Model, path: &str) -> Detail {
    let c = m.c;
    let mut detail = Detail {
        kind: "File".into(),
        headline: path.into(),
        ..Detail::default()
    };
    let mut items = Vec::new();
    for file in &c.files {
        if file.path != path && file.new_path.as_deref() != Some(path) {
            continue;
        }
        if detail.facts.is_empty() {
            detail.facts.push(field("Area", AREAS[area(file)], false));
            if let Some(host) = &file.host_path {
                detail.facts.push(field("On your Mac", host, true));
            }
        }
        let what = match &file.new_path {
            Some(new) if file.path == path => format!("Renamed to {new}"),
            Some(_) => format!("Renamed from {}", file.path),
            None => capitalize(&file.access.replace(',', ", ")),
        };
        let (by, key) = match m.by_exec.get(&(file.run, file.execution)) {
            Some(&g) => (m.process(g), m.key(g)),
            None => ("an unknown process".into(), String::new()),
        };
        items.push(item(
            &what,
            &format!("{} · by {by}", file.time),
            Tone::Muted,
            key,
            false,
        ));
        if items.len() >= ENTRY_LIMIT {
            break;
        }
    }
    detail
        .sections
        .push(section("What happened, in order", "", items));
    detail
}

fn reads_detail(m: &Model, dir: &str, files: impl Iterator<Item = usize>) -> Detail {
    let c = m.c;
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for f in files {
        let file = &c.files[f];
        if changes(&file.access) || folder(&file.path) != dir || !seen.insert(&file.path) {
            continue;
        }
        let by = m
            .by_exec
            .get(&(file.run, file.execution))
            .map_or_else(|| "an unknown process".into(), |&g| m.process(g));
        let detail = format!("{} · by {by}", file.time);
        items.push(item(
            &file.path,
            &detail,
            Tone::Muted,
            format!("file:{}", file.path),
            true,
        ));
        if items.len() >= ENTRY_LIMIT {
            break;
        }
    }
    Detail {
        kind: "Files read".into(),
        headline: format!("{dir}/"),
        facts: vec![field("Files", &items.len().to_string(), false)],
        sections: vec![section("Files", "", items)],
        note: "Only the first open of a file by a process is recorded. An open for reading \
               doesn't prove the content was read."
            .into(),
        ..Detail::default()
    }
}

fn destination_detail(m: &Model, protocol: &str, address: &str, port: i64) -> Detail {
    let c = m.c;
    let mut record: Vec<(i64, Entry)> = Vec::new();
    let mut seen: Vec<(i64, &str)> = Vec::new();
    let mut outcomes = Vec::new();
    let mut who = BTreeSet::new();
    // Per process and outcome: the connections.
    let mut by_process: BTreeMap<(String, String), (Vec<usize>, Tone, String)> = BTreeMap::new();
    for (i, conn) in c.conns.iter().enumerate() {
        if conn.protocol != protocol || conn.address != address || conn.port != port {
            continue;
        }
        let (process, key) = match m.by_exec.get(&(conn.run, conn.execution)) {
            Some(&g) => (m.process(g), m.key(g)),
            None => ("an unknown process".to_owned(), String::new()),
        };
        who.insert(process.clone());
        let (outcome, tone) = m.conn_outcome(i);
        outcomes.push((outcome.clone(), tone));
        seen.push((conn.at, &conn.time));
        record.push((
            conn.at,
            toned(&conn.time, format!("{process}: {outcome}"), tone),
        ));
        by_process
            .entry((process, outcome))
            .or_insert_with(|| (Vec::new(), tone, key))
            .0
            .push(i);
    }
    let mut items: Vec<Item> = by_process
        .into_iter()
        .map(|((process, outcome), (conns, tone, key))| {
            let when = span(
                conns
                    .iter()
                    .map(|&i| (c.conns[i].at, c.conns[i].time.as_str())),
            );
            let detail = format!(
                "{} · {outcome} · {when}",
                plural(conns.len(), "connection", "connections")
            );
            item(&process, &detail, tone, key, false)
        })
        .collect();
    // Per outcome: how many flows, their tone and times.
    type Flows<'a> = (usize, Tone, Vec<(i64, &'a str)>);
    let mut mac_only: BTreeMap<String, Flows> = BTreeMap::new();
    for (f, flow) in c.flows.iter().enumerate() {
        if m.flow_matched[f]
            || flow.protocol != protocol
            || flow.address != address
            || flow.port != port
        {
            continue;
        }
        let (outcome, tone) = flow_outcome(flow);
        outcomes.push((outcome.clone(), tone));
        seen.push((flow.at, &flow.time));
        record.push((
            flow.at,
            toned(&flow.time, format!("Seen on the Mac only: {outcome}"), tone),
        ));
        let entry = mac_only.entry(outcome).or_insert((0, tone, Vec::new()));
        entry.0 += 1;
        entry.2.push((flow.at, &flow.time));
    }
    for (outcome, (n, tone, times)) in mac_only {
        let detail = format!(
            "{} · {outcome} · {}",
            plural(n, "connection", "connections"),
            span(times.into_iter())
        );
        items.push(item(
            "Seen on the Mac only",
            &detail,
            tone,
            String::new(),
            false,
        ));
    }

    // Lookups that returned the address, per name.
    let mut names: BTreeMap<&str, Vec<(usize, i64)>> = BTreeMap::new();
    for &(l, ttl) in m.answered.get(address).into_iter().flatten() {
        let lookup = &c.lookups[l];
        names.entry(&lookup.name).or_default().push((l, ttl));
        record.push((
            lookup.at,
            entry(
                &lookup.time,
                format!(
                    "DNS lookup: {} returned this address (TTL {ttl} s)",
                    lookup.name
                ),
            ),
        ));
    }
    let lookups: Vec<Item> = names
        .into_iter()
        .map(|(name, found)| {
            let low = found.iter().map(|(_, ttl)| *ttl).min().unwrap_or(0);
            let high = found.iter().map(|(_, ttl)| *ttl).max().unwrap_or(0);
            let ttl = if low == high {
                format!("TTL {low} s")
            } else {
                format!("TTL {low}–{high} s")
            };
            let when = span(
                found
                    .iter()
                    .map(|&(l, _)| (c.lookups[l].at, c.lookups[l].time.as_str())),
            );
            let detail = format!(
                "{} · {ttl} · {when}",
                plural(found.len(), "lookup", "lookups")
            );
            item(name, &detail, Tone::Muted, format!("dns:{name}"), true)
        })
        .collect();
    record.sort_by_key(|(at, _)| *at);

    let first_at = seen.iter().map(|(at, _)| *at).min().unwrap_or(0);
    let (domain, _) = m.domain_text(address, port, first_at);
    let (outcome, _) = summarize(outcomes.into_iter());
    let mut facts = vec![
        field(
            "Address",
            &format!("{} {}", endpoint(address, port), protocol.to_uppercase()),
            true,
        ),
        field("Domain", &domain, true),
        field("Outcome", &outcome, false),
    ];
    if !seen.is_empty() {
        facts.push(field("Seen", &span(seen.into_iter()), false));
    }
    if !who.is_empty() {
        facts.push(field(
            "Processes",
            &who.into_iter().collect::<Vec<_>>().join(", "),
            false,
        ));
    }
    Detail {
        kind: "Destination".into(),
        headline: if domain == "No domain recorded" {
            endpoint(address, port)
        } else {
            domain
        },
        facts,
        sections: vec![
            section("Connections", "", items),
            section("DNS lookups returning this address", "", lookups),
        ]
        .into_iter()
        .filter(|section| !section.items.is_empty())
        .collect(),
        record: record
            .into_iter()
            .map(|(_, entry)| entry)
            .take(ENTRY_LIMIT)
            .collect(),
        note: "The domain comes from DNS answers inside this instance. It shows which name \
               pointed to this address, not which hostname the request used."
            .into(),
        ..Detail::default()
    }
}

fn dns_detail(m: &Model, name: &str) -> Detail {
    let c = m.c;
    let lookups: Vec<usize> = (0..c.lookups.len())
        .filter(|&l| c.lookups[l].name == name)
        .collect();
    let (result, _) = lookup_result(c, &lookups);
    let kinds: BTreeSet<&str> = lookups
        .iter()
        .map(|&l| c.lookups[l].kind.as_str())
        .collect();
    let mut facts = vec![
        field("Result", &result, false),
        field(
            "Types",
            &kinds.into_iter().collect::<Vec<_>>().join(", "),
            false,
        ),
    ];
    let when = span(
        lookups
            .iter()
            .map(|&l| (c.lookups[l].at, c.lookups[l].time.as_str())),
    );
    facts.push(field(
        "Lookups",
        &format!("{} · {when}", lookups.len()),
        false,
    ));

    let addresses: BTreeSet<&str> = lookups
        .iter()
        .flat_map(|&l| {
            c.lookups[l]
                .answers
                .iter()
                .map(|(address, _)| address.as_str())
        })
        .collect();
    let answers = addresses
        .into_iter()
        .map(|address| {
            let conns: Vec<&GuestConn> = c
                .conns
                .iter()
                .filter(|conn| conn.address == address)
                .collect();
            match conns.first() {
                Some(conn) => {
                    let who = processes(m, conns.iter().map(|conn| (conn.run, conn.execution)));
                    let key = format!("dest:{}:{}:{}", conn.protocol, conn.address, conn.port);
                    item(
                        address,
                        &format!("Connected by {who}"),
                        Tone::Neutral,
                        key,
                        true,
                    )
                }
                None => item(
                    address,
                    "No connection recorded",
                    Tone::Muted,
                    String::new(),
                    true,
                ),
            }
        })
        .collect();

    let record = lookups
        .iter()
        .map(|&l| {
            let lookup = &c.lookups[l];
            let text = if lookup.refused {
                format!("{}: refused by policy", lookup.kind)
            } else if lookup.answers.is_empty() {
                format!("{}: no addresses ({})", lookup.kind, lookup.rcode)
            } else {
                let answers: Vec<String> = lookup
                    .answers
                    .iter()
                    .map(|(address, ttl)| format!("{address} (TTL {ttl} s)"))
                    .collect();
                let via = if lookup.cnames.is_empty() {
                    String::new()
                } else {
                    format!(" via {}", lookup.cnames.join(", "))
                };
                format!("{}: {}{via}", lookup.kind, answers.join(", "))
            };
            toned(
                &lookup.time,
                text,
                if lookup.refused {
                    Tone::Bad
                } else {
                    Tone::Neutral
                },
            )
        })
        .take(ENTRY_LIMIT)
        .collect();
    Detail {
        kind: "DNS name".into(),
        headline: name.into(),
        facts,
        sections: vec![section("Answers", "", answers)]
            .into_iter()
            .filter(|section| !section.items.is_empty())
            .collect(),
        record,
        note: "Lookups are recorded by the app on your Mac, which answers every DNS query of the \
               instance."
            .into(),
        ..Detail::default()
    }
}

/// What lookups of a name got, in short.
fn lookup_result(c: &Cache, lookups: &[usize]) -> (String, Tone) {
    let addresses: BTreeSet<&str> = lookups
        .iter()
        .flat_map(|&l| {
            c.lookups[l]
                .answers
                .iter()
                .map(|(address, _)| address.as_str())
        })
        .collect();
    if lookups.iter().any(|&l| c.lookups[l].refused) {
        ("Refused by policy".into(), Tone::Bad)
    } else if !addresses.is_empty() {
        let shown: Vec<&str> = addresses.iter().take(3).copied().collect();
        let more = addresses.len().saturating_sub(3);
        let text = if more > 0 {
            format!("Answered {} (+{more})", shown.join(", "))
        } else {
            format!("Answered {}", shown.join(", "))
        };
        (text, Tone::Neutral)
    } else if lookups.iter().any(|&l| c.lookups[l].rcode == "NameError") {
        ("No such domain".into(), Tone::Muted)
    } else {
        ("No addresses".into(), Tone::Muted)
    }
}

// Small pieces.

const AREAS: [&str; 4] = ["Shared folders", "Home", "Temporary", "System"];

fn area(file: &FileAccess) -> usize {
    let path = file.path.as_str();
    if file.host_path.is_some() {
        0
    } else if path.starts_with("/home/") || path == "/root" || path.starts_with("/root/") {
        1
    } else if ["/tmp/", "/var/tmp/", "/dev/shm/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        2
    } else {
        3
    }
}

/// Whether an access changes the file system: anything but reading.
fn changes(access: &str) -> bool {
    access.split(',').any(|word| {
        matches!(
            word,
            "write" | "create" | "truncate" | "append" | "delete" | "rename"
        )
    })
}

fn folder(path: &str) -> String {
    match path.rsplit_once('/') {
        Some(("", _)) | None => String::new(),
        Some((dir, _)) => dir.to_owned(),
    }
}

/// What happened to a file, in order: "created, written, renamed to /tmp/y".
fn operations(c: &Cache, files: &[usize]) -> String {
    let mut ops: Vec<String> = Vec::new();
    for &f in files {
        let file = &c.files[f];
        let op = if let Some(new) = &file.new_path {
            format!("renamed to {new}")
        } else if file.access.contains("delete") {
            "deleted".into()
        } else if file.access.contains("create") {
            "created or written".into()
        } else if file.access.contains("truncate") {
            "overwritten".into()
        } else if file.access.contains("append") {
            "appended to".into()
        } else {
            "written".into()
        };
        if ops.last() != Some(&op) {
            ops.push(op);
        }
    }
    capitalize(&ops.join(", "))
}

/// The distinct processes behind events, e.g. "claude 336, curl 392".
fn processes(m: &Model, owners: impl Iterator<Item = (i64, i64)>) -> String {
    let mut names: Vec<String> = Vec::new();
    for owner in owners {
        let name = m
            .by_exec
            .get(&owner)
            .map_or_else(|| "unknown".to_owned(), |&g| program(m.last(g)).to_owned());
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let more = names.len().saturating_sub(3);
    names.truncate(3);
    if more > 0 {
        format!("{} (+{more})", names.join(", "))
    } else {
        names.join(", ")
    }
}

/// One line for several outcomes: the worst first, with counts when they differ.
fn summarize(outcomes: impl Iterator<Item = (String, Tone)>) -> (String, Tone) {
    let mut counted: Vec<((String, Tone), usize)> = Vec::new();
    for outcome in outcomes {
        match counted.iter_mut().find(|(known, _)| *known == outcome) {
            Some((_, n)) => *n += 1,
            None => counted.push((outcome, 1)),
        }
    }
    counted.sort_by_key(|((_, tone), n)| (*tone != Tone::Bad, std::cmp::Reverse(*n)));
    match counted.as_slice() {
        [] => (String::new(), Tone::Muted),
        [(outcome, _)] => outcome.clone(),
        [((text, tone), n), rest @ ..] => {
            let others: usize = rest.iter().map(|(_, n)| n).sum();
            (format!("{text} ({n}), other outcomes ({others})"), *tone)
        }
    }
}

fn flow_outcome(flow: &Flow) -> (String, Tone) {
    if flow.blocked {
        return match flow.rule.as_deref() {
            Some(LOCAL) => (
                "Blocked: the Mac and local network aren't allowed".into(),
                Tone::Bad,
            ),
            Some(rule) => (format!("Blocked by rule: {rule}"), Tone::Bad),
            None => ("Blocked: not allowed by default".into(), Tone::Bad),
        };
    }
    match flow.outcome.as_deref() {
        Some("connected") => ("Connection succeeded".into(), Tone::Neutral),
        Some("failed") => (
            format!(
                "Connection failed: {}",
                flow.error.as_deref().unwrap_or("unknown error")
            ),
            Tone::Bad,
        ),
        Some("timed out") => ("Connection timed out".into(), Tone::Bad),
        _ => ("Allowed".into(), Tone::Neutral),
    }
}

/// The program name of an execution, as it called itself.
fn program(exec: &Exec) -> &str {
    let name = exec.argv.first().map_or(exec.path.as_str(), String::as_str);
    name.rsplit('/').next().unwrap_or(name)
}

/// A one-line command line, and the shell when it ran a script (`sh -c`).
fn title(exec: &Exec) -> (String, Option<String>) {
    let name = program(exec);
    if SHELLS.contains(&name)
        && exec.argv.iter().skip(1).any(|arg| arg == "-c")
        && let Some(script) = exec.argv.iter().skip(1).find(|arg| !arg.starts_with('-'))
    {
        return (one_line(script, 400), Some(name.to_owned()));
    }
    if exec.argv.is_empty() {
        return (exec.path.clone(), None);
    }
    let line = exec
        .argv
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    (one_line(&line, 400), None)
}

fn quote(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c))
    {
        arg.to_owned()
    } else {
        format!("'{}'", arg.replace('\'', r"'\''"))
    }
}

/// Whitespace collapsed to single spaces, cut to a length a row can show.
fn one_line(text: &str, limit: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line,
    }
}

fn endpoint(address: &str, port: i64) -> String {
    if address.contains(':') {
        format!("[{address}]:{port}")
    } else {
        format!("{address}:{port}")
    }
}

/// The reason for an errno of the Linux guest.
fn errno_text(errno: i64) -> String {
    match errno {
        1 => "Operation not permitted".into(),
        2 => "No such file or directory".into(),
        7 => "Argument list too long".into(),
        8 => "Exec format error".into(),
        12 => "Out of memory".into(),
        13 => "Permission denied".into(),
        20 => "Not a directory".into(),
        21 => "Is a directory".into(),
        26 => "Text file busy".into(),
        40 => "Too many levels of symbolic links".into(),
        _ => format!("Error {errno}"),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", thousands(n))
    }
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The script a shell ran, or the full command line.
fn script(exec: &Exec) -> String {
    let name = program(exec);
    if SHELLS.contains(&name)
        && exec.argv.iter().skip(1).any(|arg| arg == "-c")
        && let Some(script) = exec.argv.iter().skip(1).find(|arg| !arg.starts_with('-'))
    {
        return script.trim().to_owned();
    }
    exec.argv
        .iter()
        .map(|arg| quote(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// "uid 1000 → 0, capabilities raised"
fn cred_text(cred: &Cred) -> String {
    let mut changes = Vec::new();
    for (name, (before, after)) in [
        ("uid", cred.uid),
        ("euid", cred.euid),
        ("gid", cred.gid),
        ("egid", cred.egid),
    ] {
        if before != after {
            changes.push(format!("{name} {before} → {after}"));
        }
    }
    if cred.caps {
        changes.push("capabilities raised".into());
    }
    changes.join(", ")
}

/// "07:54:35.428–07:55:05.457", or one time.
fn span<'t>(times: impl Iterator<Item = (i64, &'t str)>) -> String {
    let times: Vec<(i64, &str)> = times.collect();
    let first = times.iter().min_by_key(|(at, _)| *at);
    let last = times.iter().max_by_key(|(at, _)| *at);
    match (first, last) {
        (Some(first), Some(last)) if first.0 != last.0 => format!("{}–{}", first.1, last.1),
        (Some(first), _) => first.1.to_owned(),
        _ => String::new(),
    }
}

fn duration(ms: i64) -> String {
    if ms < 1_000 {
        format!("{} ms", ms.max(0))
    } else if ms < 60_000 {
        format!("{:.1} s", ms as f64 / 1000.0)
    } else {
        format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000)
    }
}

fn section(title: &str, note: &str, items: Vec<Item>) -> Section {
    Section {
        title: title.into(),
        note: note.into(),
        items,
    }
}

fn item(text: &str, detail: &str, tone: Tone, key: String, mono: bool) -> Item {
    Item {
        depth: 0,
        text: text.into(),
        detail: detail.into(),
        tone,
        key,
        mono,
    }
}

fn more_item(n: usize, one: &str, many: &str) -> Item {
    item(
        &format!("and {} more", plural(n, one, many)),
        "",
        Tone::Muted,
        String::new(),
        false,
    )
}

fn toned(time: &str, text: String, tone: Tone) -> Entry {
    Entry {
        time: time.into(),
        text,
        tone,
    }
}

fn field(label: &str, value: &str, mono: bool) -> Field {
    Field {
        label: label.into(),
        value: value.into(),
        mono,
    }
}

fn entry(time: &str, text: String) -> Entry {
    Entry {
        time: time.into(),
        text,
        tone: Tone::Neutral,
    }
}

fn more_row(key: &str, depth: i32, n: usize, one: &str, many: &str) -> Row {
    Row {
        key: format!("{key}#more{one}"),
        depth,
        title: format!("and {} more", plural(n, one, many)),
        dim: true,
        ..Row::default()
    }
}

impl Default for Row {
    fn default() -> Row {
        Row {
            key: String::new(),
            depth: 0,
            heading: false,
            expandable: false,
            expanded: false,
            time: String::new(),
            label: String::new(),
            title: String::new(),
            subtitle: String::new(),
            subtitle_tone: Tone::Muted,
            meta: String::new(),
            result: String::new(),
            tone: Tone::Neutral,
            dim: false,
        }
    }
}
