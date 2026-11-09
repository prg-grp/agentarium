# Activity: network view

The Activity tab with the Network view selected. Frame: [app-shell.md](app-shell.md). Toolbar and counts bar follow [activity-timeline.md](activity-timeline.md); a row opens its detail page as described in [activity-commands.md](activity-commands.md#detail-page). Policy semantics: [ARCHITECTURE.md](../ARCHITECTURE.md#network-policy).

## Layout

```
[Timeline|Network]  [⟲ This run, started 13:41 ▾]  [⌕ Search domains and addresses              ]
─────────────────────────────────────────────────────────────────────── ┬─────────────────────────────
47 connections │ 2 blocked attempts │ 19 DNS lookups │ 1 address without │ Blocked connection        ×
a recorded domain                                                       │ 104.20.67.143:443 TCP
─────────────────────────────────────────────────────────────────────── │
Address              Domain                    Process Attempts Outcome │ What was recorded, in order
▌104.20.67.143:443   pastebin.com              curl    1  ⊘ Blocked by  │ ⌕ DNS lookup     14:08:30.877
                                                          rule: pastebin│ │ pastebin.com returned
160.79.104.10:443    api.anthropic.com         claude  41 Connection    │ │ 104.20.67.143 and …
                                                          succeeded     │ ⛨ Policy decision 14:08:31.902
151.101.130.137:443  ⇄ Possible domains        cargo   3  Connection    │ │ Denied by rule pastebin.com,
                     index.crates.io,                     succeeded     │ │ includes subdomains. Rev 7.
                     static.crates.io                                   │ ⊘ Connection attempt 14:08:31.902
140.82.121.4:443     github.com                git     2  Connection …  │   Blocked on your Mac, outside
93.184.215.14:80     No domain recorded        python3 1  ⊘ Blocked:    │   the instance. No bytes sent.
                                                          unlisted dest.│ ─────────────────────────────
No connection        registry.npmjs.org        node    0  DNS lookup    │ Process  curl 498 from bash …
                                                          recorded; no  │ Command  curl -sS https://…
                                                          related conn. │ Address shared  No other domain…
Sample data                                                             │ ⓘ The domain comes from DNS…
                                                                        │ Show in timeline
```

- The list uses the full tab width; selecting a row opens its page.

## Table

One row per destination address (and one row per DNS-only lookup). Columns: Address (170 px, monospace `ip:port`), Domain (flexible), Process (84 px), Attempts (70 px), Outcome (250 px).

- **Address is always shown**, even when a domain is known.
- **Domain column cases:**
  - One associated name: the name in monospace.
  - Several names for the address: a `warn` 12 px label with arrows-split icon "Possible domains", then the candidates in monospace, comma-separated. Never pick one.
  - No lookup recorded: "No domain recorded" in `tx3`.
- **DNS-only row:** Address column reads "No connection" in `tx3`, attempts 0, outcome "DNS lookup recorded; no related connection recorded".
- **Outcome**, from the Mac's network log: "Connection succeeded" (`tx2`), "Connection failed: <error>" or "Connection timed out" (`bad`), "Blocked by rule: <rule>", "Blocked: not allowed by default" or "Blocked: the Mac and local network aren't allowed" (`bad`). UDP flows read "Allowed" or "Sent". Without a matching record on the Mac: "Recorded in the guest" (`tx3`). Several attempts with different outcomes show the worst first with counts.
- The implemented view lists destinations under a "Destinations" heading and lookups under a "DNS lookups" heading (one row per name: types, number of lookups, answers; "Refused by policy" in `bad`, "Answered; connected", "No such domain", or "DNS lookup recorded; no related connection recorded"). A connection seen only on the Mac says "seen on the Mac only" as its process.
- Selected row as in the timeline.

## Counts bar

"connections", "blocked attempts" (`bad`), "DNS lookups", "address without a recorded domain". No "searches" or "websites".

## Destination page

- **Kind:** "Destination". **Headline:** the domain, "Possible domains: a, b", or the address when no lookup returned it.
- **Facts:** Address (`104.20.67.143:443 TCP`), Domain, Outcome (worst first, with counts), Seen (first and last time), Processes.
- **Sections:**
  - "Connections": one line per process and outcome, "claude 393" with "17 connections · Connection succeeded · 07:54:35.428–07:55:05.457". Flows the Mac saw without a guest record read "Seen on the Mac only".
  - "DNS lookups returning this address": one line per name, "api.anthropic.com" with "4 lookups · TTL 266–300 s · 07:54:35.428–07:54:37.701". It opens the name's page.
- **Note:** the caveat "The domain comes from DNS answers inside this instance. It shows which name pointed to this address, not which hostname the request used."
- **Full record**, collapsed: every lookup and attempt in order.

## DNS name page

- **Facts:** Result ("Refused by policy", "Answered 1.2.3.4, …", "No such domain"), Types, Lookups (count and times).
- **"Answers":** each returned address, "Connected by claude" (opens the destination) or "No connection recorded".
- **Full record**, collapsed: each lookup with its answers, TTLs and CNAMEs.

Not built yet: the evidence chain with icons, policy revisions, bytes, and a "Show in timeline" link.

## Rules

- Do not invent URLs. A URL may appear only inside recorded argv (Command row).
- Show the DNS evidence time interval when an association relies on an older lookup ("Resolved 14:02, TTL expired 14:07").
- For an allowed connection to an ambiguous address, the page lists all candidate names with the lookups that produced them.
