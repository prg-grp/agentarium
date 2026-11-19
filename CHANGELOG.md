# Changelog

## [1.1.0](https://github.com/prg-grp/agentarium/compare/v1.0.0...v1.1.0) (2026-11-19)


### Features

* add feedback button to preferences ([07c095b](https://github.com/prg-grp/agentarium/commit/07c095be2bcb3b8499541cd47c7239d98663d4e7)), closes [#23](https://github.com/prg-grp/agentarium/issues/23)
* add startup and teardown scripts around the agent ([14d48e3](https://github.com/prg-grp/agentarium/commit/14d48e394fab74fe3b76a5427452ecd4c6945fa4)), closes [#21](https://github.com/prg-grp/agentarium/issues/21)
* duplicate stopped instances from the sidebar ([cf4025e](https://github.com/prg-grp/agentarium/commit/cf4025e3bf178c744143d5580db244b27e7dc2b7)), closes [#12](https://github.com/prg-grp/agentarium/issues/12)
* keep the Mac awake while instances run ([78f0c92](https://github.com/prg-grp/agentarium/commit/78f0c92f1894f4aa803510536c219ec21a37be5a)), closes [#16](https://github.com/prg-grp/agentarium/issues/16)
* remember the selected tab per instance ([8ce2d62](https://github.com/prg-grp/agentarium/commit/8ce2d62063229d0d9e0e4ae11ff20654119e2f59)), closes [#18](https://github.com/prg-grp/agentarium/issues/18)

## 1.0.0 (2026-11-16)


### Features

* add presets for Claude Code, Codex, Gemini CLI and OpenCode ([8834c7c](https://github.com/prg-grp/agentarium/commit/8834c7ca13a2d310cd74d654b3c551b133b7db9b))
* add terminal scrollbars ([ac6aa0a](https://github.com/prg-grp/agentarium/commit/ac6aa0a44b52182196cc54bc46e1bc2a730acf87))
* add the Activity tab with timeline, commands and network views ([926adec](https://github.com/prg-grp/agentarium/commit/926adec8d7dd75139a940064f6b741f2fade2300))
* add the desktop app shell ([a38358e](https://github.com/prg-grp/agentarium/commit/a38358eb10479e09c53856dd29385eaed5b0a02b))
* add the Status tab with live reports ([7d1599a](https://github.com/prg-grp/agentarium/commit/7d1599ab4cbdc667e139c3e738bfd89891ad5dbf))
* **app:** add a shell beside the agent in a Terminal tab ([0d803a5](https://github.com/prg-grp/agentarium/commit/0d803a5c0e1e33cb51046292010dd135e405b6c5))
* **app:** add Antigravity, Copilot, Hermes and Agent Canvas presets ([65430ef](https://github.com/prg-grp/agentarium/commit/65430efb49112d36b47b47b4d947c92f9d05723c))
* **app:** add context menu to sidebar instances ([4eb42ac](https://github.com/prg-grp/agentarium/commit/4eb42ac895e53029f985ae6b2f31bf65fe01bec0))
* **app:** add instance defaults, sidebar ordering and autostart ([8de681c](https://github.com/prg-grp/agentarium/commit/8de681c9e24307e550673f138b1b802d69d74080))
* **app:** add snapshots of stopped instances ([d6dc5ee](https://github.com/prg-grp/agentarium/commit/d6dc5ee4aedbc6250763a9ac9ad563e8771634c5))
* **app:** allow a separate data folder for development builds ([f8e31a2](https://github.com/prg-grp/agentarium/commit/f8e31a2daa2cd0be1e9810c1dc73a5aeaebfd8be))
* **app:** allow editing preset provisioning scripts ([ab735b1](https://github.com/prg-grp/agentarium/commit/ab735b116a7a949fe071c83c4fb6258e07683329))
* **app:** allow setting revision switches for instances ([4156695](https://github.com/prg-grp/agentarium/commit/4156695ccbb945f05ec115634c42bdd325f48975))
* **app:** apply settings to running instances on save ([21eb472](https://github.com/prg-grp/agentarium/commit/21eb47297df3ae5e4598e75c077227be4c354ea3))
* **app:** ask before downloading the guest image ([30f659d](https://github.com/prg-grp/agentarium/commit/30f659d09c4bfb13f7291bbcb5813184878085a0))
* **app:** encrypt environment variables at rest ([4110195](https://github.com/prg-grp/agentarium/commit/4110195ea4f6fdafe96848239cee9e1324b6e0d6))
* **app:** explain the VM, the workspace and shared folders ([51fbf61](https://github.com/prg-grp/agentarium/commit/51fbf6169faa390348d2833162ed7f8f085c4d78))
* **app:** forward ports from the Mac into instances ([bf34a1a](https://github.com/prg-grp/agentarium/commit/bf34a1acd2865b26a7a987c85645c90e5ba215d2))
* **app:** per-instance disk size with a small base rootfs ([6f801c0](https://github.com/prg-grp/agentarium/commit/6f801c070a78661ff4e0d9355d82a0c26c5d5d9d))
* **app:** reorder and edit launch arguments in place ([fd57ff8](https://github.com/prg-grp/agentarium/commit/fd57ff8b19ba8fe6e468562a71c8548ecad484f6))
* **app:** share the guest supervisor and observer read-only with every boot ([bb55dbc](https://github.com/prg-grp/agentarium/commit/bb55dbc2e66310414c1e466df6cc164f171193d0))
* **app:** show third-party licenses in Preferences ([54dfa90](https://github.com/prg-grp/agentarium/commit/54dfa906be17d5bd46411f01dfd118832269c5ca))
* boot Linux microVMs with an interactive terminal ([37fad69](https://github.com/prg-grp/agentarium/commit/37fad69bdb377983fe33d166b5c3df9091d70cee))
* control guests over a vsock command plane ([9520b60](https://github.com/prg-grp/agentarium/commit/9520b60309af6a3446cba0eada8c5fb02d60b99d))
* edit instance settings and apply them on restart ([aa6a7c1](https://github.com/prg-grp/agentarium/commit/aa6a7c16bc4e396201ca7e990fa1ed0715386823))
* enforce domain allow and deny network policies ([cd981c7](https://github.com/prg-grp/agentarium/commit/cd981c7ecbf8f3824084b304d7ce1e422ed9ce66))
* give guests internet access through a userspace network stack ([19917e8](https://github.com/prg-grp/agentarium/commit/19917e87bc9e5d46a4009dde87c4063d4d89d446))
* keep the guest clock in sync with the host ([23b3190](https://github.com/prg-grp/agentarium/commit/23b3190dde6246ee93f7142cfd19eed93df58111))
* mount host folders into instances ([4343818](https://github.com/prg-grp/agentarium/commit/434381847e24240cb4ef0785878705c84f9bbdfd))
* optionally let guests reach the local network ([48be0b2](https://github.com/prg-grp/agentarium/commit/48be0b2136c832f73a612e6f1f0d885a0d6abb36))
* preinstall Claude Code in the guest image ([f73ff9d](https://github.com/prg-grp/agentarium/commit/f73ff9d40b0eeb13d9fa48a8e99132c7efdac829))
* record guest activity with an eBPF observer ([279e544](https://github.com/prg-grp/agentarium/commit/279e544a7f507680be9749f1cab472d25162f69c))
* run multiple instances side by side ([4b96fc2](https://github.com/prg-grp/agentarium/commit/4b96fc2278bd10032cd8fe90571872e633deb4fa))
* show boot output in a separate Logs tab ([73e0558](https://github.com/prg-grp/agentarium/commit/73e05582ffaf6b9a03bbf0e7ca4dce8a032d0456))
* show provisioning progress and delete instances ([17c095a](https://github.com/prg-grp/agentarium/commit/17c095a19e0aec24c0bd23b116020b4b5e2d5ce8))
* shut instances down gracefully ([8d99b82](https://github.com/prg-grp/agentarium/commit/8d99b82ac81a75a049e7cae375ecb80434261e7f))
* store instances in SQLite with copy-on-write disks ([ccf0453](https://github.com/prg-grp/agentarium/commit/ccf0453c7c07a007763b3d916afcb14fbe9ba7e0))
* update Agentarium in place and download the guest image from its release ([7a51166](https://github.com/prg-grp/agentarium/commit/7a511668601dc68aa4c7f58fd40d0c49b01a58c3))


### Bug Fixes

* **app:** follow terminal links broken across lines ([f63cc6f](https://github.com/prg-grp/agentarium/commit/f63cc6f99a36f2430d2491fae441ac92ca2f9eb7))
* **app:** ignore lone modifier keys in the terminal ([dbf8b20](https://github.com/prg-grp/agentarium/commit/dbf8b20e4291420a894c2e4714fe50f1909e0d5a))
* **app:** keep the guest clock on the Mac's time ([80e337b](https://github.com/prg-grp/agentarium/commit/80e337b28bc3484508549ece5c6a98d9cfbb29b7))
* **app:** send the mouse wheel to full-screen apps ([fb12789](https://github.com/prg-grp/agentarium/commit/fb12789b513cc39ec094426823ef01f9e6768172))
* **claude:** add downloads.claude.ai to default setup allowance list ([3362674](https://github.com/prg-grp/agentarium/commit/3362674e2d21e7d7c694b2854de51647de6d0cb5))
