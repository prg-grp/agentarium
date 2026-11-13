# Snapshots tab

Snapshots of an instance's disk, each with the configuration revision that was the latest when it was taken. Restoring one brings back both. Frame: [app-shell.md](app-shell.md); the tab sits between Logs and Settings.

## Layout

```
Agent  Terminal  Activity  Status  Logs  Snapshots  Settings
──────────────────────────────────────────────────────────────────────────────────────────
Snapshots
A snapshot keeps the instance's disk and its configuration. Restoring one replaces both;
shared folders on your Mac aren't part of it.
[ Snapshot 3                    ] [+ Create snapshot]
──────────────────────────────────────────────────────────────────────────────────────────
before-upgrade                                                      [⟳ Restore] [🗑 Delete]
Nov 13, 2026, 14:03 · revision 4
──────────────────────────────────────────────────────────────────────────────────────────
Freshly provisioned          Replace the current disk and settings with this snapshot?
Nov 12, 2026, 09:41 · revision 1                                   [Cancel] [⟳ Restore]
──────────────────────────────────────────────────────────────────────────────────────────
```

- Content scrolls, 28 px side padding. The top is a Settings group: "Snapshots" (15 / 500) and the description in `tx3` 13 px.
- Create row: a 320 px name field, prefilled with "Snapshot N" (one more than the count), and "Create snapshot" (neutral, plus icon). Return in the field creates too.
- Rows, newest first, separated by 1 px `line`: the name in `tx`, below it the creation date and the linked revision in `tx3` 12 px ("Nov 13, 2026, 14:03 · revision 4"). Actions on the right: "Restore" (ghost, compact, refresh icon) and "Delete" (danger ghost, compact, trash icon).
- Confirmation is inline: the row's actions become a `tx2` sentence, "Cancel" (ghost) and the confirm button: "Restore" (neutral) or "Delete" (danger-confirm). Copy: "Replace the current disk and settings with this snapshot?" / "Delete this snapshot? This can't be undone."
- Empty: "No snapshots yet." in `tx3` below the line.
- Errors show below the create row with an alert icon in `bad`: "Couldn't create the snapshot: …", "Couldn't restore the snapshot: …", "Couldn't delete the snapshot: …".

## States

- Creating and restoring need the instance stopped (no start, stop or restart in progress) and provisioned. Otherwise the field is read-only, the buttons are disabled, and a `tx3` line says why: "Stop the instance to create or restore snapshots." or "Snapshots are available once the instance is provisioned."
- Deleting works in every state; it doesn't touch the instance's disk.

## Behavior

- A snapshot is an APFS clone of the disk: instant, and it takes space on the Mac only as the instance's disk and the snapshot drift apart.
- It links the latest saved revision, which then stays as it is: the next save in Settings goes into a new revision.
- Restoring replaces the disk with a clone of the snapshot and saves the linked revision's configuration as the instance's configuration for the next boot. Settings shows it right away; unsaved edits there are dropped. The instance's name and its other snapshots stay.
- After a successful provisioning run, a snapshot named "Freshly provisioned" is taken before the agent starts, unless it is turned off in [preferences.md](preferences.md). A failed snapshot doesn't keep the agent from starting.
- Deleting the instance deletes its snapshots ([delete-confirmation.md](delete-confirmation.md)).
