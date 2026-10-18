use eframe::egui::{self, RichText};
use rom_art_rs::{engine::Settings, gamelist::Format};

pub(crate) fn draw(ui: &mut egui::Ui, settings: &mut Settings, paths_ready: bool) -> bool {
    ui.heading("Game lists");
    crate::muted(
        ui,
        "Export game names and artwork for EmulationStation or OnionOS.",
    );
    ui.add_space(8.0);
    let mut generate = false;
    crate::card(ui, |ui| {
        ui.checkbox(
            &mut settings.gamelist,
            "Generate game lists after artwork downloads",
        )
        .on_hover_text("Turn this off to download images without changing any game list. The local export button below always generates lists when clicked.");
        ui.separator();
        ui.label(RichText::new("List format").strong());
        egui::ComboBox::from_id_salt("gamelist_format")
            .selected_text(settings.gamelist_format.label())
            .width(330.0)
            .show_ui(ui, |ui| {
                for format in [Format::EmulationStation, Format::OnionOS, Format::Both] {
                    ui.selectable_value(&mut settings.gamelist_format, format, format.label());
                }
            });
        if settings.gamelist_format != Format::EmulationStation {
            crate::muted(
                ui,
                "OnionOS: compact path, name and image fields. Missing artwork uses a transparent placeholder. Choose both files to retain full metadata in gamelist.xml.",
            );
            crate::muted(
                ui,
                "Select one system folder. OnionOS displays games from subfolders as a flat list. Refresh all ROMs on the Miyoo after exporting.",
            );
        }
        crate::muted(
            ui,
            "Select one system folder. Existing entries and custom names are retained.",
        );
        ui.checkbox(&mut settings.backup_existing, "Back up replaced files")
            .on_hover_text("Applies to artwork and game lists. Each backup has a unique name.");
        generate = ui
            .add_enabled(paths_ready, egui::Button::new("Generate lists locally"))
            .clicked();
    });
    generate
}
