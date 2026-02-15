use crate::clippo_app::ClippoApp;

use eframe::egui;

impl eframe::App for ClippoApp {
    // Handles UI updates.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Setting style once only
        if self.style_needs_update {
            let mut style = (*ctx.style()).clone();
            style.text_styles.insert(
                egui::TextStyle::Button,
                egui::FontId::new(18.0, egui::FontFamily::Proportional),
            );
            ctx.set_style(style);
            self.style_needs_update = false;
        }

        if self.config.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(5.);

            // PREFERENCES
            egui::menu::bar(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::LEFT), |ui| {
                    ui.menu_button("Preferences", |ui| {
                        // Handle the config update with the toggle_config_field method to avoid code repetition
                        if ui
                            .checkbox(&mut self.config.minimize_on_copy, "Minimize on copy")
                            .clicked()
                        {
                            self.toggle_config_field("minimize_on_copy");
                        }
                        if ui
                            .checkbox(&mut self.config.minimize_on_clear, "Minimize on clear")
                            .clicked()
                        {
                            self.toggle_config_field("minimize_on_clear");
                        }

                        if ui
                            .checkbox(
                                &mut self.config.enable_search,
                                "Enable search functionality",
                            )
                            .clicked()
                        {
                            self.toggle_config_field("enable_search");
                        }

                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.config.max_entry_display_length,
                                    10..=500,
                                )
                                .text("max entry display length"),
                            )
                            .changed()
                        {
                            self.toggle_config_field("max_entry_display_length");
                        }
                    });

                    // DARK / LIGHT MODE
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::RIGHT), |ui| {
                        let logo = if self.config.dark_mode {
                            "🔆"
                        } else {
                            "🌙"
                        };
                        if ui.button(logo).clicked() {
                            self.config.dark_mode = !self.config.dark_mode;
                            self.toggle_config_field("dark_mode");
                        }
                    });

                    ui.add_space(10.);
                })
            });
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            // Main content
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    // Search input
                    if self.config.enable_search {
                        ui.text_edit_singleline(&mut self.search_query);
                        ui.add_space(10.0);
                    };

                    // Clear history
                    if ui
                        .button("🗑")
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                    {
                        let _ = self.clear_history();
                        // Optionally minimize after clearing the history
                        if self.config.minimize_on_clear {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                        }
                        tracing::info!("History cleared.");
                    }
                });
                ui.add_space(10.0);

                // Iterate through every value of the history
                if let Ok(history) = self.history_cache.lock() {
                    for value in history.iter() {
                        // Filtering based on search query
                        if self.search_query.trim().is_empty()
                            || value.trim().contains(&self.search_query)
                        {
                            // Display entry with helper method
                            self.display_history_entry(ui, ctx, &value);
                        }
                    }
                }
            });
        });

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(10.);
                ui.add(egui::Hyperlink::from_label_and_url(
                    "Made with egui",
                    "https://github.com/emilk/egui",
                ));
                ui.add_space(7.);
                ui.add(egui::Hyperlink::from_label_and_url(
                    "Source Code",
                    "https://github.com/Rayanworkout/clippo",
                ))
            });
            ui.add_space(7.);
        });

        // Ensure UI updates regularly
        ctx.request_repaint();
    }
}
