use crate::{
    app::{App, Dialog},
    explorer::popup_height,
    fs_worker,
    icons::{self, Icon},
    model::*,
};
use eframe::egui::{self, Color32};
use egui_extras::{Column, TableBuilder};
use std::sync::atomic::Ordering;

fn readonly(ui: &mut egui::Ui, text: &str) {
    let mut text = text.to_owned();
    ui.add(
        egui::TextEdit::singleline(&mut text)
            .interactive(false)
            .desired_width(f32::INFINITY),
    );
}
fn focus_select(
    ui: &egui::Ui,
    output: &mut egui::text_edit::TextEditOutput,
    text: &str,
    first: &mut bool,
) {
    if *first {
        output.response.request_focus();
        output
            .state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(text.chars().count()),
            )));
        output.state.clone().store(ui.ctx(), output.response.id);
        *first = false;
    }
}
impl App {
    pub fn draw_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let title = match &dialog {
            Dialog::Search { .. } => "Поиск файла",
            Dialog::Delete(_) => "Подтверждение удаления",
            Dialog::New(_) => "Новая задача",
            Dialog::Exit => "Закрытие программы",
        };
        let mut accept = false;
        let mut dismiss = false;
        let mut spec = None;
        let response = egui::Modal::new(egui::Id::new("modal")).show(ctx, |ui| {
            ui.set_width(if matches!(dialog, Dialog::New(_)) { 760.0 } else { 470.0 });
            ui.heading(title); ui.separator();
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            match &mut dialog {
                Dialog::Search { mask, focus } => {
                    ui.label("Маска имени (например: *фильм*.mkv, ? — один символ):");
                    let mut output = egui::TextEdit::singleline(mask).desired_width(f32::INFINITY).show(ui);
                    focus_select(ui, &mut output, mask, focus);
                    let valid = globset::GlobBuilder::new(mask).case_insensitive(true).build().is_ok() && !mask.is_empty();
                    if !valid { ui.colored_label(Color32::LIGHT_RED, "Некорректная маска"); }
                    ui.horizontal(|ui| {
                        accept = valid && (ui.add_enabled(valid, egui::Button::new("Ок")).clicked() || enter);
                        dismiss = ui.button("Отмена").clicked();
                    });
                }
                Dialog::Delete(path) => {
                    ui.label("Удалить выбранный объект? Папка будет удалена рекурсивно, без корзины.");
                    readonly(ui, &path.to_string_lossy());
                    ui.horizontal(|ui| { accept = ui.button("Удалить").clicked() || enter; dismiss = ui.button("Отмена").clicked(); });
                }
                Dialog::New(task) => {
                    ui.label("Source:"); readonly(ui, &task.source.to_string_lossy());
                    ui.label("Destination:"); readonly(ui, &task.folder.to_string_lossy());
                    ui.label("Имя файла (расширение сохраняется):");
                    ui.horizontal(|ui| {
                        let mut output = egui::TextEdit::singleline(&mut task.name).desired_width(600.0).show(ui);
                        focus_select(ui, &mut output, &task.name, &mut task.focus);
                        ui.strong(format!(".{}", task.source.extension().unwrap_or_default().to_string_lossy()));
                    });
                    let mut validation = None;
                    let candidate = match output_path(&task.folder, &task.name, &task.source) {
                        Ok(output) => {
                            if output == task.source || output.exists() || self.jobs.iter().any(|j| j.spec.output == output) {
                                validation = Some("Итоговый файл уже существует или занят задачей. Выберите другое имя.".to_owned());
                            }
                            if !task.folder.is_dir() { validation = Some("Папка назначения недоступна".into()); }
                            Some(JobSpec { source: task.source.clone(), output, tracks: task.tracks, codec: task.codec })
                        }
                        Err(e) => { validation = Some(e); None }
                    };
                    ui.label("Команда ffmpeg:");
                    let mut text = candidate.as_ref().map(JobSpec::command_text).unwrap_or_default();
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::multiline(&mut text).interactive(false).desired_width(704.0).desired_rows(4));
                        if icons::button(ui, Icon::Copy, candidate.is_some(), "Копировать команду — два листа").clicked() { ui.ctx().copy_text(text.clone()); }
                    });
                    if let Some(error) = validation.as_ref().or(task.error.as_ref()) { ui.colored_label(Color32::LIGHT_RED, error); }
                    let valid = validation.is_none() && candidate.is_some();
                    ui.horizontal(|ui| {
                        if (ui.add_enabled(valid, egui::Button::new("Запустить")).clicked() || enter) && valid { accept = true; spec = candidate; }
                        dismiss = ui.button("Отмена").clicked();
                    });
                }
                Dialog::Exit => {
                    ui.label("Есть незавершённые задачи. Отменить их, удалить незавершённые файлы и закрыть приложение?");
                    ui.horizontal(|ui| { accept = ui.button("Отменить задачи и выйти").clicked() || enter; dismiss = ui.button("Остаться").clicked(); });
                }
            }
        });
        if response.should_close() {
            dismiss = true;
        }
        if dismiss {
            return;
        }
        if accept {
            match dialog {
                Dialog::Search { mask, .. } => self.explorers[0].start_search(mask, &self.tx),
                Dialog::Delete(path) => fs_worker::delete(path, self.tx.clone()),
                Dialog::New(_) => {
                    if let Some(spec) = spec {
                        self.start(spec);
                    }
                }
                Dialog::Exit => {
                    self.closing = true;
                    self.cancel_all();
                }
            }
        } else {
            self.dialog = Some(dialog);
        }
    }
    pub fn draw_details(&mut self, ctx: &egui::Context) {
        if !self.details {
            return;
        }
        let mut open = self.details;
        let mut cancels = Vec::new();
        egui::Window::new("Детали по текущим задачам")
            .id(egui::Id::new("details"))
            .open(&mut open)
            .default_size(egui::vec2(1250.0, 420.0))
            .min_width(900.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("Действующие незавершённые задачи:");
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .column(Column::exact(60.0))
                    .column(Column::remainder().at_least(220.0))
                    .column(Column::initial(150.0))
                    .column(Column::initial(205.0))
                    .column(Column::remainder().at_least(220.0))
                    .max_scroll_height(240.0)
                    .header(36.0, |mut h| {
                        for text in [
                            "Action",
                            "Source-файл",
                            "Общая длительность",
                            "Оставшаяся к обработке длительность",
                            "Destination-файл",
                        ] {
                            h.col(|ui| {
                                ui.strong(text);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(34.0, self.jobs.len(), |mut row| {
                            let j = &self.jobs[row.index()];
                            row.col(|ui| {
                                if icons::button(ui, Icon::Cancel, !j.stopping, "Отменить задачу")
                                    .clicked()
                                {
                                    cancels.push(j.id);
                                }
                            });
                            row.col(|ui| {
                                let p = j.spec.source.to_string_lossy();
                                ui.add(egui::Label::new(p.as_ref()).truncate())
                                    .on_hover_text(p.as_ref());
                            });
                            row.col(|ui| {
                                ui.label(
                                    j.duration.map(clock).unwrap_or_else(|| "неизвестна".into()),
                                );
                            });
                            row.col(|ui| {
                                ui.label(if j.stopping {
                                    "отмена…".into()
                                } else {
                                    j.remaining()
                                        .map(clock)
                                        .unwrap_or_else(|| "неизвестна".into())
                                });
                            });
                            row.col(|ui| {
                                let p = j.spec.output.to_string_lossy();
                                ui.add(egui::Label::new(p.as_ref()).truncate())
                                    .on_hover_text(p.as_ref());
                            });
                        });
                    });
                ui.separator();
                ui.label("Лог (последние 5 строк):");
                for i in 0..5 {
                    let line = self.logs.get(i).map(String::as_str).unwrap_or("");
                    ui.add(egui::Label::new(egui::RichText::new(line).monospace()).truncate())
                        .on_hover_text(line);
                }
                let _ = popup_height(ui, 5); // shared sizing policy remains available in this window
            });
        self.details = open;
        for id in cancels {
            if let Some(j) = self.jobs.iter_mut().find(|j| j.id == id) {
                j.cancel.store(true, Ordering::Relaxed);
                j.stopping = true;
            }
        }
    }
}
