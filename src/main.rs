mod artwork_ui;
mod collection_ui;
mod connection_ui;
mod gamelist_ui;

use eframe::egui::{self, Color32, RichText};
use keyring::Entry;
use rom_art_rs::{
    engine::{Settings, generate_local_gamelist, run},
    provider::Candidate,
    worker::{Event, ReviewAction},
};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

const BG: Color32 = Color32::from_rgb(27, 30, 32);
const PANEL: Color32 = Color32::from_rgb(35, 38, 41);
const CARD: Color32 = Color32::from_rgb(42, 46, 50);
const TEXT: Color32 = Color32::from_rgb(239, 240, 241);
const MUTED: Color32 = Color32::from_rgb(167, 176, 184);
const BLUE: Color32 = Color32::from_rgb(61, 174, 233);
const KEYRING_SERVICE: &str = "miyoo-romscrapper";
const KEYRING_USER: &str = "steamgriddb-api-key";

fn keyring_entry() -> keyring::Result<Entry> {
    Entry::new(KEYRING_SERVICE, KEYRING_USER)
}

fn save_api_key(key: &str) -> keyring::Result<()> {
    keyring_entry()?.set_password(key)
}

fn delete_api_key() -> keyring::Result<()> {
    match keyring_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(error),
    }
}

struct Review {
    rom: PathBuf,
    query: String,
    candidates: Vec<Candidate>,
    arcade: bool,
    answer: mpsc::Sender<ReviewAction>,
}
struct App {
    settings: Settings,
    roots: Vec<collection_ui::SystemFolder>,
    output: String,
    receiver: Option<mpsc::Receiver<Event>>,
    stop: Arc<AtomicBool>,
    logs: Vec<String>,
    progress: (usize, usize),
    system_progress: (usize, usize, Option<String>),
    preview: Option<egui::TextureHandle>,
    preview_name: String,
    tab: usize,
    review: Option<Review>,
    artwork_review: Option<artwork_ui::ArtworkChoice>,
    remember_api_key: bool,
    credential_status: Option<String>,
}
impl Default for App {
    fn default() -> Self {
        let mut settings = Settings::default();
        let environment_key = std::env::var("STEAMGRIDDB_API_KEY").unwrap_or_default();
        let (saved_key, remember_api_key, credential_status) =
            match keyring_entry().and_then(|entry| entry.get_password()) {
                Ok(key) => (key, true, None),
                Err(keyring::Error::NoEntry) => (String::new(), false, None),
                Err(error) => (
                    String::new(),
                    false,
                    Some(format!(
                        "Could not read the system credential store: {error}"
                    )),
                ),
            };
        settings.credentials.api_key = if environment_key.is_empty() {
            saved_key
        } else {
            environment_key
        };
        Self {
            settings,
            roots: Vec::new(),
            output: String::new(),
            receiver: None,
            stop: Arc::new(AtomicBool::new(false)),
            logs: vec!["Ready to organize artwork for your collection.".into()],
            progress: (0, 0),
            system_progress: (0, 0, None),
            preview: None,
            preview_name: String::new(),
            tab: 0,
            review: None,
            artwork_review: None,
            remember_api_key,
            credential_status,
        }
    }
}

fn breeze_dark(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = BLUE;
    v.selection.bg_fill = Color32::from_rgb(35, 83, 109);
    v.selection.stroke = egui::Stroke::new(1.0_f32, TEXT);
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(61, 66, 71));
    v.widgets.inactive.bg_fill = CARD;
    v.widgets.inactive.weak_bg_fill = CARD;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0_f32, Color32::from_rgb(77, 84, 91));
    v.widgets.hovered.bg_fill = Color32::from_rgb(43, 65, 79);
    v.widgets.hovered.weak_bg_fill = v.widgets.hovered.bg_fill;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0_f32, BLUE);
    v.widgets.active.bg_fill = Color32::from_rgb(35, 83, 109);
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(12.0, 12.0);
        s.spacing.button_padding = egui::vec2(14.0, 9.0);
        s.spacing.interact_size.y = 32.0;
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(15.0));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(15.0));
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(25.0));
    });
}
pub(crate) fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED));
}
pub(crate) fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(7)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}
pub(crate) fn folder(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(RichText::new(label).strong());
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 110.0).max(150.0);
        ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(width)
                .hint_text("Select a folder…"),
        );
        if ui.button("Browse").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            *value = path.display().to_string();
        }
    });
}

fn settings_for_roots(
    base: &Settings,
    roots: &[collection_ui::SystemFolder],
    output: &Path,
) -> Vec<Settings> {
    let mut names = HashSet::new();
    roots
        .iter()
        .map(|root| {
            let mut settings = base.clone();
            settings.root = PathBuf::from(&root.path);
            settings.arcade_mode = root.arcade;
            if !root.output.trim().is_empty() {
                settings.output = PathBuf::from(root.output.trim());
            } else {
                settings.output = output.to_path_buf();
            }
            if root.output.trim().is_empty() && !settings.onion_images && roots.len() > 1 {
                let name = PathBuf::from(&root.path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let name = if name.is_empty() {
                    "System".to_owned()
                } else {
                    name
                };
                let mut output_name = name.clone();
                let mut suffix = 2;
                while !names.insert(output_name.clone()) {
                    output_name = format!("{name}-{suffix}");
                    suffix += 1;
                }
                settings.output.push(output_name);
            }
            settings
        })
        .collect()
}

fn effective_settings(settings: &Settings) -> Settings {
    let mut effective = settings.clone();
    if effective.turbo_mode {
        effective.auto_first_result = true;
        effective.choose_artwork = false;
    }
    effective
}

impl App {
    fn paths_ready(&self) -> bool {
        !self.roots.is_empty()
            && (self.settings.onion_images
                || self
                    .roots
                    .iter()
                    .all(|root| !root.output.trim().is_empty() || !self.output.trim().is_empty()))
    }
    fn start(&mut self, local_gamelist: bool) {
        let base_settings = effective_settings(&self.settings);
        let jobs = settings_for_roots(
            &base_settings,
            &self.roots,
            &PathBuf::from(self.output.trim()),
        );
        let job_count = jobs.len();
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.progress = (0, 0);
        self.system_progress = (0, job_count, None);
        self.logs.clear();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                for (index, settings) in jobs.into_iter().enumerate() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let system = settings
                        .root
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    let _ = tx.send(Event::SystemProgress(
                        index,
                        job_count,
                        Some(system.clone()),
                    ));
                    let _ = tx.send(Event::Log(format!(
                        "Processing system: {system} ({}/{job_count})",
                        index + 1
                    )));
                    let emit = |event| {
                        let _ = tx.send(event);
                    };
                    let result = if local_gamelist {
                        generate_local_gamelist(settings, &stop, emit)
                    } else {
                        run(settings, &stop, emit)
                    };
                    match result {
                        Ok(()) => {
                            let _ = tx.send(Event::Log(format!("System complete: {system}")));
                        }
                        Err(error) => {
                            let _ =
                                tx.send(Event::Log(format!("System failed: {system}: {error}")));
                        }
                    }
                    let _ = tx.send(Event::SystemProgress(index + 1, job_count, None));
                }
            }));
            if outcome.is_err() {
                let _ = tx.send(Event::Log("Unexpected processing error".into()));
            }
            let _ = tx.send(Event::Finished);
        });
    }
    fn poll(&mut self, ctx: &egui::Context) {
        let events: Vec<_> = self
            .receiver
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                Event::Log(text) => {
                    self.logs.push(text);
                    if self.logs.len() > 2000 {
                        self.logs.remove(0);
                    }
                }
                Event::Progress(a, b) => self.progress = (a, b),
                Event::SystemProgress(done, total, current) => {
                    self.system_progress = (done, total, current)
                }
                Event::Finished => {
                    self.receiver = None;
                    self.review = None;
                    self.artwork_review = None;
                }
                Event::Review {
                    rom,
                    query,
                    candidates,
                    arcade,
                    answer,
                } => {
                    self.review = Some(Review {
                        rom,
                        query,
                        candidates,
                        arcade,
                        answer,
                    })
                }
                Event::ArtworkReview {
                    rom,
                    game,
                    media,
                    previews,
                    answer,
                } => {
                    let previews = previews
                        .into_iter()
                        .enumerate()
                        .map(|(index, bytes)| {
                            bytes.and_then(|bytes| {
                                let image = image::load_from_memory(&bytes).ok()?.to_rgba8();
                                Some(ctx.load_texture(
                                    format!("artwork-option-{index}"),
                                    egui::ColorImage::from_rgba_unmultiplied(
                                        [image.width() as usize, image.height() as usize],
                                        image.as_raw(),
                                    ),
                                    egui::TextureOptions::LINEAR,
                                ))
                            })
                        })
                        .collect();
                    self.artwork_review = Some(artwork_ui::ArtworkChoice {
                        rom,
                        game,
                        media,
                        previews,
                        answer,
                    });
                }
                Event::Preview(path) => {
                    if let Ok(img) = image::open(&path) {
                        self.preview_name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into();
                        let img = img.thumbnail(400, 420).to_rgba8();
                        self.preview = Some(ctx.load_texture(
                            "preview",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [img.width() as usize, img.height() as usize],
                                img.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
            }
        }
    }
    fn review_window(&mut self, ctx: &egui::Context) {
        let mut action = None;
        if let Some(review) = &mut self.review {
            egui::Window::new("Choose a game")
                .collapsible(false)
                .resizable(true)
                .default_width(580.0)
                .show(ctx, |ui| {
                    ui.label(review.rom.file_name().unwrap_or_default().to_string_lossy());
                    let local_fallback = review.query.starts_with("Artwork unavailable:");
                    muted(
                        ui,
                        if local_fallback {
                            &review.query
                        } else {
                            "Confirm the correct edition or adjust the search title."
                        },
                    );
                    if !local_fallback
                        && ui
                            .checkbox(&mut review.arcade, "Arcade ROM (MAME / FBNeo)")
                            .on_hover_text("Use a local MAME/FBNeo DAT or XML catalog first, then existing gamelist names, then saved title aliases. If none match, search or select the correct game manually.")
                            .changed()
                    {
                        let _ = review.answer.send(ReviewAction::SetArcade(review.arcade));
                    }
                    if !local_fallback {
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut review.query).desired_width(380.0),
                            );
                            if ui
                                .add_enabled(
                                    !review.query.trim().is_empty(),
                                    egui::Button::new("Search"),
                                )
                                .clicked()
                            {
                                action = Some(ReviewAction::Search(review.query.trim().into()));
                            }
                        });
                    }
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            if review.candidates.is_empty() && !local_fallback {
                                muted(ui, "No results. Try another title.");
                            }
                            for candidate in &review.candidates {
                                ui.horizontal(|ui| {
                                    if ui.button("Use").clicked() {
                                        action = Some(ReviewAction::Select(candidate.clone()));
                                    }
                                    ui.label(&candidate.name);
                                    ui.hyperlink_to(
                                        format!("View #{} ↗", candidate.id),
                                        format!(
                                            "https://www.steamgriddb.com/game/{}",
                                            candidate.id
                                        ),
                                    );
                                });
                            }
                        });
                    if ui.button("Skip this ROM").clicked() {
                        action = Some(ReviewAction::Skip);
                    }
                    if ui.button("Choose a local image…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                            .pick_file()
                    {
                        action = Some(ReviewAction::LocalImage(path));
                    }
                });
        }
        if let Some(action) = action
            && let Some(review) = self.review.take()
        {
            let _ = review.answer.send(action);
        }
    }
    fn show(&mut self, ctx: &egui::Context) {
        let busy = self.receiver.is_some();
        egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(PANEL).inner_margin(16))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let (systems_done, systems_total, current_system) = &self.system_progress;
                    let system_fraction = if *systems_total > 0 {
                        *systems_done as f32 / *systems_total as f32
                    } else {
                        0.0
                    };
                    let system_text = current_system.as_ref().map_or_else(
                        || format!("{systems_done} / {systems_total} systems complete"),
                        |name| format!("System {} / {systems_total}: {name}", systems_done + 1),
                    );
                    let (done, total) = self.progress;
                    ui.vertical(|ui| {
                        ui.add(
                            egui::ProgressBar::new(system_fraction)
                                .desired_width(260.0)
                                .fill(BLUE)
                                .text(system_text),
                        );
                        ui.add(
                            egui::ProgressBar::new(if total > 0 {
                                done as f32 / total as f32
                            } else {
                                0.0
                            })
                            .desired_width(260.0)
                            .fill(BLUE)
                            .text(format!("{done} / {total} ROMs")),
                        );
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        muted(
                            ui,
                            if self.review.is_some() {
                                "Waiting for your choice"
                            } else if busy {
                                "Processing…"
                            } else {
                                "Ready"
                            },
                        );
                        if busy {
                            if ui.button("Cancel").clicked() {
                                self.stop.store(true, Ordering::Relaxed);
                            }
                        } else if ui
                            .add_enabled(
                                self.paths_ready()
                                    && !self.settings.credentials.api_key.trim().is_empty(),
                                egui::Button::new(
                                    RichText::new("Download artwork").color(BG).strong(),
                                )
                                .fill(BLUE),
                            )
                            .on_hover_text(
                                "Select the system folders and enter your key on Connection.",
                            )
                            .clicked()
                        {
                            self.start(false);
                        }
                    });
                });
            });
        egui::SidePanel::left("navigation")
            .exact_width(190.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(20))
            .show(ctx, |ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("ROM Art").size(28.0).strong());
                ui.label(
                    RichText::new("Your library, illustrated.")
                        .size(12.0)
                        .color(MUTED),
                );
                ui.add_space(30.0);
                for (index, label) in ["Collection", "Artwork", "Gamelist", "Connection"]
                    .iter()
                    .enumerate()
                {
                    let selected = self.tab == index;
                    let button = egui::Button::new(RichText::new(*label).color(if selected {
                        BLUE
                    } else {
                        TEXT
                    }))
                    .fill(if selected {
                        Color32::from_rgb(40, 60, 73)
                    } else {
                        PANEL
                    })
                    .stroke(egui::Stroke::NONE);
                    if ui.add_sized([150.0, 42.0], button).clicked() {
                        self.tab = index;
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    muted(ui, "Breeze Dark · 0.2");
                    ui.separator();
                    ui.label(RichText::new("SteamGridDB").strong());
                    ui.label(
                        RichText::new(if self.settings.credentials.api_key.trim().is_empty() {
                            "API key not configured"
                        } else {
                            "API key configured"
                        })
                        .size(12.0)
                        .color(MUTED),
                    );
                });
            });
        egui::TopBottomPanel::bottom("activity")
            .exact_height(155.0)
            .resizable(true)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(16))
            .show(ctx, |ui| {
                ui.label(RichText::new("Activity").strong());
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for log in &self.logs {
                            ui.label(RichText::new(log).size(13.0).color(MUTED));
                        }
                    });
            });
        egui::SidePanel::right("preview")
            .exact_width(300.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(BG).inner_margin(22))
            .show(ctx, |ui| {
                ui.label(RichText::new("PREVIEW").size(12.0).color(MUTED));
                ui.add_space(16.0);
                if let Some(texture) = &self.preview {
                    ui.add(
                        egui::Image::new(texture)
                            .max_size(egui::vec2(256.0, 350.0))
                            .maintain_aspect_ratio(true),
                    );
                    ui.label(&self.preview_name);
                } else {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(256.0, 280.0), egui::Sense::hover());
                    let painter = ui.painter();
                    painter.rect_filled(rect, 8.0, CARD);
                    let center = rect.center();
                    let icon = egui::Rect::from_center_size(
                        center - egui::vec2(0.0, 25.0),
                        egui::vec2(64.0, 84.0),
                    );
                    painter.rect_stroke(
                        icon,
                        4.0,
                        egui::Stroke::new(2.0_f32, MUTED),
                        egui::StrokeKind::Inside,
                    );
                    painter.line_segment(
                        [
                            icon.left_bottom() + egui::vec2(8.0, -18.0),
                            icon.center() + egui::vec2(0.0, 5.0),
                        ],
                        egui::Stroke::new(2.0_f32, BLUE),
                    );
                    painter.line_segment(
                        [
                            icon.center() + egui::vec2(0.0, 5.0),
                            icon.right_bottom() + egui::vec2(-8.0, -18.0),
                        ],
                        egui::Stroke::new(2.0_f32, BLUE),
                    );
                    painter.circle_filled(icon.left_top() + egui::vec2(18.0, 20.0), 5.0, BLUE);
                    painter.text(
                        center + egui::vec2(0.0, 55.0),
                        egui::Align2::CENTER_CENTER,
                        "Your next cover",
                        egui::FontId::proportional(16.0),
                        TEXT,
                    );
                    ui.add_space(6.0);
                    muted(ui, "Your latest saved artwork appears here.");
                }
                ui.add_space(16.0);
                ui.separator();
                ui.label(self.settings.kind.label());
                muted(
                    ui,
                    &format!(
                        "{} × {} px · PNG",
                        self.settings.width, self.settings.height
                    ),
                );
                muted(ui, "No cropping. Aspect ratio preserved.");
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BG).inner_margin(24))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.add_enabled_ui(!busy, |ui| match self.tab {
                        0 => collection_ui::draw(
                            ui,
                            &mut self.settings,
                            &mut self.roots,
                            &mut self.output,
                        ),
                        1 => artwork_ui::settings_panel(ui, &mut self.settings),
                        2 => {
                            let paths_ready = self.paths_ready();
                            if gamelist_ui::draw(ui, &mut self.settings, paths_ready) {
                                self.start(true);
                            }
                        }
                        _ => connection_ui::draw(
                            ui,
                            &mut self.settings.credentials.api_key,
                            &mut self.remember_api_key,
                            &mut self.credential_status,
                            |key| save_api_key(key).map_err(|error| error.to_string()),
                            || delete_api_key().map_err(|error| error.to_string()),
                        ),
                    });
                });
            });
        self.review_window(ctx);
        artwork_ui::review_window(ctx, &mut self.artwork_review);
    }
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if self.receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.show(ctx);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Miyoo ROMScrapper",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 820.0])
                .with_min_inner_size([1080.0, 740.0])
                .with_app_id("miyoo-romscrapper")
                .with_icon(app_icon()),
            ..Default::default()
        },
        Box::new(|cc| {
            breeze_dark(&cc.egui_ctx);
            Ok(Box::<App>::default())
        }),
    )
}

fn app_icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/miyoo-romscrapper.png"))
        .expect("embedded application icon should be valid")
        .to_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiple_systems_get_separate_output_folders() {
        let settings = Settings::default();
        let roots = vec![
            collection_ui::SystemFolder {
                path: "/roms/GBA".into(),
                output: String::new(),
                arcade: false,
            },
            collection_ui::SystemFolder {
                path: "/backup/GBA".into(),
                output: String::new(),
                arcade: true,
            },
        ];
        let jobs = settings_for_roots(&settings, &roots, Path::new("/artwork"));

        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].root, PathBuf::from("/roms/GBA"));
        assert_eq!(jobs[0].output, PathBuf::from("/artwork/GBA"));
        assert!(!jobs[0].arcade_mode);
        assert_eq!(jobs[1].root, PathBuf::from("/backup/GBA"));
        assert_eq!(jobs[1].output, PathBuf::from("/artwork/GBA-2"));
        assert!(jobs[1].arcade_mode);
    }

    #[test]
    fn turbo_uses_automatic_game_and_top_artwork_selection() {
        let settings = Settings {
            turbo_mode: true,
            auto_first_result: false,
            choose_artwork: true,
            concurrency: 6,
            ..Settings::default()
        };

        let effective = effective_settings(&settings);

        assert!(effective.auto_first_result);
        assert!(!effective.choose_artwork);
        assert_eq!(effective.concurrency, 6);
        assert!(settings.choose_artwork);
    }

    #[test]
    fn system_output_overrides_shared_output_base() {
        let settings = Settings::default();
        let roots = vec![collection_ui::SystemFolder {
            path: "/roms/GBA".into(),
            output: "/artwork/custom-gba".into(),
            arcade: false,
        }];
        let jobs = settings_for_roots(&settings, &roots, Path::new("/artwork/shared"));

        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].output, PathBuf::from("/artwork/custom-gba"));
    }

    #[test]
    fn all_pages_render_with_dark_theme() {
        let ctx = egui::Context::default();
        breeze_dark(&ctx);
        let mut app = App::default();
        for tab in 0..4 {
            app.tab = tab;
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1180.0, 820.0),
                    )),
                    ..Default::default()
                },
                |ctx| app.show(ctx),
            );
            assert!(!output.shapes.is_empty());
        }
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(ctx.style().visuals.panel_fill, PANEL);
    }

    #[test]
    fn embedded_window_icon_has_square_rgba_pixels() {
        let icon = app_icon();
        assert_eq!(icon.width, icon.height);
        assert_eq!(icon.rgba.len(), (icon.width * icon.height * 4) as usize);
    }
}
