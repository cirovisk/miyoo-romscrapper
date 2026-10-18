use eframe::egui::{self, RichText};

pub(crate) fn draw(
    ui: &mut egui::Ui,
    api_key: &mut String,
    remember_api_key: &mut bool,
    credential_status: &mut Option<String>,
    mut save_key: impl FnMut(&str) -> Result<(), String>,
    mut delete_key: impl FnMut() -> Result<(), String>,
) {
    ui.heading("Connection");
    crate::muted(
        ui,
        "Community artwork from SteamGridDB for your collection.",
    );
    ui.add_space(8.0);
    crate::card(ui, |ui| {
        ui.label(RichText::new("Personal API key").strong());
        ui.add(
            egui::TextEdit::singleline(api_key)
                .password(true)
                .desired_width(f32::INFINITY)
                .hint_text("Paste your SteamGridDB API key"),
        );
        ui.hyperlink_to(
            "Get my SteamGridDB key ↗",
            "https://www.steamgriddb.com/profile/preferences/api",
        );
        crate::muted(
            ui,
            "Sign in and generate a key in your account preferences. Developer credentials are not required.",
        );
        ui.separator();
        if ui
            .checkbox(
                remember_api_key,
                "Remember this key securely between sessions",
            )
            .on_hover_text(
                "Save stores the key in your desktop credential wallet, not in app settings.",
            )
            .changed()
            && !*remember_api_key
        {
            *credential_status = Some(match delete_key() {
                Ok(()) => "Saved key removed from the system credential store.".into(),
                Err(error) => format!("Could not remove the saved key: {error}"),
            });
        }
        if *remember_api_key {
            if ui
                .add_enabled(
                    !api_key.trim().is_empty(),
                    egui::Button::new("Save key securely"),
                )
                .clicked()
            {
                *credential_status = Some(match save_key(api_key.trim()) {
                    Ok(()) => "API key saved to the system credential store.".into(),
                    Err(error) => format!("Could not save the key: {error}"),
                });
            }
        } else {
            crate::muted(ui, "Your key stays in memory for this session only.");
        }
        if let Some(status) = credential_status {
            crate::muted(ui, status);
        }
    });
}
