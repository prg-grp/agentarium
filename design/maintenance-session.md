# Maintenance session

A running instance booted in maintenance mode: no agent, a root shell, internet restrictions lifted. Frame: [app-shell.md](app-shell.md) with the amber mode treatment described here. Entry dialog: [maintenance-confirmation.md](maintenance-confirmation.md).

## Layout

```
┌──────────────────┬═══════════════════════════════ (3 px amber line) ═══════════════════════════════┐
│ Instances    [+] │ ⚒ Maintenance session  Internet open, your Mac and local network blocked.      │
│  ● api-refactor  │   Shared folders stay writable.                        [↻ Return to agent mode] │
│ ▌⚒ scraper-lab   ├─────────────────────────────────────────────────────────────────────────────────┤
│   Maintenance,   │ scraper-lab  │ VM Running │ Agent Not running in maintenance │ Monitoring Running │
│   internet open  │ Claude Code                                                   [Stop] │ Delete   │
│  ○ docs-site     ├─────────────────────────────────────────────────────────────────────────────────┤
│                  │ Terminal (amber underline)   Activity   Settings              [Live activity]  │
│                  ├─────────────────────────────────────────────────────────────────────────────────┤
│                  │ Root shell in scraper-lab                    Boot log │ Live activity    Pause  │
│                  │ root@scraper-lab:~# apt-get install -y chromium      │ 10:41:20 /usr/lib/…     │
│                  │ Reading package lists... Done                        │   Created by dpkg       │
│                  │ …                                                    │   Maintenance           │
│                  │ root@scraper-lab:~# █                                │ 10:41:02 151.101.2.132  │
│                  │                                                      │   Allowed by maintenance│
│                  │                                                      │   exception.            │
└──────────────────┴──────────────────────────────────────────────────────┴─────────────────────────┘
```

## Persistent mode indication

Maintenance must be unmistakable on every tab for the whole session:

1. A 3 px `warn` line along the top edge of the main area.
2. A banner directly below it, above the instance header: `warn` at 12% fill, `warn` at 35% bottom border, tool icon. "Maintenance session" in `warn` 500, followed by the effective network state in `tx2`: "Internet open, your Mac and local network blocked. Shared folders stay writable." The banner states the network state, not just the mode name.
3. "Return to agent mode" (neutral with a `warn` border, refresh icon) at the right end of the banner. It reboots the instance in agent mode and restores the configured network policy.
4. The active tab underline and the selected sidebar item's left bar use `warn` instead of `acc`.
5. Sidebar summary: tool icon in `warn`, "Maintenance, internet open".

The banner cannot be dismissed.

## Header

- Agent reading: pause icon in `tx3`, "Not running in maintenance" in `tx2`.
- Monitoring reading reflects the real state. If the collector is changed or stopped by root, it switches to Degraded or Not running immediately, with the notice from [agent-exited-degraded.md](agent-exited-degraded.md#degraded-monitoring-notice).
- Actions: "Restart in agent mode" (ghost, sparkles icon), "Stop" and "Delete". No "Restart in maintenance" while already in maintenance. "Restart in agent mode" does the same as the banner's "Return to agent mode"; the header button exists until the banner is built.

## Terminal

- Header row: "Root shell in `scraper-lab`" with the instance name in `warn`.
- Prompt shows `root@<instance>`; the app colors nothing inside the shell output itself beyond what the shell emits. (The mock colors the prompt amber to illustrate.)

## Activity

- Events from this session carry the Maintenance source tag, shown as a `warn` word in the live rail ("Created by dpkg Maintenance") and filterable via the Maintenance chip in [activity-timeline.md](activity-timeline.md).
- Connections allowed only because of maintenance say so: "Connection succeeded, deb.debian.org. Allowed by maintenance exception."
- A lifecycle row records the boot: "Booted in maintenance / Internet rules lifted for this boot".
- Do not imply that root activity is tamper-proof.

## After the session

- An ordinary reboot or "Return to agent mode" always boots in agent mode with the configured policy. Maintenance is never sticky.
- The agent is never started automatically when maintenance preparation completes.
