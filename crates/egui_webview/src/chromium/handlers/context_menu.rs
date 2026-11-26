//! The page's context menu, which egui shows in place of CEF.

use std::sync::Arc;

// The `wrap_*` macros call `add_ref` from this trait.
use cef::rc::Rc as _;
use cef::{
    Browser, CefString, ContextMenuHandler, ImplContextMenuHandler, ImplContextMenuParams as _,
    ImplMenuModel as _, ImplRunContextMenuCallback as _, WrapContextMenuHandler, sys,
    wrap_context_menu_handler,
};
use egui::{Pos2, pos2};

use crate::chromium::context::Context;

/// A context menu that CEF asked to show, which egui shows instead.
pub struct ContextMenu {
    /// Where the page asked for the menu, in view points. It becomes the menu's top-left corner.
    pub pos: Pos2,
    pub items: Vec<MenuItem>,

    /// Receives the ID of the command the user picked (`cont`), or `cancel` when the menu closes
    /// without a pick. One of the two must be called.
    pub callback: cef::RunContextMenuCallback,

    /// Whether egui has shown the menu yet. Its first pass is a sizing pass, because its
    /// [`egui::Area`] still remembers the size of the previous menu.
    pub shown: bool,
}

impl std::fmt::Debug for ContextMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextMenu")
            .field("pos", &self.pos)
            .field("items", &self.items)
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

/// An item of a [`ContextMenu`].
#[derive(Debug)]
pub enum MenuItem {
    Command {
        id: i32,
        label: String,
        enabled: bool,
        checked: bool,
    },
    Separator,
}

wrap_context_menu_handler! {
    pub struct EguiContextMenuHandler {
        context: Arc<Context>,
    }

    impl ContextMenuHandler {
        /// Removes the Print and View Source items, which a windowless browser doesn't support.
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            _params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
        ) {
            if let Some(model) = model {
                for id in [sys::cef_menu_id_t::MENU_ID_PRINT, sys::cef_menu_id_t::MENU_ID_VIEW_SOURCE] {
                    model.remove(id as i32);
                }
            }
        }

        /// Hands the menu to egui, because a windowless browser can't show CEF's native context
        /// menu. A menu that is still open is canceled.
        fn run_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut cef::Frame>,
            params: Option<&mut cef::ContextMenuParams>,
            model: Option<&mut cef::MenuModel>,
            callback: Option<&mut cef::RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let (Some(params), Some(model), Some(callback)) = (params, model, callback) else {
                return 0;
            };
            #[expect(clippy::cast_precision_loss)] // View points are far below 2^24.
            let pos = pos2(params.xcoord() as f32, params.ycoord() as f32);
            let menu = ContextMenu {
                pos,
                items: menu_items(model),
                callback: callback.clone(),
                shown: false,
            };
            // Unlock the state before the call into CEF.
            let old = self.context.state.lock().context_menu.replace(menu);
            if let Some(old) = old {
                old.callback.cancel();
            }
            self.context.request_repaint();
            1
        }
    }
}

/// The items of `model`, without leading, trailing or repeated separators.
///
/// Submenus are left out, because this backend doesn't show them.
fn menu_items(model: &cef::MenuModel) -> Vec<MenuItem> {
    use sys::cef_menu_item_type_t as T;
    let mut items = Vec::new();
    for index in 0..model.count() {
        let item = match *model.type_at(index).as_ref() {
            T::MENUITEMTYPE_SEPARATOR => MenuItem::Separator,
            T::MENUITEMTYPE_SUBMENU | T::MENUITEMTYPE_NONE => continue,
            _ => MenuItem::Command {
                id: model.command_id_at(index),
                label: menu_label(&CefString::from(&model.label_at(index)).to_string()),
                enabled: model.is_enabled_at(index) == 1,
                checked: model.is_checked_at(index) == 1,
            },
        };
        // Skip a separator at the start or right after another one.
        let after_separator = items
            .last()
            .is_none_or(|last| matches!(last, MenuItem::Separator));
        if !(matches!(item, MenuItem::Separator) && after_separator) {
            items.push(item);
        }
    }
    if matches!(items.last(), Some(MenuItem::Separator)) {
        items.pop();
    }
    items
}

/// `label` without the `&` that marks its access key. `&&` stands for a literal `&`.
fn menu_label(label: &str) -> String {
    let mut text = String::with_capacity(label.len());
    let mut chars = label.chars();
    while let Some(c) = chars.next() {
        if c == '&' {
            text.extend(chars.next());
        } else {
            text.push(c);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels() {
        assert_eq!(menu_label("&Copy"), "Copy");
        assert_eq!(menu_label("Save &As"), "Save As");
        assert_eq!(menu_label("Fish && Chips"), "Fish & Chips");
        assert_eq!(menu_label("&&&x"), "&x");
        assert_eq!(menu_label("Trailing&"), "Trailing");
        assert_eq!(menu_label(""), "");
    }
}
