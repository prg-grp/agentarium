# Create: shared folders (step 3 of 5)

Third creation step. Flow rules (draft in sidebar, step indicator, footer) are in [create-harness.md](create-harness.md#creation-flow-shared-by-all-five-steps). The same table is reused in Settings ([settings-pending.md](settings-pending.md)).

## Layout

```
Shared folders                                                     [Add folder]
Optional. The agent works in ~/workspace on the instance's own disk. Each folder you
share appears at the path you set, and the agent works on your Mac's files there directly.
Nothing else on your Mac is shared. Up to 16 folders: each gets a virtual device of its
own, and a virtual machine's devices are fixed when it starts.

On your Mac               Inside the instance        Access
──────────────────────────────────────────────────────────────────────────────
~/code/billing-service    [~/workspace/billing-service]  [Read only|Read and write]  ×
⚠ Changes inside this instance modify these host files immediately.
──────────────────────────────────────────────────────────────────────────────
~/code/shared-protos      [~/workspace/shared-protos]    [Read only|Read and write]  ×
──────────────────────────────────────────────────────────────────────────────
~/code/legacy-billing     [~/workspace/legacy-billing]   [Read only|Read and write]  ×
(struck through)
⊘ This folder no longer exists on your Mac. Choose another folder or remove it. Choose folder

Inside the instance
~/workspace                Instance disk, kept across restarts
  billing-service          ← ~/code/billing-service on your Mac, read and write
  shared-protos            ← ~/code/shared-protos on your Mac, read only
A shared folder hides anything the instance already has at that path while it is shared.
                                                                               2/16 folders
──────────────────────────────────────────────────────────────────────────────
Cancel                                          [< Back]  [Continue to network]
```

## Elements

- Title "Shared folders" (16 / 500) and the description in `tx2`. The description says the step is optional, where the agent works without folders, and that a shared folder is the Mac's folder itself, not a copy.
- "Add folder" (neutral, folder-plus icon) opens the macOS folder picker. It is disabled once 16 folders are listed (`SHARE_SLOTS`): each shared folder gets a virtio-fs device of its own, and a VM's devices are fixed at boot. The description ends with the one-sentence reason: "Up to 16 folders: each gets a virtual device of its own, and a virtual machine's devices are fixed when it starts."
- Below the table and the "Inside the instance" lines, always: "<n>/16 folders" in `tx3` 12 px, right-aligned. The same line shows in the Settings tab.
- Without folders, a `tx3` line says "No folders are shared. That's fine if you don't need the results on your Mac right away: the agent's work stays on the instance's disk, and you can share a folder later."
- **Inside the instance** (always shown below the table, also without folders): what the agent sees, built from the rows.
  - Header "Inside the instance" in `tx3` 12 px. One line per entry: the path in monospace in a 280 px column, then the source in 13 px.
  - Entries: `~/workspace` with "Instance disk, kept across restarts" in `tx3`, and each folder whose guest path is valid with "← <Mac path> on your Mac, read only / read and write" in `tx2`. Rows with a guest path error are left out. A folder shared at `~/workspace` itself replaces the disk line.
  - Sorted by path. A folder inside another entry follows it, indented 16 px, with its path relative to that entry. Top-level paths are absolute, with `/home/ubuntu` shown as `~`.
  - A `tx3` hint below: "A shared folder hides anything the instance already has at that path while it is shared." Folders are mounted at their exact path on every boot, over whatever the instance's disk has there.
- Table columns: On your Mac (monospace, wraps anywhere), Inside the instance (editable monospace field on `well`), Access (segmented control), remove (icon-only ghost, labeled "Remove folder"). Column header row in `tx3` 12 px.
- Guest path default: `/home/ubuntu/workspace/<folder name>`, inside the agent user's `~/workspace` (the default working directory). If another folder already uses that path, a number is added (`-2`, `-3`, …). The stored path is always absolute. Keep host-to-guest mapping stable for activity display.
- Access segmented control labels: "Read only" and "Read and write". New folders default to Read only.

## States

- **Read and write:** a line under the row in `warn` 13 px with an alert icon: "Changes inside this instance modify these host files immediately."
- **Missing or inaccessible folder:** host path in `tx2` with a `tx3` strike-through; guest field in `tx2`; a line under the row in `bad` with a folder-off icon: "This folder no longer exists on your Mac. Choose another folder or remove it." followed by a "Choose folder" link in `acc`. For access failures: "The app can't read this folder. Allow access in System Settings, or choose another folder."
- **Duplicate guest path:** error below the second row: "Another folder already uses this path inside the instance."
- **Invalid guest path** (not absolute, `/` itself, or containing `..`): error below the row: "Enter an absolute path inside the instance, like /home/ubuntu/workspace/app."
- Continue stays enabled; blocking errors are listed again on Review and prevent start, not navigation.

## Not offered

- No copy/diff or "protected" mode. Do not show it as an option, even disabled.
- No implicit sharing of the home folder, credential stores, or unrelated projects.
