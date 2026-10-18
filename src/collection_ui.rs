use eframe::egui::{self, RichText};
use rom_art_rs::engine::Settings;
use std::path::PathBuf;

pub(crate) fn draw(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    root: &mut String,
    output: &mut String,
) {
    ui.heading("Your collection");
    crate::muted(ui, "Choose your games folder and where to save artwork.");
    ui.add_space(8.0);
    crate::card(ui, |ui| {
        crate::folder(ui, "ROM folder", root);
        crate::muted(ui, "Subfolders are included.");
        ui.add_space(4.0);
        if settings.onion_images {
            ui.label(RichText::new("OnionOS artwork folder").strong());
            let directory = if root.trim().is_empty() {
                "<ROM folder>/Imgs".into()
            } else {
                PathBuf::from(root.trim())
                    .join("Imgs")
                    .display()
                    .to_string()
            };
            crate::muted(ui, &directory);
        } else {
            crate::folder(ui, "Save artwork to", output);
        }
    });
    ui.add_space(4.0);
    crate::card(ui, |ui| {
        ui.checkbox(&mut settings.arcade_mode, "Arcade ROMs (MAME / FBNeo)")
            .on_hover_text("Enable this for short arcade filenames. The app checks DAT/XML catalogs in the ROM folder, existing gamelists, then saved aliases. Manual matches are remembered. Turn off automatic first-result selection if you want to review each game.");
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
            crate::muted(ui, "Comma-separated extensions. ZIPs use the archive filename without extracting its contents.");
        });
        ui.checkbox(
            &mut settings.only_missing,
            "Only process ROMs without artwork in the destination",
        );
    });
}
