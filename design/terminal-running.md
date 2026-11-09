# Running instance: terminal

The main working screen: a running instance in agent mode with the Terminal tab open. Frame: [app-shell.md](app-shell.md). This was the agreed "overview" mockup.

## Layout

```
api-refactor                                                    [Restart in maintenance] [Stop] │ Delete
Claude Code
─────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings 2 pending                                   [Live activity]
─────────────────────────────────────────────────────────────────────────────────────────────────
Agent session in /work/api                         Boot log │ Live activity                 Pause
─────────────────────────────────────────────────────────── │ ───────────────────────────────────
> add rate limiting to the /upload endpoint                 │ 14:08:52 ✎ /work/api/src/routes/
                                                            │            upload.rs
● I'll start with the upload handler and its tests.         │            Write succeeded, cargo
● Read(src/routes/upload.rs)                                │ ▌14:08:31 ⊘ 104.20.67.143:443
  └ Read 142 lines                                          │            Blocked by rule: pastebin.com
● Bash(cargo add governor@0.6)                              │ 14:08:30 ⌕ pastebin.com
  …                                                         │            DNS lookup, 2 answers
╭──────────────────────────────────────╮                    │ 14:08:11 ⇌ 151.101.130.137:443
│ > █                                  │                    │            Connection succeeded. Possible
╰──────────────────────────────────────╯                    │            domains: index.crates.io, …
                                                            │ Sample data
```

- Content grid: terminal (flexible) + live activity rail (340 px). With the rail hidden, the terminal fills the width.
- Minimum content height about 460 px; the terminal grows with the window.

## Terminal

- Background `well`; text SF Mono 12.5 / 21, base color `#D6DCE5`. Terminal colors come from the agent's own output (ANSI); do not restyle them.
- A thin header row above the terminal (`tx3` 12 px, 1 px `line` bottom border): "Agent session in `/work/api`" on the left (path monospace, `tx2`), "Boot log" link on the right, which opens the Logs tab ([app-shell.md](app-shell.md#tab-bar)). Boot diagnostics stay separate from the agent session and never print into it.
- Normal terminal behavior: keyboard input, selection, copy, paste, resize, scrolling, full-screen terminal apps. Do not intercept the agent's ordinary shortcuts. Keep focus predictable: clicking the terminal focuses it; tab switches do not steal focus back unexpectedly.
- http and https links in the output are underlined (1 px, terminal foreground) while the pointer is over them, with a pointer cursor. Cmd+click opens the link in the default browser; a plain click or drag selects as usual. Links that wrap across rows count as one link; trailing sentence punctuation is not part of it. Also in the boot log.
- When the user scrolls up, keep the position and show a "Jump to latest" control at the bottom right of the terminal. Remove it when the user returns to the end.
- Scrollback holds up to 100,000 lines. While there is history (not on the alternate screen of full-screen apps), a thin scrollbar sits in the right gutter: a `tx3` thumb at 35% (70% on hover or drag, track `line` at 60%), at least 28 px tall. Dragging the thumb scrolls; clicking the track jumps there. Cmd+↑ / Cmd+↓ go to the top / latest output, Shift+PageUp / Shift+PageDown scroll a page. The busy scrim covers the scrollbar too.
- Agent exit and restart behavior: see [agent-exited-degraded.md](agent-exited-degraded.md).
- While the instance starts, stops or restarts, the terminal is covered and takes no input (keys, clicks, selection, scrolling). Its content stays visible under a `well` scrim at 50%, with a centered indeterminate spinner (28 px) and the transition in `tx2` 12 px: "Starting…", "Provisioning…", "Stopping…", "Restarting in maintenance…", "Restarting in agent mode…", "Deleting…". Starting lasts until the guest supervisor has received the boot mode, not just until the VM runs. A provisioning boot is covered from start to end ("Provisioning…"); the script's output shows dimmed behind the scrim, and the agent boot that follows continues with "Starting…". The cursor is hidden meanwhile. The Logs tab is never covered, since it shows the progress. The spinner is a deliberate exception to "no animated status effects" in [DESIGN.md](../DESIGN.md): it follows a user action and ends with it.

## Live activity rail

- `panel` background, 1 px `line` left border.
- Header row: "Live activity" (`tx2`, 500, 12 px) and "Pause" (`tx3`) which freezes the list without stopping collection; resuming shows how many events arrived while paused.
- Rows, newest first: time (monospace `tx3`, 62 px column), type icon (18 px, labeled), target on the first line (monospace, wraps anywhere), result line in 12 px (`tx2`, or `bad` for blocks).
- Blocked rows get a 2 px `bad` inset bar on the left.
- Monitoring gaps appear as hatched rows (see [activity-timeline.md](activity-timeline.md#monitoring-gap-row)).
- Footer: "Sample data" in mockups; in the product, a link "Open in Activity" that switches to the Activity tab with the same time window.
- Clicking a row opens that event in the Activity tab's detail pane.

## Copy examples (sample data)

- "Write succeeded, cargo"
- "Blocked by rule: pastebin.com"
- "DNS lookup, 2 answers"
- "Connection succeeded. Possible domains: index.crates.io, static.crates.io"
- "Executed by claude"

## Notes

- Telemetry backpressure must never freeze the terminal. The rail may lag or show a gap; the terminal stays responsive.
- Terminal recording is undecided. Do not label the terminal as recorded.
