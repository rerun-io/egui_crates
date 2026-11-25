//! Gives pages egui's look where they set no style of their own.

use std::fmt::Write as _;

use egui::{Color32, Context, FontDefinitions, FontFamily, FontId, Style, TextStyle, ViewportId};

/// The id of the `<style>` element that holds the [`PageStyle`].
const ELEMENT_ID: &str = "egui-webview-style";

/// The cascade layer of the [`PageStyle`].
const LAYER: &str = "egui-webview";

/// How a page looks where it sets no style of its own.
///
/// Set it with [`crate::WebViewInstance::set_page_style`] or [`crate::WebView::style`].
///
/// The style goes in a cascade layer at the start of each document, so every style of the page
/// itself wins over it, whatever its specificity. It only applies to the page's main frame.
///
/// The style is a `<style>` element that a script adds. A page whose content security policy
/// forbids inline styles doesn't get it. On the web, neither does a cross-origin page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PageStyle {
    /// The browser's defaults: a serif font of 16 pixels, and a white or dark background that
    /// follows egui's theme.
    #[default]
    Browser,

    /// egui's style, as [`style_sheet`] translates it. The page follows changes to the style.
    Egui,

    /// This CSS.
    ///
    /// Start from [`style_sheet`] to extend egui's style. The page doesn't follow changes to
    /// egui's style then, unless you set the new CSS.
    Css(String),
}

impl PageStyle {
    /// The CSS of this style now, if it has any.
    pub(crate) fn css(&self, ctx: &Context) -> Option<String> {
        match self {
            Self::Browser => None,
            Self::Egui => Some(style_sheet(ctx)),
            Self::Css(css) => Some(css.clone()),
        }
    }
}

/// The global style of `ctx` as CSS: the background, text, link and selection colors, and the
/// font families and sizes of [`TextStyle::Body`] and [`TextStyle::Monospace`].
///
/// One ui point is one CSS pixel, since pages zoom together with egui.
///
/// The page gets the names of egui's fonts, not the fonts themselves. So it only shows
/// a font that is installed on the system, and that the engine lets pages use. Otherwise it
/// falls back to the system's default font.
pub fn style_sheet(ctx: &Context) -> String {
    let style = ctx.global_style();
    if 0 < ctx.cumulative_pass_nr_for(ViewportId::ROOT) {
        ctx.fonts(|fonts| style_sheet_of(&style, fonts.definitions()))
    } else {
        // egui has no fonts before its first pass, so guess that it will use the default ones.
        style_sheet_of(&style, &FontDefinitions::default())
    }
}

fn style_sheet_of(style: &Style, fonts: &FontDefinitions) -> String {
    let visuals = &style.visuals;
    let body = style.text_styles.get(&TextStyle::Body);
    let monospace = style.text_styles.get(&TextStyle::Monospace);

    let mut css = String::new();

    let mut font_names: Vec<&str> = [body, monospace]
        .into_iter()
        .flatten()
        .flat_map(|font| font_names(fonts, &font.family))
        .collect();
    font_names.sort_unstable();
    font_names.dedup();
    for name in font_names {
        // `font-family` matches family names, and `local()` matches full and `PostScript` names.
        // egui's fonts are named freely, so try the name as both.
        writeln!(
            css,
            "@font-face {{ font-family: {}; src: local({}); }}",
            css_string(&local_alias(name)),
            css_string(name),
        )
        .ok();
    }

    let color_scheme = if visuals.dark_mode { "dark" } else { "light" };
    writeln!(css, ":root {{").ok();
    writeln!(css, "  color-scheme: {color_scheme};").ok();
    writeln!(css, "  background-color: {};", color(visuals.panel_fill)).ok();
    writeln!(css, "  color: {};", color(visuals.text_color())).ok();
    writeln!(css, "  accent-color: {};", color(visuals.selection.bg_fill)).ok();
    if let Some(body) = body {
        write_font(&mut css, "  ", fonts, body);
    }
    writeln!(css, "}}").ok();

    writeln!(
        css,
        "a:any-link {{ color: {}; }}",
        color(visuals.hyperlink_color)
    )
    .ok();
    writeln!(
        css,
        "::selection {{ background-color: {}; }}",
        color(visuals.selection.bg_fill)
    )
    .ok();
    if let Some(monospace) = monospace {
        writeln!(css, "code, kbd, pre, samp {{").ok();
        write_font(&mut css, "  ", fonts, monospace);
        writeln!(css, "}}").ok();
    }
    // Browsers give form controls a font of their own.
    writeln!(css, "button, input, select, textarea {{ font: inherit; }}").ok();

    css
}

/// The names that egui has for the fonts of `family`, most important first.
fn font_names<'a>(
    fonts: &'a FontDefinitions,
    family: &FontFamily,
) -> impl Iterator<Item = &'a str> + use<'a> {
    fonts
        .families
        .get(family)
        .into_iter()
        .flatten()
        .map(String::as_str)
}

/// The `@font-face` family that stands for the installed font whose full or `PostScript` name
/// is `name`.
fn local_alias(name: &str) -> String {
    format!("egui {name}")
}

/// Writes the `font-family` and `font-size` of `font`.
fn write_font(css: &mut String, indent: &str, fonts: &FontDefinitions, font: &FontId) {
    let FontId { size, family } = font;
    write!(css, "{indent}font-family: ").ok();
    for name in font_names(fonts, family) {
        write!(
            css,
            "{}, {}, ",
            css_string(&local_alias(name)),
            css_string(name)
        )
        .ok();
    }
    let generic = match family {
        FontFamily::Monospace => "ui-monospace, monospace",
        FontFamily::Proportional | FontFamily::Name(_) => "system-ui, sans-serif",
    };
    writeln!(css, "{generic};").ok();
    writeln!(css, "{indent}font-size: {size}px;").ok();
}

fn color(color: Color32) -> String {
    color.to_hex()
}

/// `text` as a CSS string.
fn css_string(text: &str) -> String {
    let mut string = String::with_capacity(text.len() + 2);
    string.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                string.push('\\');
                string.push(c);
            }
            // A CSS string can't span lines, and the escape of a code point ends with a space.
            c if c.is_control() => {
                write!(string, "\\{:x} ", u32::from(c)).ok();
            }
            c => string.push(c),
        }
    }
    string.push('"');
    string
}

/// `text` as a `JavaScript` string literal, which is also safe inside a `<script>` element.
fn js_string(text: &str) -> String {
    let mut string = String::with_capacity(text.len() + 2);
    string.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                string.push('\\');
                string.push(c);
            }
            // `<` could end a `<script>` element, and the separators end a line in JavaScript.
            c if matches!(c, '<' | '\u{2028}' | '\u{2029}') || c.is_control() => {
                write!(string, "\\u{:04x}", u32::from(c)).ok();
            }
            c => string.push(c),
        }
    }
    string.push('"');
    string
}

/// A script that gives the current document the style sheet `css`, in place of the one from
/// an earlier run. `None` removes the style sheet.
///
/// The script works at any time in the life of a document, including before the document has
/// its root element.
pub(crate) fn script(css: Option<&str>) -> String {
    let id = js_string(ELEMENT_ID);
    let Some(css) = css else {
        return format!("document.getElementById({id})?.remove();");
    };
    // The layer is the first of the document, so the page's own layers win over it too.
    let css = js_string(&format!("@layer {LAYER} {{\n{css}\n}}"));
    format!(
        r"(() => {{
    const css = {css};
    const apply = () => {{
        let style = document.getElementById({id});
        if (!style) {{
            style = document.createElement('style');
            style.id = {id};
            (document.head ?? document.documentElement).prepend(style);
        }}
        if (style.textContent !== css) {{
            style.textContent = css;
        }}
    }};
    if (document.documentElement) {{
        apply();
    }} else {{
        new MutationObserver((_, observer) => {{
            if (document.documentElement) {{
                observer.disconnect();
                apply();
            }}
        }}).observe(document, {{ childList: true }});
    }}
}})();"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_cannot_escape_their_quotes() {
        assert_eq!(css_string("a\"b\\c\nd"), r#""a\"b\\c\a d""#);
        assert_eq!(
            js_string("</script>\"\\\n\u{2028}"),
            r#""\u003c/script>\"\\\u000a\u2028""#
        );
    }

    #[test]
    fn fonts_are_tried_by_both_kinds_of_name() {
        let mut fonts = FontDefinitions::empty();
        fonts.families.insert(
            FontFamily::Proportional,
            vec!["Ubuntu-Light".to_owned(), "Emoji".to_owned()],
        );
        // Monospace falls back to a font that the proportional family also uses.
        fonts.families.insert(
            FontFamily::Monospace,
            vec!["Hack".to_owned(), "Ubuntu-Light".to_owned()],
        );
        let css = style_sheet_of(&Style::default(), &fonts);

        assert_eq!(
            css.matches("@font-face").count(),
            3,
            "One per font, not per use:\n{css}"
        );
        assert!(
            css.contains(
                r#"font-family: "egui Ubuntu-Light", "Ubuntu-Light", "egui Emoji", "Emoji", system-ui, sans-serif;"#
            ),
            "{css}"
        );
        assert!(
            css.contains(
                r#"font-family: "egui Hack", "Hack", "egui Ubuntu-Light", "Ubuntu-Light", ui-monospace, monospace;"#
            ),
            "{css}"
        );
    }

    #[test]
    fn a_style_without_fonts_still_has_colors() {
        let mut style = Style::default();
        style.text_styles.clear();
        let css = style_sheet_of(&style, &FontDefinitions::empty());
        assert!(!css.contains("font-family"), "{css}");
        assert!(css.contains("background-color"), "{css}");
    }

    #[test]
    fn no_style_removes_the_element() {
        assert!(script(None).contains("remove()"));
        assert!(script(Some("</style>")).contains(r"\u003c/style>"));
    }
}
