//! Install tab: pick applications by category, choose a package manager, and run the installs.
//!
//! Installs run on a background thread. Each finished package is sent back over a channel and
//! drained once per frame, so the window stays responsive during long installs.

use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use eframe::egui;
use winutil_core::config::Config;
use winutil_core::install::run::{Manager, Outcome, Report, run_plan};
use winutil_core::install::{self, Availability, Needs, PackageManager};

pub struct InstallTab {
    config: Config,
    selected: HashSet<String>,
    preference: PackageManager,
    available: Availability,
    running: Option<Receiver<Report>>,
    reports: Vec<Report>,
}

impl InstallTab {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            selected: HashSet::new(),
            preference: PackageManager::Winget,
            available: Availability::detect(),
            running: None,
            reports: Vec::new(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain();
        let busy = self.running.is_some();
        let mut start = false;

        ui.horizontal(|ui| {
            ui.label("Prefer:");
            egui::ComboBox::from_id_salt("install_preference")
                .selected_text(preference_label(self.preference))
                .show_ui(ui, |ui| {
                    for choice in [PackageManager::Winget, PackageManager::Choco] {
                        ui.selectable_value(&mut self.preference, choice, preference_label(choice));
                    }
                });
            let label = format!("Install {} selected", self.selected.len());
            if ui
                .add_enabled(!busy && !self.selected.is_empty(), egui::Button::new(label))
                .clicked()
            {
                start = true;
            }
            if busy {
                ui.spinner();
                ui.ctx().request_repaint();
            }
        });
        if !self.available.winget {
            ui.colored_label(
                egui::Color32::RED,
                "WinGet is not installed. Packages that need it are skipped. Install App Installer from the Microsoft Store.",
            );
        }
        if !self.available.choco {
            ui.colored_label(
                egui::Color32::RED,
                "Chocolatey is not installed. Packages that need it are skipped. Install it from https://chocolatey.org/install.",
            );
        }
        ui.separator();

        let preference = self.preference;
        let available = self.available;
        let config = &self.config;
        let selected = &mut self.selected;
        let reports = &self.reports;
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical().id_salt("install_apps").show(&mut columns[0], |ui| {
                for (category, keys) in install::group_by_category(&config.applications) {
                    egui::CollapsingHeader::new(format!("{category} ({})", keys.len()))
                        .show(ui, |ui| {
                            for key in keys {
                                let app = &config.applications[key];
                                let mut on = selected.contains(key);
                                let missing = missing_text(install::needs(app, preference), available);
                                let text = match missing {
                                    Some(_) => egui::RichText::new(&app.content).color(egui::Color32::RED),
                                    None => egui::RichText::new(&app.content),
                                };
                                let changed = ui
                                    .horizontal(|ui| {
                                        let response = ui.checkbox(&mut on, text).on_hover_text(&app.description);
                                        if let Some(missing) = missing {
                                            ui.colored_label(egui::Color32::RED, missing);
                                        }
                                        response.changed()
                                    })
                                    .inner;
                                if changed {
                                    if on {
                                        selected.insert(key.to_owned());
                                    } else {
                                        selected.remove(key);
                                    }
                                }
                            }
                        });
                }
            });

            egui::ScrollArea::vertical().id_salt("install_log").show(&mut columns[1], |ui| {
                for report in reports {
                    let color = match report.outcome {
                        Outcome::Succeeded => egui::Color32::GREEN,
                        Outcome::Skipped => egui::Color32::GRAY,
                        Outcome::Failed => egui::Color32::RED,
                    };
                    let line = format!(
                        "{} [{}] {}: {}",
                        outcome_label(report.outcome),
                        manager_label(report.manager),
                        report.package,
                        report.detail
                    );
                    let response = ui.colored_label(color, line);
                    if !report.tail.is_empty() {
                        response.on_hover_text(report.tail.join("\n"));
                    }
                }
            });
        });

        if start {
            self.start();
        }
    }

    /// Moves finished package reports from the worker thread into the log.
    fn drain(&mut self) {
        let Some(rx) = self.running.as_ref() else {
            return;
        };
        let finished = loop {
            match rx.try_recv() {
                Ok(report) => self.reports.push(report),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        if finished {
            self.running = None;
        }
    }

    fn start(&mut self) {
        let apps = self
            .config
            .applications
            .iter()
            .filter(|(key, _)| self.selected.contains(key.as_str()))
            .map(|(_, app)| app);
        let plan = install::plan(apps, self.preference);
        let (plan, skipped) = install::skip_unavailable(plan, self.available);
        let (tx, rx) = mpsc::channel();
        self.reports.clear();
        for report in skipped {
            let _ = tx.send(report);
        }
        std::thread::spawn(move || {
            run_plan(&plan, |report| {
                let _ = tx.send(report.clone());
            });
        });
        self.running = Some(rx);
    }
}

fn preference_label(preference: PackageManager) -> &'static str {
    match preference {
        PackageManager::Winget => "WinGet",
        PackageManager::Choco => "Chocolatey",
    }
}

fn manager_label(manager: Manager) -> &'static str {
    match manager {
        Manager::Winget => "WinGet",
        Manager::Choco => "Chocolatey",
        Manager::Msi => "MSI",
    }
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Succeeded => "Succeeded",
        Outcome::Skipped => "Skipped",
        Outcome::Failed => "Failed",
    }
}

/// Names the managers an app needs that are not installed, or `None` when nothing it needs is missing.
fn missing_text(needs: Needs, available: Availability) -> Option<&'static str> {
    match (needs.winget && !available.winget, needs.choco && !available.choco) {
        (true, true) => Some("needs WinGet and Chocolatey"),
        (true, false) => Some("needs WinGet"),
        (false, true) => Some("needs Chocolatey"),
        (false, false) => None,
    }
}
