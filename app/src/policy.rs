//! A VM's network policy: which destinations its guest may reach, decided on the Mac.
//!
//! Address rules match a destination directly. Domain rules match through DNS: the guest's
//! lookups all go through `net`, which asks `check_query` before forwarding one and hands
//! every answer to `record_answer`, so each address the guest learned from DNS is known
//! with the names that returned it. An address can't be traced to the hostname a request
//! really used (shared hosting, CDNs, encrypted DNS); it only carries the names that
//! pointed to it.
//!
//! A destination is decided in this order: the Mac and the local network are never
//! reachable; a matching deny rule blocks; a matching allow rule allows; otherwise the
//! default action applies. A lookup is decided the same way by its name, and a refused one
//! is answered with REFUSED.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use simple_dns::rdata::RData;
use simple_dns::{Name, Packet, RCODE};

use crate::db::{NetworkAction, NetworkRule, RuleTarget, max_prefix};

/// How long an address stays tied to its names after the TTL of the answer ran out,
/// as programs in the guest may keep using an address a little longer than that.
const DNS_GRACE: Duration = Duration::from_secs(5 * 60);
/// The longest TTL honored, so a long one doesn't tie an address to a name for days.
const MAX_TTL: u32 = 60 * 60;

pub struct Policy {
    default: NetworkAction,
    rules: Vec<NetworkRule>,
    /// Each address from a DNS answer with the names that returned it, and until when.
    names: Mutex<HashMap<IpAddr, Vec<(String, Instant)>>>,
}

impl Policy {
    pub fn new(default: NetworkAction, rules: Vec<NetworkRule>) -> Policy {
        Policy {
            default,
            rules,
            names: Mutex::default(),
        }
    }

    /// Maintenance and provision boots: the internet is open, the Mac and the local
    /// network stay unreachable.
    pub fn open() -> Policy {
        Policy::new(NetworkAction::Allow, Vec::new())
    }

    /// Whether the guest may connect to `destination`.
    pub fn allows(&self, destination: IpAddr) -> bool {
        if local(destination) {
            return false;
        }
        let now = Instant::now();
        let mut targets = vec![RuleTarget::Cidr {
            network: format!("{destination}/{}", max_prefix(destination)),
        }];
        if let Some(names) = self.names().get(&destination) {
            targets.extend(
                names
                    .iter()
                    .filter(|(_, expires)| *expires > now)
                    .map(|(name, _)| domain(name.clone())),
            );
        }
        self.decide(&targets)
    }

    /// Decides a DNS query of the guest: `None` to forward it, or the reply that refuses
    /// it. A query that can't be read is refused too.
    pub fn check_query(&self, query: &[u8]) -> Option<Vec<u8>> {
        if self.rules.is_empty() && self.default == NetworkAction::Allow {
            return None;
        }
        let (id, questions, rcode) = match Packet::parse(query) {
            Ok(packet) => {
                let allowed = !packet.questions.is_empty()
                    && packet
                        .questions
                        .iter()
                        .all(|question| self.decide(&[domain(name(&question.qname))]));
                if allowed {
                    return None;
                }
                (packet.id(), packet.questions, RCODE::Refused)
            }
            Err(_) => {
                let id = query
                    .get(..2)
                    .map_or(0, |id| u16::from_be_bytes([id[0], id[1]]));
                (id, Vec::new(), RCODE::FormatError)
            }
        };
        let mut reply = Packet::new_reply(id);
        reply.questions = questions;
        *reply.rcode_mut() = rcode;
        reply.build_bytes_vec().ok()
    }

    /// Records the addresses of a DNS answer from the Mac's nameserver, each with the
    /// queried names and the names of its CNAME chain.
    pub fn record_answer(&self, reply: &[u8]) {
        let Ok(packet) = Packet::parse(reply) else {
            return;
        };
        let mut names: Vec<String> = packet
            .questions
            .iter()
            .map(|question| &question.qname)
            .chain(packet.answers.iter().map(|record| &record.name))
            .map(name)
            .collect();
        names.sort();
        names.dedup();

        let now = Instant::now();
        let mut recorded = self.names();
        recorded.retain(|_, names| {
            names.retain(|(_, expires)| *expires > now);
            !names.is_empty()
        });
        for record in &packet.answers {
            let address = match &record.rdata {
                RData::A(a) => IpAddr::V4(Ipv4Addr::from(a.address)),
                RData::AAAA(aaaa) => IpAddr::V6(Ipv6Addr::from(aaaa.address)),
                _ => continue,
            };
            let expires = now + Duration::from_secs(record.ttl.min(MAX_TTL).into()) + DNS_GRACE;
            let known = recorded.entry(address).or_default();
            for name in &names {
                match known.iter_mut().find(|(known, _)| known == name) {
                    Some((_, until)) => *until = (*until).max(expires),
                    None => known.push((name.clone(), expires)),
                }
            }
        }
    }

    /// Deny rules win over allow rules; without a matching rule, the default applies.
    fn decide(&self, targets: &[RuleTarget]) -> bool {
        let matches = |action| {
            self.rules.iter().any(|rule| {
                rule.action == action && targets.iter().any(|target| rule.target.covers(target))
            })
        };
        if matches(NetworkAction::Deny) {
            false
        } else if matches(NetworkAction::Allow) {
            true
        } else {
            self.default == NetworkAction::Allow
        }
    }

    fn names(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Vec<(String, Instant)>>> {
        self.names.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A DNS name as rules store domains: lowercase, without a trailing dot.
fn name(name: &Name) -> String {
    name.to_string().to_ascii_lowercase()
}

fn domain(name: String) -> RuleTarget {
    RuleTarget::Domain {
        name,
        include_subdomains: false,
    }
}

/// The Mac and the local network, which no rule can open: loopback, private, link-local,
/// shared (carrier-grade NAT, also used by VPNs such as Tailscale), multicast, and
/// reserved addresses. The guest's gateway is a private address, so this includes it.
fn local(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, ..] = address.octets();
            address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_multicast()
                || first == 0
                || first >= 240
                || (first == 100 && (64..128).contains(&second))
        }
        IpAddr::V6(address) => {
            address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|address| local(IpAddr::V4(address)))
        }
    }
}
