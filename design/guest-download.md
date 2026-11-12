# Guest image download

A blocking dialog over the whole window, shown at launch when this build's guest kernel or root filesystem isn't on the Mac yet: on first launch, and after an app update whose release ships a new guest image. Nothing in the app works without these files, so the dialog can't be dismissed. Uses the same dialog layout as [delete-confirmation.md](delete-confirmation.md). Development builds never show it.

## Layout

```
┌──────────────────────────────────────────────────────────────┐
│ Agentarium needs two files to get started                    │
│ Agentarium runs every agent in its own small virtual         │
│ computer, kept apart from your Mac. To build these           │
│ computers, it needs the two files below. They download once  │
│ and stay on your Mac.                                        │
│ Needed                                                       │
│ 🗄 Linux kernel: the core of the operating system that starts │
│    each virtual computer.                                    │
│ ──────────────────────────────────────────────────────────── │
│ 📦 Root filesystem: the base system that every new instance   │
│    starts from, like a fresh install.                        │
│ The download is a few hundred megabytes and can take a few   │
│ minutes. Agentarium can't be used until it's finished.       │
│                                  [Quit] [⤓ Start download]   │
└──────────────────────────────────────────────────────────────┘
```

While downloading, the note is replaced by:

```
│ Downloading root filesystem                    230 of 640 MB │
│ ███████████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ │
│                                    [Quit] [⤓ Downloading…]   │
```

## Elements

- Scrim: black at 45% over the whole window; clicks don't reach the app below. The top 28 px still drag the window.
- Panel: 520 px wide, `panel` background, 12 px radius, `line2` border, 24 px padding.
- Title 17 / 500. Intro in `tx2`.
- "Needed" label in `tx3` 12 px, then the two files as icon list items with `tx2` icons (server for the kernel, box for the root filesystem), split by a `line` divider.
- Before the download: the note in `tx3` 13 px.
- Downloading: the step ("Downloading kernel", then "Downloading root filesystem") in `tx` 13 px with the size ("230 of 640 MB") in `tx3` 12 px at the right, and a 6 px progress track (`raise`, filled `tx2`) below.
- Failed: the reason with an alert icon in `bad`.
- Actions: "Quit" (ghost, always enabled) and one primary button: "Start download" (download icon), "Downloading…" (disabled) while downloading, "Try again" (refresh icon) after a failure. Escape does nothing.

## Copy

- First launch title: "Agentarium needs two files to get started"
- First launch intro: "Agentarium runs every agent in its own small virtual computer, kept apart from your Mac. To build these computers, it needs the two files below. They download once and stay on your Mac."
- Update title (the database already has instances): "This update needs new system files"
- Update intro: "This version of Agentarium comes with an improved virtual computer for new instances. Your existing instances and their settings stay as they are."
- Note: "The download is a few hundred megabytes and can take a few minutes. Agentarium can't be used until it's finished."

## Behavior

- The download starts only when the user clicks "Start download". Only the missing files are fetched; both are listed either way.
- The dialog closes once both files are downloaded and their hashes match.
- "Quit" quits the app right away; an unfinished download is thrown away and cleaned up later.
- Existing instances keep the kernel they were created with; their files aren't touched.
