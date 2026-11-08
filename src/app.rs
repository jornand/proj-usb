use crate::{
    drives,
    explorer::{popup_height, Action, Explorer},
    model::*,
    process,
    settings::{self, Settings},
};
use eframe::egui;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    time::Duration,
};

pub struct NewTask {
    pub source: PathBuf,
    pub folder: PathBuf,
    pub name: String,
    pub tracks: AudioTracks,
    pub codec: Codec,
    pub focus: bool,
    pub error: Option<String>,
}
pub enum Dialog {
    Search { mask: String, focus: bool },
    Delete(PathBuf),
    New(NewTask),
    Exit,
}
pub struct App {
    pub explorers: [Explorer; 2],
    pub drives: Vec<Drive>,
    pub jobs: Vec<Job>,
    pub tx: Sender<Event>,
    rx: Receiver<Event>,
    stop: Arc<AtomicBool>,
    pub tracks: AudioTracks,
    pub codec: Codec,
    pub info: String,
    pub info_path: Option<PathBuf>,
    pub dialog: Option<Dialog>,
    pub details: bool,
    pub logs: VecDeque<String>,
    pub next_id: u64,
    pub closing: bool,
}
impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(8.0, 7.0);
        style
            .text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
        style
            .text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(16.0));
        cc.egui_ctx.set_style(style);
        let config = settings::load();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        drives::watch(tx.clone(), stop.clone());
        let mut app = Self {
            explorers: [
                Explorer::new(0, config.source),
                Explorer::new(1, config.destination),
            ],
            drives: Vec::new(),
            jobs: Vec::new(),
            tx,
            rx,
            stop,
            tracks: config.tracks,
            codec: config.codec,
            info: String::new(),
            info_path: None,
            dialog: None,
            details: false,
            logs: VecDeque::new(),
            next_id: 1,
            closing: false,
        };
        for e in &mut app.explorers {
            e.refresh(&app.tx);
        }
        app
    }
    pub fn log(&mut self, text: String) {
        for line in text.lines() {
            if self.logs.len() == 5 {
                self.logs.pop_front();
            }
            self.logs.push_back(line.to_owned());
        }
    }
    fn events(&mut self) {
        // Bound work per frame; large searches or verbose ffmpeg cannot starve input.
        for _ in 0..512 {
            let event = match self.rx.try_recv() {
                Ok(e) => e,
                Err(_) => break,
            };
            match event {
                Event::Snapshot {
                    side,
                    generation,
                    result,
                } => {
                    if self.explorers[side].generation != generation
                        || self.explorers[side].search.is_some()
                    {
                        continue;
                    }
                    let failed = result.is_err();
                    self.explorers[side].apply_snapshot(generation, result);
                    // Only on initial failed opening, not on a background refresh.
                    if failed
                        && !self.explorers[side].loaded
                        && self.explorers[side].path != drives::home()
                    {
                        self.explorers[side].navigate(drives::home(), &self.tx);
                    }
                    if side == 1 {
                        for j in &self.jobs {
                            self.explorers[1].add_pending(&j.spec.output);
                        }
                    }
                }
                Event::Found {
                    generation,
                    entries,
                } => {
                    let e = &mut self.explorers[0];
                    if e.generation == generation && e.search.is_some() {
                        e.entries.extend(entries);
                    }
                }
                Event::SearchDone { generation, error } => {
                    let e = &mut self.explorers[0];
                    if e.generation == generation {
                        if let Some(s) = &mut e.search {
                            s.running = false;
                        }
                        e.error = error;
                    }
                }
                Event::Drives(Ok(new)) => {
                    for e in &mut self.explorers {
                        if let Some(old) = drives::current_drive(&e.path, &self.drives) {
                            if !new.iter().any(|d| d.root == old.root) {
                                e.navigate(drives::home(), &self.tx);
                            }
                        }
                    }
                    self.drives = new;
                }
                Event::Drives(Err(e)) => self.log(format!("Список устройств: {e}")),
                Event::Info { path, result } => {
                    if self.info_path.as_ref() == Some(&path) {
                        self.info = result.unwrap_or_else(|e| e);
                    }
                }
                Event::Duration { id, seconds } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.duration = seconds;
                    }
                }
                Event::Progress { id, seconds } => {
                    if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                        j.processed = seconds;
                    }
                }
                Event::Finished {
                    id,
                    result,
                    cancelled,
                } => {
                    if let Some(i) = self.jobs.iter().position(|j| j.id == id) {
                        let j = self.jobs.remove(i);
                        if cancelled || result.is_err() {
                            self.explorers[1]
                                .entries
                                .retain(|e| e.path != j.spec.output);
                            if self.explorers[1].selected.as_ref() == Some(&j.spec.output) {
                                self.explorers[1].selected = None;
                            }
                        }
                        self.log(match result {
                            Ok(()) if !cancelled => {
                                format!("#{id} Готово: {}", j.spec.output.display())
                            }
                            Ok(()) => format!("#{id} Отменено"),
                            Err(e) => format!(
                                "#{id} {}: {e}",
                                if cancelled {
                                    "Отменено"
                                } else {
                                    "Ошибка"
                                }
                            ),
                        });
                    }
                }
                Event::Deleted(result) => match result {
                    Ok(()) => {
                        self.explorers[1].refresh(&self.tx);
                        self.log("Удалено".into());
                    }
                    Err(e) => self.log(e),
                },
                Event::Log(s) => self.log(s),
            }
        }
    }
    fn action(&mut self, side: usize, action: Action) {
        match action {
            Action::Refresh => self.explorers[side].refresh(&self.tx),
            Action::Navigate(path) => self.explorers[side].navigate(path, &self.tx),
            Action::Search => {
                self.dialog = Some(Dialog::Search {
                    mask: "*".into(),
                    focus: true,
                })
            }
            Action::CancelSearch => self.explorers[side].cancel_search(&self.tx),
            Action::Delete(path) => self.dialog = Some(Dialog::Delete(path)),
        }
    }
    pub fn start(&mut self, spec: JobSpec) {
        let id = self.next_id;
        self.next_id += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.explorers[1].add_pending(&spec.output);
        self.log(format!("#{id} Запущено: {}", spec.source.display()));
        process::run(id, spec.clone(), cancel.clone(), self.tx.clone());
        self.jobs.push(Job {
            id,
            spec,
            duration: None,
            processed: 0.0,
            cancel,
            stopping: false,
        });
    }
    pub fn cancel_all(&mut self) {
        for j in &mut self.jobs {
            j.cancel.store(true, Ordering::Relaxed);
            j.stopping = true;
        }
    }
    fn save_settings(&self) -> Result<(), String> {
        settings::save(&Settings {
            source: self.explorers[0].path.clone(),
            destination: self.explorers[1].path.clone(),
            tracks: self.tracks,
            codec: self.codec,
        })
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        let source = self.explorers[0].selected_video();
        ui.columns(2, |cols| {
            if cols[0].add_enabled(source.is_some(), egui::Button::new("Информация о файле")).clicked() {
                let p = source.clone().unwrap(); self.info_path = Some(p.clone()); self.info = "Запрос ffprobe…".into(); process::info(p, self.tx.clone());
            }
            let h = (cols[0].available_height() - 8.0).max(60.0);
            egui::ScrollArea::vertical().id_salt("info").max_height(h).show(&mut cols[0], |ui| {
                ui.add(egui::TextEdit::multiline(&mut self.info).interactive(false).desired_width(f32::INFINITY).desired_rows(10));
            });
            cols[1].horizontal(|ui| {
                ui.label("Аудиодорожки:");
                egui::ComboBox::from_id_salt("tracks").selected_text(self.tracks.label()).height(popup_height(ui, 2)).show_ui(ui, |ui| {
                    for t in [AudioTracks::First, AudioTracks::All] { ui.selectable_value(&mut self.tracks, t, t.label()); }
                });
                ui.label("Кодек:");
                egui::ComboBox::from_id_salt("codec").selected_text(self.codec.label()).width(190.0).height(popup_height(ui, 3)).show_ui(ui, |ui| {
                    for c in [Codec::Copy, Codec::Aac, Codec::Mp3] { ui.selectable_value(&mut self.codec, c, c.label()); }
                });
            });
            cols[1].add_space(14.0);
            if cols[1].add_enabled(source.is_some() && !self.closing, egui::Button::new(egui::RichText::new("Запустить задачу").size(19.0)).min_size(egui::vec2(250.0, 42.0))).clicked() {
                let source = source.unwrap(); let name = source.file_stem().unwrap_or_default().to_string_lossy().into_owned();
                self.dialog = Some(Dialog::New(NewTask { source, folder: self.explorers[1].path.clone(), name, tracks: self.tracks, codec: self.codec, focus: true, error: None }));
            }
            cols[1].add_space(12.0);
            cols[1].label("Видео копируется без перекодирования. Контейнер сохраняется.");
            cols[1].label("Субтитры и data-дорожки не переносятся; метаданные и главы сохраняются, если контейнер их поддерживает.");
            cols[1].label("Совместимость видеокодека с проектором следует проверить отдельно.");
        });
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.events();
        if ctx.input(|i| i.viewport().close_requested()) && !self.jobs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !self.closing {
                self.dialog = Some(Dialog::Exit);
            }
        }
        if self.closing && self.jobs.is_empty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            let height = ui.available_height();
            let upper = height * 0.55;
            let middle = height * 0.32;
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), upper),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_height(upper);
                    ui.set_min_height(upper);
                    ui.columns(2, |cols| {
                        let left = self.explorers[0].draw(&mut cols[0], &self.drives, &self.jobs);
                        let right = self.explorers[1].draw(&mut cols[1], &self.drives, &self.jobs);
                        for a in left {
                            self.action(0, a);
                        }
                        for a in right {
                            self.action(1, a);
                        }
                    });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), middle),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_max_height(middle);
                    ui.set_min_height(middle);
                    self.controls(ui);
                },
            );
            ui.separator();
            ui.horizontal(|ui| {
                let remaining: f64 = self.jobs.iter().filter_map(Job::remaining).sum();
                let unknown = self.jobs.iter().filter(|j| j.duration.is_none()).count();
                ui.strong(format!("Запущено задач: {}", self.jobs.len()));
                ui.label(format!(
                    "Осталось видео: {}{}",
                    clock(remaining),
                    if unknown > 0 {
                        format!(" + {unknown} с неизвестной длительностью")
                    } else {
                        String::new()
                    }
                ));
                if ui.button("Детали по текущим задачам").clicked() {
                    self.details = true;
                }
                if self.closing {
                    ui.label("Отмена задач перед закрытием…");
                }
            });
            if let Some(last) = self.logs.back() {
                ui.add(egui::Label::new(last).truncate())
                    .on_hover_text(last);
            }
        });
        self.draw_details(ctx);
        self.draw_dialog(ctx);
        ctx.request_repaint_after(Duration::from_millis(100));
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop.store(true, Ordering::Relaxed);
        self.cancel_all();
        for e in &mut self.explorers {
            e.stop_search();
        }
        if let Err(e) = self.save_settings() {
            eprintln!("Настройки: {e}");
        }
    }
}
