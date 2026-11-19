//! `widget_tree` run against a real egui frame.
//!
//! The hand-built trees in [`crate::tree`]'s tests pin one rule each, on a tree of five nodes.
//! This module builds a small but ordinary app — panels, a heading, buttons, a text field, a
//! check box, a slider, a collapsing section — lays out an actual frame with `AccessKit`
//! enabled, and snapshots what an agent would receive. That is where the rules meet egui's real
//! output: the containers it emits, where it puts a widget's text, and how deep the nesting
//! gets.
//!
//! The snapshots carry no positions (a [`Widget`] has no bounds), so they don't move with
//! fonts or platform.

use accesskit_consumer::Tree;

use crate::tree::{self, Exclusion, Query, QueryFilter, Widget, query};

/// Logical size of the frame the tests lay out. Wide enough that nothing is clipped, which
/// would otherwise show up as a `hidden` flag that differs between platforms.
const SCREEN_SIZE: egui::Vec2 = egui::vec2(1280.0, 900.0);

/// A label long enough to wrap over several lines, to show whether an agent is handed the whole
/// text or only what fits.
const LONG_LABEL: &str = "This paragraph is here to be long: it wraps over several lines in the central panel, so the frame has to lay it out as more than one run of text. An agent reading the tree should still get every word of it, because the text it can read is the only thing telling it what this part of the app is for.";

/// The demo app's mutable state, so its widgets report real values rather than defaults.
struct DemoApp {
    search: String,
    wrap_lines: bool,
    volume: f32,
    project: usize,

    /// egui's own gallery, for coverage of the widget kinds we wouldn't think to write out:
    /// combo box, radio group, colour picker, progress bar, hyperlink, and the rest.
    gallery: egui_demo_lib::WidgetGallery,
}

impl Default for DemoApp {
    fn default() -> Self {
        Self {
            search: "tree".to_owned(),
            wrap_lines: true,
            volume: 42.0,
            project: 1,
            gallery: egui_demo_lib::WidgetGallery::default(),
        }
    }
}

impl DemoApp {
    fn ui(&mut self, ui: &mut egui::Ui) {
        // The gallery shows an image, which needs a loader to be anything but a broken icon.
        egui_extras::install_image_loaders(ui.ctx());

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = ui.label("Search");
                ui.text_edit_singleline(&mut self.search)
                    .labelled_by(label.id);
                ui.button("Run").clicked();
            });
        });

        egui::Panel::left("projects").show(ui, |ui| {
            ui.heading("Projects");
            for (i, name) in ["egui", "kittest", "rerun"].iter().enumerate() {
                ui.selectable_value(&mut self.project, i, *name);
            }
        });

        // An agent panel living inside the app it drives, as a host embedding this server has.
        // It echoes the user's own words, so a text query hits it before it hits the app.
        egui::Panel::right("agent chat").show(ui, |ui| {
            ui.heading("Chat");
            ui.label("where is the volume slider?");
            ui.label("Second row of the settings panel.");
        });

        egui::CentralPanel::default().show(ui, |ui| {
            ui.label("Settings");
            ui.label(LONG_LABEL);
            ui.checkbox(&mut self.wrap_lines, "Wrap long lines");
            ui.add(egui::Slider::new(&mut self.volume, 0.0..=100.0).text("Volume"));
            egui::CollapsingHeader::new("Advanced")
                .default_open(true)
                .show(ui, |ui| {
                    ui.label("Nothing to see here");
                    ui.add_enabled(false, egui::Button::new("Reset"));
                });

            // Neither of these is a widget an agent can act on, so neither should reach it.
            ui.separator();
            ui.add_space(16.0);

            egui_demo_lib::View::ui(&mut self.gallery, ui);
        });
    }
}

/// Lay out two frames of the demo app and hand back the `AccessKit` tree of the second.
///
/// Two, because egui sizes several containers from what the previous frame measured — one pass
/// alone would snapshot a half-laid-out app.
fn demo_app_tree() -> Tree {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN_SIZE)),
        ..Default::default()
    };

    let mut app = DemoApp::default();
    // `drop_without_applying_deltas` because these frames are never painted: nothing here
    // consumes the texture deltas an output carries, and dropping one that still holds them
    // panics.
    ctx.run_ui(input.clone(), |ui| app.ui(ui))
        .drop_without_applying_deltas();
    let mut output = ctx.run_ui(input, |ui| app.ui(ui));
    let update = output
        .platform_output
        .accesskit_update
        .take()
        .expect("accesskit is enabled, so every frame carries an update");
    output.drop_without_applying_deltas();
    // `false` — the same as the bridge passes for an app whose window isn't focused.
    Tree::new(update, false)
}

fn query_app(filter: &QueryFilter) -> String {
    let nodes = query(&demo_app_tree(), filter, 1.0).expect("no missing `root`");
    serde_json::to_string_pretty(&nodes).expect("serialize")
}

/// A picture of the frame the tree snapshots describe, so a person can read one against the
/// other: which widget each node is, and what the containers between them are doing.
#[test]
fn the_demo_app_on_screen() {
    let mut app = DemoApp::default();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(SCREEN_SIZE)
        .wgpu()
        .build_ui(move |ui| app.ui(ui));
    harness.run();
    harness.snapshot("demo_app");
}

/// The whole app, the way `widget_tree` with no filter hands it to an agent.
///
/// Without a `limit`, so the snapshot stays the whole picture — what `limit` does to it is its
/// own test.
#[test]
fn the_whole_app() {
    insta::assert_snapshot!(query_app(&QueryFilter {
        limit: usize::MAX,
        ..Default::default()
    }));
}

/// Every button, lifted out of the panels and rows that hold them.
#[test]
fn every_button() {
    insta::assert_snapshot!(query_app(&QueryFilter {
        query: Query {
            role: Some("Button".to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }));
}

/// A text match, the way an agent looks for one widget it means to act on. `content_contains`
/// because a `Label`'s text is in `value`, while a slider's is in `label`.
#[test]
fn one_widget_by_its_text() {
    insta::assert_snapshot!(query_app(&QueryFilter {
        query: Query {
            content_contains: Some("volume".to_owned()),
            ..Default::default()
        },
        ..Default::default()
    }));
}

/// A long label is cut short, but stays findable: the filters match the whole text, so a phrase
/// from past the cut still resolves to the node that holds it.
#[test]
fn a_long_label_is_cut_short_but_still_matchable() {
    let matched = query(
        &demo_app_tree(),
        &QueryFilter {
            query: Query {
                // Deep into the paragraph, well past where the text is cut.
                content_contains: Some("every word of it".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        },
        1.0,
    )
    .expect("no missing `root`");
    let label = matched.first().expect("the long label matched");
    let text = label.value.as_deref().expect("a label's text is its value");
    assert!(
        LONG_LABEL.starts_with(text.trim_end_matches(tree::TRUNCATION_MARKER)),
        "what came back is the start of the label: {text}"
    );
    assert!(
        text.ends_with(tree::TRUNCATION_MARKER),
        "and it is marked as cut: {text}"
    );
    assert!(text.chars().count() < LONG_LABEL.chars().count());
    assert!(
        label.children.is_empty(),
        "the per-line runs repeat the label, so they are folded into it"
    );
}

/// The cut is not a loss: the tree hands out an `id`, and `get_widget` turns that `id` back
/// into the whole text. This is the contract that lets `widget_tree` stay small.
#[test]
fn get_widget_returns_the_text_the_tree_cut_short() {
    let app_tree = demo_app_tree();
    let matched = query(
        &app_tree,
        &QueryFilter {
            query: Query {
                content_contains: Some("every word of it".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        },
        1.0,
    )
    .expect("no missing `root`");
    let label = matched.first().expect("the long label matched");
    let cut = label.value.as_deref().expect("a label's text is its value");
    assert!(
        cut.ends_with(tree::TRUNCATION_MARKER),
        "the tree cut it short: {cut}"
    );

    let node = tree::resolve_unique(&app_tree, &tree::Locator::Id { id: label.id }, 1.0)
        .expect("the id the tree just handed out resolves");
    let full = tree::widget_detail(&node, 1.0)
        .value
        .expect("the same text, in full");
    assert_eq!(full, LONG_LABEL);
    assert!(tree::MAX_TEXT_CHARS < full.chars().count(), "past the cut");
}

/// A tight `limit` keeps the top of the app — the panels and rows an agent navigates by — and
/// says how many children each cut node lost.
#[test]
fn a_tight_limit_keeps_the_top_of_the_tree() {
    insta::assert_snapshot!(query_app(&QueryFilter {
        limit: 12,
        ..Default::default()
    }));
}

/// An exclusion takes a whole subtree with it: the chat panel's own text no longer answers a
/// query meant for the app.
#[test]
fn an_excluded_panel_takes_its_text_with_it() {
    let filter = |exclude| QueryFilter {
        query: Query {
            content_contains: Some("volume".to_owned()),
            ..Default::default()
        },
        exclude,
        ..Default::default()
    };

    let tree = demo_app_tree();
    let unfiltered = query(&tree, &filter(None), 1.0).expect("no missing `root`");
    assert!(
        unfiltered
            .iter()
            .any(|node| node.value.as_deref() == Some("where is the volume slider?")),
        "the chat's echo of the question is in the way"
    );

    // The panel itself carries no text, so it is excluded by the id of the node holding it.
    let chat_panel = find_container_of(
        &query(&tree, &QueryFilter::default(), 1.0).expect("no missing `root`"),
        "Chat",
    )
    .expect("the chat panel is in the tree");
    let excluded = query(
        &tree,
        &filter(Some(Exclusion {
            id: Some(chat_panel),
            ..Default::default()
        })),
        1.0,
    );
    insta::assert_snapshot!(serde_json::to_string_pretty(&excluded).expect("serialize"));
}

/// The id of the nearest node above a `heading`, i.e. the container that holds that section.
fn find_container_of(nodes: &[Widget], heading: &str) -> Option<tree::Id> {
    nodes.iter().find_map(|node| {
        let holds_heading = node
            .children
            .iter()
            .any(|child| child.value.as_deref() == Some(heading));
        if holds_heading {
            Some(node.id)
        } else {
            find_container_of(&node.children, heading)
        }
    })
}

/// Nothing an agent can act on should reach it without an `id` to act on it with, and the
/// pruning rules shouldn't leave a node that carries nothing at all.
#[test]
fn every_returned_node_is_actionable() {
    fn check(nodes: &[Widget]) {
        for node in nodes {
            assert!(
                node.role.is_some()
                    || node.label.is_some()
                    || node.value.is_some()
                    || !node.children.is_empty(),
                "a node that says nothing should have been pruned: {node:?}"
            );
            check(&node.children);
        }
    }

    let nodes = query(&demo_app_tree(), &QueryFilter::default(), 1.0).expect("no missing `root`");
    assert!(!nodes.is_empty(), "the app has widgets");
    check(&nodes);
}
