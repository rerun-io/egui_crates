//! Encode the frame history as a GIF and put a file reference on the system clipboard.
//! Called from a detached worker thread in `AppState::handle` so a slow
//! encode doesn't stall the UI.

use std::path::PathBuf;

use egui_kittest::inspector_api::Frame;

/// Encode the entire history as a looping GIF, write it to a timestamped file in the system
/// temp dir, and put a *file reference* for that path onto the system clipboard via arboard.
/// Pasting into Slack / Discord / GitHub / Finder etc. attaches the GIF with animation intact.
/// Mirrors the recorder's GIF behaviour: animation plays at `frame_rate`, last frame held
/// for one second so the loop point is obvious.
pub fn copy_history_as_gif(history: &[Frame], frame_rate: f32) -> Result<PathBuf, String> {
    use image::codecs::gif::{GifEncoder, Repeat};

    if history.is_empty() {
        return Err("history is empty".into());
    }
    crate::log_diag(&format!(
        "encoding {} frame(s) @ {frame_rate} fps",
        history.len()
    ));

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Stable-across-processes temp path is fine here: each invocation wants a fresh file.
    #[expect(clippy::disallowed_methods)]
    let path = std::env::temp_dir().join(format!("kittest_inspector_{ts}.gif"));
    crate::log_diag(&format!("writing to {}", path.display()));

    let file = std::fs::File::create(&path)
        .map_err(|err| format!("couldn't create {}: {err}", path.display()))?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = GifEncoder::new(writer);
    encoder
        .set_repeat(Repeat::Infinite)
        .map_err(|err| format!("set_repeat: {err}"))?;

    // The `clamp` above keeps the value in range, so the cast cannot truncate.
    #[expect(clippy::cast_possible_truncation)]
    let denom = frame_rate.max(0.1).round().clamp(1.0, u32::MAX as f32) as u32;
    let frame_delay = image::Delay::from_numer_denom_ms(1000, denom);
    let hold_delay = image::Delay::from_numer_denom_ms(1000, 1);

    let last_idx = history.len() - 1;
    for (i, frame) in history.iter().enumerate() {
        let Some(buffer) =
            image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        else {
            return Err(format!(
                "frame {i} has inconsistent rgba size for {}×{}",
                frame.width, frame.height
            ));
        };
        let delay = if i == last_idx {
            hold_delay
        } else {
            frame_delay
        };
        let anim_frame = image::Frame::from_parts(buffer, 0, 0, delay);
        encoder
            .encode_frame(anim_frame)
            .map_err(|err| format!("encode frame {i}: {err}"))?;
    }
    // Finalise the GIF write before handing the path to the clipboard.
    drop(encoder);
    crate::log_diag("GIF encoded, opening clipboard…");

    let mut clipboard =
        arboard::Clipboard::new().map_err(|err| format!("open clipboard: {err}"))?;
    crate::log_diag("clipboard opened, setting file_list…");
    clipboard
        .set()
        .file_list(&[&path])
        .map_err(|err| format!("set clipboard file list: {err}"))?;
    crate::log_diag("clipboard file_list set");

    Ok(path)
}
