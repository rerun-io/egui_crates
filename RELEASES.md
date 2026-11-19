# Release Checklist

Each crate has its own version and its own `CHANGELOG.md`; there is no shared
version number. We release all published crates together, in one PR, usually
after an egui release. Leave out a crate that has no changes.

**Pick the version by hand, and treat an egui update as breaking.** These crates
put egui types in their public API, so a crate that compiles against egui 0.37
is a breaking change for everyone still on 0.36, even when nothing else changed.
For a `0.x` version that means bumping the minor: 0.17.1 → 0.18.0. No tool
catches this: `cargo-semver-checks` reads this crate's own rustdoc, where
`egui::Ui` looks the same before and after.

## Published crates

| crate | package | tag prefix |
| --- | --- | --- |
| `crates/egui_tiles` | `egui_tiles` | `egui_tiles-v` |
| `crates/egui_table` | `egui_table` | `egui_table-v` |
| `crates/egui_mcp` | `egui_mcp` | `egui_mcp-v` |

`crates/egui_table_demo` and `crates/kittest_inspector` are not published; both
set `publish = false`.

## Steps

Each crate gets its own new version, e.g. `egui_tiles 0.18.0`, `egui_table 0.11.0`
and `egui_mcp 0.3.0`. Below, `CRATE` is the package name and `0.x.0` its new version.

* [ ] Make a branch called `release-SUMMARY`, e.g. `release-egui-0.37`
* [ ] For each crate: `uv run scripts/generate_changelog.py --crate CRATE --version 0.x.0`, then paste the result into `crates/CRATE/CHANGELOG.md` and edit it by hand
* [ ] For each crate: bump `version` in `crates/CRATE/Cargo.toml`. Also bump the `egui_table` entry in `[workspace.dependencies]`. Run `cargo check`
* [ ] Open a PR titled `Release egui_tiles 0.x.0, egui_table 0.x.0, egui_mcp 0.x.0 - summary`, and wait for CI to pass. Do not merge it yet
* [ ] On the branch: `cargo publish --quiet -p egui_table -p egui_tiles -p egui_mcp`
* [ ] For each crate: `git tag -a CRATE-v0.x.0 -m 'Release CRATE 0.x.0 - summary'` on the published commit
* [ ] `git push origin egui_table-v0.x.0 egui_tiles-v0.x.0 egui_mcp-v0.x.0`
* [ ] Merge the PR
* [ ] For each tag, do a GitHub release with the crate's changelog section: <https://github.com/rerun-io/egui_crates/releases/new>
