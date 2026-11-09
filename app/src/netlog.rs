//! The network log of an agent boot: the guest's DNS lookups and the policy's decision on
//! each of its TCP connections and UDP flows, as the app's network stack on the Mac saw
//! them (see net.rs). Recorded outside the guest, so nothing in it can hide them.
//!
//! The network tasks hand events to `NetLog::record`, which never waits: when the queue is
//! full the event is dropped and counted. A thread of its own stores them in the database,
//! through a connection of its own, one transaction per batch. It ends once the VM's
//! network is gone.

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rusqlite::{Transaction, params};

use crate::db;
use crate::policy::{Answer, Query, Verdict};

/// Events waiting for the database; more are dropped.
const QUEUE: usize = 4096;
/// How long events are collected into one transaction.
const BATCH: Duration = Duration::from_millis(200);

pub enum Event {
    /// The Mac's nameserver answered a lookup.
    Answered(Answer),
    /// The policy refused a lookup.
    Refused(Vec<Query>),
    Flow(Flow),
}

/// A TCP connection or UDP flow of the guest.
pub struct Flow {
    pub protocol: &'static str,
    /// The guest's address and source port.
    pub guest: SocketAddr,
    pub destination: SocketAddr,
    pub verdict: Verdict,
    /// Whether the Mac reached the destination; `None` when blocked or not known yet.
    pub outcome: Option<Outcome>,
}

pub enum Outcome {
    Connected,
    Failed(String),
    TimedOut,
}

/// Where the network tasks record events. Cloned into each task.
#[derive(Clone)]
pub struct NetLog {
    sender: SyncSender<(i64, Event)>,
    dropped: Arc<AtomicU64>,
}

impl NetLog {
    pub fn record(&self, event: Event) {
        if let Err(TrySendError::Full(_)) = self.sender.try_send((now_ms(), event)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Starts storing the network log of the boot `boot`.
pub fn spawn(boot: i64) -> io::Result<NetLog> {
    let (sender, receiver) = mpsc::sync_channel(QUEUE);
    let dropped = Arc::new(AtomicU64::new(0));
    let counted = dropped.clone();
    std::thread::Builder::new()
        .name("vm-net-log".into())
        .spawn(move || {
            if let Err(error) = store(boot, &receiver) {
                eprintln!("The network log stopped: {error}");
            }
            let dropped = counted.load(Ordering::Relaxed);
            if dropped > 0 {
                eprintln!("The network log dropped {dropped} events: its queue was full");
            }
        })?;
    Ok(NetLog { sender, dropped })
}

fn store(boot: i64, receiver: &Receiver<(i64, Event)>) -> Result<(), Box<dyn Error>> {
    let mut conn = db::open_writer()?;
    // Waits for an event, then takes what else arrives within BATCH. Ends when every
    // sender is gone.
    while let Ok(first) = receiver.recv() {
        let deadline = Instant::now() + BATCH;
        let tx = conn.transaction()?;
        insert(&tx, boot, first)?;
        while let Ok(event) =
            receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            insert(&tx, boot, event)?;
        }
        tx.commit()?;
    }
    Ok(())
}

fn insert(tx: &Transaction, boot: i64, (at, event): (i64, Event)) -> rusqlite::Result<()> {
    match event {
        Event::Answered(answer) => {
            tx.prepare_cached(
                "INSERT INTO dns_lookups (boot_id, at, name, type, result, rcode, cnames)
                 VALUES (?1, ?2, ?3, ?4, 'answered', ?5, ?6)",
            )?
            .execute(params![
                boot,
                at,
                answer.query.name,
                answer.query.kind,
                answer.rcode,
                serde_json::to_string(&answer.cnames).unwrap_or_else(|_| "[]".into()),
            ])?;
            let lookup = tx.last_insert_rowid();
            for (address, ttl) in answer.addresses {
                tx.prepare_cached(
                    "INSERT INTO dns_answers (lookup_id, boot_id, address, ttl)
                     VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(params![lookup, boot, address.to_string(), ttl])?;
            }
        }
        Event::Refused(queries) => {
            for query in queries {
                tx.prepare_cached(
                    "INSERT INTO dns_lookups (boot_id, at, name, type, result, rcode)
                     VALUES (?1, ?2, ?3, ?4, 'refused', 'Refused')",
                )?
                .execute(params![boot, at, query.name, query.kind])?;
            }
        }
        Event::Flow(flow) => {
            let (outcome, error) = match flow.outcome {
                None => (None, None),
                Some(Outcome::Connected) => (Some("connected"), None),
                Some(Outcome::Failed(error)) => (Some("failed"), Some(error)),
                Some(Outcome::TimedOut) => (Some("timed out"), None),
            };
            tx.prepare_cached(
                "INSERT INTO network_flows (boot_id, at, protocol, local_port, address, port,
                 decision, rule, outcome, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?
            .execute(params![
                boot,
                at,
                flow.protocol,
                flow.guest.port(),
                flow.destination.ip().to_string(),
                flow.destination.port(),
                if flow.verdict.allowed {
                    "allowed"
                } else {
                    "blocked"
                },
                flow.verdict.rule,
                outcome,
                error,
            ])?;
        }
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |now| now.as_millis() as i64)
}
