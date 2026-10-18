use crate::{CARD, card, muted};
use eframe::egui;
use rom_art_rs::{
    engine::{MAX_CONCURRENCY, Settings},
    provider::{ArtworkType, Candidate, Media},
    worker::ReviewAction,
};
use std::{path::PathBuf, sync::mpsc};

pub(crate) struct ArtworkChoice {
    pub rom: PathBuf,
    pub game: Candidate,
    pub media: Vec<Media>,
    pub previews: Vec<Option<egui::TextureHandle>>,
    pub answer: mpsc::Sender<ReviewAction>,
}

pub(crate) fn settings_panel(ui: &mut egui::Ui, settings: &mut Settings) {
    ui.heading("Artwork");
    muted(ui, "Choose artwork that fits your library.");
    ui.add_space(8.0);
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut settings.turbo_mode, "Turbo mode")
                .on_hover_text("Suggested for large collections. Automatically uses the first game result and the top-ranked image, skips confirmation and gallery previews, and processes more ROMs in parallel.");
            ui.separator();
            ui.label("Concurrent workers");
            egui::ComboBox::from_id_salt("concurrency")
                .selected_text(settings.concurrency.to_string())
                .width(64.0)
                .show_ui(ui, |ui| {
                    for workers in 1..=MAX_CONCURRENCY {
                        ui.selectable_value(
                            &mut settings.concurrency,
                            workers,
                            workers.to_string(),
                        );
                    }
                })
                .response
                .on_hover_text("Sets the maximum number of concurrent workers for each stage: game searches and artwork downloads. Higher values may use more bandwidth or encounter API throttling.");
        });
        ui.add_enabled_ui(!settings.turbo_mode, |ui| {
            ui.checkbox(
                &mut settings.auto_first_result,
                "Automatically use the first SteamGridDB game result",
            )
            .on_hover_text(
                "Turn this off to review results and choose the correct game before downloading.",
            );
            if !settings.auto_first_result {
                muted(
                    ui,
                    "Manual selection: edit the title and click Search, then click Use beside the correct game. If none match, search again, choose a local image, or skip the ROM.",
                );
            }
            ui.checkbox(
                &mut settings.choose_artwork,
                "Choose artwork from a gallery",
            )
            .on_hover_text(
                "After matching a game, compare available SteamGridDB images for the selected type.",
            );
        });
        if settings.choose_artwork && !settings.turbo_mode {
            muted(
                ui,
                "Gallery mode may take longer while artwork previews are downloaded.",
            );
        }
        ui.label(egui::RichText::new("Miyoo Mini / OnionOS presets").strong());
        ui.horizontal_wrapped(|ui| {
            for (label, width, height) in [
                ("Portrait · 250 × 360", 250, 360),
                ("Square · 250 × 250", 250, 250),
                ("Landscape · 250 × 180", 250, 180),
            ] {
                if ui.button(label).clicked() {
                    settings.onion_preset(width, height);
                }
            }
        });
        ui.separator();
        ui.label(egui::RichText::new("Artwork type").strong());
        egui::ComboBox::from_id_salt("kind")
            .width(280.0)
            .selected_text(settings.kind.label())
            .show_ui(ui, |ui| {
                for kind in ArtworkType::ALL {
                    ui.selectable_value(&mut settings.kind, kind, kind.label());
                }
            });
        muted(
            ui,
            "Selection prioritizes image orientation and community ratings.",
        );
        ui.separator();
        ui.label(egui::RichText::new("PNG dimensions").strong());
        ui.horizontal(|ui| {
            ui.label("Width");
            ui.add(
                egui::DragValue::new(&mut settings.width)
                    .range(1..=4096)
                    .suffix(" px"),
            );
            ui.label("Height");
            ui.add(
                egui::DragValue::new(&mut settings.height)
                    .range(1..=4096)
                    .suffix(" px"),
            );
        });
        ui.horizontal(|ui| {
            for (name, width, height) in [
                ("Handheld", 250, 360),
                ("Steam cover", 600, 900),
                ("Banner", 920, 430),
            ] {
                if ui.button(name).clicked() {
                    settings.width = width;
                    settings.height = height;
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
        ui.checkbox(&mut settings.overwrite, "Replace existing artwork")
            .on_hover_text("Leave unchecked to keep your existing artwork.");
        ui.checkbox(&mut settings.backup_existing, "Back up replaced files")
            .on_hover_text("Applies to artwork and game lists. Backups stay in the same folder.");
    });
}

pub(crate) fn review_window(ctx: &egui::Context, review: &mut Option<ArtworkChoice>) {
    let mut action = None;
    if let Some(review) = review {
        egui::Window::new("Choose artwork")
            .collapsible(false)
            .resizable(true)
            .default_width(760.0)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} · {}",
                    review.rom.file_name().unwrap_or_default().to_string_lossy(),
                    review.game.name
                ));
                muted(ui, "Options are ordered by matching orientation, then community score. Some previews may be unavailable; the source file remains selectable.");
                let columns = (ui.available_width() / 220.0).floor().max(1.0) as usize;
                egui::ScrollArea::both()
                    .max_height(520.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        egui::Grid::new("artwork_options_grid")
                            .num_columns(columns)
                            .spacing(egui::vec2(10.0, 10.0))
                            .show(ui, |ui| {
                                for (index, media) in review.media.iter().enumerate() {
                                    egui::Frame::new()
                                        .fill(CARD)
                                        .corner_radius(6)
                                        .inner_margin(10)
                                        .show(ui, |ui| {
                                            ui.set_width(190.0);
                                            ui.set_min_height(300.0);
                                            ui.vertical(|ui| {
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(170.0, 180.0),
                                                    egui::Layout::centered_and_justified(
                                                        egui::Direction::TopDown,
                                                    ),
                                                    |ui| {
                                                    if let Some(Some(texture)) =
                                                        review.previews.get(index)
                                                    {
                                                        ui.add(
                                                            egui::Image::new(texture)
                                                                .max_size(egui::vec2(165.0, 175.0))
                                                                .maintain_aspect_ratio(true),
                                                        );
                                                    } else {
                                                        muted(ui, "Preview unavailable");
                                                    }
                                                    },
                                                );
                                                ui.add_sized(
                                                    [ui.available_width(), 20.0],
                                                    egui::Label::new(format!(
                                                        "{} × {}",
                                                        media.width, media.height
                                                    )),
                                                );
                                                ui.add_sized(
                                                    [ui.available_width(), 20.0],
                                                    egui::Label::new(format!("Score: {}", media.score)),
                                                );
                                                ui.add_sized(
                                                    [ui.available_width(), 22.0],
                                                    egui::Hyperlink::from_label_and_url(
                                                        "Open original ↗",
                                                        media.url.clone(),
                                                    ),
                                                );
                                                if ui
                                                    .add_sized(
                                                        [ui.available_width(), 30.0],
                                                        egui::Button::new("Use this artwork"),
                                                    )
                                                    .clicked()
                                                {
                                                    action = Some(ReviewAction::SelectArtwork(
                                                        media.clone(),
                                                    ));
                                                }
                                            });
                                        });
                                }
                            });
                    });
                if ui.button("Skip this ROM").clicked() {
                    action = Some(ReviewAction::Skip);
                }
            });
    }
    if let Some(action) = action
        && let Some(review) = review.take()
    {
        let _ = review.answer.send(action);
    }
}
