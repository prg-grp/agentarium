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
- All sections sit on one scrolling page; clicking a section scrolls to it, and the selection follows the scroll position.
- Selected section: `raise` background, `tx`. Sections with saved but inactive changes show "<n> changes" in `acc` 12 px. In Network, the default action and the Mac and local network setting each count as one change; so does each rule or forwarded port added or removed, and a forwarded port switched between This Mac and Local network.
- The Network description follows the Mac and local network setting: "Applies to agent boots. Your Mac and local network stay unreachable, whatever the rules say." or "Applies to agent boots. Your Mac and local network follow the rules, and maintenance boots can reach them too."
- Agent and launch also holds the instance name. Renaming applies on save; the name isn't part of a revision.
- Resources: description "The virtual machine's processors, memory and disk. The instance takes memory and disk space from your Mac only as it uses them." The disk stepper starts at the disk's current size, since disks can grow but not shrink. A larger disk counts as one change and applies on the next boot, like processors and memory.
- There is no preset in Settings: the preset only fills in a new instance's first revision (see [create-harness.md](create-harness.md#presets)). Every setting, including the command and the network rules, is edited directly.

## Change marks inside sections

Not built yet; for now the bar, the tab label and the section navigation show the pending changes.

- Changed rows get a 2 px `acc` inset bar on the left and a short mark in `acc` 12 px:
  - Added: plus icon, "Added, not active yet".
  - Removed: domain struck through in `tx2`, minus icon, "Removed, still active", and an undo icon instead of remove.
  - Changed value (e.g. access level, scope, variable value): "Changed, not active yet" with the running value available on hover or in a second line ("Active now: Read only").
- Unchanged controls say nothing; for single-value controls like the default action, a quiet "Unchanged" in `tx2` is acceptable when the section has changes elsewhere.

## Behavior

- Settings is available from creation on, so settings can be changed before the first run. Until provisioning has succeeded, Agent and launch also shows the provisioning script, editable, with "Runs once as root with internet access before the first start. It can't be changed once provisioning has succeeded."; afterwards the script is gone from Settings. An empty script or the unchanged custom template allows Save but blocks "Save and start", with the reason in the footer.
- Before the first boot nothing is active yet, so saving shows no pending-changes bar.
- Edits are not saved automatically. A footer under the page holds "Discard changes" (ghost), "Save and restart" ("Save and start" while stopped; neutral) and "Save" (primary), enabled while the edits differ from the saved configuration, with "Unsaved changes" in `tx2` beside them.
- Save writes the edits into the pending revision, or a new one if the latest was booted, and shows the pending-changes bar. Save and restart also boots the instance in agent mode.
- Validation follows the creation steps: invalid values block saving, with "Fix the marked fields to save." in `bad`. A shared folder that no longer exists allows Save but blocks Save and restart, with the reason in the footer.
- Selecting another instance drops unsaved edits; switching tabs keeps them.
- "Revert to revision 7" is not built yet.
- The effective configuration of every run is stored, so the Activity tab can show the policy that applied at the time (see [activity-network.md](activity-network.md)).
- Hot application of settings is deferred. Nothing in this screen suggests a change is already live.
