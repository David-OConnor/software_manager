use egui::{CentralPanel, Panel, RichText, Ui};

use crate::{Application, State};

pub const WINDOW_WIDTH: f32 = 800.;
pub const WINDOW_HEIGHT: f32 = 600.;

pub fn draw(state: &mut State, ui: &mut Ui) {
    ui.heading(RichText::new("Software applications"));

    ui.add_space(20.);

    for app in [
        Application::Molchanica,
        Application::PlasCad,
        Application::Peregrine,
        Application::Peregrine,
        Application::SoftwareManager,
    ] {
        ui.label(format!("- {app}"));
    }

    // Panel::top("main").resizable(false).show(ui, |ui| {
    // });
}
