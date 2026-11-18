//! Features tab: a checkbox for each Windows optional feature, and one button that installs the
//! ticked ones in tick order.
//!
//! The work runs on one worker thread, one job at a time. The button is disabled while a job runs.

use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use winutil_core::config::Config;
use winutil_core::features::system::SystemFeatures;
use winutil_core::features::{self, Outcome, Report};

/// Height kept free under the list for the log.
const LOG_HEIGHT: f32 = 180.0;

enum Event {
    Report { key: String, report: Report },
    Finished,
}

pub struct FeaturesTab {
    config: Config,
    jobs: Sender<Vec<String>>,
    events: Receiver<Event>,
    /// True from the moment a job is sent until its `Finished` event arrives.
    busy: bool,
    /// Ticked checkboxes, in the order they were ticked.
    selected: Vec<String>,
    log: Vec<(String, Report)>,
}

impl FeaturesTab {
    pub fn new(config: Config) -> Self {
        let (jobs, job_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let worker_config = config.clone();
        std::thread::spawn(move || worker(worker_config, job_rx, event_tx));
        Self {
            config,
            jobs,
            events,
            busy: false,
            selected: Vec::new(),
            log: Vec::new(),
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain();
        let busy = self.busy;
        let mut install = false;

        ui.horizontal(|ui| {
            let nothing_selected = self.selected.is_empty();
            if ui
                .add_enabled(
                    !busy && !nothing_selected,
                    egui::Button::new("Install Features"),
                )
                .clicked()
            {
                install = true;
            }
            if busy {
                ui.spinner();
                ui.ctx().request_repaint();
            }
        });
        ui.separator();

        let list_height = (ui.available_height() - LOG_HEIGHT).max(120.0);
        let config = &self.config;
        let selected = &mut self.selected;
        egui::ScrollArea::vertical()
            .id_salt("features_list")
            .max_height(list_height)
            .show(ui, |ui| {
                for (key, feature) in features::checkboxes(&config.features) {
                    let was_ticked = selected.iter().any(|ticked| ticked == key);
                    let mut ticked = was_ticked;
                    let mut response =
                        ui.add_enabled(!busy, egui::Checkbox::new(&mut ticked, &feature.content));
                    if let Some(description) = &feature.description {
                        response = response.on_hover_text(description);
                    }
                    if ticked != was_ticked {
                        if ticked {
                            selected.push(key.to_owned());
                        } else {
                            selected.retain(|item| item != key);
                        }
                    }
                }
            });

        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("features_log")
            .max_height(LOG_HEIGHT)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (key, report) in &self.log {
                    let color = match report.outcome {
                        Outcome::Done => egui::Color32::GREEN,
                        Outcome::Failed => egui::Color32::RED,
                    };
                    let name = self
                        .config
                        .features
                        .get(key)
                        .map_or(key.as_str(), |feature| feature.content.as_str());
                    let label = match report.outcome {
                        Outcome::Done => "Done",
                        Outcome::Failed => "Failed",
                    };
                    ui.colored_label(color, format!("{label} {name}: {}", report.detail));
                }
                if !busy
                    && self
                        .log
                        .iter()
                        .any(|(_, report)| report.outcome == Outcome::Done)
                {
                    ui.label("A reboot may be required.");
                }
            });

        if install {
            self.start();
        }
    }

    /// Moves finished work from the worker thread into the tab's state.
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Report { key, report } => self.log.push((key, report)),
                Event::Finished => self.busy = false,
            }
        }
    }

    /// Clears the log and sends the ticked features to the worker.
    fn start(&mut self) {
        self.log.clear();
        self.busy = true;
        // The worker lives as long as the tab, so the send cannot fail.
        let _ = self.jobs.send(self.selected.clone());
    }
}

/// Installs each feature in order on this thread. Each job ends with a `Finished` event.
fn worker(config: Config, jobs: Receiver<Vec<String>>, events: Sender<Event>) {
    let mut host = SystemFeatures;
    for keys in jobs {
        for key in keys {
            let feature = config
                .features
                .get(&key)
                .expect("the tab only sends keys from this config");
            let report = features::install(feature, &mut host);
            let _ = events.send(Event::Report { key, report });
        }
        let _ = events.send(Event::Finished);
    }
}
