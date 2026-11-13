# Delete confirmation

The dialog shown from the header's "Delete" action. Uses the same dialog layout as [maintenance-confirmation.md](maintenance-confirmation.md).

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│ Delete api-refactor?                                         │
│ The instance is running and will be shut down first. This    │
│ can't be undone.                                             │
│ Removed                                                      │
│ ──────────────────────────────────────────────────────────── │
│ 🗄 Virtual machine disk, 18.4 GB, including ~/workspace and   │
│    software installed in maintenance                         │
│ ⚙  Configuration and its 7 revisions                         │
│ 💾 2 snapshots                                                │
│ ⟲  Activity history from 23 runs                             │
│ Stays on your Mac                                            │
│ ──────────────────────────────────────────────────────────── │
│ 📁 ~/code/api                                                 │
│    Shared folders and their files aren't touched.            │
│ ⚿  Keychain items, which you can remove in Keychain Access   │
│                               [Cancel] [🗑 Delete instance]   │
└──────────────────────────────────────────────────────────────┘
```

## Elements

- Title "Delete <name>?" 17 / 500.
- Intro in `tx2`. Running instance: "The instance is running and will be shut down first. This can't be undone." Stopped instance: "This can't be undone."
- Two labeled groups ("Removed", "Stays on your Mac") in `tx3` 12 px.
- Removed items use `bad` icons and give concrete sizes and counts: disk size, number of configuration revisions, number of snapshots (only when there are any), number of runs.
- Stays items use `tx2` icons and list every shared folder by host path.
- Actions: "Cancel" (ghost) and "Delete instance" (danger-confirm: `bad` fill, text `#2B0C0E`, trash icon). Escape cancels; Return does not confirm by default.

## Behavior

- After deletion, select the next instance in the sidebar, or show [empty-state.md](empty-state.md) if none remain.
- If deletion partly fails (for example the disk file can't be removed), say which part remains and where.
