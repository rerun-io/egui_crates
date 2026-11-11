//! Top toolbar: playback controls, history navigation, GIF copy, Control-mode toggle, and
//! a one-line connection status label.

use crate::state::{AppStateRef, Command, PlayState};

pub fn controls_panel(ui: &mut egui::Ui, state: &AppStateRef<'_>) {
    egui::Panel::top("controls").show_inside(ui, |ui| {
        ui.horizontal(|ui| {
            let playing = state.play_state == PlayState::Playing;
            let play_response = ui
                .add_enabled_ui(!state.control_enabled, |ui| {
                    ui.selectable_label(playing, "▶ Play")
                })
                .inner
                .on_disabled_hover_text("Disabled while Control mode is on");
            if play_response.clicked() {
                state.send(Command::Play);
            }
            if ui
                .selectable_label(!playing, "⏸ Pause")
                .on_hover_text("Pause harness after the next frame")
                .clicked()
            {
                state.send(Command::Pause);
            }
            let can_step = state.play_state == PlayState::Paused && state.worker_waiting;
            if ui
                .add_enabled(can_step, egui::Button::new("⏩ Step"))
                .on_hover_text("Advance one harness internal step")
                .clicked()
            {
                state.send(Command::Step);
            }
            if ui
                .add_enabled(can_step, egui::Button::new("⏭ Next"))
                .on_hover_text(
                    "Fast-forward until the test reaches the next `run()` / `step()` call",
                )
                .clicked()
            {
                state.send(Command::NextCall);
            }

            ui.separator();

            // History navigation.
            let total = state.history.len();
            let can_back = state.view_index > 0;
            let can_forward = state.view_index + 1 < total;
            if ui
                .add_enabled(can_back, egui::Button::new("⏴"))
                .on_hover_text("Previous frame in history")
                .clicked()
            {
                state.send(Command::SetViewIndex(state.view_index.saturating_sub(1)));
            }
            if ui
                .add_enabled(can_forward, egui::Button::new("⏵"))
                .on_hover_text("Next frame in history")
                .clicked()
            {
                state.send(Command::SetViewIndex(state.view_index + 1));
            }
            if ui
                .add_enabled(can_forward, egui::Button::new("⏩ Live"))
                .on_hover_text("Jump to the newest frame (follow live updates)")
                .clicked()
            {
                state.send(Command::GoLive);
            }
            if total > 0 {
                // Both the slider value and the label are 1-indexed for display.
                let mut scrub = state.view_index + 1;
                let response = ui.add(
                    egui::Slider::new(&mut scrub, 1..=total)
                        .text(format!("/ {total}"))
                        .clamping(egui::SliderClamping::Always),
                );
                if response.changed() {
                    state.send(Command::SetViewIndex(scrub.saturating_sub(1)));
                }
            }

            if ui
                .add_enabled(total > 0, egui::Button::new("📋 Copy as GIF"))
                .on_hover_text(
                    "Encode the whole history as a GIF and put it on the system clipboard \
                     as a file reference — paste into Slack / Discord / Finder etc.",
                )
                .clicked()
            {
                state.send(Command::StartCopyAsGif);
            }
            if let Some(msg) = state.status_message.as_deref() {
                ui.weak(msg);
            }

            ui.separator();

            let mut control = state.control_enabled;
            ui.add_enabled_ui(state.connected, |ui| {
                ui.checkbox(&mut control, "🎮 Control")
                    .on_hover_text(
                        "Forward pointer and keyboard events on the rendered frame to the harness",
                    )
                    .on_disabled_hover_text("Harness disconnected");
            });
            if control != state.control_enabled {
                state.send(Command::SetControlEnabled(control));
            }

            ui.separator();
            ui.label(if state.connected {
                if state.worker_waiting {
                    "harness blocked"
                } else {
                    "harness running"
                }
            } else {
                "harness disconnected"
            });
        });
    });
}
