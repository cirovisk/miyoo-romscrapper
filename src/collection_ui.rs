use eframe::egui::{self, RichText};
use rom_art_rs::engine::Settings;
use std::path::PathBuf;

pub(crate) struct SystemFolder {
    pub path: String,
    pub output: String,
    pub arcade: bool,
}

pub(crate) fn draw(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    roots: &mut Vec<SystemFolder>,
    output: &mut String,
) {
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
        if ui.button("Add system folder").clicked()
            && let Some(path) = rfd::FileDialog::new().pick_folder()
        {
            let path = path.display().to_string();
            if !roots.iter().any(|folder| folder.path == path) {
                roots.push(SystemFolder {
                    path,
                    output: String::new(),
                    arcade: false,
                });
            }
        }
        if roots.is_empty() {
            crate::muted(ui, "No system folders selected.");
        } else {
            crate::muted(
                ui,
                "Add folders one at a time; click the button again for each system.",
            );
        }
        let mut remove = None;
        let controls_width = 200.0;
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
                });
        }
        if let Some(index) = remove {
            roots.remove(index);
        }
        crate::muted(ui, "Subfolders are included.");
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
    });
}
