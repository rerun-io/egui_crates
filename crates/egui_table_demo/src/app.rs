#[derive(Default, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
enum Demo {
    #[default]
    Table,
    Scroll,
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
#[serde(default)]
pub struct DemoApp {
    demo: Demo,

    table_demo: crate::table_demo::TableDemo,
    scroll_demo: crate::split_scroll_demo::SplitScrollDemo,
}

impl DemoApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        if let Some(storage) = cc.storage {
            return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        }

        Default::default()
    }
}

impl eframe::App for DemoApp {
    /// Called by the frame work to save state before shutdown.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, eframe::APP_KEY, self);
    }

    /// Called each time the UI needs repainting, which may be many times per second.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("top_panel").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                egui::widgets::global_theme_preference_buttons(ui);
                ui.add_space(16.0);

                ui.horizontal(|ui| {
                    ui.label("Demo:");
                    ui.radio_value(&mut self.demo, Demo::Table, "Table");
                    ui.radio_value(&mut self.demo, Demo::Scroll, "Scroll");
                });

                ui.add_space(16.0);

                use egui::special_emojis::GITHUB;
                ui.hyperlink_to(
                    format!("{GITHUB} egui_table on GitHub"),
                    "https://github.com/rerun-io/egui_table",
                );
            });
        });

        egui::CentralPanel::default().show(ui, |ui| match self.demo {
            Demo::Table => self.table_demo.ui(ui),
            Demo::Scroll => self.scroll_demo.ui(ui),
        });
    }
}
