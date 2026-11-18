//! Tweaks tab. Panel 1 holds the plain checkboxes, which are selected and then run or undone
//! together, plus the advanced buttons. Panel 2 holds toggles, comboboxes, and buttons that act
//! on click.
//!
//! Every change runs on one worker thread, one job at a time. Each job ends with a re-read of
//! every toggle and combobox, so the window shows what the machine reports rather than what was
//! last clicked.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui;
use winutil_core::config::Config;
use winutil_core::config::tweaks::{Tweak, WidgetKind};
use winutil_core::tweaks::Direction;
use winutil_core::tweaks::run::{self, Action, Outcome, Report};
use winutil_core::tweaks::system::SystemTweaks;

/// Taken before the other selected tweaks run.
const RESTORE_POINT: &str = "WPFTweaksRestorePoint";
/// The DNS combobox belongs to the DNS feature, not this tab.
const DNS_COMBO: &str = "WPFchangedns";
/// Height kept free under the panels for the log.
const LOG_HEIGHT: f32 = 180.0;

/// One action on one tweak. The key is the tweak's name in `tweaks.json`.
#[derive(Debug, Clone)]
enum Command {
    Tweak(Direction),
    Choice(String),
    Button,
}

/// Steps run in order. An empty job only re-reads status.
type Job = Vec<(String, Command)>;

enum Event {
    Report { key: String, report: Report },
    Toggle { key: String, state: Result<bool, String> },
    Choice { key: String, state: Result<Option<String>, String> },
    Finished,
}

pub struct TweaksTab {
    config: Config,
    jobs: Sender<Job>,
    events: Receiver<Event>,
    /// Jobs sent and not yet finished. Nonzero means the worker is busy.
    pending: usize,
    /// Plain checkboxes, in the order they were ticked.
    selected: Vec<String>,
    toggles: HashMap<String, Result<bool, String>>,
    choices: HashMap<String, Result<Option<String>, String>>,
    log: Vec<(String, Report)>,
}

impl TweaksTab {
    pub fn new(config: Config) -> Self {
        let selected = config
            .tweaks
            .iter()
            .filter(|(_, tweak)| tweak.kind.is_none() && tweak.checked)
            .map(|(key, _)| key.clone())
            .collect();
        let (jobs, job_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        let worker_config = config.clone();
        std::thread::spawn(move || worker(worker_config, job_rx, event_tx));
        let mut tab = Self {
            config,
            jobs,
            events,
            pending: 0,
            selected,
            toggles: HashMap::new(),
            choices: HashMap::new(),
            log: Vec::new(),
        };
        // The first status read; the window opens before it finishes.
        tab.submit(Vec::new());
        tab
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.drain();
        let busy = self.pending > 0;
        let mut start: Option<Job> = None;

        ui.horizontal(|ui| {
            let nothing_selected = self.selected.is_empty();
            if ui
                .add_enabled(!busy && !nothing_selected, egui::Button::new("Run Tweaks"))
                .clicked()
            {
                start = Some(self.selected_job(Direction::Apply));
            }
            if ui
                .add_enabled(!busy && !nothing_selected, egui::Button::new("Undo Tweaks"))
                .clicked()
            {
                start = Some(self.selected_job(Direction::Undo));
            }
            if busy {
                ui.spinner();
                ui.ctx().request_repaint();
            }
        });
        ui.separator();

        let panel_height = (ui.available_height() - LOG_HEIGHT).max(120.0);
        let mut clicks: Job = Vec::new();
        let config = &self.config;
        let selected = &mut self.selected;
        let status = Status {
            busy,
            toggles: &self.toggles,
            choices: &self.choices,
        };
        let (panel_1, panel_2): (Vec<_>, Vec<_>) = config
            .tweaks
            .iter()
            .filter(|(key, _)| key.as_str() != DNS_COMBO)
            .partition(|(_, tweak)| tweak.panel == 1);
        ui.columns(2, |columns| {
            for (column, (id, tweaks)) in columns
                .iter_mut()
                .zip([("tweaks_panel_1", panel_1), ("tweaks_panel_2", panel_2)])
            {
                egui::ScrollArea::vertical()
                    .id_salt(id)
                    .max_height(panel_height)
                    .show(column, |ui| {
                        for (category, group) in by_category(tweaks.into_iter()) {
                            egui::CollapsingHeader::new(format!("{category} ({})", group.len()))
                                .default_open(true)
                                .show(ui, |ui| {
                                    for (key, tweak) in group {
                                        clicks.extend(draw_row(
                                            ui,
                                            key,
                                            tweak,
                                            &status,
                                            selected,
                                        ));
                                    }
                                });
                        }
                    });
            }
        });

        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("tweaks_log")
            .max_height(LOG_HEIGHT)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for (key, report) in &self.log {
                    let color = match report.outcome {
                        Outcome::Done => egui::Color32::GREEN,
                        Outcome::Skipped => egui::Color32::GRAY,
                        Outcome::Failed => egui::Color32::RED,
                    };
                    let name = self
                        .config
                        .tweaks
                        .get(key)
                        .map_or(key.as_str(), |tweak| tweak.content.as_str());
                    ui.colored_label(
                        color,
                        format!("{} {name}: {}", outcome_label(report.outcome), report.detail),
                    );
                }
            });

        if !clicks.is_empty() {
            start = Some(clicks);
        }
        if let Some(job) = start {
            self.start(job);
        }
    }

    /// Moves finished work from the worker thread into the tab's state.
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Report { key, report } => self.log.push((key, report)),
                Event::Toggle { key, state } => {
                    self.toggles.insert(key, state);
                }
                Event::Choice { key, state } => {
                    self.choices.insert(key, state);
                }
                Event::Finished => self.pending -= 1,
            }
        }
    }

    /// Runs `job` after clearing the log, as the install tab does for a new run.
    fn start(&mut self, job: Job) {
        self.log.clear();
        self.submit(job);
    }

    fn submit(&mut self, job: Job) {
        self.pending += 1;
        // The worker lives as long as the tab, so the send cannot fail.
        let _ = self.jobs.send(job);
    }

    /// The checked plain tweaks in tick order. Applying runs the restore point first.
    fn selected_job(&self, direction: Direction) -> Job {
        let mut job: Job = self
            .selected
            .iter()
            .map(|key| (key.clone(), Command::Tweak(direction)))
            .collect();
        if direction == Direction::Apply
            && let Some(index) = job.iter().position(|(key, _)| key.as_str() == RESTORE_POINT)
        {
            let step = job.remove(index);
            job.insert(0, step);
        }
        job
    }
}

/// Read-only state the rows draw from.
struct Status<'a> {
    busy: bool,
    toggles: &'a HashMap<String, Result<bool, String>>,
    choices: &'a HashMap<String, Result<Option<String>, String>>,
}

/// Draws one tweak. Returns the step to run when the user acts on it.
fn draw_row(
    ui: &mut egui::Ui,
    key: &str,
    tweak: &Tweak,
    status: &Status,
    selected: &mut Vec<String>,
) -> Option<(String, Command)> {
    match tweak.kind {
        None => {
            let mut on = selected.iter().any(|k| k.as_str() == key);
            let response = hover(
                ui.add_enabled(!status.busy, egui::Checkbox::new(&mut on, tweak.content.as_str())),
                tweak,
            );
            if response.changed() {
                if on {
                    selected.push(key.to_owned());
                } else {
                    selected.retain(|k| k.as_str() != key);
                }
            }
            None
        }
        Some(WidgetKind::Toggle) => match status.toggles.get(key) {
            None => {
                ui.label(format!("{} (checking…)", tweak.content));
                None
            }
            Some(Err(error)) => {
                ui.colored_label(egui::Color32::RED, format!("{} (status unknown)", tweak.content))
                    .on_hover_text(error);
                None
            }
            Some(Ok(applied)) => {
                let mut on = *applied;
                let response = hover(
                    ui.add_enabled(!status.busy, egui::Checkbox::new(&mut on, tweak.content.as_str())),
                    tweak,
                );
                response
                    .changed()
                    .then(|| (key.to_owned(), Command::Tweak(if on { Direction::Apply } else { Direction::Undo })))
            }
        },
        Some(WidgetKind::Combobox) => {
            let current = match status.choices.get(key) {
                None => "checking…".to_owned(),
                Some(Ok(Some(choice))) => choice.clone(),
                Some(Ok(None)) => "unknown".to_owned(),
                Some(Err(_)) => "status unknown".to_owned(),
            };
            let mut next = None;
            ui.horizontal(|ui| {
                ui.label(tweak.content.as_str());
                ui.add_enabled_ui(!status.busy, |ui| {
                    egui::ComboBox::from_id_salt(key)
                        .selected_text(current.as_str())
                        .show_ui(ui, |ui| {
                            for choice in tweak.combo_choices() {
                                let clicked = ui.selectable_label(current == choice, choice).clicked();
                                if clicked && current != choice {
                                    next = Some(choice.to_owned());
                                }
                            }
                        });
                });
            });
            next.map(|choice| (key.to_owned(), Command::Choice(choice)))
        }
        Some(WidgetKind::Button) => {
            let mut button = egui::Button::new(tweak.content.as_str());
            if let Some(width) = tweak.button_width {
                button = button.min_size(egui::vec2(width as f32, 0.0));
            }
            let response = hover(ui.add_enabled(!status.busy, button), tweak);
            response.clicked().then(|| (key.to_owned(), Command::Button))
        }
    }
}

fn hover(response: egui::Response, tweak: &Tweak) -> egui::Response {
    match tweak.description.as_deref() {
        Some(text) if !text.is_empty() => response.on_hover_text(text),
        _ => response,
    }
}

/// Groups tweaks by category, keeping the first-seen order of both categories and tweaks.
fn by_category<'a>(
    tweaks: impl Iterator<Item = (&'a String, &'a Tweak)>,
) -> Vec<(&'a str, Vec<(&'a str, &'a Tweak)>)> {
    let mut groups: Vec<(&str, Vec<(&str, &Tweak)>)> = Vec::new();
    for (key, tweak) in tweaks {
        let entry = (key.as_str(), tweak);
        match groups.iter_mut().find(|(category, _)| *category == tweak.category.as_str()) {
            Some((_, group)) => group.push(entry),
            None => groups.push((tweak.category.as_str(), vec![entry])),
        }
    }
    groups
}

fn outcome_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Skipped => "skipped",
        Outcome::Failed => "failed",
    }
}

/// Runs jobs in order on this thread. Each job ends with a status re-read and a `Finished` event.
fn worker(config: Config, jobs: Receiver<Job>, events: Sender<Event>) {
    let mut host = SystemTweaks;
    for job in jobs {
        for (key, command) in job {
            let tweak = config
                .tweaks
                .get(&key)
                .expect("the tab only sends keys from this config");
            let reports = match &command {
                Command::Tweak(direction) => {
                    run::run(&mut host, &key, tweak, Action::Tweak(*direction))
                }
                Command::Choice(choice) => {
                    run::run(&mut host, &key, tweak, Action::Choice(choice.as_str()))
                }
                Command::Button => run::run(&mut host, &key, tweak, Action::Button),
            };
            for report in reports {
                let _ = events.send(Event::Report {
                    key: key.clone(),
                    report,
                });
            }
        }
        refresh(&mut host, &config, &events);
        let _ = events.send(Event::Finished);
    }
}

fn refresh(host: &mut SystemTweaks, config: &Config, events: &Sender<Event>) {
    for (key, tweak) in &config.tweaks {
        match tweak.kind {
            Some(WidgetKind::Toggle) => {
                let state = run::is_applied(host, tweak);
                let _ = events.send(Event::Toggle {
                    key: key.clone(),
                    state,
                });
            }
            Some(WidgetKind::Combobox) if key.as_str() != DNS_COMBO => {
                let state = run::current_choice(host, tweak);
                let _ = events.send(Event::Choice {
                    key: key.clone(),
                    state,
                });
            }
            _ => {}
        }
    }
}
