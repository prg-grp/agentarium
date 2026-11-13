# Settings

The Settings tab of an instance: its configuration, edited and saved explicitly, and applied to the running instance on save where it can be. Frame: [app-shell.md](app-shell.md). Section contents reuse the creation steps: [create-agent.md](create-agent.md), [create-shared-folders.md](create-shared-folders.md), [create-network.md](create-network.md), resources from [create-review.md](create-review.md).

## Layout

```
api-refactor                                                    [Restart in maintenance] [Stop] │ Delete
──────────────────────────────────────────────────────────────────────────────────────────────────
Terminal   Logs   Status   Activity   Settings
──────────────────────────────────────────────────────────────────────────────────────────────────
Agent and launch     │ Domains not listed below
Shared folders       │ [Deny|Allow]
▌Network             │
Environment          │ ✓ Allowed
Resources            │ api.anthropic.com      Exact name                                 ×
                     │ github.com             Includes subdomains                         ×
                     │ [Add a domain to allow]
                     │ ⊘ Denied
                     │ pastebin.com           Includes subdomains                         ×
──────────────────────────────────────────────────────────────────────────────────────────────────
[Discard changes]                          Saved and applied to the running instance.   [Save]
```

There is no pending-changes bar, tab label or per-section change count: Save puts everything into effect itself (applying, restarting the agent or rebooting), and the footer says what it did.

## Section navigation

- Left column, 220 px, 1 px `line` right border: Agent and launch, Shared folders, Network, Environment, Resources.
- All sections sit on one scrolling page; clicking a section scrolls to it, and the selection follows the scroll position.
- Selected section: `raise` background, `tx`.
- The Network description follows the Mac and local network setting: "Applies to agent boots, to new connections as soon as you save. Your Mac and local network stay unreachable, whatever the rules say." or "Applies to agent boots, to new connections as soon as you save. Your Mac and local network follow the rules, and maintenance boots can reach them too."
- Agent and launch also holds the instance name. Renaming applies on save; the name isn't part of a revision.
- Resources: description "The virtual machine's processors, memory and disk. The instance takes memory and disk space from your Mac only as it uses them." The disk stepper starts at the disk's current size, since disks can grow but not shrink. A larger disk applies on the next boot, like processors and memory; these are the only settings that need a reboot.
- Shared folders: the folder table ends with "<n>/16 folders" (see [create-shared-folders.md](create-shared-folders.md)); "Add folder" is disabled at 16.
- There is no preset in Settings: the preset only fills in a new instance's first revision (see [create-harness.md](create-harness.md#presets)). Every setting, including the command and the network rules, is edited directly.

## Behavior

- Settings is available from creation on, so settings can be changed before the first run. Until provisioning has succeeded, Agent and launch also shows the provisioning script, editable, with "Runs once as root with internet access before the first start. It can't be changed once provisioning has succeeded."; afterwards the script is gone from Settings. An empty script or the unchanged custom template allows Save, with the reason in the footer.
- Edits are not saved automatically. A footer under the page holds "Discard changes" (ghost) and one primary save button, enabled while the edits differ from the saved configuration, with "Unsaved changes" in `tx2` beside them. Its label says what saving does to a running agent or maintenance boot, compared with the configuration in effect:
  - "Save": everything edited applies at once, or the instance isn't running (or is still starting or provisioning), so the edits are only saved.
  - "Save and restart agent" (refresh icon): the edits include launch settings or environment variables; the agent restarts with them, the VM keeps running.
  - "Save and reboot" (refresh icon): the edits include processors, memory or disk size; the instance reboots in agent mode.
- Save writes the edits into the pending revision, or a new one if the latest was booted. While an agent or maintenance boot runs, Save also applies them to it:
  - **Live:** the network rules and settings (new connections and lookups; open connections stay), the shared folders (mounted or unmounted in the guest right away), and the forwarded ports.
  - **At the agent's next start:** the launch settings and environment variables. New Terminal shells get the new environment right away.
  - **At the next boot:** processors, memory and disk size.

  The revision then counts as the one the instance runs.
- After a save, the footer says what happened, in `tx2`, until the next edit:
  - "Saved and applied to the running instance."
  - "Saved and applied; the agent is restarting." (Save and restart agent)
  - "Saved; the instance is rebooting." (Save and reboot)
  - "Saved. Changes apply on the next boot." (the instance was starting, provisioning or stopping)
  - "Saved. Changes apply when you start the instance." (stopped)
  - "Saved." (only the name changed)
- Settings that couldn't be applied, such as a shared folder that is gone from the Mac or didn't mount, replace that line in `bad`: "Not applied: <reasons>". It clears with the next applied save and when the VM stops.
- "Save and restart agent" and "Save and reboot" are disabled while a start, stop or restart is in progress.
- Validation follows the creation steps: invalid values block saving, with "Fix the marked fields to save." in `bad`. A shared folder that no longer exists allows Save (it isn't shared until it is back) but blocks "Save and reboot", with the reason in the footer.
- Selecting another instance drops unsaved edits; switching tabs keeps them.
- The configuration each boot started with is stored, along with the revisions applied while it ran. When during a boot each revision applied isn't recorded.
