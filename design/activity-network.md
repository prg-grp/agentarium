# Activity: network view

The Activity tab with the Network view selected and a blocked connection open in the detail pane. Frame: [app-shell.md](app-shell.md). Toolbar, counts bar, and detail pane follow [activity-timeline.md](activity-timeline.md). Policy semantics: [ARCHITECTURE.md](../ARCHITECTURE.md#network-policy).

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

- Main column + 400 px detail pane.

## Table

One row per destination address (and one row per DNS-only lookup). Columns: Address (170 px, monospace `ip:port`), Domain (flexible), Process (84 px), Attempts (70 px), Outcome (250 px).

- **Address is always shown**, even when a domain is known.
- **Domain column cases:**
  - One associated name: the name in monospace.
  - Several names for the address: a `warn` 12 px label with arrows-split icon "Possible domains", then the candidates in monospace, comma-separated. Never pick one.
  - No lookup recorded: "No domain recorded" in `tx3`.
- **DNS-only row:** Address column reads "No connection" in `tx3`, attempts 0, outcome "DNS lookup recorded; no related connection recorded".
- **Outcome:** "Connection succeeded" (`tx2`), "Connection failed" (`bad`, x icon), "Blocked by rule: <rule>" or "Blocked: unlisted destination" (`bad`, ban icon).
- Selected row as in the timeline.

## Counts bar

"connections", "blocked attempts" (`bad`), "DNS lookups", "address without a recorded domain". No "searches" or "websites".

## Detail pane (blocked connection)

- Title "Blocked connection", subtitle `104.20.67.143:443 TCP` in monospace.
- "What was recorded, in order": a vertical evidence chain (icons joined by a 1 px `line2` line), each item with its type, monospace timestamp on the right, and a 13 px description:
  1. DNS lookup: the queried name, the answers, TTL, CNAME chain ("no CNAME").
  2. Policy decision: action, matching rule and its scope, network policy revision.
  3. Connection attempt: result, where enforcement happened ("Blocked on your Mac, outside the instance."), bytes ("No bytes were sent.").
  Policy decisions and connection outcomes stay separate items. Show only links that the evidence supports.
- Key/value rows: Process (parent chain), Command (full argv when attributed), Address shared ("No other domain returned this address during this run." or the list of other domains).
- Caveat box (1 px `line2` border, 8 px radius, info icon, 13 px `tx2`): "The domain comes from DNS answers inside this instance. It shows which name pointed to this address, not which hostname the request used."
- "Show in timeline" link.

## Rules

- Do not invent URLs. A URL may appear only inside recorded argv (Command row).
- Show the DNS evidence time interval when an association relies on an older lookup ("Resolved 14:02, TTL expired 14:07").
- For an allowed connection to an ambiguous address, the pane lists all candidate names with the lookup times that produced them.
