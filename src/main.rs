use std::{
    fmt::{self, Display},
    sync::{Arc, LazyLock},
};

use crate::ui::{WINDOW_HEIGHT, WINDOW_WIDTH};

mod ui;

#[derive(Default)]
struct State {}

impl eframe::App for State {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::draw(self, ui);
    }
}

fn app_icon() -> Arc<egui::IconData> {
    static ICON: LazyLock<Arc<egui::IconData>> = LazyLock::new(|| {
        let png = include_bytes!("../resources/icon.png");
        Arc::new(eframe::icon_data::from_png_bytes(png).expect("resources/icon.png is a valid PNG"))
    });
    Arc::clone(&ICON)
}

fn main() {
    let state = State::default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([WINDOW_WIDTH, WINDOW_HEIGHT])
            .with_icon(app_icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Software manager",
        options,
        Box::new(|cc| {
            // ui::install_fonts(&cc.egui_ctx);
            // Without a saved choice, egui follows the OS theme.
            // if let Some(dark) = state.dark_mode {
            //     cc.egui_ctx.set_theme(state::theme(dark));
            // }
            Ok(Box::new(state))
        }),
    )
    .unwrap();
}
