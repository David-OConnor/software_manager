use crate::State;

use egui::{Ui, CentralPanel};

pub const WINDOW_WIDTH: f32 = 800.;
pub const WINDOW_HEIGHT: f32 = 600.;

pub fn draw(state: &mut State, ui: &mut Ui) {
    CentralPanel::default().show(ui, |ui| {
        ui.label("Hellow world")
    });

}