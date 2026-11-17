# egui_inspection_tools

[![Latest version](https://img.shields.io/crates/v/egui_inspection_tools.svg)](https://crates.io/crates/egui_inspection_tools)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Apache](https://img.shields.io/badge/license-Apache-blue.svg)

The tools that let an AI agent drive a live [egui](https://github.com/emilk/egui) app: read the AccessKit widget tree (`widget_tree`, `get_widget`, `wait_for`), take a `screenshot`, and `click`, `hover`, `scroll`, `drag`, `type_text`, `press_key`, `resize`, and `batch`.

They talk to the app over the [`egui_inspection`](https://crates.io/crates/egui_inspection) protocol, through a `Transport` you supply.
There is no MCP and no async runtime here, so the tools also run in-process and on the web (`wasm32`).
For an MCP server, use [`egui_mcp`](https://crates.io/crates/egui_mcp), which serves these same tools.

```rust,ignore
use egui_inspection_tools::{BoxFuture, call, tools};

// Your timer, for `wait_for` to poll with.
fn sleep(duration: std::time::Duration) -> BoxFuture<'static, ()> {
    Box::pin(tokio::time::sleep(duration))
}

// Hand `tools()` (name, description, JSON Schema) to your model, then run what it calls:
let output = call(&my_transport, &sleep, "click", args).await;
```
