# Release Checklist

All crates share one version, set in `[workspace.package]` in the root
`Cargo.toml`, and we release them together, like egui. We usually do one release
per egui release, and maybe a patch release after it. A crate with no changes
still gets the new version.

**Pick the version by hand, and treat an egui update as breaking.** These crates
put egui types in their public API, so a crate that compiles against egui 0.37
is a breaking change for everyone still on 0.36, even when nothing else changed.
For a `0.x` version that means bumping the minor: 0.17.1 → 0.18.0. No tool
catches this: `cargo-semver-checks` reads this crate's own rustdoc, where
`egui::Ui` looks the same before and after.

## Published crates

* `crates/egui_tiles`
* `crates/egui_table`
* `crates/egui_mcp`

`crates/egui_table_demo` and `crates/kittest_inspector` are not published; both
set `publish = false`.

## Steps

Replace `0.x.0` with the new version.

We publish from the release branch, before we merge. If publishing fails, fix
it on the branch, so `main` never has a version that is not on crates.io.

* [ ] Make a branch called `release-0.x.0`
* [ ] Bump `version` in `[workspace.package]` and in the `egui_table` entry of `[workspace.dependencies]`, then run `cargo check`
* [ ] `uv run scripts/generate_changelog.py --version 0.x.0 --write`, then edit the changelogs by hand
* [ ] Open a PR titled `Release 0.x.0 - summary`, and wait for CI to pass. Do not merge it yet
* [ ] `cargo publish --quiet -p egui_tiles -p egui_table -p egui_mcp`
* [ ] `git tag -a 0.x.0 -m 'Release 0.x.0 - summary'` on the branch
* [ ] `git push --tags`
* [ ] Merge the PR
* [ ] Do a GitHub release: <https://github.com/rerun-io/egui_crates/releases/new>

