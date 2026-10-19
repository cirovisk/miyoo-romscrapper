use eframe::egui::{self, RichText};
use rom_art_rs::{engine::Settings, systems::ONION_SYSTEMS};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SystemFolder {
    pub path: String,
    pub output: String,
    pub arcade: bool,
    #[serde(default)]
    pub system_id: String,
    #[serde(default)]
    pub nointro_dat: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CollectionAction {
    ImportLocalArtwork,
    ReportOrphanArtwork,
    RenameNoIntro,
}

fn detect_system_id(path: &str) -> String {
    let folder = PathBuf::from(path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_uppercase();
    ONION_SYSTEMS
        .iter()
        .find(|system| system.id == folder)
        .map_or_else(String::new, |system| system.id.to_owned())
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    roots: &mut Vec<SystemFolder>,
    output: &mut String,
    local_artwork: &mut String,
) -> Option<CollectionAction> {
    let mut action = None;
    ui.heading("Your collection");
    crate::muted(
        ui,
        "Choose one or more system folders and where to save artwork.",
    );
    ui.add_space(8.0);
    crate::card(ui, |ui| {
        ui.label(RichText::new("System folders").strong());
        ui.checkbox(
            &mut settings.onion_images,
            "Use OnionOS folders and filenames",
        )
        .on_hover_text("Saves artwork to <ROM folder>/Imgs/Game.png for Game.zip. Preserves aspect ratio without cropping. Select the list format on the Gamelist page.");
        if ui.button("Add system folders").clicked()
            && let Some(paths) = rfd::FileDialog::new().pick_folders()
        {
            for path in paths {
                let path = path.display().to_string();
                if !roots.iter().any(|folder| folder.path == path) {
                    let system_id = detect_system_id(&path);
                    roots.push(SystemFolder {
                        arcade: system_id == "ARCADE",
                        system_id,
                        path,
                        output: String::new(),
                        nointro_dat: String::new(),
                    });
                }
            }
        }
        if roots.is_empty() {
            crate::muted(ui, "No system folders selected.");
        } else {
            crate::muted(
                ui,
                "Select system types for device profiles; duplicate folders are ignored.",
            );
        }
        let mut remove = None;
        let controls_width = 370.0;
        let input_width = (ui.available_width() - controls_width - 12.0).max(180.0);
        for (index, root) in roots.iter_mut().enumerate() {
            egui::Grid::new(("system_folder", index))
                .num_columns(2)
                .spacing(egui::vec2(12.0, 8.0))
                .show(ui, |ui| {
                    ui.add_sized(
                        [input_width, 32.0],
                        egui::Label::new(&root.path)
                            .truncate()
                            .halign(egui::Align::Min),
                    );
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt(("onion_system", index))
                            .selected_text(if root.system_id.is_empty() {
                                "Other"
                            } else {
                                &root.system_id
                            })
                            .width(116.0)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_label(root.system_id.is_empty(), "Other")
                                    .clicked()
                                {
                                    root.system_id.clear();
                                }
                                for system in ONION_SYSTEMS {
                                    if ui
                                        .selectable_label(root.system_id == system.id, system.id)
                                        .on_hover_text(format!(
                                            "Onionscraper platform mapping: {}",
                                            system.platforms
                                        ))
                                        .clicked()
                                    {
                                        root.system_id = system.id.into();
                                        if system.id == "ARCADE" {
                                            root.arcade = true;
                                        }
                                    }
                                }
                            });
                        ui.checkbox(&mut root.arcade, "Arcade").on_hover_text(
                            "Enable MAME / FBNeo title lookup for this system folder.",
                        );
                        if ui
                            .add_sized([76.0, 32.0], egui::Button::new("Remove"))
                            .clicked()
                        {
                            remove = Some(index);
                        }
                    });
                    ui.end_row();
                    if !settings.onion_images {
                        ui.add_sized(
                            [input_width, 32.0],
                            egui::TextEdit::singleline(&mut root.output)
                                .hint_text("Use shared output / system subfolder"),
                        );
                        if ui
                            .add_sized([controls_width, 32.0], egui::Button::new("Browse output"))
                            .clicked()
                            && let Some(path) = rfd::FileDialog::new().pick_folder()
                        {
                            root.output = path.display().to_string();
                        }
                        ui.end_row();
                    }
                    ui.add_sized(
                        [input_width, 32.0],
                        egui::TextEdit::singleline(&mut root.nointro_dat)
                            .hint_text("Select this system's No-Intro DAT file"),
                    );
                    if ui
                        .add_sized([controls_width, 32.0], egui::Button::new("Browse DAT…"))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("No-Intro DAT", &["dat", "xml"])
                            .pick_file()
                    {
                        root.nointro_dat = path.display().to_string();
                    }
                    ui.end_row();
                });
        }
        if let Some(index) = remove {
            roots.remove(index);
        }
        crate::muted(ui, "Subfolders are included.");
        let dats_ready = !roots.is_empty()
            && roots
                .iter()
                .all(|root| PathBuf::from(&root.nointro_dat).is_file());
        if ui
            .add_enabled(dats_ready, egui::Button::new("Preview No-Intro renaming"))
            .on_hover_text(
                "Matches ROM content against each system's DAT before proposing filenames.",
            )
            .clicked()
        {
            action = Some(CollectionAction::RenameNoIntro);
        }
        crate::muted(
            ui,
            "Uses exact CRC32 / SHA-1 matches. Unmatched ROMs are shown for review and never renamed automatically.",
        );
        ui.add_space(4.0);
        if settings.onion_images {
            ui.label(RichText::new("OnionOS artwork folder").strong());
            for root in roots.iter() {
                let directory = PathBuf::from(&root.path).join("Imgs").display().to_string();
                crate::muted(ui, &directory);
            }
        } else {
            crate::folder(ui, "Shared output base (optional)", output);
            crate::muted(
                ui,
                "A system-specific output above overrides this shared location.",
            );
        }
    });
    ui.add_space(4.0);
    crate::card(ui, |ui| {
        ui.label(RichText::new("Local artwork and cleanup report").strong());
        crate::muted(
            ui,
            "Match images by title from a folder, preview matches, and review orphaned PNGs without deleting them.",
        );
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(local_artwork)
                    .desired_width((ui.available_width() - 100.0).max(180.0))
                    .hint_text("Choose an artwork folder…"),
            );
            if ui.button("Browse").clicked()
                && let Some(path) = rfd::FileDialog::new().pick_folder()
            {
                *local_artwork = path.display().to_string();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !roots.is_empty() && PathBuf::from(local_artwork.as_str()).is_dir(),
                    egui::Button::new("Preview local import"),
                )
                .clicked()
            {
                action = Some(CollectionAction::ImportLocalArtwork);
            }
            if ui
                .add_enabled(
                    !roots.is_empty(),
                    egui::Button::new("Preview orphan artwork"),
                )
                .on_hover_text("Creates a report only. No files are removed.")
                .clicked()
            {
                action = Some(CollectionAction::ReportOrphanArtwork);
            }
        });
    });
    ui.add_space(4.0);
    crate::card(ui, |ui| {
        ui.label(RichText::new("Title matching").strong());
        crate::muted(
            ui,
            "Region and revision tags are removed from searches. You can choose the game when results are ambiguous.",
        );
        egui::CollapsingHeader::new("File types").show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut settings.extensions).desired_width(f32::INFINITY));
            crate::muted(ui, "ROM extensions separated by commas, spaces or semicolons; a leading dot is optional. Archives are matched by filename without extracting their contents.");
        });
        ui.checkbox(
            &mut settings.only_missing,
            "Only process ROMs without artwork in the destination",
        );
        ui.checkbox(
            &mut settings.only_new_or_changed,
            "Only process new or changed ROMs",
        )
        .on_hover_text("Tracks ROM size and modification time in .rom-art-state.json inside each system folder. Missing artwork is always processed. If both filters are enabled, ROMs missing artwork or changed since the last run are processed.");
    });
    action
}
