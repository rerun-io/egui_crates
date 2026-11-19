//! The pixels of a page that CEF painted, and the egui texture they are uploaded to.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// The latest frame of the view or of its popup widget, as BGRA pixels with premultiplied alpha.
///
/// Sizes and rects are in physical pixels.
#[derive(Default)]
pub struct Layer {
    size: [usize; 2],
    bgra: Vec<u8>,
    /// The bounding rect of what changed since the last upload, as `[x, y, width, height]`.
    dirty: Option<[usize; 4]>,
    /// Whether the size changed since the last upload, so that the texture must be recreated.
    resized: bool,
    /// The egui texture that the frame is uploaded to.
    texture: Option<TextureHandle>,
}

impl Layer {
    /// Copies the `dirty` rects of a new frame from `buffer`. A frame of a new size replaces the
    /// whole layer.
    pub fn paint(&mut self, buffer: &[u8], size: [usize; 2], dirty: &[cef::Rect]) {
        if size != self.size {
            self.size = size;
            // Keep the allocation. A resize by dragging sends a new size in every frame.
            self.bgra.clear();
            self.bgra.extend_from_slice(buffer);
            self.resized = true;
            self.dirty = Some([0, 0, size[0], size[1]]);
            return;
        }
        let stride = size[0] * 4;
        for rect in dirty {
            let [x, y, w, h] = clamp_rect(rect, size);
            for row in y..y + h {
                let range = row * stride + x * 4..row * stride + (x + w) * 4;
                self.bgra[range.clone()].copy_from_slice(&buffer[range]);
            }
            self.dirty = Some(match self.dirty {
                None => [x, y, w, h],
                Some([dx, dy, dw, dh]) => {
                    let (x0, y0) = (dx.min(x), dy.min(y));
                    let (x1, y1) = ((dx + dw).max(x + w), (dy + dh).max(y + h));
                    [x0, y0, x1 - x0, y1 - y0]
                }
            });
        }
    }

    /// Uploads what changed to the texture, and returns it. Creates the texture if it doesn't
    /// exist yet or the size changed. Returns `None` until there is a first frame.
    pub fn upload(&mut self, ctx: &egui::Context, name: &str) -> Option<&TextureHandle> {
        let Some([x, y, w, h]) = self.dirty.take() else {
            return self.texture.as_ref();
        };
        let stride = self.size[0] * 4;
        let image = |x: usize, y: usize, w: usize, h: usize| {
            let mut pixels = Vec::with_capacity(w * h);
            for row in y..y + h {
                let start = row * stride + x * 4;
                pixels.extend(
                    self.bgra[start..start + w * 4]
                        .chunks_exact(4)
                        .map(|p| Color32::from_rgba_premultiplied(p[2], p[1], p[0], p[3])),
                );
            }
            ColorImage::new([w, h], pixels)
        };
        match &mut self.texture {
            Some(texture) if !std::mem::take(&mut self.resized) => {
                texture.set_partial([x, y], image(x, y, w, h), TextureOptions::NEAREST);
            }
            texture => {
                *texture = Some(ctx.load_texture(
                    name,
                    image(0, 0, self.size[0], self.size[1]),
                    TextureOptions::NEAREST,
                ));
            }
        }
        self.texture.as_ref()
    }
}

/// `rect` as `[x, y, width, height]`, clipped to a frame of `size`.
fn clamp_rect(rect: &cef::Rect, size: [usize; 2]) -> [usize; 4] {
    let x = usize::try_from(rect.x).unwrap_or(0).min(size[0]);
    let y = usize::try_from(rect.y).unwrap_or(0).min(size[1]);
    let w = usize::try_from(rect.width).unwrap_or(0).min(size[0] - x);
    let h = usize::try_from(rect.height).unwrap_or(0).min(size[1] - y);
    [x, y, w, h]
}
