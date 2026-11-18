use eframe::egui;

struct WinUtilApp;

impl eframe::App for WinUtilApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(winutil_core::APP_NAME);
        });
    }
}

fn main() -> eframe::Result {
    eframe::run_native(
        winutil_core::APP_NAME,
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(WinUtilApp))),
    )
}
