use crate::clippo_app::{ClipboardHistoryEntry, ClippoApp};

use eframe::egui;
use std::time::Duration;

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

        if self.config.enable_search
            && ctx.input(|input| input.modifiers.command && input.key_pressed(egui::Key::F))
        {
            self.search_focus_requested = true;
        }

        if self.config.enable_search
            && ctx.input(|input| input.key_pressed(egui::Key::Escape))
            && !self.search_query.is_empty()
        {
            self.search_query.clear();
            self.confirm_clear = false;
            self.set_last_action("Search cleared.");
        }

        let normalized_query = if self.config.enable_search {
            self.search_query.trim().to_lowercase()
        } else {
            String::new()
        };

        let (total_entries, filtered_history) = if let Ok(history) = self.history_cache.lock() {
            let total = history.len();
            let filtered = history
                .iter()
                .filter(|entry| {
                    if normalized_query.is_empty() {
                        return true;
                    }

                    match entry {
                        ClipboardHistoryEntry::Text(value) => {
                            value.to_lowercase().contains(&normalized_query)
                        }
                        ClipboardHistoryEntry::Image(image) => {
                            format!("image {}x{}", image.width, image.height)
                                .contains(&normalized_query)
                        }
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            (total, filtered)
        } else {
            (0, Vec::new())
        };
        let filtered_entries = filtered_history.len();
        let search_input_id = egui::Id::new("search_input");

        if filtered_entries == 0 {
            self.selected_entry_index = None;
        } else {
            let max_index = filtered_entries - 1;
            self.selected_entry_index = Some(
                self.selected_entry_index
                    .map(|idx| idx.min(max_index))
                    .unwrap_or(0),
            );
        }

        let search_has_focus =
            self.config.enable_search && ctx.memory(|memory| memory.has_focus(search_input_id));
        let mut selection_changed_with_keyboard = false;

        if !search_has_focus && filtered_entries > 0 {
            if ctx.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
                let next_idx = match self.selected_entry_index {
                    Some(idx) => (idx + 1).min(filtered_entries - 1),
                    None => 0,
                };
                self.selected_entry_index = Some(next_idx);
                selection_changed_with_keyboard = true;
            }

            if ctx.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
                let next_idx = match self.selected_entry_index {
                    Some(idx) => idx.saturating_sub(1),
                    None => 0,
                };
                self.selected_entry_index = Some(next_idx);
                selection_changed_with_keyboard = true;
            }

            if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
                if let Some(selected_idx) = self.selected_entry_index {
                    if let Some(selected_value) = filtered_history.get(selected_idx).cloned() {
                        if let Err(error) = self.copy_to_clipboard(&selected_value) {
                            tracing::error!("Could not copy selected entry with Enter: {error:#}");
                            self.set_last_action("Failed to copy entry to clipboard.");
                        } else {
                            self.confirm_clear = false;
                            self.set_last_action("Entry copied to clipboard.");
                            if self.config.minimize_on_copy {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                    }
                }
            }
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.heading("Clippo");
                ui.label(
                    egui::RichText::new(format!("{filtered_entries}/{total_entries} shown")).weak(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let theme_icon = if self.config.dark_mode {
                        "🔆"
                    } else {
                        "🌙"
                    };
                    let theme_hint = if self.config.dark_mode {
                        "Switch to light mode"
                    } else {
                        "Switch to dark mode"
                    };
                    if ui.button(theme_icon).on_hover_text(theme_hint).clicked() {
                        self.config.dark_mode = !self.config.dark_mode;
                        self.toggle_config_field("dark_mode");
                    }
                    ui.menu_button("☰", |ui| {
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
                            .checkbox(&mut self.config.enable_search, "Enable search")
                            .clicked()
                        {
                            self.search_query.clear();
                            self.search_focus_requested = false;
                            self.toggle_config_field("enable_search");
                        }

                        if ui
                            .add(
                                egui::Slider::new(
                                    &mut self.config.max_entry_display_length,
                                    10..=500,
                                )
                                .text("Preview length"),
                            )
                            .changed()
                        {
                            self.toggle_config_field("max_entry_display_length");
                        }
                    });
                });
            });
            ui.add_space(2.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.config.enable_search {
                ui.horizontal(|ui| {
                    let search_input = egui::TextEdit::singleline(&mut self.search_query)
                        .id(search_input_id)
                        .hint_text("Search history (Ctrl+F, Esc clears)");
                    let response = ui.add_sized([ui.available_width() - 64.0, 30.0], search_input);
                    if self.search_focus_requested {
                        response.request_focus();
                        self.search_focus_requested = false;
                    }

                    if ui
                        .add_enabled(
                            !self.search_query.trim().is_empty(),
                            egui::Button::new("Clear"),
                        )
                        .clicked()
                    {
                        self.search_query.clear();
                        self.confirm_clear = false;
                        self.set_last_action("Search cleared.");
                    }
                });
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let clear_label = if self.confirm_clear {
                    "Confirm clear"
                } else {
                    "Clear history"
                };
                if ui
                    .add_enabled(total_entries > 0, egui::Button::new(clear_label))
                    .clicked()
                {
                    if self.confirm_clear {
                        if let Err(error) = self.clear_history() {
                            tracing::error!("Could not clear history in UI: {error}");
                            self.set_last_action("Failed to clear history.");
                        } else {
                            self.set_last_action("History cleared.");
                            tracing::info!("History cleared.");
                            if self.config.minimize_on_clear {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                        self.confirm_clear = false;
                    } else {
                        self.confirm_clear = true;
                        self.set_last_action("Press clear again to remove all history.");
                    }
                }
                if self.confirm_clear {
                    ui.label(egui::RichText::new("Awaiting confirmation").italics());
                }
            });

            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);

            // Main content
            egui::ScrollArea::vertical().show(ui, |ui| {
                if filtered_history.is_empty() {
                    let message = if total_entries == 0 {
                        "Clipboard history is empty."
                    } else {
                        "No entries match your search."
                    };
                    ui.add_space(20.0);
                    ui.centered_and_justified(|ui| {
                        ui.label(egui::RichText::new(message).italics());
                    });
                    return;
                }

                for (idx, value) in filtered_history.iter().enumerate() {
                    let preview = self.preview_entry(value);
                    let metadata = match value {
                        ClipboardHistoryEntry::Text(text) => {
                            let chars = text.chars().count();
                            let lines = text.lines().count().max(1);
                            format!("{chars} chars, {lines} lines")
                        }
                        ClipboardHistoryEntry::Image(image) => {
                            let kb = image.bytes.len() / 1024;
                            format!("{}x{}, {} KB", image.width, image.height, kb)
                        }
                    };
                    let is_selected = self.selected_entry_index == Some(idx);

                    let mut entry_frame =
                        egui::Frame::group(ui.style()).inner_margin(egui::Margin::same(8));
                    if is_selected {
                        let selection = ui.visuals().selection;
                        entry_frame = entry_frame.fill(selection.bg_fill).stroke(selection.stroke);
                    }

                    let entry = entry_frame.show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(preview).monospace().size(15.0),
                                )
                                .wrap(),
                            );
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(metadata).small().weak());
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new("Click or Enter to copy").small(),
                                        );
                                    },
                                );
                            });
                        });
                    });
                    if selection_changed_with_keyboard && is_selected {
                        entry.response.scroll_to_me(Some(egui::Align::Center));
                    }
                    let clickable = ui.interact(
                        entry.response.rect,
                        ui.id().with(("history_entry", idx)),
                        egui::Sense::click(),
                    );
                    let clicked = clickable.clicked();
                    let hovered = clickable.hovered();
                    clickable.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if hovered && !is_selected {
                        let hover_style = ui.visuals().widgets.hovered;
                        ui.painter().rect_stroke(
                            entry.response.rect,
                            hover_style.corner_radius,
                            hover_style.bg_stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                    if clicked {
                        self.selected_entry_index = Some(idx);
                        if let Err(error) = self.copy_to_clipboard(value) {
                            tracing::error!("Could not set clipboard value on click: {error:#}");
                            self.set_last_action("Failed to copy entry to clipboard.");
                        } else {
                            self.confirm_clear = false;
                            self.set_last_action("Entry copied to clipboard.");
                            if self.config.minimize_on_copy {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                    }
                    ui.add_space(6.0);
                }
            });
        });

        let action_is_expired = self
            .last_action
            .as_ref()
            .map(|(_, at)| at.elapsed() > Duration::from_secs(4))
            .unwrap_or(false);
        if action_is_expired {
            self.last_action = None;
        }

        egui::TopBottomPanel::bottom("footer").show(ctx, |ui| {
            if let Some((message, _)) = &self.last_action {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(message).small().strong());
                });
                ui.add_space(3.0);
            }
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::Hyperlink::from_label_and_url(
                    "Made with egui",
                    "https://github.com/emilk/egui",
                ));
                ui.separator();
                ui.add(egui::Hyperlink::from_label_and_url(
                    "Source code",
                    "https://github.com/Rayanworkout/clippo",
                ));
            });
            ui.add_space(3.0);
        });

        // Poll daemon updates without forcing full-speed repainting.
        if self.last_action.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}
