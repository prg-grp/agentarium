# Settings with pending changes

The Settings tab of a running instance after the user saved changes that apply on the next boot. Frame: [app-shell.md](app-shell.md). Section contents reuse the creation steps: [create-agent.md](create-agent.md), [create-shared-folders.md](create-shared-folders.md), [create-network.md](create-network.md), resources from [create-review.md](create-review.md).

## Layout

```
api-refactor                                                    [Restart in maintenance] [Stop] │ Delete
──────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings 2 pending
──────────────────────────────────────────────────────────────────────────────────────────────────
💾 Saved; restart required. The instance is running revision 7. Your 2 changes are in revision 8
   and apply on the next boot.                              [Revert to revision 7] [↻ Restart now]
──────────────────────────────────────────────────────────────────────────────────────────────────
Agent and launch     │ Domains not listed below
Shared folders       │ [Deny|Allow]  Unchanged
▌Network   2 changes │
Environment          │ ✓ Allowed
Resources            │ api.anthropic.com      Exact name                                 ×
                     │ github.com             Includes subdomains                         ×
                     │▌pypi.org               Includes subdomains  + Added, not active yet ×
                     │▌registry.npmjs.org     Exact name           − Removed, still active ↶
                     │ [Add a domain to allow]
                     │ ⊘ Denied
                     │ pastebin.com           Includes subdomains                         ×
```

## Pending-changes bar

- Shown under the tab bar whenever saved changes are not yet active. `acc` at 8% fill, floppy icon in `acc`.
- Copy: "Saved; restart required." in `tx`, then in `tx2`: "The instance is running revision 7. Your 2 changes are in revision 8 and apply on the next boot."
- Actions (compact 28 px): "Revert to revision 7" (ghost) and "Restart now" (neutral, refresh icon). "Restart now" reboots in agent mode.
- When the instance is stopped, the bar reads "Saved. Changes apply when you start the instance." with no restart action.
- The tab label shows "2 pending" while the bar is visible.

## Section navigation

- Left column, 220 px, 1 px `line` right border: Agent and launch, Shared folders, Network, Environment, Resources.
- Selected section: `raise` background, `tx` 500. Sections with saved but inactive changes show "<n> changes" in `acc` 12 px.

## Change marks inside sections

- Changed rows get a 2 px `acc` inset bar on the left and a short mark in `acc` 12 px:
  - Added: plus icon, "Added, not active yet".
  - Removed: domain struck through in `tx2`, minus icon, "Removed, still active", and an undo icon instead of remove.
  - Changed value (e.g. access level, scope, variable value): "Changed, not active yet" with the running value available on hover or in a second line ("Active now: Read only").
- Unchanged controls say nothing; for single-value controls like the default action, a quiet "Unchanged" in `tx2` is acceptable when the section has changes elsewhere.

## Behavior

- Edits save directly; there is no separate "Save" button per field. Validation follows the creation steps and blocks only invalid values.
- The effective configuration of every run is stored, so the Activity tab can show the policy that applied at the time (see [activity-network.md](activity-network.md)).
- Hot application of settings is deferred. Nothing in this screen suggests a change is already live.
