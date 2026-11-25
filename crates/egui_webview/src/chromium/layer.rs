//! The pixels of a page that CEF painted, and the egui texture they are uploaded to.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// The egui texture of the view or of its popup widget, and what CEF painted since the texture
/// was last updated.
///
/// Sizes and rects are in physical pixels.
#[derive(Default)]
pub struct Layer {
    /// The size of the latest frame.
    size: [usize; 2],
    /// What changed since the last upload: where its top-left corner is in the frame, and its
    /// pixels.
    pending: Option<([usize; 2], ColorImage)>,
    /// Whether the size changed since the last upload, so that the texture must be recreated.
    /// `pending` is the whole frame then.
    resized: bool,
    /// The egui texture that the frame is uploaded to.
    texture: Option<TextureHandle>,
}

impl std::fmt::Debug for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Layer")
            .field("size", &self.size)
            .field("pending", &self.pending)
            .field("resized", &self.resized)
            .field("texture", &self.texture.as_ref().map(|_| ()))
            .finish()
    }
}

impl Layer {
    /// Takes the `dirty` rects of a new frame from `buffer`, which holds the whole frame as BGRA
    /// pixels with premultiplied alpha. A frame of a new size replaces the whole layer.
    ///
    /// The pixels are converted straight into the image that [`Self::upload`] hands to egui, so
    /// the layer keeps no copy of the frame.
    pub fn paint(&mut self, buffer: &[u8], size: [usize; 2], dirty: &[cef::Rect]) {
        let [x, y, w, h] = if size == self.size {
            let painted = dirty
                .iter()
                .map(|rect| clamp_rect(rect, size))
                .filter(|[_, _, w, h]| 0 < w * h)
                .reduce(union);
            let Some(painted) = painted else {
                return;
            };
            // `buffer` holds all of the latest frame. So what still waits for its upload is
            // taken from it again, as one image with what changed now.
            match &self.pending {
                Some(([x, y], image)) => union(painted, [*x, *y, image.width(), image.height()]),
                None => painted,
            }
        } else {
            self.size = size;
            self.resized = true;
            [0, 0, size[0], size[1]]
        };

        let mut pixels = Vec::with_capacity(w * h);
        for row in y..y + h {
            let start = (row * size[0] + x) * 4;
            let (bgra, _) = buffer[start..start + w * 4].as_chunks();
            pixels.extend(bgra.iter().copied().map(bgra_to_color));
        }
        self.pending = Some(([x, y], ColorImage::new([w, h], pixels)));
    }

    /// Uploads what changed to the texture, and returns it. Creates the texture if it doesn't
    /// exist yet or the size changed. Returns `None` until there is a first frame.
    pub fn upload(&mut self, ctx: &egui::Context, name: &str) -> Option<&TextureHandle> {
        if let Some((pos, image)) = self.pending.take() {
            let resized = std::mem::take(&mut self.resized);
            match &mut self.texture {
                Some(texture) if !resized => {
                    texture.set_partial(pos, image, TextureOptions::NEAREST);
                }
                texture => {
                    *texture = Some(ctx.load_texture(name, image, TextureOptions::NEAREST));
                }
            }
        }
        self.texture.as_ref()
    }
}

/// A BGRA pixel with premultiplied alpha as a [`Color32`].
///
/// Swapping the red and blue bytes as one integer lets the compiler convert several pixels at
/// once. Shuffling the four bytes is 2× slower.
#[inline]
fn bgra_to_color(bgra: [u8; 4]) -> Color32 {
    let bgra = u32::from_le_bytes(bgra);
    let rgba = (bgra & 0xFF00_FF00) | (bgra.rotate_left(16) & 0x00FF_00FF);
    let [r, g, b, a] = rgba.to_le_bytes();
    Color32::from_rgba_premultiplied(r, g, b, a)
}

/// The smallest `[x, y, width, height]` that contains both `a` and `b`.
fn union([ax, ay, aw, ah]: [usize; 4], [bx, by, bw, bh]: [usize; 4]) -> [usize; 4] {
    let (x0, y0) = (ax.min(bx), ay.min(by));
    let (x1, y1) = ((ax + aw).max(bx + bw), (ay + ah).max(by + bh));
    [x0, y0, x1 - x0, y1 - y0]
}

/// `rect` as `[x, y, width, height]`, clipped to a frame of `size`.
fn clamp_rect(rect: &cef::Rect, size: [usize; 2]) -> [usize; 4] {
    let clamp = |start: i32, len: i32, max: usize| {
        let end = i64::from(start) + i64::from(len.max(0));
        let to_index = |i: i64| usize::try_from(i).unwrap_or(0).min(max);
        let (start, end) = (to_index(i64::from(start)), to_index(end));
        (start, end - start)
    };
    let (x, w) = clamp(rect.x, rect.width, size[0]);
    let (y, h) = clamp(rect.y, rect.height, size[1]);
    [x, y, w, h]
}
