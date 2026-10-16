# ☰ `egui_table`

[<img alt="github" src="https://img.shields.io/badge/github-rerun_io/egui_crates-8da0cb?logo=github" height="20">](https://github.com/rerun-io/egui_crates)
[![Latest version](https://img.shields.io/crates/v/egui_table.svg)](https://crates.io/crates/egui_table)
[![Documentation](https://docs.rs/egui_table/badge.svg)](https://docs.rs/egui_table)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)
[![Apache](https://img.shields.io/badge/license-Apache-blue.svg)](https://github.com/rerun-io/egui_crates/blob/main/LICENSE-APACHE)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/rerun-io/egui_crates/blob/main/LICENSE-MIT)

Table viewer for [egui](https://www.egui.rs/).

### Design
egui_table has a "batteries not included" design.

### Features
* Auto-sized, resizable columns
* Hierarchical column titles
* Sticky columns and header
* Expanding rows
* Support for millions of rows
* Heterogenous row heights


### Testing
* Locally: `cargo run -p demo`
* Web locally: `(cd crates/egui_table_demo && trunk serve)`
* Web: <https://rerun-io.github.io/egui_crates/>
