# `egui_tiles` Changelog

## 0.17.1 - 2026-08-18
Full diff at https://github.com/rerun-io/egui_crates/compare/0.17.0..HEAD

* Fix visibility layouting [#156](https://github.com/rerun-io/egui_crates/pull/156) by [@IsseW](https://github.com/IsseW)


## 0.17.0 - 2026-08-05
Full diff at https://github.com/rerun-io/egui_crates/compare/0.16.0..HEAD

### Breaking changes:
* ⚠️ `Linear::layout` is no longer `pub`. It took `&mut Tiles` and a `&mut dyn Behavior`, so it could not be called usefully from outside the crate.

### Other changes:
* Add `SimplificationOptions::flatten_tabs_in_tabs` [#77](https://github.com/rerun-io/egui_crates/pull/77)
* Add `Tiles::recompute_next_tile_id` [#114](https://github.com/rerun-io/egui_crates/pull/114)
* Update dependencies [#138](https://github.com/rerun-io/egui_crates/pull/138)
* Add `Behavior::tab_bar_trailing_ui` (trailing tab-bar slot) [#135](https://github.com/rerun-io/egui_crates/pull/135)
* Fix unstable widget IDs for tab scroll arrows [#137](https://github.com/rerun-io/egui_crates/pull/137)
* Update MSRV to 1.95 [#140](https://github.com/rerun-io/egui_crates/pull/140)
* Keep the pane type out of the layout pass [#141](https://github.com/rerun-io/egui_crates/pull/141)
* Add an example for apps that rebuild the tree every frame [#145](https://github.com/rerun-io/egui_crates/pull/145)
* Report tabs to accessibility tools [#143](https://github.com/rerun-io/egui_crates/pull/143)
* Drag panes by their tab in the drag-and-drop tests [#146](https://github.com/rerun-io/egui_crates/pull/146)
* Keep a tile's TileId when it gets wrapped in a container [#144](https://github.com/rerun-io/egui_crates/pull/144)
* Keep tile ids stable through simplification too [#147](https://github.com/rerun-io/egui_crates/pull/147)
* Tabs: Add support for adding buttons before the left arrow in the tab bar [#131](https://github.com/rerun-io/egui_crates/pull/131)
* Clear the open tab when `simplify` removes it [#149](https://github.com/rerun-io/egui_crates/pull/149)
* Don't delete a shared tile during garbage collection [#148](https://github.com/rerun-io/egui_crates/pull/148)
* Update egui to 0.36 [#152](https://github.com/rerun-io/egui_crates/pull/152)
* Clear the root when garbage collection had to drop it [#150](https://github.com/rerun-io/egui_crates/pull/150)


## 0.16.0 - 2026-06-26
Full diff at https://github.com/rerun-io/egui_crates/compare/0.15.0..HEAD

* Tabs: Derive scroll-arrow size from tab_bar_height [#130](https://github.com/rerun-io/egui_crates/pull/130) by [@SpruceCloud](https://github.com/SpruceCloud)
* Update to egui 0.35 [#133](https://github.com/rerun-io/egui_crates/pull/133) by [@lucasmerlin](https://github.com/lucasmerlin)



## 0.15.0 - 2026-03-27
Full diff at https://github.com/rerun-io/egui_crates/compare/0.14.0..HEAD

* Update to egui 0.34 [#125](https://github.com/rerun-io/egui_crates/pull/125) by [@emilk](https://github.com/emilk)
* Update MSRV to 1.92 [#124](https://github.com/rerun-io/egui_crates/pull/124)
* Add `Behavior::is_tile_draggable` and `is_container_resizable` [#123](https://github.com/rerun-io/egui_crates/pull/123) by [@enomado](https://github.com/enomado)
* Close tabs with middle mouse button [#122](https://github.com/rerun-io/egui_crates/pull/122) by [@chicoferreira](https://github.com/chicoferreira)
* Make `Behavior::on_tab_button` take `&mut Tiles` instead of `&Tiles` [#121](https://github.com/rerun-io/egui_crates/pull/121) by [@chicoferreira](https://github.com/chicoferreira)
* Fix active tab not updating when dragged to another container [#115](https://github.com/rerun-io/egui_crates/pull/115) by [@stargazing-dino](https://github.com/stargazing-dino)
* Use the next active tab when iterating active tiles [#111](https://github.com/rerun-io/egui_crates/pull/111) by [@IsseW](https://github.com/IsseW)


## 0.14.0 - 2025-10-10
Full diff at https://github.com/rerun-io/egui_crates/compare/0.13.0..HEAD

* Update to egui 0.33.0 [#108](https://github.com/rerun-io/egui_crates/pull/108) by [@lucasmerlin](https://github.com/lucasmerlin)
* Update MSRV to 1.88 [#107](https://github.com/rerun-io/egui_crates/pull/107) by [@lucasmerlin](https://github.com/lucasmerlin)
* Add `Tree::inactive_tiles` for getting non-visible tiles [#106](https://github.com/rerun-io/egui_crates/pull/106) by [@ivanrg99](https://github.com/ivanrg99)


## 0.13.0 - 2025-07-11
Full diff at https://github.com/rerun-io/egui_crates/compare/0.12.0..HEAD

* Allow setting a custom cursor icon when hovering over a tab [#97](https://github.com/rerun-io/egui_crates/pull/97) by [@s-nie](https://github.com/s-nie)
* Update to egui 0.32 [#104](https://github.com/rerun-io/egui_crates/pull/104) by [@emilk](https://github.com/emilk)
* Update to Rust 1.85 [#102](https://github.com/rerun-io/egui_crates/pull/102) by [@emilk](https://github.com/emilk)
* Update itertools to 0.14 [#105](https://github.com/rerun-io/egui_crates/pull/105) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.11.0 - 2024-12-17
* Update MSRV to 1.80 [#91](https://github.com/rerun-io/egui_crates/pull/91) by [@emilk](https://github.com/emilk)
* Update to egui 0.30.0 [#92](https://github.com/rerun-io/egui_crates/pull/92) by [@emilk](https://github.com/emilk)


## 0.10.1 - 2024-10-01
* Fix `Tree` serialization [#87](https://github.com/rerun-io/egui_crates/pull/87)


## 0.10.0 - 2024-09-26
* Update to egui 0.29 [#78](https://github.com/rerun-io/egui_crates/pull/78)
* Add `Tree::set_width` and `set_height` functions [#73](https://github.com/rerun-io/egui_crates/pull/73) (thanks [@rafaga](https://github.com/rafaga)!)
* Fix for eagerly starting a drag when clicking tab background [#80](https://github.com/rerun-io/egui_crates/pull/80)
* Fix `Tree` deserialization using JSON [#85](https://github.com/rerun-io/egui_crates/pull/85) (thanks [@hastri](https://github.com/hastri)!)


## [0.9.1](https://github.com/rerun-io/egui_crates/compare/0.9.0...0.9.1) - 2024-08-27
* Add `Tree::set_width` and `set_height` functions [#73](https://github.com/rerun-io/egui_crates/pull/73) (thanks [@rafaga](https://github.com/rafaga)!)
* Fix for eagerly starting a drag when clicking tab background [#80](https://github.com/rerun-io/egui_crates/pull/80)


## [0.9.0](https://github.com/rerun-io/egui_crates/compare/0.8.0...0.9.0) - 2024-07-03 - egui 0.28 and tab close buttons
Full diff at https://github.com/rerun-io/egui_crates/compare/0.8.0..HEAD

* Update to egui 0.28.0 [#67](https://github.com/rerun-io/egui_crates/pull/67)
* Update to Rust 1.76 [#60](https://github.com/rerun-io/egui_crates/pull/60) [#66](https://github.com/rerun-io/egui_crates/pull/66)
* Optional close-buttons on tabs [#70](https://github.com/rerun-io/egui_crates/pull/70) (thanks [@voidburn](https://github.com/voidburn)!)
* Add `Tiles::rect` to read where a tile is [#61](https://github.com/rerun-io/egui_crates/pull/61)
* Add `Behavior::paint_on_top_of_tile` [#62](https://github.com/rerun-io/egui_crates/pull/62)
* Fix: make sure `Tree::ui` allocates the space it uses in parent `Ui` [#71](https://github.com/rerun-io/egui_crates/pull/71) (thanks [@rydb](https://github.com/rydb)!)
* Fix bugs when having multiple `Tree`s visible at the same time [#68](https://github.com/rerun-io/egui_crates/pull/68) (thanks [@GuillaumeSchmid](https://github.com/GuillaumeSchmid)!)
* Fix drag-and-drop of tiles on touchscreen devices [#74](https://github.com/rerun-io/egui_crates/pull/74) (thanks [@mcoroz](https://github.com/mcoroz)!)
* Fix container resize drag for touchscreens [#75](https://github.com/rerun-io/egui_crates/pull/75) (thanks [@mcoroz](https://github.com/mcoroz)!)
* Update release instructions [62ecb4c](https://github.com/rerun-io/egui_crates/commit/62ecb4ccd52bdabd11e688e4e6e29e4d1a3783ab)
* Add clippy lint `match_bool` [fadf41a](https://github.com/rerun-io/egui_crates/commit/fadf41ab42af5527e8a17af436a5608dd7dbd7bf)
* Add a PR template [87110a9](https://github.com/rerun-io/egui_crates/commit/87110a98a280f73c77b80507367290691f75d33b)
* Expose `egui_tiles::TabState` [6e88ea9](https://github.com/rerun-io/egui_crates/commit/6e88ea9774d63b0a7a8a67af9a90c13a4b3efb10)
* Pass `&TabState` to all relevant functions in Behavior [ee1286a](https://github.com/rerun-io/egui_crates/commit/ee1286a975239ffa34258313a11d2bf03ec4cea9)


## [0.8.0](https://github.com/rerun-io/egui_crates/compare/0.7.2...0.8.0) - 2024-03-26
* Update to egui 0.27.0 [#58](https://github.com/rerun-io/egui_crates/pull/58)
* Re-export `Shares` [#56](https://github.com/rerun-io/egui_crates/pull/56) (thanks [@Gohla](https://github.com/Gohla)!)
* Propagate `enabled` status for tile `Ui` [#55](https://github.com/rerun-io/egui_crates/pull/55) (thanks [@Gohla](https://github.com/Gohla)!)


## [0.7.2](https://github.com/rerun-io/egui_crates/compare/0.7.1...0.7.2) - 2024-02-07
* Fix `move_tile_to_container` behavior for grid-to-same-grid moves with reflow enabled [#53](https://github.com/rerun-io/egui_crates/pull/53)


## [0.7.1](https://github.com/rerun-io/egui_crates/compare/0.7.0...0.7.1) - 2024-02-06
* Make sure there is always an active tab [#50](https://github.com/rerun-io/egui_crates/pull/50)
* Derive `Clone, Debug, PartialEq, Eq` for `EditAction` [#51](https://github.com/rerun-io/egui_crates/pull/51)


## [0.7.0](https://github.com/rerun-io/egui_crates/compare/0.6.0...0.7.0) - 2024-02-06
* Add an API to move an existing tile to an give container and position index [#44](https://github.com/rerun-io/egui_crates/pull/44)
* Properly handle grid layout with `Tree::move_tile_to_container()` [#45](https://github.com/rerun-io/egui_crates/pull/45)
* Turn some warn logging to debug logging [#47](https://github.com/rerun-io/egui_crates/pull/47)
* Add an `EditAction` parameter to the `Behavior::on_edit()` call [#48](https://github.com/rerun-io/egui_crates/pull/48)
* Update to `egui` 0.26 [#49](https://github.com/rerun-io/egui_crates/pull/49)


## [0.6.0](https://github.com/rerun-io/egui_crates/compare/0.5.0...0.6.0) - 2024-01-08
* Update to egui 0.25 [#43](https://github.com/rerun-io/egui_crates/pull/43)


## [0.5.0](https://github.com/rerun-io/egui_crates/compare/0.4.0...0.5.0) - 2024-01-04
* Pass `TileId` to `make_active` closure [#35](https://github.com/rerun-io/egui_crates/pull/35)
* Add `SimplificationOptions::OFF` [#38](https://github.com/rerun-io/egui_crates/pull/38)
* Add `Tree::simplify_children_of_tile` [#39) [#41](https://github.com/rerun-io/egui_crates/pull/41)
* Expose the internal `u64` part of `TileId` [#40](https://github.com/rerun-io/egui_crates/pull/40)
* Fix simplification errors that result in warnings after removing panes [#41](https://github.com/rerun-io/egui_crates/pull/41)
* Add `Tree::active_tiles` for getting visible tiles [#42](https://github.com/rerun-io/egui_crates/pull/42)


## [0.4.0](https://github.com/rerun-io/egui_crates/compare/0.3.1...0.4.0) - 2023-11-23
* Fix Id clash when using multiple `Tree`s [#32](https://github.com/rerun-io/egui_crates/pull/32)
* Scrollable tab bar [#9](https://github.com/rerun-io/egui_crates/pull/9)
* `Behavior::on_tab_button` can now add context menus, on hover ui etc [#23](https://github.com/rerun-io/egui_crates/pull/23)
* `serde` is now and optional dependency [#13](https://github.com/rerun-io/egui_crates/pull/13)
* Update to egui 0.24
* Update MSRV to Rust 1.72


## [0.3.1](https://github.com/rerun-io/egui_crates/compare/0.3.0...0.3.1) - 2023-09-29
* Report edits to user with `Behavior::on_edit` [#29](https://github.com/rerun-io/egui_crates/pull/29)
* Make `Tree::simplify` public [#28](https://github.com/rerun-io/egui_crates/pull/28)
* Add `Shares::set_share` method [#25](https://github.com/rerun-io/egui_crates/pull/25)


## [0.3.0](https://github.com/rerun-io/egui_crates/compare/0.2.0...0.3.0) - 2023-09-28
* Update to egui 0.23
* Better grid column-count heuristic
* Make drag preview style customizable


## [0.2.0](https://github.com/rerun-io/egui_crates/compare/0.1.0...0.2.0) - Invisible tiles - 2023-07-06
* Add support for invisible tiles
* `PartialEq` for `Tiles` now ignores internal state
* Add `Tiles::find_pane`
* Add `Tiles::remove_recursively`


## 0.1.0 - Initial Release - 2023-05-24
