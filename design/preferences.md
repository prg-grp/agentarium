# Preferences

App-wide settings: updates of Agentarium itself, the guest image, and its licenses. Frame: [app-shell.md](app-shell.md) sidebar; the main area shows this view instead of an instance, with no instance header or tab bar.

## Layout

```
┌──────────────────┬──────────────────────────────────────────────────────────────────────┐
│ ● ● ●            │ Preferences                                                          │
│ Instances    [+] ├──────────────────────────────────────────────────────────────────────┤
│  ● api-refactor  │ Updates                                                              │
│    Running       │ Agentarium checks for new versions every few hours and downloads     │
│                  │ them in the background.                                              │
│                  │ Version     1.2.0-dev.57                                             │
│                  │ Channel     [ Stable | Dev builds ]                                  │
│                  │             Every change to Agentarium as soon as it's built, …      │
│                  │ Status      ✓ Version 1.2.0 is ready to install.                     │
│                  │ [⟳ Restart to update]  Running instances shut down first.            │
│                  │ What's new in 1.2.0                                                  │
│                  │ ### Features …                                                       │
│                  ├──────────────────────────────────────────────────────────────────────┤
│                  │ Guest image                                                          │
│                  │ The Linux kernel and root filesystem that new instances start from.  │
│                  │ Source      From release v1.2.0                                      │
│                  │ Status      Ready                                                    │
│                  ├──────────────────────────────────────────────────────────────────────┤
│                  │ About                                                                │
│                  │ Agentarium is free software under the GNU Affero General Public …    │
│                  │ License     AGPL-3.0-or-later                                        │
│                  │ [Third-party licenses]                                               │
│ ⚙ Preferences  Update ready                                                             │
└──────────────────┴──────────────────────────────────────────────────────────────────────┘
```

- Header: "Preferences" (22 / 28, 500), padding 26 px vertical, 28 px horizontal, 1 px `line` below.
- Sections are the Settings tab's groups (title 15 / 500, description 13 `tx3`, 22 px vertical padding, `line` between groups), in a scroll area with 28 px side padding.
- Rows: label column 120 px in `tx2` 13 px, then the value or control. The version is monospace.
- Channel: segmented control "Stable" / "Dev builds", with one line below in `tx3` describing the selected channel.
- Status: `tx2` text while checking, downloading or up to date; a check icon and `ok` text when an update is ready; an alert icon and `bad` text when a check or download failed. While downloading, a 6 px progress track (`raise`, filled `tx2`) sits below the text.
- Actions: "Restart to update" (primary, refresh icon) only when an update is ready, with "Running instances shut down first." in `tx3`; otherwise "Check now" (neutral), disabled while a check or download runs. Neither shows in development builds.
- About: the License row shows `AGPL-3.0-or-later` in monospace; below, "Third-party licenses" (neutral) opens [licenses.md](licenses.md) in its own window.
- "What's new in X" (`tx` / 500) and the release notes (`tx2`, 13 px, plain text) show while the update downloads and once it's ready.

## Sidebar

- The Preferences row (48 px, `line` top border) opens this view. While the view shows, the row has the `raise` background and `tx` / 500 text, and no instance item shows as selected.
- When an update is ready, the row shows "Update ready" in `acc` 12 px at its right end.

## Copy

- Updates description: "Agentarium checks for new versions every few hours and downloads them in the background."
- Channel lines: "Releases only." / "Every change to Agentarium as soon as it's built, and every release. Dev builds can have rough edges."
- Status: "Checking for updates…", "Agentarium is up to date.", "Downloading version X · 42%", "Version X is ready to install.", "Couldn't check for updates: …", "Couldn't download the update: …", "Updates work only in installed builds.", "Shutting instances down to restart…"
- Guest image description: "The Linux kernel and root filesystem that new instances start from. Existing instances keep the one they were created with."
- About description: "Agentarium is free software under the GNU Affero General Public License v3.0 or later. It's built on open-source crates, each under its own license."
- Guest source: "From release vX.Y.Z" or "Built locally with make guest".

## Behavior

- The app checks at launch and every 4 hours. An update is downloaded right away (as a delta when possible); nothing installs until the user restarts. An update that was downloaded but not installed is applied at the next launch.
- "Restart to update" stops every running instance gracefully, like quitting, then installs and relaunches.
- Switching the channel saves it and checks again; an update found for the other channel is dropped. Dev builds also receive stable releases once those are newer. Switching to Stable never downgrades: the install stays on its version until a newer stable release exists.
- Until the guest image is ready, [guest-download.md](guest-download.md) blocks the whole app, so this view only ever shows the image as ready.
