use eframe::egui::{self, Color32, RichText};
use rom_art_rs::{
    engine::{Event, ReviewAction, Settings, generate_local_gamelist, run},
    provider::Candidate,
};
use std::{
    path::PathBuf,
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

struct Review {
    rom: PathBuf,
    query: String,
    candidates: Vec<Candidate>,
    answer: mpsc::Sender<ReviewAction>,
}
struct App {
    settings: Settings,
    root: String,
    output: String,
    receiver: Option<mpsc::Receiver<Event>>,
    stop: Arc<AtomicBool>,
    logs: Vec<String>,
    progress: (usize, usize),
    preview: Option<egui::TextureHandle>,
    preview_name: String,
    tab: usize,
    review: Option<Review>,
}
impl Default for App {
    fn default() -> Self {
        let mut settings = Settings::default();
        settings.credentials.api_key = std::env::var("STEAMGRIDDB_API_KEY").unwrap_or_default();
        Self {
            settings,
            root: String::new(),
            output: String::new(),
            receiver: None,
            stop: Arc::new(AtomicBool::new(false)),
            logs: vec!["Ready to organize artwork for your collection.".into()],
            progress: (0, 0),
            preview: None,
            preview_name: String::new(),
            tab: 0,
            review: None,
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
fn muted(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).color(MUTED));
}
fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(7)
        .inner_margin(18)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            content(ui);
        });
}
fn folder(ui: &mut egui::Ui, label: &str, value: &mut String) {
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
fn media_label(kind: &str) -> &str {
    match kind {
        "grids" => "Covers",
        "heroes" => "Banners / heroes",
        "logos" => "Logos",
        "icons" => "Icons",
        "mix3" => "3-image composition",
        _ => kind,
    }
}

impl App {
    fn paths_ready(&self) -> bool {
        !self.root.trim().is_empty()
            && (self.settings.onion_images || !self.output.trim().is_empty())
    }
    fn start(&mut self, local_gamelist: bool) {
        self.settings.root = PathBuf::from(self.root.trim());
        self.settings.output = PathBuf::from(self.output.trim());
        self.stop = Arc::new(AtomicBool::new(false));
        let stop = self.stop.clone();
        let settings = self.settings.clone();
        let (tx, rx) = mpsc::channel();
        self.receiver = Some(rx);
        self.progress = (0, 0);
        self.logs.clear();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let emit = |event| {
                    let _ = tx.send(event);
                };
                if local_gamelist {
                    generate_local_gamelist(settings, &stop, emit)
                } else {
                    run(settings, &stop, emit)
                }
            }));
            match outcome {
                Ok(Err(e)) => {
                    let _ = tx.send(Event::Log(e.to_string()));
                }
                Err(_) => {
                    let _ = tx.send(Event::Log("Unexpected processing error".into()));
                }
                _ => {}
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
                Event::Finished => {
                    self.receiver = None;
                    self.review = None;
                }
                Event::Review {
                    rom,
                    query,
                    candidates,
                    answer,
                } => {
                    self.review = Some(Review {
                        rom,
                        query,
                        candidates,
                        answer,
                    })
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
    fn collection(&mut self, ui: &mut egui::Ui) {
        ui.heading("Your collection");
        muted(ui, "Choose your games folder and where to save artwork.");
        ui.add_space(8.0);
        card(ui, |ui| {
            folder(ui, "ROM folder", &mut self.root);
            muted(ui, "Subfolders are included.");
            ui.add_space(4.0);
            if self.settings.onion_images {
                ui.label(RichText::new("OnionOS artwork folder").strong());
                let directory = if self.root.trim().is_empty() {
                    "<ROM folder>/Imgs".into()
                } else {
                    PathBuf::from(self.root.trim())
                        .join("Imgs")
                        .display()
                        .to_string()
                };
                muted(ui, &directory);
            } else {
                folder(ui, "Save artwork to", &mut self.output);
            }
        });
        ui.add_space(4.0);
        card(ui, |ui| {
            ui.label(RichText::new("Title matching").strong());
            muted(
                ui,
                "Region and revision tags are removed from searches. You can choose the game when results are ambiguous.",
            );
            egui::CollapsingHeader::new("File types").show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.settings.extensions).desired_width(f32::INFINITY));
                muted(ui, "Comma-separated extensions. ZIPs use the archive filename without extracting its contents.");
            });
            ui.checkbox(
                &mut self.settings.only_missing,
                "Only process ROMs without artwork in the destination",
            );
        });
    }
    fn gamelist(&mut self, ui: &mut egui::Ui) {
        ui.heading("Game lists");
        muted(
            ui,
            "Export game names and artwork for EmulationStation or OnionOS.",
        );
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("List format").strong());
            egui::ComboBox::from_id_salt("gamelist_format")
                .selected_text(self.settings.gamelist_format.label())
                .width(330.0)
                .show_ui(ui, |ui| {
                    for format in [
                        rom_art_rs::gamelist::Format::EmulationStation,
                        rom_art_rs::gamelist::Format::OnionOS,
                        rom_art_rs::gamelist::Format::Both,
                    ] {
                        ui.selectable_value(
                            &mut self.settings.gamelist_format,
                            format,
                            format.label(),
                        );
                    }
                });
            if self.settings.gamelist_format != rom_art_rs::gamelist::Format::EmulationStation {
                muted(
                    ui,
                    "OnionOS: compact path, name and image fields. Missing artwork uses a transparent placeholder. Choose both files to retain full metadata in gamelist.xml.",
                );
                muted(
                    ui,
                    "Select one system folder. OnionOS displays games from subfolders as a flat list. Refresh all ROMs on the Miyoo after exporting.",
                );
            }
            ui.checkbox(
                &mut self.settings.gamelist,
                "Create / update the selected lists after downloading",
            );
            muted(
                ui,
                "Select one system folder. Existing entries and custom names are retained.",
            );
            ui.checkbox(&mut self.settings.backup_existing, "Back up replaced files");
            muted(
                ui,
                "Applies to artwork and game lists. Each backup has a unique name.",
            );
            if ui
                .add_enabled(
                    self.paths_ready(),
                    egui::Button::new("Generate lists locally"),
                )
                .clicked()
            {
                self.start(true);
            }
        });
    }
    fn images(&mut self, ui: &mut egui::Ui) {
        ui.heading("Artwork");
        muted(ui, "Choose artwork that fits your library.");
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.checkbox(
                &mut self.settings.auto_first_result,
                "Automatically use the first SteamGridDB game result",
            );
            muted(
                ui,
                "Turn this off to review search results before downloading.",
            );
            ui.label(RichText::new("Miyoo Mini / OnionOS presets").strong());
            ui.horizontal_wrapped(|ui| {
                for (label, width, height) in [
                    ("Portrait · 250 × 360", 250, 360),
                    ("Square · 250 × 250", 250, 250),
                    ("Landscape · 250 × 180", 250, 180),
                ] {
                    if ui.button(label).clicked() {
                        self.settings.onion_preset(width, height);
                    }
                }
            });
            ui.checkbox(
                &mut self.settings.onion_images,
                "Use OnionOS folder and filenames",
            );
            muted(
                ui,
                "Saves to <ROM folder>/Imgs/Game.png for Game.zip. Preserves aspect ratio without cropping. Select the list format on the Gamelist page.",
            );
            ui.separator();
            ui.label(RichText::new("Artwork type").strong());
            egui::ComboBox::from_id_salt("kind")
                .width(280.0)
                .selected_text(media_label(&self.settings.kind))
                .show_ui(ui, |ui| {
                    for kind in ["grids", "heroes", "logos", "icons", "mix3"] {
                        ui.selectable_value(
                            &mut self.settings.kind,
                            kind.into(),
                            media_label(kind),
                        );
                    }
                });
            muted(
                ui,
                "Selection prioritizes image orientation and community ratings.",
            );
            if self.settings.kind == "mix3" {
                muted(
                    ui,
                    "Combines a banner, cover and logo. All three components must be available.",
                );
            }
            ui.separator();
            ui.label(RichText::new("PNG dimensions").strong());
            ui.horizontal(|ui| {
                ui.label("Width");
                ui.add(
                    egui::DragValue::new(&mut self.settings.width)
                        .range(1..=4096)
                        .suffix(" px"),
                );
                ui.label("Height");
                ui.add(
                    egui::DragValue::new(&mut self.settings.height)
                        .range(1..=4096)
                        .suffix(" px"),
                );
            });
            ui.horizontal(|ui| {
                for (name, w, h) in [
                    ("Handheld", 250, 360),
                    ("Steam cover", 600, 900),
                    ("Banner", 920, 430),
                ] {
                    if ui.button(name).clicked() {
                        self.settings.width = w;
                        self.settings.height = h;
                    }
                }
            });
            muted(
                ui,
                "Aspect ratio is preserved with transparent padding and no cropping.",
            );
        });
        ui.add_space(4.0);
        card(ui, |ui| {
            ui.checkbox(&mut self.settings.overwrite, "Replace existing artwork");
            muted(ui, "Leave unchecked to keep your existing artwork.");
            ui.checkbox(&mut self.settings.backup_existing, "Back up replaced files");
            muted(
                ui,
                "Applies to artwork and game lists. Backups stay in the same folder.",
            );
        });
    }
    fn connection(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection");
        muted(
            ui,
            "Community artwork from SteamGridDB for your collection.",
        );
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("Personal API key").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.credentials.api_key)
                    .password(true)
                    .desired_width(f32::INFINITY)
                    .hint_text("Paste your SteamGridDB API key"),
            );
            ui.hyperlink_to(
                "Get my SteamGridDB key ↗",
                "https://www.steamgriddb.com/profile/preferences/api",
            );
            muted(
                ui,
                "Sign in and generate a key in your account preferences. Developer credentials are not required.",
            );
            ui.separator();
            muted(ui, "Your key stays in memory for this session only.");
        });
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
                    muted(
                        ui,
                        "Confirm the correct edition or adjust the search title.",
                    );
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut review.query).desired_width(380.0));
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
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            if review.candidates.is_empty() {
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
                    let (done, total) = self.progress;
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
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                            .on_hover_text("Selecione as pastas e informe a chave em Connection")
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
                ui.label(media_label(&self.settings.kind));
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
                        0 => self.collection(ui),
                        1 => self.images(ui),
                        2 => self.gamelist(ui),
                        _ => self.connection(ui),
                    });
                });
            });
        self.review_window(ctx);
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
        "ROM Art — SteamGridDB",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 820.0])
                .with_min_inner_size([1080.0, 740.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            breeze_dark(&cc.egui_ctx);
            Ok(Box::<App>::default())
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
