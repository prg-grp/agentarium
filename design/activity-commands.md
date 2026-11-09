# Activity: commands and files

The Activity tab opens on the Commands view: what the agent ran in one run, each command with what it did. The Files view lists the same run's file activity by area. Frame: [app-shell.md](app-shell.md). All events: [activity-timeline.md](activity-timeline.md). Network: [activity-network.md](activity-network.md). Data: [ARCHITECTURE.md](../ARCHITECTURE.md#collector-choice).

Nothing here depends on the agent's harness. The **agent process** is the program the supervisor started and anything it ran in its place. Every process it started is a **command**, and everything that command started belongs to it. A process that ran another program in its place (same pid) stays one row: "dpkg → dpkg-query".

## Layout

The list uses the full tab width. Selecting a row opens its **detail page** in place of the list; the list stays alive underneath, so going back returns to the same scroll position and selection.

```
[Commands|Files|Network|All events]  [⟲ Run 4, Nov 09, 09:12 to now ▾]   [Search commands, paths, addresses, domains]
(Show reads) (Failed or blocked only)
──────────────────────────────────────────────────────────────────────────────────────────────
12 commands  9 file changes  1,012 files read  21 connections  2 blocked    Monitoring complete for this run
──────────────────────────────────────────────────────────────────────────────────────────────
09:11:49.012  ▾ claude                                                               running
                462 files changed · 1,004 read · 17 connections
09:14:22.416  ▸ source …snapshot.sh … && eval 'sudo -n true; echo hi > /tmp/x; …'     exit 0
                bash -c · ran sudo, mv, rm · became root · 3 files changed
09:13:42.120  ▸ source …snapshot.sh … && eval 'curl -sI https://google.com …'         exit 0
                bash -c · ran curl, mv, rm · 1 connection
Sample data
```

Detail page (after clicking the 09:14:22 command):

```
← Commands   Run 4, Nov 09, 09:12 to now
──────────────────────────────────────────────────────────────────────────────────────────────
Command
bash -c source …snapshot.sh … && eval 'sudo -n true; echo hi > /tmp/x; mv /tmp/x /tmp/y; …'

Started      09:14:22.416 guest clock
Ended        09:14:22.433 guest clock
Duration     17 ms
Result       exit 0
Process      bash, pid 401
Started by   claude 336
Working dir. /home/ubuntu/workspace

What ran
┌────────────────────────────────────────────────────────────────────────────────────────┐
│ source /home/ubuntu/.claude/shell-snapshots/snapshot-bash-….sh 2>/dev/null || true &&  │
│ … eval 'sudo -n true; echo hi > /tmp/x; mv /tmp/x /tmp/y; rm /tmp/y' < /dev/null && …  │
└────────────────────────────────────────────────────────────────────────────────────────┘
▸ Arguments (3)

Privilege
  sudo 402 became root (euid 1000 → 0, capabilities raised)                              ›
  13 privilege changes
Changed files
  /tmp/x                                                                                 ›
  Created or written, renamed to /tmp/y · bash, mv
Processes started
  sudo -n true                                                                           ›
  exit 1 · became root
  mv /tmp/x /tmp/y                                                                       ›
  exit 0 · 1 file changed

┌ Environment variables and output aren't recorded. ┐
▸ Full record (12)
Sample data
```

## Toolbar

- Segmented control "Commands | Files | Network | All events". Switching views clears the selection.
- Run picker: neutral compact button with the history icon, "Run N, <start> to <end>" ("now" while it runs).
  - Its popover (`#1C2028`, 1 px `line2`, 10 px radius, 440 px) is headed "Agent runs" (`tx3` 12 px).
  - It lists the instance's agent runs newest first: the title, "Configuration revision N" in `tx3`, and "Running" in `ok`. The selected run is on `#232A38`.
  - Maintenance and provision runs aren't listed: nothing is observed there.
- Search field (320 px): "Search commands, paths, addresses, domains". Case-insensitive; it applies to the current view.
- Chips: "Show reads" (on by default) and "Failed or blocked only".

## Counts bar

As in [activity-timeline.md](activity-timeline.md#counts-bar). Counts are for the whole run, independent of filters.

## Commands view

- **Pinned first row:** the agent process, labeled with its command line. Expanded by default, it lists what the agent did itself: file changes one per row, reads grouped by folder, connections grouped by destination.
- **Then one row per command, newest first.** Collapsed by default; ▸/▾ in the caret column (14 px) expands it. Expanded, it shows the command's own effects and then the processes it started (indented 18 px per level, expanded by default).
- **Rows** are 44 px high, all alike. **First line:** time (monospace `tx3`, 100 px), then the command line in monospace.
  - For a shell started with `-c` (`sh`, `bash`, `dash`, `zsh`, `fish`, `ash`, `ksh`), the line is the script, whitespace collapsed.
  - Programs the process ran in its place follow with "→".
- **Row, second line (12 px):** "bash -c", then "ran git, mv, rm (+2)" (the distinct programs started below it). Then its effects:
  - "became root", "changed user" or "gained capabilities" (`warn`)
  - "couldn't run X" (`bad`)
  - "N files changed", "N read" (with Show reads)
  - "N connections", "N blocked" / "N failed to connect" (`bad`)

  The line takes the most severe tone.
- **Result** (220 px, right): "exit 0" (`tx2`), "exit N" or "signal N" (`bad`), "running" or "no exit recorded" (`tx3`).
- **Effect rows** under a process:
  - "couldn't run <path>" with the reason (`bad`).
  - One privilege row with the number of changes (result "Privilege" in `warn`).
  - Changed files, one per row with what happened in order ("Created or written, renamed to /tmp/y"), at most 30, then "and N more file changes".
  - Read folders ("/etc/", "25 files read", in `tx2`), at most 30.
  - Destinations ("160.79.104.10:443 tcp", the domain line, the outcome).
- **Activity no process can be linked to** (e.g. after an observer restart): one last row, "Activity without a known process".
- **Failed or blocked only** keeps commands with a non-zero exit, a refused start, or a failed or blocked connection anywhere below them.

## Files view

- **Area headings** (`panel` background, 13 px medium, expandable, expanded by default), each with "N files changed · N read":
  - Shared folders: files under a shared folder, with "on your Mac: <path>".
  - Home (`/home`, `/root`).
  - Temporary (`/tmp`, `/var/tmp`, `/dev/shm`).
  - System.
- **Under each heading:**
  - Changed files, newest first: path, what happened in order, and the processes (150 px column).
  - Then read folders, most files first (`tx2`), with "N files read" and the processes. Hidden when Show reads is off.
- Failed or blocked only shows nothing here: file events carry no failure.

## Detail page

- **Top bar** (44 px, 1 px `line` below): a ghost back button, "← Commands" (the view's name) from the first page and "← Back" from deeper pages, then the run's title in `tx3`. Back goes one page up; from the first page it returns to the list.
- **Content:** a scrolling column, 24 px padding, at most 880 px wide, left-aligned. Every wrapping text has an explicit width, so nothing overlaps and the page scrolls to its end.
- **Order:**
  1. Kind (`tx3` 12 px: "Command", "Agent process", "Process", "Destination", "DNS name", "File", "Files read") and a one-line headline (17 px medium, wraps).
  2. Facts: a 140 px label column in `tx3`, values in `tx` (monospace for paths and addresses).
  3. "What ran" (processes only): the shell script, or the command line, in a `well` block in monospace, wrapping anywhere. Below it, "Arguments (N)" collapsed (a numbered list when open) and "Truncated after 4 KB" in `warn` when cut.
  4. Summary sections, each with a 12 px medium `tx3` title, then items. An item is its text (monospace for paths, addresses and command lines) and a 12 px detail line in `tx2`, or in the item's tone. An item that has a page of its own shows "›" and a hover background, and opens on click.
  5. The note box (1 px `line2` border, 8 px radius).
  6. "Full record (N)" collapsed: every recorded event in order, with a 96 px monospace time column.

  The collapsed parts close again for each new page. The page refreshes with the run, keeping them as they are.
- **Command or process.**
  - Facts: Started, Ended, Duration, Result, Process, Started by, Then ran, Working directory, User, Program.
  - Sections cover the process and everything it started; for the agent process, only itself:
    - "Couldn't run" (`bad`).
    - "Privilege": one line per process, "sudo 402 became root (euid 1000 → 0, capabilities raised)", with the number of changes, in `warn`.
    - "Changed files": up to 50, what happened and by which processes.
    - "Network": one line per destination, the domain or the address, then "160.79.104.10:443 TCP · 17 connections · Connection succeeded · 07:54:35.428–07:55:05.457".
    - "DNS lookups while it ran": only when it sent DNS queries. Note: "Lookups can't be tied to a process exactly; these happened while it ran." Items are each name with its result ("Refused by policy" in `bad`).
    - "Files read" (with Show reads, by folder).
    - "Processes started": the tree below, indented 18 px per level, each with its exit and effects.
  - Note: "Environment variables and output aren't recorded."
- **File:** facts Area and On your Mac; "What happened, in order" with each access and its process.
- **Files read:** each file once with its process. Note: only the first open per process is recorded, and an open doesn't prove the content was read.
- **Destination and DNS name:** see [activity-network.md](activity-network.md#destination-page).

## Behavior

- **While the run goes,** the tab reads new activity every 2 seconds when visible; expansion and selection stay. A new run shows up in the picker and is followed when the newest run was shown.
- **Empty states:**
  - "No agent runs yet. Activity is recorded while the agent runs." (no tab content)
  - "No activity recorded in this run."
  - "No matching activity in this run." when filters hide everything.
- **Wording** follows DESIGN.md "Evidence wording": rows state what was recorded, never intent.
- **Not built yet:** copying, export, the live activity rail beside the terminal, bytes per connection, and a time range across runs.
