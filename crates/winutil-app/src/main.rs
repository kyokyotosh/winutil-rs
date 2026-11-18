mod install_tab;
mod tweaks_tab;

use eframe::egui;
use install_tab::InstallTab;
use tweaks_tab::TweaksTab;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Install,
    Tweaks,
}

struct WinUtilApp {
    page: Page,
    install: InstallTab,
    tweaks: TweaksTab,
}

impl eframe::App for WinUtilApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.heading(winutil_core::APP_NAME);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.page, Page::Install, "Install");
                ui.selectable_value(&mut self.page, Page::Tweaks, "Tweaks");
            });
            ui.separator();
            match self.page {
                Page::Install => self.install.ui(ui),
                Page::Tweaks => self.tweaks.ui(ui),
            }
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
                page: Page::Install,
                install: InstallTab::new(config.clone()),
                tweaks: TweaksTab::new(config),
            }))
        }),
    )
}
