//! Central panel: renders the harness frame and either highlights widgets under hover
//! (inspection mode) or forwards pointer/keyboard events to the harness (Control mode).

use egui::accesskit::{NodeId, Rect as AkRect};

use crate::state::{AppStateRef, Command};

pub fn central_panel(ui: &mut egui::Ui, state: &AppStateRef<'_>) {
    egui::CentralPanel::default().show_inside(ui, |ui| {
        let Some(tex) = state.texture.cloned() else {
            ui.centered_and_justified(|ui| {
                ui.label("Waiting for harness to connect...");
            });
            return;
        };
        let Some(frame) = state.view_frame else {
            return;
        };

        let physical = tex.size_vec2(); // physical pixels of the rendered frame
        let avail = ui.available_size();
        let scale = (avail.x / physical.x)
            .min(avail.y / physical.y)
            .clamp(0.05, 1.0);
        let display_size = physical * scale;

        let (image_rect, response) = ui.allocate_exact_size(
            display_size,
            egui::Sense::click().union(egui::Sense::hover()),
        );
        ui.painter().image(
            tex.id(),
            image_rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        // Cache the image placement so the next frame's `hit_test_pointer` can run before
        // the tree is rendered and keep the two in sync.
        state.send(Command::SetImageRect {
            rect: image_rect,
            scale,
        });

        // logical_point → screen_position:
        //     screen = image_rect.min + ak_rect * pixels_per_point * scale
        // AccessKit measures in f64, egui in f32. Screen coordinates are small enough that
        // the narrowing loses nothing that can be seen.
        #[expect(clippy::cast_possible_truncation)]
        let logical_to_screen = |r: AkRect| -> egui::Rect {
            let f = frame.pixels_per_point * scale;
            egui::Rect::from_min_max(
                image_rect.min + egui::vec2(r.x0 as f32 * f, r.y0 as f32 * f),
                image_rect.min + egui::vec2(r.x1 as f32 * f, r.y1 as f32 * f),
            )
        };

        if state.control_enabled {
            // In Control mode clicks/hovers drive the harness, not the inspector.
            forward_events(
                ui,
                state,
                image_rect,
                frame.pixels_per_point,
                scale,
                &response,
            );
        } else {
            // Inspection mode: hover was already resolved (pre-render hit-test + tree hover)
            // — we only need to handle the click here.
            if response.clicked() {
                state.send(Command::SetSelectedNode(state.hovered_node.get()));
            }

            let painter = ui.painter_at(image_rect);
            if let Some(update) = &frame.accesskit {
                let draw = |id: NodeId, color: egui::Color32| {
                    if let Some((_, node)) = update.nodes.iter().find(|(nid, _)| *nid == id)
                        && let Some(b) = node.bounds()
                    {
                        painter.rect_stroke(
                            logical_to_screen(b),
                            2.0,
                            egui::Stroke::new(1.5, color),
                            egui::StrokeKind::Outside,
                        );
                    }
                };
                if let Some(id) = state.selected_node {
                    draw(id, egui::Color32::from_rgb(80, 180, 255));
                }
                let hover = state.hovered_node.get();
                if let Some(id) = hover
                    && hover != state.selected_node
                {
                    draw(id, egui::Color32::from_rgb(255, 220, 90));
                }
            }
        }
    });
}

/// Inspect the inspector's own input events and forward those relevant to the harness. Events
/// are accumulated in `state.captured_events` so end-of-frame can promote them into
/// `state.queued_events` synchronously before the auto-release decision runs.
fn forward_events(
    ui: &egui::Ui,
    state: &AppStateRef<'_>,
    image_rect: egui::Rect,
    pixels_per_point: f32,
    scale: f32,
    image_response: &egui::Response,
) {
    let to_logical = |pos: egui::Pos2| -> egui::Pos2 {
        let f = pixels_per_point * scale;
        egui::pos2(
            (pos.x - image_rect.min.x) / f,
            (pos.y - image_rect.min.y) / f,
        )
    };

    let input_events = ui.ctx().input(|i| i.events.clone());
    let mut buf = state.captured_events.borrow_mut();
    for ev in input_events {
        match ev {
            egui::Event::PointerMoved(pos) if image_rect.contains(pos) => {
                buf.push(egui::Event::PointerMoved(to_logical(pos)));
            }
            egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers,
            } if image_rect.contains(pos) => {
                buf.push(egui::Event::PointerButton {
                    pos: to_logical(pos),
                    button,
                    pressed,
                    modifiers,
                });
            }
            egui::Event::PointerGone => {
                buf.push(egui::Event::PointerGone);
            }
            mw @ egui::Event::MouseWheel { .. } if image_response.hovered() => {
                buf.push(mw);
            }
            ev @ (egui::Event::Text(_)
            | egui::Event::Key { .. }
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::Ime(_)) => {
                buf.push(ev);
            }
            _ => {}
        }
    }
}
