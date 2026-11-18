mod install_tab;

use eframe::egui;
use install_tab::InstallTab;

struct WinUtilApp {
    install: InstallTab,
}

impl eframe::App for WinUtilApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(winutil_core::APP_NAME);
            self.install.ui(ui);
        });
    }
}

fn main() -> eframe::Result {
    // The config is embedded at compile time and checked by the core tests, so a parse failure
    // here means a broken build rather than bad user input.
    let config = winutil_core::config::load().expect("embedded config is valid");
    eframe::run_native(
        winutil_core::APP_NAME,
        eframe::NativeOptions::default(),
        Box::new(move |_cc| {
            Ok(Box::new(WinUtilApp {
                install: InstallTab::new(config),
            }))
        }),
    )
}
