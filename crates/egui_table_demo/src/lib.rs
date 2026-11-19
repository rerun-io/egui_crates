//! Demo code for the [`egui_table`], hosted at <https://rerun-io.github.io/egui_crates/>.
//!
//! Each push to `main` re-deploys the demo.

#![warn(clippy::all, rust_2018_idioms)]

mod app;
mod split_scroll_demo;
mod table_demo;

pub use app::DemoApp;

/// Create a [`Hyperlink`](egui::Hyperlink) to this egui source code file on github.
#[macro_export]
macro_rules! egui_github_link_file {
    () => {
        $crate::egui_github_link_file!("(source code)")
    };
    ($label: expr) => {
        egui::github_link_file!(
            "https://github.com/rerun-io/egui_table/blob/main/",
            egui::RichText::new($label).small()
        )
    };
}

/// Create a [`Hyperlink`](egui::Hyperlink) to this egui source code file and line on github.
#[macro_export]
macro_rules! egui_github_link_file_line {
    () => {
        $crate::egui_github_link_file_line!("(source code)")
    };
    ($label: expr) => {
        egui::github_link_file_line!(
            "https://github.com/rerun-io/egui_table/blob/main/",
            egui::RichText::new($label).small()
        )
    };
}
