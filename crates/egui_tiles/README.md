# `egui_tiles`

[<img alt="github" src="https://img.shields.io/badge/github-rerun_io/egui_crates-8da0cb?logo=github" height="20">](https://github.com/rerun-io/egui_crates)
[![Latest version](https://img.shields.io/crates/v/egui_tiles.svg)](https://crates.io/crates/egui_tiles)
[![Documentation](https://docs.rs/egui_tiles/badge.svg)](https://docs.rs/egui_tiles)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](https://github.com/rust-secure-code/safety-dance/)
[![Build Status](https://github.com/rerun-io/egui_crates/workflows/Rust/badge.svg)](https://github.com/rerun-io/egui_crates/actions?workflow=Rust)
[![MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/rerun-io/egui_crates/blob/main/LICENSE-MIT)
[![Apache](https://img.shields.io/badge/license-Apache-blue.svg)](https://github.com/rerun-io/egui_crates/blob/main/LICENSE-APACHE)

Layouting and docking for [egui](https://github.com/rerun-io/egui).

Supports:
* Horizontal and vertical layouts
* Grid layouts
* Tabs
* Drag-and-drop docking

![egui_tiles](https://github.com/rerun-io/egui_crates/assets/1148717/f86bee40-2506-4484-8a82-37ffdc805b81)

### Trying it
`cargo r --example simple`

### Comparison with [egui_dock](https://github.com/Adanos020/egui_dock)
[egui_dock](https://github.com/Adanos020/egui_dock) is an excellent crate serving similar needs. `egui_tiles` aims to become a more flexible and feature-rich alternative to `egui_dock`.

`egui_dock` only supports binary splits (left/right or top/bottom), while `egui_tiles` support full horizontal and vertical layouts, as well as grid layouts. `egui_tiles` also strives to be more customizable, enabling users to override the default style and behavior by implementing methods on a `Behavior` `trait`.

`egui_dock` supports some features that `egui_tiles` does not yet support, such as close-buttons on each tab, and built-in scroll areas.

---

<div align="center">
<img src="https://user-images.githubusercontent.com/1148717/236840584-f4795fb3-89e3-40ac-b570-ac2869e6e8fa.png" width="50%">

`egui_tiles` development is sponsored by [Rerun](https://www.rerun.io/), a startup doing<br>
visualizations for computer vision and robotics.
</div>
