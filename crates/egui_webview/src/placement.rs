//! [`Placement`]: where eframe puts a native view after a pass.

use egui::{NativeViewOutput, Rect};

/// Where a page in a native view goes after this pass, and which parts of it egui covers.
///
/// All rects are in window points: egui's ui points multiplied by the zoom factor. Multiply them
/// by the native pixels-per-point to get physical pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The page's full rect, including any part that is clipped.
    pub rect: Rect,

    /// The part of `rect` inside the clip rect of its [`egui::Ui`], rounded to physical pixels.
    pub visible: Rect,

    /// The areas of `visible` that egui layers above the page (windows, popups, …) paint over.
    pub egui_paint_areas: Vec<Rect>,

    /// The areas of `visible` in which egui, not the page, receives pointer input: the
    /// interactive widgets in layers above the page, such as the backdrop of a modal.
    pub egui_input_areas: Vec<Rect>,

    /// Whether any part of `visible` receives pointer input. This is `false` when
    /// `egui_input_areas` cover all of `visible`, as under a modal.
    pub takes_input: bool,

    /// The position of the page's layer in egui's paint order. A higher index is in front.
    ///
    /// Backends stack their native views in this order, so that a page in an egui window behind
    /// another doesn't cover the page in the front window.
    pub z_index: usize,
}

impl Placement {
    /// Converts egui's output for a native view from ui points to window points.
    pub fn new(output: &NativeViewOutput, zoom_factor: f32) -> Self {
        let scale = |rects: &[Rect]| rects.iter().map(|rect| *rect * zoom_factor).collect();
        Self {
            rect: output.rect * zoom_factor,
            visible: output.visible * zoom_factor,
            egui_paint_areas: scale(&output.egui_paint_areas),
            egui_input_areas: scale(&output.egui_input_areas),
            takes_input: !output.view_input_areas().is_empty(),
            z_index: output.z_index,
        }
    }
}
