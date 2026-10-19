# Changelog

## 0.11.0 - 2026-10-19
Full diff at https://github.com/rerun-io/egui_crates/compare/egui_table-0.10.0..egui_table-v0.11.0

* Pass the scroll offset to SplitScrollDelegate instead of inferring it [#64](https://github.com/rerun-io/egui_table/pull/64) by [@jccampagne](https://github.com/jccampagne)
* Improve split table behavior with eguis new scroll bounce [#66](https://github.com/rerun-io/egui_table/pull/66) by [@lucasmerlin](https://github.com/lucasmerlin)
* Update egui to 0.37 [#170](https://github.com/rerun-io/egui_crates/pull/170) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.10.0 - 2026-08-05

* Increase MSRV to 1.95 [#60](https://github.com/rerun-io/egui_table/pull/60) by [@emilk](https://github.com/emilk)
* Update egui to 0.36 [#61](https://github.com/rerun-io/egui_table/pull/61) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.9.0 - 2026-06-26

* Update egui to 0.35 [#57](https://github.com/rerun-io/egui_table/pull/57) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.8.0 - 2026-03-26 - Update to egui 0.34
* Update to egui 0.34 [#55](https://github.com/rerun-io/egui_table/pull/55) by [@emilk](https://github.com/emilk)
* Update MSRV to 1.92 [#51](https://github.com/rerun-io/egui_table/pull/51) by [@emilk](https://github.com/emilk)
* Add `stick_to_bottom` flag to Table and SplitScroll [#47](https://github.com/rerun-io/egui_table/pull/47) by [@AmmarAbouZor](https://github.com/AmmarAbouZor)
* Fix slow shrinking of wide columns [#53](https://github.com/rerun-io/egui_table/pull/53) by [@emilk](https://github.com/emilk)


## 0.7.0 - 2026-02-19
* Fix  `scroll_to_row` not taking the header into account [#44](https://github.com/rerun-io/egui_table/pull/44) by [@abey79](https://github.com/abey79)
* Fix table placement within containers [#43](https://github.com/rerun-io/egui_table/pull/43) by [@agalakhov](https://github.com/agalakhov)


## 0.6.0 - 2025-11-25
* Change return type to `Response` in `Table::show` [#36](https://github.com/rerun-io/egui_table/pull/36) by [@TihonovNV](https://github.com/TihonovNV)
* Add `row_ui` to allow interacting with the whole row at once [#38](https://github.com/rerun-io/egui_table/pull/38) by [@lucasmerlin](https://github.com/lucasmerlin)


## 0.5.0 - 2025-10-10
* Update msrv to 1.88 [#32](https://github.com/rerun-io/egui_table/pull/32) by [@lucasmerlin](https://github.com/lucasmerlin)
* Update to egui 0.33.0 [#35](https://github.com/rerun-io/egui_table/pull/35) by [@emilk](https://github.com/emilk)


## 0.4.0 - 2025-07-10 - Update to egui 0.32
* Update to egui 0.32 [#30](https://github.com/rerun-io/egui_table/pull/30) by [@emilk](https://github.com/emilk)
* Update to Rust 1.85 [#29](https://github.com/rerun-io/egui_table/pull/29) by [@emilk](https://github.com/emilk)


## 0.3.0 - 2025-02-05 - Update to egui 0.31
* Update to egui 0.31 [#22](https://github.com/rerun-io/egui_table/pull/22) by [@emilk](https://github.com/emilk)
* Update MSRV to 1.81 [#21](https://github.com/rerun-io/egui_table/pull/21) by [@emilk](https://github.com/emilk)
* Fix ghost column when no sticky columns [#20](https://github.com/rerun-io/egui_table/pull/20) by [@EpixScripts](https://github.com/EpixScripts)


## 0.2.0 - 2024-12-17 - Update egui to 0.30
* Update MSRV to 1.80 [#17](https://github.com/rerun-io/egui_table/pull/17) by [@emilk](https://github.com/emilk)
* Update to egui 0.30 [#18](https://github.com/rerun-io/egui_table/pull/18) by [@emilk](https://github.com/emilk)


## 0.1.0 - 2024-10-14 - Initial release
Features
* Auto-sized, resizable columns
* Hierarchical column titles
* Sticky columns and header
* Expanding rows
* Support for millions of rows
* Heterogenous row heights
