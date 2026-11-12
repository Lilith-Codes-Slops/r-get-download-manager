//! GUI-side coordination for confirmed starts. It never mutates the task list.
use crate::{
    download_confirmation, modal_style, native_file_dialog, settings_client, theme::Theme,
};
use eframe::egui::{self, RichText};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
};

type DirectoryResult = std::io::Result<PathBuf>;

#[derive(Default)]
pub struct ConfirmationFlow {
    sources: VecDeque<String>,
    directory: Option<Receiver<DirectoryResult>>,
    dialog: Option<download_confirmation::Confirmation>,
    picker: Option<Receiver<native_file_dialog::PickResult>>,
    start: Option<Receiver<std::io::Result<()>>>,
}

impl ConfirmationFlow {
    pub fn is_open(&self) -> bool {
        !self.sources.is_empty()
    }

    pub fn begin(&mut self, sources: Vec<String>, ctx: &egui::Context) {
        self.sources = sources.into();
        self.fetch_directory(ctx);
    }

    fn fetch_directory(&mut self, ctx: &egui::Context) {
        if self.sources.is_empty() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = sender.send(settings_client::load_directory());
            ctx.request_repaint();
        });
        self.directory = Some(receiver);
    }

    fn close(&mut self) {
        self.sources.clear();
        self.directory = None;
        self.dialog = None;
        self.picker = None;
        self.start = None;
    }

    pub fn show(&mut self, ctx: &egui::Context, palette: Theme) -> Option<String> {
        if !self.is_open() {
            return None;
        }
        if let Some(receiver) = &self.directory {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err(std::io::Error::other("Folder lookup stopped")))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.directory = None;
                match result {
                    Ok(folder) => {
                        // No trustworthy metadata lookup is wired yet. Never invent a
                        // proposed name from a URL or the torrent's filename.
                        self.dialog = Some(download_confirmation::Confirmation::new(
                            self.sources.front().unwrap().clone(),
                            String::new(),
                            &folder,
                        ));
                    }
                    Err(error) => {
                        self.close();
                        return Some(format!(
                            "Could not load the daemon's download folder: {error}"
                        ));
                    }
                }
            }
        }
        if let Some(receiver) = &self.picker {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err("The folder dialog closed unexpectedly.".into()))
                }
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.picker = None;
                if let Some(dialog) = &mut self.dialog {
                    match result {
                        Ok(Some(folder)) => dialog.set_folder(&folder),
                        Ok(None) => {}
                        Err(error) => dialog.set_error(error),
                    }
                }
            }
        }
        if let Some(receiver) = &self.start {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(std::io::Error::other(
                    "The start worker stopped; check the daemon queue before retrying.",
                ))),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                self.start = None;
                match result {
                    Ok(()) => {
                        self.sources.pop_front();
                        self.dialog = None;
                        self.fetch_directory(ctx);
                        if !self.is_open() {
                            return None;
                        }
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut self.dialog {
                            dialog.set_error(error.to_string());
                        }
                    }
                }
            }
        }
        if self.start.is_some() {
            // Once Confirm has been clicked, the outcome may be uncertain until
            // the daemon replies; do not offer a misleading cancellation.
            modal_style::modal("confirm_download_sending", palette).show(ctx, |ui| {
                ui.set_width(modal_style::width(ctx, 420.0));
                ui.label(
                    RichText::new("Starting download…")
                        .strong()
                        .color(palette.text),
                );
                ui.spinner();
            });
        } else if let Some(dialog) = &mut self.dialog {
            match dialog.show(ctx, palette, self.picker.is_some()) {
                Some(download_confirmation::Action::Cancel) => self.close(),
                Some(download_confirmation::Action::Browse) => {
                    self.picker = Some(native_file_dialog::pick_directory(ctx.clone()));
                }
                Some(download_confirmation::Action::Confirm { filename, folder }) => {
                    let url = self.sources.front().unwrap().clone();
                    let (sender, receiver) = mpsc::channel();
                    let ctx = ctx.clone();
                    std::thread::spawn(move || {
                        let _ =
                            sender.send(settings_client::start_confirmed(url, filename, &folder));
                        ctx.request_repaint();
                    });
                    self.start = Some(receiver);
                }
                None => {}
            }
        } else if self.directory.is_some() {
            let mut cancel = false;
            let response =
                modal_style::modal("confirm_download_loading", palette).show(ctx, |ui| {
                    ui.set_width(modal_style::width(ctx, 420.0));
                    ui.label(
                        RichText::new("Loading download folder…")
                            .strong()
                            .color(palette.text),
                    );
                    ui.spinner();
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            if cancel || response.should_close() {
                self.close();
            }
        }
        None
    }
}
