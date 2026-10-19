# Release Checklist

Each crate has its own version and its own `CHANGELOG.md`; there is no shared
version number. Release one crate at a time.

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

Replace `CRATE` with the package name and `0.x.0` with the new version.

* [ ] Update `crates/CRATE/CHANGELOG.md` using `uv run scripts/generate_changelog.py --crate CRATE --version 0.x.0`
* [ ] Bump `version` in `crates/CRATE/Cargo.toml` and run `cargo check`
* [ ] Open a PR titled `Release CRATE 0.x.0 - summary`, and merge it
* [ ] `cargo publish --quiet -p CRATE`
* [ ] `git tag -a CRATE-v0.x.0 -m 'Release CRATE 0.x.0 - summary'`
* [ ] `git push --tags`
* [ ] Do a GitHub release: <https://github.com/rerun-io/egui_crates/releases/new>

## History

The tags imported from the two merged repos keep their old, prefixed names:
`egui_table-0.1.0` … `egui_table-0.10.0` and `kittest_inspector-0.1.0` …
`kittest_inspector-0.2.0`. Tags from this repo's own history are unprefixed
(`0.1.0` … `0.17.1`). New tags use the `CRATE-vX.Y.Z` form above.
