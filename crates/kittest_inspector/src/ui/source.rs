//! Test-source code viewer: renders the relevant `.rs` file with the call-site line + any
//! event lines highlighted, and scrolls to the focused line once per new frame.

use egui_kittest::inspector_api::Frame;

pub fn source_section(ui: &mut egui::Ui, frame: &Frame, scroll_pending: bool) {
    let Some(source) = &frame.source else {
        ui.weak("No source location for this frame.");
        return;
    };

    ui.horizontal(|ui| {
        ui.monospace(shorten_path(&source.path));
        if let Some(line) = source.call_site_line {
            ui.weak(format!("(producer: line {line})"));
        }
    });

    let Some(contents) = source.contents.as_deref() else {
        ui.weak(format!("(couldn't read {})", source.path));
        return;
    };

    let call_site_line = source.call_site_line;
    let event_lines: std::collections::HashSet<u32> = source.event_lines.iter().copied().collect();
    let focus_line = call_site_line.or_else(|| source.event_lines.first().copied());

    // Semi-transparent tints so the highlight works in both light and dark themes without
    // darkening the text. Alpha ~72/255 keeps the underlying text fully legible.
    let call_bg = egui::Color32::from_rgba_unmultiplied(80, 160, 255, 72);
    let event_bg = egui::Color32::from_rgba_unmultiplied(255, 180, 60, 72);

    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let lines: Vec<&str> = contents.lines().collect();
    let total_height = lines.len() as f32 * row_height;

    // Estimated monospace advance width. For fixed-pitch fonts (like Hack) the ratio between
    // character height and advance is ~0.55; being slightly generous avoids clipping.
    let char_width = row_height * 0.6_f32;
    let longest_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as f32;
    let gutter_width = char_width * 5.0 + ui.spacing().item_spacing.x; // "{:>4} " column
    let content_width: f32 = gutter_width + char_width * longest_chars + 16.0;

    let scroll_area = egui::ScrollArea::both().auto_shrink([false, false]);
    scroll_area.show_viewport(ui, |ui, viewport| {
        let row_width = content_width.max(viewport.width());
        ui.set_height(total_height);
        ui.set_width(row_width);
        let content_top = ui.min_rect().top();
        let content_left = ui.min_rect().left();
        // A viewport is at most a few thousand rows tall, and `max`/`min` bound both ends.
        #[expect(clippy::cast_possible_truncation)]
        let start = (viewport.min.y / row_height).floor().max(0.0) as usize;
        #[expect(clippy::cast_possible_truncation)]
        let end = ((viewport.max.y / row_height).ceil() as usize)
            .min(lines.len())
            .max(start);

        for (idx, line) in lines.iter().enumerate().take(end).skip(start) {
            #[expect(clippy::cast_possible_truncation)] // no source file has 4 billion lines
            let line_no = idx as u32 + 1;
            let y = idx as f32 * row_height;
            let row_rect = egui::Rect::from_min_size(
                egui::pos2(content_left, content_top + y),
                egui::vec2(row_width, row_height),
            );
            let is_call = Some(line_no) == call_site_line;
            let is_event = event_lines.contains(&line_no);
            let bg = if is_call {
                Some(call_bg)
            } else if is_event {
                Some(event_bg)
            } else {
                None
            };
            let mut row_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(row_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            source_line_row(&mut row_ui, line_no, line, bg, row_rect);
        }

        if scroll_pending && let Some(focus) = focus_line {
            let y = focus.saturating_sub(1) as f32 * row_height;
            let target = egui::Rect::from_min_size(
                egui::pos2(content_left, content_top + y),
                egui::vec2(1.0, row_height),
            );
            ui.scroll_to_rect(target, Some(egui::Align::Center));
        }
    });
}

fn source_line_row(
    ui: &mut egui::Ui,
    line_no: u32,
    text: &str,
    bg: Option<egui::Color32>,
    row_rect: egui::Rect,
) {
    if let Some(color) = bg {
        ui.painter().rect_filled(row_rect, 2.0, color);
    }
    ui.add(egui::Label::new(
        egui::RichText::new(format!("{line_no:>4} "))
            .monospace()
            .weak(),
    ));
    ui.add(
        egui::Label::new(egui::RichText::new(text).monospace())
            .wrap_mode(egui::TextWrapMode::Extend),
    );
}

/// Shorten a `rustc`-reported path for display — keep the last two components so we show
/// `tests/menu.rs` instead of a long absolute path, while still disambiguating.
fn shorten_path(path: &str) -> String {
    let components: Vec<&str> = path.split(['/', '\\']).collect();
    if components.len() <= 2 {
        path.to_owned()
    } else {
        let n = components.len();
        format!("{}/{}", components[n - 2], components[n - 1])
    }
}
