# Third-party licenses

A separate window that lists the licenses of the crates Agentarium is built from. Opened by "Third-party licenses" in the About group of [preferences.md](preferences.md). The app's own crates are not listed. The data comes from `cargo about` (about.hbs generates `app/src/licenses.rs` at build time).

## Layout

```
┌──────────────────────────────────────────────────────────────────────────────────────┐
│ ● ● ●                                                                                │
│ Third-party licenses                                          [ Search crates      ] │
├──────────────────────┬──────────────────────────┬────────────────────────────────────┤
│ Apache License 2.0 451│ aho-corasick       1.1.3 │ anyhow 1.0.98            Repository│
│ MIT License        128│▌anyhow             1.0.98│ Apache License 2.0 · Apache-2.0    │
│ Unicode License v3  26│ arboard            3.6.1 │ ┌────────────────────────────────┐ │
│ ISC License         24│ …                        │ │ Apache License                 │ │
│ …                     │                          │ │ Version 2.0, January 2004 …    │ │
└──────────────────────┴──────────────────────────┴────────────────────────────────────┘
```

- Window: `bg`, minimum 900 × 560, preferred 1100 × 720. The title bar is transparent like the main window; the top 28 px move the window.
- Header: "Third-party licenses" (17 / 500) left, search field (260 px, placeholder "Search crates") right; padding 40 px top (below the window buttons), 18 px bottom, 28 px sides; 1 px `line` below.
- License types (sidebar width, `panel`, `line` right border): one row per license type with its name and the number of crates (`tx3`, 12 px) on the right, sorted by count, most first. Rows use the sidebar item style: 8 px radius, selected `raise` with `tx` / 500 text, others `tx2`.
- Crates (300 px, `line` right border): the selected type's crates, sorted by name. Name in `tx`, version monospace 12 px `tx3`. Table row style: 1 px `line` separators, selected `#1B2130` with a 2 px `acc` bar on the left.
- License (rest, 24 px padding): crate name (15 / 500) and version (monospace `tx3`), a "Repository" link at the right end; below, "License name · SPDX id" in `tx2` 13 px. The license text fills the rest: a `well` block (7 px radius, `line` border, 16 px padding), monospace 12.5 px `tx2`, wrapped, scrolling on its own.

## Behavior

- The window opens with the first license type and its first crate selected. Closing it only hides it; opening it again keeps the selection and search.
- Selecting a license type selects its first crate. A newly selected crate's text starts scrolled to the top.
- The search matches crate names (case-insensitive, substring). License types without matches disappear, the counts show matches only. If the selected type has no matches, the first type with matches is selected.
- "Repository" opens the crate's repository in the default browser, or its crates.io page when it names none.
- Without matches, columns 2 and 3 are replaced by: "No crates match “…”." in `tx3` 13 px.
