//! [`WebView`], a web page you show with one call, and [`WebViews`], which owns the pages.

use std::sync::Arc;

use eframe::EmbeddedViews;
use egui::{Context, EmbeddedViewOutput, Id, Sense, Ui, Vec2, ViewportId};

use crate::instance::show_widget;
use crate::mailbox::{Command, Mailbox, ToPage, ToWidget, is_latest, now};
use crate::{Error, PageWidget, WebViewEvent, WebViewInstance, WebViewResponse, WebViewSource};

type Create = Box<dyn FnMut(&Context, Id, &WebViewSource) -> Result<WebViewInstance, Error>>;

/// Owns the pages of every [`WebView`] in an app.
///
/// Keep one in your app, and call [`Self::update`] from [`eframe::App::output_hook`]:
///
/// ```ignore
/// struct MyApp {
///     webviews: egui_webview::WebViews,
/// }
///
/// impl MyApp {
///     fn new(cc: &eframe::CreationContext<'_>) -> Self {
///         Self {
///             webviews: egui_webview::WebViews::new(&cc.egui_ctx, cc.embedded_views()),
///         }
///     }
/// }
///
/// impl eframe::App for MyApp {
///     fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
///         egui_webview::WebView::new(egui::Id::unique("docs"))
///             .url("https://docs.rs/egui")
///             .show(ui);
///     }
///
///     fn output_hook(&mut self, ctx: &egui::Context, output: &mut egui::FullOutput) {
///         self.webviews.update(ctx, output);
///     }
/// }
/// ```
///
/// Pages aren't `Send`, so they live here, in the app. The code that shows a page only needs a
/// [`Ui`]. That makes [`WebView`] usable from places that can't hold a [`WebViewInstance`].
///
/// A page lives only while it is shown. The first frame in which nothing calls
/// [`WebView::show`] or [`WebView::keep_alive`] for it, [`Self::update`] closes it.
pub struct WebViews {
    mailbox: Mailbox,
    create: Create,
    pages: Vec<Page>,
}

struct Page {
    id: Id,

    /// The source that [`WebView::show`] last asked for.
    source: Option<Arc<WebViewSource>>,

    /// The page, or why it couldn't be created. A failed page is tried again when the source
    /// changes, and not before.
    instance: Result<WebViewInstance, String>,

    /// Whether the widget still has to be told about this page.
    new: bool,
}

impl std::fmt::Debug for WebViews {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebViews")
            .field("pages", &self.pages.len())
            .finish_non_exhaustive()
    }
}

impl WebViews {
    /// Creates the registry for the [`WebView`]s of `ctx`. Pages use the platform's default
    /// backend, as with [`WebViewInstance::new`].
    pub fn new(ctx: &Context, embedded_views: &EmbeddedViews) -> Self {
        let embedded_views = embedded_views.clone();
        // On the web every page is an embedded view. The widget can reserve its rect before
        // the page exists, so the page is placed in its first frame.
        let fallback: Option<Arc<dyn PageWidget>> = if cfg!(target_arch = "wasm32") {
            Some(Arc::new(crate::EmbeddedWidget))
        } else {
            None
        };
        Self::from_parts(
            ctx,
            fallback,
            Box::new(move |ctx, id, source| WebViewInstance::new(ctx, id, source, &embedded_views)),
        )
    }

    /// Creates the registry for the [`WebView`]s of `ctx`, with pages that `create` makes.
    ///
    /// Use this for a backend of your own, with [`WebViewInstance::with_backend`].
    ///
    /// A page made this way is shown from its second frame on. The first frame only reserves
    /// its space, because the widget doesn't know the backend yet.
    pub fn with_factory(
        ctx: &Context,
        create: impl FnMut(&Context, Id, &WebViewSource) -> Result<WebViewInstance, Error> + 'static,
    ) -> Self {
        Self::from_parts(ctx, None, Box::new(create))
    }

    fn from_parts(ctx: &Context, fallback: Option<Arc<dyn PageWidget>>, create: Create) -> Self {
        let mailbox = Mailbox::new(fallback);
        mailbox.install(ctx);
        Self {
            mailbox,
            create,
            pages: Vec::new(),
        }
    }

    /// Does everything the pages need after a pass. Call this from
    /// [`eframe::App::output_hook`], in every frame.
    ///
    /// It creates the pages of new [`WebView`]s, runs their commands, moves the keyboard focus,
    /// puts every page where egui put its rect, and closes the pages that were neither shown
    /// nor kept alive.
    pub fn update(&mut self, ctx: &Context, output: &egui::FullOutput) {
        let views = &output.platform_output.embedded_views;

        // Put the mailbox back in every frame. If something reset egui's memory, the mailbox
        // went with it. The widgets couldn't find it in this frame, so their pages now count
        // as not shown and are closed below. They are created again in the next frame.
        self.mailbox.install(ctx);

        let requests = self.mailbox.with(|inner| {
            inner
                .slots
                .iter_mut()
                .map(|(id, slot)| (*id, slot.to_page.take()))
                .collect::<Vec<_>>()
        });

        let mut closed = Vec::new();
        for (id, request) in requests {
            if !request.alive.is_some_and(|pass| is_latest(ctx, pass)) {
                closed.push(id);
                continue;
            }
            let shown = request.shown.is_some_and(|pass| is_latest(ctx, pass));

            let news = self.update_page(ctx, id, request, shown, views);
            let repaint = self.mailbox.with(|inner| {
                let slot = inner.existing_slot(id);
                slot.is_some_and(|slot| slot.to_widget.merge(news, shown))
            });
            if repaint {
                // The widget reads the mailbox in its next pass, so make sure there is one.
                ctx.request_repaint_of(ViewportId::ROOT);
            }
        }

        if !closed.is_empty() {
            // Dropping a page closes it.
            self.pages.retain(|page| !closed.contains(&page.id));
            self.mailbox
                .with(|inner| inner.slots.retain(|(id, _)| !closed.contains(id)));
        }
    }

    /// Does one page's work for this frame. `shown` is whether its widget was shown in the
    /// latest pass.
    fn update_page(
        &mut self,
        ctx: &Context,
        id: Id,
        request: ToPage,
        shown: bool,
        views: &[EmbeddedViewOutput],
    ) -> ToWidget {
        let Self { create, pages, .. } = self;
        let ToPage {
            commands,
            focus,
            source,
            focused,
            ..
        } = request;
        // What this frame has to tell the widget.
        let mut news = ToWidget::default();

        let index = pages.iter().position(|page| page.id == id);
        let page = if let Some(index) = index {
            let page = &mut pages[index];
            if source.is_some() && source != page.source {
                // The builder was told to show something else.
                if let Ok(instance) = &mut page.instance {
                    if let Some(source) = &source {
                        warn(instance.load(source));
                    }
                } else {
                    page.instance = create_page(create, ctx, id, source.as_deref());
                    page.new = true;
                }
                page.source = source;
            }
            page
        } else {
            pages.push(Page {
                id,
                instance: create_page(create, ctx, id, source.as_deref()),
                source,
                new: true,
            });
            pages.last_mut().expect("just pushed")
        };

        let instance = match &mut page.instance {
            Ok(instance) => instance,
            Err(failed) => {
                news.failed = Some(failed.clone());
                return news;
            }
        };

        for command in commands {
            warn(match command {
                Command::Load(source) => instance.load(&source),
                Command::EvaluateScript(script) => instance.evaluate_script(&script),
                Command::Reload => {
                    instance.reload();
                    Ok(())
                }
                Command::Back => {
                    instance.back();
                    Ok(())
                }
                Command::Forward => {
                    instance.forward();
                    Ok(())
                }
            });
        }

        // The order matters here. First tell the page what the widget saw in the pass, and only
        // then ask the page for news. Asked first, a page that just got the focus from egui
        // would look like it took the focus by itself.
        if let Some(change) = focus {
            instance.apply_focus(change);
        }
        instance.blur_unless(shown && focused);
        news.focus = instance.poll_focus();
        news.events = instance.take_events();

        instance.place(ctx, views);

        if std::mem::take(&mut page.new) {
            news.widget = Some(instance.widget());
        }
        news
    }
}

/// Creates a page. A [`WebView`] without a source gets an empty page.
fn create_page(
    create: &mut Create,
    ctx: &Context,
    id: Id,
    source: Option<&WebViewSource>,
) -> Result<WebViewInstance, String> {
    let empty = WebViewSource::Html(String::new());
    create(ctx, id, source.unwrap_or(&empty)).map_err(|err| {
        log::warn!("egui_webview: failed to create the page of {id:?}: {err}");
        err.to_string()
    })
}

fn warn(result: Result<(), Error>) {
    if let Err(err) = result {
        log::warn!("egui_webview: {err}");
    }
}

/// A web page, shown with one call:
///
/// ```ignore
/// let response = egui_webview::WebView::new(egui::Id::unique("docs"))
///     .url("https://docs.rs/egui")
///     .show(ui);
/// ```
///
/// Use it when the code that shows the page can't keep a [`WebViewInstance`]. The pages live in
/// the app's [`WebViews`], which must exist, or this only shows an error text.
///
/// The id is global, like the id of an [`egui::Window`]. It doesn't depend on the [`Ui`]. So
/// the same id shows the same page wherever you call this.
///
/// A page lives only while it is shown. If no [`Self::show`] and no [`Self::keep_alive`] runs
/// for its id in a frame, the page is closed, and showing it again loads it from scratch.
///
/// Only show it in the root viewport. Other viewports aren't supported yet.
///
/// Everything here takes effect after the pass, when the app calls [`WebViews::update`]. So
/// what the page does in return, such as its events, arrives in the next pass.
#[must_use = "Call `show` to add the page to the ui"]
#[derive(Clone, Copy, Debug)]
pub struct WebView<'a> {
    id: Id,
    source: Option<SourceRef<'a>>,
    size: Option<Vec2>,
}

/// A [`WebViewSource`] that borrows its text.
#[derive(Clone, Copy, Debug)]
enum SourceRef<'a> {
    Url(&'a str),
    Html(&'a str),
}

impl SourceRef<'_> {
    fn is(self, source: &WebViewSource) -> bool {
        match (self, source) {
            (Self::Url(a), WebViewSource::Url(b)) | (Self::Html(a), WebViewSource::Html(b)) => {
                a == b
            }
            _ => false,
        }
    }

    fn to_source(self) -> WebViewSource {
        match self {
            Self::Url(url) => WebViewSource::Url(url.to_owned()),
            Self::Html(html) => WebViewSource::Html(html.to_owned()),
        }
    }
}

impl<'a> WebView<'a> {
    /// A webview with the given id. Without [`Self::url`] or [`Self::html`] it shows an empty
    /// page.
    pub fn new(id: impl Into<Id>) -> Self {
        Self {
            id: id.into(),
            source: None,
            size: None,
        }
    }

    /// Shows the page at `url`.
    ///
    /// The page is loaded when the value you pass changes. It isn't loaded again when the user
    /// navigates somewhere else in the page. Use [`Self::load_url`] for that.
    #[inline]
    pub fn url(mut self, url: &'a str) -> Self {
        self.source = Some(SourceRef::Url(url));
        self
    }

    /// Shows an HTML document, given as its source text.
    ///
    /// The page is loaded when the value you pass changes.
    #[inline]
    pub fn html(mut self, html: &'a str) -> Self {
        self.source = Some(SourceRef::Html(html));
        self
    }

    /// Shows `source`. See [`Self::url`] and [`Self::html`].
    #[inline]
    pub fn source(self, source: &'a WebViewSource) -> Self {
        match source {
            WebViewSource::Url(url) => self.url(url),
            WebViewSource::Html(html) => self.html(html),
        }
    }

    /// The size of the page's rect. Default: all of the available space.
    #[inline]
    pub fn size(mut self, size: Vec2) -> Self {
        self.size = Some(size);
        self
    }

    /// Shows the page, and returns the page's events since the last call.
    ///
    /// The page takes part in egui's keyboard focus, as with [`WebViewInstance::ui`].
    ///
    /// If the page can't be created, this shows the error as text in the page's rect.
    pub fn show(self, ui: &mut Ui) -> WebViewResponse {
        let Self { id, source, size } = self;
        let size = size.unwrap_or_else(|| ui.available_size());

        let Some(mailbox) = Mailbox::of(ui.ctx()) else {
            let text = "This app has no `egui_webview::WebViews`";
            return placeholder(ui, id, size, Some(text), Vec::new());
        };
        let at = now(ui.ctx());

        let (widget, events, page_focus, failed) = mailbox.with(|inner| {
            let fallback = inner.fallback.clone();
            let slot = inner.slot(id);

            let to_page = &mut slot.to_page;
            to_page.alive = Some(at);
            match source {
                None => to_page.source = None,
                Some(source) => {
                    // Only copy the text when it changed.
                    let same = to_page.source.as_deref().is_some_and(|old| source.is(old));
                    if !same {
                        to_page.source = Some(Arc::new(source.to_source()));
                    }
                }
            }

            let to_widget = &mut slot.to_widget;
            (
                to_widget.widget.clone().or(fallback),
                std::mem::take(&mut to_widget.events),
                to_widget.focus.take(),
                to_widget.failed.clone(),
            )
        });

        if let Some(failed) = failed {
            return placeholder(ui, id, size, Some(&failed), events);
        }
        let Some(widget) = widget else {
            // The page doesn't exist yet. `WebViews::update` creates it after this pass.
            return placeholder(ui, id, size, None, events);
        };

        let shown = show_widget(ui, id, size, &*widget, page_focus);

        mailbox.with(|inner| {
            let to_page = &mut inner.slot(id).to_page;
            to_page.shown = Some(at);
            to_page.focused = shown.focused;
            if let Some(change) = shown.focus_change {
                to_page.focus = Some(change);
            }
        });

        WebViewResponse {
            events,
            response: shown.response,
        }
    }
}

/// These work on a page by its id, so you don't need the [`WebViewResponse`] for them.
///
/// They do nothing until [`Self::show`] has run for that id, and after the page is closed.
impl WebView<'_> {
    /// Keeps the page of `id` for this frame without showing it.
    ///
    /// Call this in every frame in which the page is hidden but should keep its state, such as
    /// in a tab that isn't selected.
    pub fn keep_alive(ctx: &Context, id: impl Into<Id>) {
        let at = now(ctx);
        if let Some(mailbox) = Mailbox::of(ctx) {
            mailbox.with(|inner| {
                if let Some(slot) = inner.existing_slot(id.into()) {
                    slot.to_page.alive = Some(at);
                }
            });
        }
    }

    /// Loads `source` in the page of `id`, replacing the current page.
    pub fn load(ctx: &Context, id: impl Into<Id>, source: WebViewSource) {
        command(ctx, id.into(), Command::Load(source));
    }

    /// Loads the page at `url` in the page of `id`.
    pub fn load_url(ctx: &Context, id: impl Into<Id>, url: impl Into<String>) {
        Self::load(ctx, id, WebViewSource::Url(url.into()));
    }

    /// Runs `script` in the page of `id`. See [`WebViewInstance::evaluate_script`].
    pub fn evaluate_script(ctx: &Context, id: impl Into<Id>, script: impl Into<String>) {
        command(ctx, id.into(), Command::EvaluateScript(script.into()));
    }

    /// Reloads the page of `id`.
    pub fn reload(ctx: &Context, id: impl Into<Id>) {
        command(ctx, id.into(), Command::Reload);
    }

    /// Navigates back in the history of the page of `id`. See [`WebViewInstance::back`].
    pub fn back(ctx: &Context, id: impl Into<Id>) {
        command(ctx, id.into(), Command::Back);
    }

    /// Navigates forward in the history of the page of `id`.
    pub fn forward(ctx: &Context, id: impl Into<Id>) {
        command(ctx, id.into(), Command::Forward);
    }
}

fn command(ctx: &Context, id: Id, command: Command) {
    if let Some(mailbox) = Mailbox::of(ctx) {
        mailbox.with(|inner| {
            if let Some(slot) = inner.existing_slot(id) {
                slot.to_page.commands.push(command);
            }
        });
    }
}

/// Allocates the page's space without showing a page, with `text` in its place.
fn placeholder(
    ui: &mut Ui,
    id: Id,
    size: Vec2,
    text: Option<&str>,
    events: Vec<WebViewEvent>,
) -> WebViewResponse {
    let (_, rect) = ui.allocate_space(size);
    let response = ui.interact(rect, id, Sense::hover());
    if let Some(text) = text
        && ui.is_rect_visible(rect)
    {
        ui.painter_at(rect).text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::TextStyle::Body.resolve(ui.style()),
            ui.visuals().error_fg_color,
        );
    }
    WebViewResponse { events, response }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use egui::{EmbeddedViewOutput, FocusDirection, vec2};

    use super::*;
    use crate::{Backend, EmbeddedWidget};

    type Log = Rc<RefCell<Vec<String>>>;

    /// A page that only writes down what it is told.
    struct FakeBackend {
        log: Log,
        events: Rc<RefCell<Vec<WebViewEvent>>>,
    }

    impl Backend for FakeBackend {
        fn load(&mut self, source: &WebViewSource) -> Result<(), Error> {
            self.log.borrow_mut().push(format!("load {source:?}"));
            Ok(())
        }

        fn evaluate_script(&mut self, script: &str) -> Result<(), Error> {
            self.log.borrow_mut().push(format!("script {script}"));
            Ok(())
        }

        fn reload(&mut self) {
            self.log.borrow_mut().push("reload".to_owned());
        }

        fn back(&mut self) {
            self.log.borrow_mut().push("back".to_owned());
        }

        fn forward(&mut self) {
            self.log.borrow_mut().push("forward".to_owned());
        }

        fn widget(&self) -> Arc<dyn PageWidget> {
            Arc::new(EmbeddedWidget)
        }

        fn place(&mut self, _ctx: &Context, output: Option<&EmbeddedViewOutput>) {
            let what = if output.is_some() { "place" } else { "hide" };
            self.log.borrow_mut().push(what.to_owned());
        }

        fn focus(&mut self, direction: FocusDirection) {
            self.log.borrow_mut().push(format!("focus {direction:?}"));
        }

        fn blur(&mut self) {
            self.log.borrow_mut().push("blur".to_owned());
        }

        fn take_events(&mut self) -> Vec<WebViewEvent> {
            std::mem::take(&mut self.events.borrow_mut())
        }
    }

    impl Drop for FakeBackend {
        fn drop(&mut self) {
            self.log.borrow_mut().push("close".to_owned());
        }
    }

    struct Harness {
        ctx: Context,
        webviews: WebViews,
        log: Log,
        events: Rc<RefCell<Vec<WebViewEvent>>>,
    }

    impl Harness {
        /// `fail` makes every attempt to create a page fail.
        fn new(fail: bool) -> Self {
            let ctx = Context::default();
            let log = Log::default();
            let events = Rc::<RefCell<Vec<WebViewEvent>>>::default();
            let create: Create = {
                let (log, events) = (Rc::clone(&log), Rc::clone(&events));
                Box::new(move |_ctx, id, source| {
                    log.borrow_mut().push(format!("create {source:?}"));
                    if fail {
                        return Err(Error::Unsupported);
                    }
                    let backend = FakeBackend {
                        log: Rc::clone(&log),
                        events: Rc::clone(&events),
                    };
                    Ok(WebViewInstance::with_backend(id, backend))
                })
            };
            let webviews = WebViews::from_parts(&ctx, Some(Arc::new(EmbeddedWidget)), create);
            Self {
                ctx,
                webviews,
                log,
                events,
            }
        }

        /// Runs one frame: a pass with `add`, then [`WebViews::update`].
        fn frame(&mut self, events: Vec<egui::Event>, mut add: impl FnMut(&mut Ui)) {
            let input = egui::RawInput {
                events,
                ..Default::default()
            };
            let mut output = self.ctx.run_ui(input, |ui| add(ui));
            self.webviews.update(&self.ctx, &output);
            // Nothing paints here, and egui panics when a texture delta is dropped unused.
            output.textures_delta.clear();
        }

        /// What the factory and the pages were told since the last call.
        fn log(&self) -> Vec<String> {
            std::mem::take(&mut self.log.borrow_mut())
        }
    }

    fn id() -> Id {
        Id::unique("page")
    }

    fn show(ui: &mut Ui, url: &str) -> WebViewResponse {
        WebView::new(id())
            .url(url)
            .size(vec2(100.0, 100.0))
            .show(ui)
    }

    fn tab() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_page_is_created_and_placed_in_its_first_frame() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["create Url(\"a\")", "place"]);
    }

    #[test]
    fn a_page_that_is_not_shown_is_closed() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.log();

        harness.frame(vec![], |_ui| {});
        assert_eq!(harness.log(), ["close"]);

        // Showing it again starts from scratch.
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["create Url(\"a\")", "place"]);
    }

    #[test]
    fn keep_alive_keeps_a_hidden_page() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.log();

        harness.frame(vec![], |ui| WebView::keep_alive(ui.ctx(), id()));
        assert_eq!(harness.log(), ["hide"]);

        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["place"]);
    }

    #[test]
    fn commands_reach_the_page() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.log();

        // Before the page is shown in the pass, and without a response.
        harness.frame(vec![], |ui| {
            WebView::reload(ui.ctx(), id());
            WebView::evaluate_script(ui.ctx(), id(), "1 + 1");
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["reload", "script 1 + 1", "place"]);
    }

    #[test]
    fn a_command_for_a_page_that_is_not_shown_does_nothing() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| WebView::reload(ui.ctx(), id()));
        assert!(harness.log().is_empty());

        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["create Url(\"a\")", "place"]);
    }

    #[test]
    fn a_new_source_loads_and_the_same_one_does_not() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.log();

        harness.frame(vec![], |ui| {
            show(ui, "b");
        });
        assert_eq!(harness.log(), ["load Url(\"b\")", "place"]);
    }

    #[test]
    fn events_arrive_in_the_next_pass() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });

        harness
            .events
            .borrow_mut()
            .push(WebViewEvent::Message("hi".to_owned()));
        let mut seen = Vec::new();
        // This frame's `update` picks the event up, so the frame after it shows it.
        for _ in 0..2 {
            harness.frame(vec![], |ui| seen.push(show(ui, "a").events));
        }
        assert_eq!(seen[0], []);
        assert_eq!(seen[1], [WebViewEvent::Message("hi".to_owned())]);
    }

    #[test]
    fn tab_moves_the_focus_into_the_page_and_out_again() {
        let mut harness = Harness::new(false);
        let mut ui = |ui: &mut Ui| {
            _ = ui.button("Before");
            show(ui, "a");
        };
        harness.frame(vec![], &mut ui);
        harness.log();

        // The first Tab goes to the button, the second to the page.
        harness.frame(vec![tab()], &mut ui);
        assert_eq!(harness.log(), ["place"]);
        harness.frame(vec![tab()], &mut ui);
        assert_eq!(harness.log(), ["focus Next", "place"]);

        // The third Tab wraps around to the button, so the page is blurred.
        harness.frame(vec![tab()], &mut ui);
        harness.frame(vec![], &mut ui);
        assert!(harness.log().contains(&"blur".to_owned()));
    }

    #[test]
    fn a_page_that_cannot_be_created_is_tried_once_per_source() {
        let mut harness = Harness::new(true);
        for _ in 0..3 {
            harness.frame(vec![], |ui| {
                show(ui, "a");
            });
        }
        assert_eq!(harness.log(), ["create Url(\"a\")"]);

        harness.frame(vec![], |ui| {
            show(ui, "b");
        });
        assert_eq!(harness.log(), ["create Url(\"b\")"]);
    }

    #[test]
    fn a_memory_reset_closes_the_pages() {
        let mut harness = Harness::new(false);
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        harness.log();

        // The widget finds no mailbox in this frame, so it can't say that it is shown.
        harness
            .ctx
            .memory_mut(|memory| *memory = Default::default());
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["close"]);

        // The mailbox is back, so the page is created again.
        harness.frame(vec![], |ui| {
            show(ui, "a");
        });
        assert_eq!(harness.log(), ["create Url(\"a\")", "place"]);
    }
}
