use crate::{
    drives, fs_worker,
    icons::{self, Icon},
    model::{Drive, Entry, Event, Job},
};
use eframe::egui::{self, Color32, Sense};
use egui_extras::{Column, TableBuilder};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
};

pub struct SearchState {
    pub mask: String,
    pub cancel: Arc<AtomicBool>,
    pub running: bool,
}
pub struct Explorer {
    pub side: usize,
    pub path: PathBuf,
    pub edit: String,
    pub entries: Vec<Entry>,
    pub selected: Option<PathBuf>,
    pub generation: u64,
    pub epoch: u64,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub search: Option<SearchState>,
}
pub enum Action {
    Refresh,
    Navigate(PathBuf),
    Search,
    CancelSearch,
    Delete(PathBuf),
}
impl Explorer {
    pub fn new(side: usize, path: PathBuf) -> Self {
        Self {
            side,
            edit: path.to_string_lossy().into_owned(),
            path,
            entries: Vec::new(),
            selected: None,
            generation: 0,
            epoch: 0,
            loaded: false,
            loading: false,
            error: None,
            search: None,
        }
    }
    pub fn refresh(&mut self, tx: &Sender<Event>) {
        if self.search.is_some() {
            return;
        }
        self.generation += 1;
        self.loading = true;
        self.error = None;
        fs_worker::load(self.side, self.generation, self.path.clone(), tx.clone());
    }
    pub fn navigate(&mut self, path: PathBuf, tx: &Sender<Event>) {
        let was_search = self.search.is_some();
        self.stop_search();
        if was_search || path != self.path || !self.loaded {
            self.epoch += 1;
            self.path = path;
            self.entries.clear();
            self.selected = None;
            self.loaded = false;
        }
        self.edit = self.path.to_string_lossy().into_owned();
        self.refresh(tx);
    }
    pub fn stop_search(&mut self) {
        if let Some(search) = self.search.take() {
            search.cancel.store(true, Ordering::Relaxed);
            self.generation += 1;
        }
    }
    pub fn cancel_search(&mut self, tx: &Sender<Event>) {
        self.stop_search();
        self.epoch += 1;
        self.entries.clear();
        self.selected = None;
        self.loaded = false;
        self.refresh(tx);
    }
    pub fn start_search(&mut self, mask: String, tx: &Sender<Event>) {
        self.stop_search();
        self.generation += 1;
        self.epoch += 1;
        self.entries.clear();
        self.selected = None;
        self.error = None;
        self.loading = false;
        self.loaded = true;
        let cancel = Arc::new(AtomicBool::new(false));
        fs_worker::search(
            self.generation,
            self.path.clone(),
            mask.clone(),
            cancel.clone(),
            tx.clone(),
        );
        self.search = Some(SearchState {
            mask,
            cancel,
            running: true,
        });
    }
    pub fn apply_snapshot(&mut self, generation: u64, result: Result<Vec<Entry>, String>) {
        if generation != self.generation || self.search.is_some() {
            return;
        }
        self.loading = false;
        match result {
            Ok(entries) => {
                // One assignment per frame, and no assignment at all if the snapshot is unchanged.
                if self.entries != entries {
                    self.entries = entries;
                }
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|s| !self.entries.iter().any(|e| &e.path == s))
                {
                    self.selected = None;
                }
                self.loaded = true;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
    pub fn selected_video(&self) -> Option<PathBuf> {
        self.selected
            .as_ref()
            .filter(|s| self.entries.iter().any(|e| &e.path == *s && !e.dir))
            .cloned()
    }
    pub fn add_pending(&mut self, path: &std::path::Path) {
        if path.parent() == Some(self.path.as_path())
            && !self.entries.iter().any(|e| e.path == path)
        {
            self.entries.push(Entry {
                path: path.to_owned(),
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                dir: false,
                relative: String::new(),
            });
            fs_worker::sort(&mut self.entries);
        }
    }
    pub fn draw(&mut self, ui: &mut egui::Ui, drives: &[Drive], jobs: &[Job]) -> Vec<Action> {
        let mut actions = Vec::new();
        ui.heading(if self.side == 0 {
            "Source — источник"
        } else {
            "Destination — назначение"
        });
        let search_mode = self.search.is_some();
        let root = drives::root(&self.path, drives);
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Refresh, !search_mode, "Обновить").clicked() {
                actions.push(Action::Refresh);
            }
            if icons::button(ui, Icon::Home, true, "Домашняя папка").clicked() {
                actions.push(Action::Navigate(drives::home()));
            }
            if icons::button(ui, Icon::Root, self.path != root, "Перейти к корню").clicked()
            {
                actions.push(Action::Navigate(root.clone()));
            }
            let parent = self
                .path
                .parent()
                .filter(|p| *p != self.path && !p.as_os_str().is_empty());
            if icons::button(
                ui,
                Icon::Up,
                parent.is_some() && self.path != root,
                "Перейти наверх",
            )
            .clicked()
            {
                if let Some(p) = parent {
                    actions.push(Action::Navigate(p.to_owned()));
                }
            }
            let selected = drives::current_drive(&self.path, drives)
                .map(|d| d.label())
                .unwrap_or_else(|| "Выбрать диск / сеть".into());
            egui::ComboBox::from_id_salt((self.side, "drive"))
                .selected_text(selected)
                .width(280.0)
                .height(popup_height(ui, drives.len()))
                .show_ui(ui, |ui| {
                    for d in drives {
                        if ui
                            .selectable_label(self.path.starts_with(&d.root), d.label())
                            .clicked()
                        {
                            actions.push(Action::Navigate(d.root.clone()));
                        }
                    }
                });
            if self.side == 0 {
                if icons::button(ui, Icon::Search, !search_mode, "Поиск по маске имени").clicked()
                {
                    actions.push(Action::Search);
                }
            } else {
                let deletable = self.selected.as_ref().is_some_and(|p| {
                    !jobs
                        .iter()
                        .any(|j| j.spec.source.starts_with(p) || j.spec.output.starts_with(p))
                });
                if icons::button(ui, Icon::Trash, deletable, "Удалить файл / папку").clicked()
                {
                    if let Some(p) = &self.selected {
                        actions.push(Action::Delete(p.clone()));
                    }
                }
            }
            // Fixed-size slot prevents toolbar/list movement when the spinner appears.
            ui.allocate_ui(egui::vec2(24.0, 24.0), |ui| {
                ui.set_min_size(egui::vec2(24.0, 24.0));
                if self.loading || self.search.as_ref().is_some_and(|s| s.running) {
                    ui.add(egui::Spinner::new().size(18.0));
                }
            });
        });
        ui.horizontal(|ui| {
            ui.label("Текущая папка:");
            let button_width = if search_mode { 94.0 } else { 76.0 };
            let edit = ui.add_enabled(
                !search_mode,
                egui::TextEdit::singleline(&mut self.edit)
                    .desired_width((ui.available_width() - button_width - 10.0).max(100.0)),
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if search_mode {
                if ui.button("Отменить").clicked() {
                    actions.push(Action::CancelSearch);
                }
            } else if ui.button("Перейти").clicked() || enter {
                let p = PathBuf::from(self.edit.trim());
                actions.push(Action::Navigate(if p.is_absolute() {
                    lexical(p)
                } else {
                    lexical(self.path.join(p))
                }));
            }
        });
        if let Some(search) = &self.search {
            ui.label(format!("Поиск файла {}", search.mask));
        }
        // Errors use a fixed single-line area outside the file table.
        ui.allocate_ui(egui::vec2(ui.available_width(), 22.0), |ui| {
            ui.set_min_height(22.0);
            if let Some(e) = &self.error {
                ui.add(
                    egui::Label::new(egui::RichText::new(e).color(Color32::LIGHT_RED)).truncate(),
                )
                .on_hover_text(e);
            }
        });
        if !self.loaded && self.entries.is_empty() && self.loading {
            ui.centered_and_justified(|ui| {
                ui.add(egui::Spinner::new().size(20.0));
            });
            return actions;
        }
        let row_height = 27.0;
        let max_height = (ui.available_height() - 24.0 - ui.spacing().item_spacing.y).max(40.0);
        let mut table = TableBuilder::new(ui)
            .id_salt((self.side, self.epoch, "files"))
            .striped(true)
            .resizable(false)
            .sense(Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(26.0))
            .column(if search_mode {
                Column::remainder().at_least(150.0)
            } else {
                Column::remainder()
            })
            .min_scrolled_height(40.0)
            .max_scroll_height(max_height);
        if search_mode {
            table = table.column(Column::remainder().at_least(160.0));
        }
        let mut selected = None;
        let mut open = None;
        table
            .header(24.0, |mut header| {
                header.col(|_| {});
                header.col(|ui| {
                    ui.strong("Имя");
                });
                if search_mode {
                    header.col(|ui| {
                        ui.strong("Относительный путь");
                    });
                }
            })
            .body(|body| {
                body.rows(row_height, self.entries.len(), |mut row| {
                    let e = &self.entries[row.index()];
                    row.set_selected(self.selected.as_ref() == Some(&e.path));
                    row.col(|ui| {
                        if jobs.iter().any(|j| j.spec.output == e.path) {
                            ui.add(egui::Spinner::new().size(18.0));
                        } else {
                            icons::file(ui, e.dir);
                        }
                    });
                    row.col(|ui| {
                        let label = ui.add(egui::Label::new(&e.name).truncate());
                        let response = ui
                            .interact(
                                label.rect,
                                egui::Id::new((self.side, self.epoch, &e.path, "entry")),
                                Sense::click(),
                            )
                            .on_hover_text(e.path.to_string_lossy());
                        if response.clicked() {
                            selected = Some(e.path.clone());
                            response.request_focus();
                        }
                        if response.double_clicked()
                            || (response.has_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                        {
                            if e.dir {
                                open = Some(e.path.clone());
                            } else {
                                selected = Some(e.path.clone());
                            }
                        }
                    });
                    if search_mode {
                        row.col(|ui| {
                            ui.add(egui::Label::new(&e.relative).truncate())
                                .on_hover_text(&e.relative);
                        });
                    }
                    let response = row.response();
                    if response.clicked() {
                        selected = Some(e.path.clone());
                    }
                    if response.double_clicked() && e.dir {
                        open = Some(e.path.clone());
                    }
                });
            });
        if let Some(p) = selected {
            self.selected = Some(p);
        }
        if let Some(p) = open {
            actions.push(Action::Navigate(p));
        }
        actions
    }
}
pub fn popup_height(ui: &egui::Ui, count: usize) -> f32 {
    let row = ui
        .text_style_height(&egui::TextStyle::Button)
        .max(ui.spacing().interact_size.y)
        + ui.spacing().item_spacing.y;
    row * count.clamp(1, 5) as f32 + 8.0
}
fn lexical(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c.as_os_str()),
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unchanged_snapshot_preserves_selection() {
        let mut e = Explorer::new(0, PathBuf::from("/tmp"));
        e.generation = 4;
        let row = Entry {
            path: "/tmp/a.mp4".into(),
            name: "a.mp4".into(),
            dir: false,
            relative: String::new(),
        };
        e.entries = vec![row.clone()];
        e.selected = Some(row.path.clone());
        e.apply_snapshot(4, Ok(vec![row.clone()]));
        assert_eq!(e.selected, Some(row.path));
        e.apply_snapshot(3, Ok(vec![]));
        assert_eq!(e.entries.len(), 1);
        e.apply_snapshot(4, Err("error".into()));
        assert_eq!(e.entries.len(), 1);
        e.apply_snapshot(4, Ok(vec![]));
        assert!(e.selected.is_none());
    }
}
