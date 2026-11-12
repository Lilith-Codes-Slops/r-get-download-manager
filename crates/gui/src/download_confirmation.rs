//! Compact confirmation dialog. Starting a task remains the caller's responsibility.
use crate::{icons, modal_style, theme::Theme};
use eframe::egui::{self, Align, Button, Color32, Layout, RichText, Stroke, TextEdit, Vec2};
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Cancel,
    Browse,
    Confirm { filename: String, folder: PathBuf },
}

pub struct Confirmation {
    source: String,
    filename: String,
    folder: String,
    error: Option<String>,
}

impl Confirmation {
    pub fn new(source: String, filename: String, folder: &Path) -> Self {
        let error = filename
            .is_empty()
            .then(|| "Filename metadata is unavailable; enter a filename.".into());
        Self {
            source,
            filename,
            folder: folder.to_string_lossy().into_owned(),
            error,
        }
    }

    pub fn set_error(&mut self, error: String) {
        self.error = Some(error);
    }

    pub fn set_folder(&mut self, folder: &Path) {
        self.folder = folder.to_string_lossy().into_owned();
        self.error = None;
    }

    fn confirmed(&self) -> Result<Action, &'static str> {
        let (filename, folder) =
            crate::download_confirmation_logic::validate(&self.filename, &self.folder)?;
        Ok(Action::Confirm { filename, folder })
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: Theme, picking: bool) -> Option<Action> {
        let mut action = None;
        let response = modal_style::modal("confirm_download", palette).show(ctx, |ui| {
            ui.set_width(modal_style::width(ctx, 420.0));
            ui.label(
                RichText::new("Confirm download")
                    .size(16.0)
                    .strong()
                    .color(palette.text),
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(10.0);
            ui.add(
                egui::Label::new(RichText::new(&self.source).color(palette.text_dim)).truncate(),
            )
            .on_hover_text(&self.source);
            ui.add_space(6.0);
            ui.label(RichText::new("File name").strong().color(palette.text));
            if ui
                .add(TextEdit::singleline(&mut self.filename).desired_width(f32::INFINITY))
                .changed()
            {
                self.error = None;
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new("Download folder")
                    .strong()
                    .color(palette.text),
            );
            ui.horizontal(|ui| {
                let field_width = (ui.available_width() - 38.0).max(0.0);
                if ui
                    .add(TextEdit::singleline(&mut self.folder).desired_width(field_width))
                    .changed()
                {
                    self.error = None;
                }
                let browse = ui.add_enabled(
                    !picking,
                    Button::new(icons::atom(icons::Icon::Folder, 16.0, palette.text))
                        .min_size(Vec2::splat(30.0)),
                );
                browse.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        !picking,
                        "Browse download folder",
                    )
                });
                if browse.on_hover_text("Browse download folder").clicked() {
                    action = Some(Action::Browse);
                }
            });
            if let Some(error) = &self.error {
                ui.colored_label(palette.red, error);
            }
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let confirm = ui.add_enabled(
                    !picking,
                    Button::new(RichText::new("Confirm").color(Color32::WHITE))
                        .fill(palette.accent)
                        .stroke(Stroke::NONE)
                        .corner_radius(8),
                );
                if confirm.clicked() {
                    match self.confirmed() {
                        Ok(value) => action = Some(value),
                        Err(error) => self.error = Some(error.to_owned()),
                    }
                }
                if ui.button("Cancel").clicked() {
                    action = Some(Action::Cancel);
                }
            });
        });
        if response.should_close() {
            Some(Action::Cancel)
        } else {
            action
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_metadata_never_invents_a_filename() {
        let mut confirmation = Confirmation::new(
            "magnet:?xt=example".into(),
            String::new(),
            Path::new("/saved"),
        );
        assert!(confirmation
            .error
            .as_deref()
            .unwrap()
            .contains("metadata is unavailable"));
        assert!(confirmation.confirmed().is_err());
        confirmation.filename = "chosen.bin".into();
        confirmation.set_folder(Path::new("/one-time folder"));
        assert_eq!(
            confirmation.confirmed().unwrap(),
            Action::Confirm {
                filename: "chosen.bin".into(),
                folder: PathBuf::from("/one-time folder"),
            }
        );
    }

    #[test]
    fn confirmation_rejects_unsafe_name_and_folder() {
        let mut confirmation = Confirmation::new(
            "https://example.test/file".into(),
            "file.txt".into(),
            Path::new("/downloads"),
        );
        assert_eq!(
            confirmation.confirmed().unwrap(),
            Action::Confirm {
                filename: "file.txt".into(),
                folder: PathBuf::from("/downloads")
            }
        );
        confirmation.filename = "../escape".into();
        assert!(confirmation.confirmed().is_err());
        confirmation.filename = "file.txt".into();
        confirmation.folder = "relative".into();
        assert!(confirmation.confirmed().is_err());
        confirmation.folder = "/downloads/../other".into();
        assert!(confirmation.confirmed().is_err());
    }
}
