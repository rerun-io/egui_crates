//! Right-hand side panel: source view on top (scrolls to the current call-site or event) and
//! collapsible sections for frame metadata, the hovered/selected widget, and the full
//! `AccessKit` tree.

use egui::accesskit::{self, Node, NodeId};

use crate::state::{AppStateRef, Command};
use crate::ui::source::source_section;

pub fn details_panel(ui: &mut egui::Ui, state: &AppStateRef<'_>) {
    egui::Panel::right("details")
        .resizable(true)
        .default_size(380.0)
        .show_inside(ui, |ui| {
            let Some(frame) = state.view_frame else {
                ui.weak("Waiting for frames...");
                return;
            };

            // The Source view sits in its own resizable top panel so the user can drop it out
            // of the way when they want more room for the widget / AccessKit sections below.
            egui::Panel::top("details_source")
                .resizable(true)
                .default_size(280.0)
                .show_inside(ui, |ui| {
                    ui.heading("Source");
                    source_section(ui, frame, state.scroll_pending);
                });

            egui::ScrollArea::vertical().show(ui, |ui| {
                // Make long values wrap inside the fixed-width side panel instead of overflowing.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);

                egui::CollapsingHeader::new("Frame")
                    .default_open(true)
                    .show(ui, |ui| {
                        kv_grid(ui, "frame_grid", |ui| {
                            if let Some(label) = &frame.label {
                                ui.label("Test:");
                                ui.monospace(label);
                                ui.end_row();
                            }
                            ui.label("Step:");
                            ui.monospace(frame.step.to_string());
                            ui.end_row();
                            ui.label("Size (px):");
                            ui.monospace(format!("{} × {}", frame.width, frame.height));
                            ui.end_row();
                            ui.label("Pixels per point:");
                            ui.monospace(format!("{:.2}", frame.pixels_per_point));
                            ui.end_row();
                            let node_count = frame.accesskit.as_ref().map_or(0, |u| u.nodes.len());
                            ui.label("AccessKit nodes:");
                            ui.monospace(node_count.to_string());
                            ui.end_row();
                        });
                    });

                let target = state.selected_node.or_else(|| state.hovered_node.get());
                let header = if state.selected_node.is_some() {
                    "Selected widget"
                } else if state.hovered_node.get().is_some() {
                    "Hovered widget"
                } else {
                    "Widget"
                };
                egui::CollapsingHeader::new(header)
                    .default_open(true)
                    .show(ui, |ui| match (target, &frame.accesskit) {
                        (Some(id), Some(update)) => {
                            if let Some((_, node)) = update.nodes.iter().find(|(nid, _)| *nid == id)
                            {
                                widget_details(ui, id, node);
                            } else {
                                ui.weak("(node not in latest tree)");
                            }
                        }
                        _ => {
                            ui.weak("Hover over the rendered frame to inspect a widget.");
                        }
                    });

                if state.selected_node.is_some()
                    && ui
                        .small_button("clear selection")
                        .on_hover_text("Stop pinning the selected widget")
                        .clicked()
                {
                    state.send(Command::SetSelectedNode(None));
                }

                egui::CollapsingHeader::new("AccessKit tree")
                    .default_open(false)
                    .show(ui, |ui| {
                        if let Some(update) = &frame.accesskit {
                            accesskit_tree(ui, update, state);
                        } else {
                            ui.weak("(no accesskit tree)");
                        }
                    });
            });
        });
}

fn kv_grid(ui: &mut egui::Ui, id: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .striped(true)
        .show(ui, body);
}

/// Render the accesskit tree recursively, similar in style to the egui demo's `inspection_ui`.
fn accesskit_tree(ui: &mut egui::Ui, update: &accesskit::TreeUpdate, state: &AppStateRef<'_>) {
    use std::collections::{HashMap, HashSet};

    let nodes: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, n)| (*id, n)).collect();

    // Prefer the tree's declared root. If this update doesn't carry tree-level info (diff-only
    // updates can omit it), fall back to any node that no other node lists as a child.
    let root = update.tree.as_ref().map(|t| t.root).or_else(|| {
        let mut children: HashSet<NodeId> = HashSet::new();
        for (_, node) in &update.nodes {
            for c in node.children() {
                children.insert(*c);
            }
        }
        update
            .nodes
            .iter()
            .map(|(id, _)| *id)
            .find(|id| !children.contains(id))
    });

    match root {
        Some(root_id) => render_ak_node(ui, root_id, &nodes, state),
        None => {
            // Shouldn't normally happen; degrade to a flat list.
            for (id, _) in &update.nodes {
                render_ak_node(ui, *id, &nodes, state);
            }
        }
    }
}

fn render_ak_node(
    ui: &mut egui::Ui,
    id: NodeId,
    nodes: &std::collections::HashMap<NodeId, &Node>,
    state: &AppStateRef<'_>,
) {
    let Some(node) = nodes.get(&id).copied() else {
        ui.weak(format!("(missing {:?})", id.0));
        return;
    };
    let role = format!("{:?}", node.role());
    let text = match node.label().or_else(|| node.value()) {
        Some(label) if !label.is_empty() => format!("{role}  {label:?}"),
        _ => role,
    };
    // Both the image's hovered state and the tree's selection light up the same row — a row
    // shown highlighted in the tree corresponds to the rect drawn on the image.
    let highlight = state.selected_node == Some(id) || state.hovered_node.get() == Some(id);
    let children = node.children();

    if children.is_empty() {
        let response = ui.selectable_label(highlight, text);
        if response.clicked() {
            state.send(Command::SetSelectedNode(Some(id)));
        }
        if response.hovered() {
            state.hovered_node.set(Some(id));
        }
        return;
    }

    let header_id = ui.make_persistent_id(("ak_node", id.0));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), header_id, true)
        .show_header(ui, |ui| {
            let response = ui.selectable_label(highlight, text);
            if response.clicked() {
                state.send(Command::SetSelectedNode(Some(id)));
            }
            if response.hovered() {
                state.hovered_node.set(Some(id));
            }
        })
        .body(|ui| {
            for child_id in children {
                render_ak_node(ui, *child_id, nodes, state);
            }
        });
}

/// Render the inspector grid for a single accesskit node, mimicking egui's `inspection_ui`.
fn widget_details(ui: &mut egui::Ui, id: NodeId, node: &Node) {
    kv_grid(ui, "widget_grid", |ui| {
        ui.label("ID:");
        ui.monospace(format!("{:?}", id.0));
        ui.end_row();

        ui.label("Role:");
        ui.monospace(format!("{:?}", node.role()));
        ui.end_row();

        if let Some(b) = node.bounds() {
            ui.label("Bounds:");
            ui.monospace(format!(
                "({:.1}, {:.1}) → ({:.1}, {:.1})  [{:.1} × {:.1}]",
                b.x0,
                b.y0,
                b.x1,
                b.y1,
                b.x1 - b.x0,
                b.y1 - b.y0,
            ));
            ui.end_row();
        }

        for (label, value) in [
            ("Label:", node.label()),
            ("Value:", node.value()),
            ("Description:", node.description()),
            ("Placeholder:", node.placeholder()),
            ("Tooltip:", node.tooltip()),
            ("Class:", node.class_name()),
            ("Author ID:", node.author_id()),
            ("Keyboard:", node.keyboard_shortcut()),
        ] {
            if let Some(v) = value
                && !v.is_empty()
            {
                ui.label(label);
                ui.monospace(v);
                ui.end_row();
            }
        }

        let flags = [
            ("Disabled", node.is_disabled()),
            ("Hidden", node.is_hidden()),
            ("Read-only", node.is_read_only()),
        ];
        let mut on_flags: Vec<&str> = flags
            .iter()
            .filter(|(_, on)| *on)
            .map(|(n, _)| *n)
            .collect();
        if let Some(sel) = node.is_selected() {
            on_flags.push(if sel { "Selected" } else { "Unselected" });
        }
        if !on_flags.is_empty() {
            ui.label("Flags:");
            ui.monospace(on_flags.join(", "));
            ui.end_row();
        }

        if let Some(t) = node.toggled() {
            ui.label("Toggled:");
            ui.monospace(format!("{t:?}"));
            ui.end_row();
        }

        let child_count = node.children().len();
        if child_count > 0 {
            ui.label("Children:");
            ui.monospace(child_count.to_string());
            ui.end_row();
        }
    });
}
