//! Native plugin controls and declarative extension widgets.
use super::widgets;
use crate::app::App;
use crate::i18n::gettext;
use crate::model::Action;
use crate::plugins::{Permission, RuntimeResult, SettingKind, UiNode};
use crate::theme::{self, Icon};

// TextEdit counts characters; plugin settings are bounded in UTF-8 bytes.
struct SettingText<'a>(&'a mut String);

impl egui::TextBuffer for SettingText<'_> {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<SettingText<'static>>()
    }
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        self.0
    }
    fn insert_text(&mut self, text: &str, index: egui::text::CharIndex) -> usize {
        let remaining = 4096_usize.saturating_sub(self.0.len());
        let end = text.floor_char_boundary(remaining.min(text.len()));
        egui::TextBuffer::insert_text(self.0, &text[..end], index)
    }
    fn delete_char_range(&mut self, range: std::ops::Range<egui::text::CharIndex>) {
        egui::TextBuffer::delete_char_range(self.0, range);
    }
}

pub fn matches_search(app: &App, needle: &str) -> bool {
    app.plugins.manager.records().iter().any(|record| {
        record
            .package
            .manifest
            .name
            .resolve(app.locale.tag())
            .to_lowercase()
            .contains(needle)
            || record.package.manifest.settings.values().any(|setting| {
                setting
                    .label
                    .resolve(app.locale.tag())
                    .to_lowercase()
                    .contains(needle)
            })
    })
}

pub fn settings(app: &mut App, ui: &mut egui::Ui) {
    let locale = app.locale;
    let palette = app.palette;
    let mut actions = Vec::new();
    if theme::soft_button(
        ui,
        &palette,
        Some(Icon::Plus),
        &gettext(locale, "Install plugin"),
        false,
    )
    .clicked()
    {
        actions.push(Action::InstallPlugin);
    }
    if let Some(error) = &app.plugins.manager.error {
        theme::text(
            ui,
            gettext(locale, "Plugin operation failed"),
            theme::regular(13.0),
            palette.danger,
        )
        .on_hover_text(error);
    }
    for record in app.plugins.manager.records() {
        let manifest = &record.package.manifest;
        let id = &manifest.id;
        ui.push_id(id, |ui| {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                theme::text(
                    ui,
                    manifest.name.resolve(locale.tag()),
                    theme::medium(14.0),
                    palette.text,
                );
                theme::text(ui, &manifest.version, theme::regular(12.0), palette.text);
                let mut enabled = record.config.enabled;
                if widgets::switch(ui, &palette, &gettext(locale, "Enabled"), &mut enabled)
                    .changed()
                {
                    actions.push(Action::PluginEnabled(id.clone(), enabled));
                }
                if theme::pill_button(ui, &palette, &gettext(locale, "Remove"), false).clicked() {
                    actions.push(Action::RemovePlugin(id.clone()));
                }
            });
            for permission in &manifest.permissions {
                let label = match permission {
                    Permission::PlaybackRead => gettext(locale, "Read playback and tracks"),
                    Permission::PlaybackControl => gettext(locale, "Control playback"),
                    Permission::TracksWrite => gettext(locale, "Change track display"),
                    Permission::NetworkProxy => gettext(locale, "Change proxy"),
                    Permission::UiWrite => gettext(locale, "Change interface"),
                    Permission::BehaviorWrite => gettext(locale, "Change controls"),
                };
                let mut allowed = record.config.grants.contains(permission);
                if permission_switch(ui, &palette, &label, &mut allowed).changed() {
                    actions.push(Action::PluginPermission(id.clone(), *permission, allowed));
                }
            }
            for (key, definition) in &manifest.settings {
                let mut value = record
                    .config
                    .settings
                    .get(key)
                    .cloned()
                    .unwrap_or_else(|| definition.default.clone());
                let before = value.clone();
                let label = definition.label.resolve(locale.tag());
                widgets::setting_row(ui, &palette, label, "", |ui| match definition.kind {
                    SettingKind::Bool => {
                        let mut on = value.as_bool().unwrap_or_default();
                        if widgets::switch(ui, &palette, label, &mut on).changed() {
                            value = on.into();
                        }
                    }
                    SettingKind::Text => {
                        let mut text = value.as_str().unwrap_or_default().to_string();
                        let response = widgets::text_edit(
                            ui,
                            locale,
                            egui::TextEdit::singleline(&mut SettingText(&mut text))
                                .desired_width(160.0),
                        );
                        ui.ctx()
                            .accesskit_node_builder(response.id, |node| node.set_label(label));
                        if response.changed() {
                            value = text.into();
                        }
                    }
                    SettingKind::Number => {
                        let mut number = value.as_f64().unwrap_or_default();
                        let response = ui.add(egui::DragValue::new(&mut number).range(
                            definition.min.unwrap_or(-1e12)..=definition.max.unwrap_or(1e12),
                        ));
                        ui.ctx()
                            .accesskit_node_builder(response.id, |node| node.set_label(label));
                        if response.changed()
                            && let Some(number) = serde_json::Number::from_f64(number)
                        {
                            value = number.into();
                        }
                    }
                    SettingKind::Select => {
                        let width = 160.0_f32.min(ui.available_width());
                        let response = egui::ComboBox::from_id_salt(key)
                            .selected_text(crate::bidi::layout(
                                ui.painter(),
                                &value_label(&value),
                                theme::regular(14.0),
                                palette.text,
                                (width - 28.0).max(0.0),
                                1,
                                Some(crate::bidi::ELLIPSIS),
                            ))
                            .width(width)
                            .show_ui(ui, |ui| {
                                for option in &definition.options {
                                    ui.selectable_value(
                                        &mut value,
                                        option.clone(),
                                        crate::bidi::layout(
                                            ui.painter(),
                                            &value_label(option),
                                            theme::regular(14.0),
                                            palette.text,
                                            (width - 16.0).max(0.0),
                                            1,
                                            Some(crate::bidi::ELLIPSIS),
                                        ),
                                    );
                                }
                            });
                        ui.ctx()
                            .accesskit_node_builder(response.response.id, |node| {
                                node.set_label(label)
                            });
                    }
                });
                if value != before {
                    actions.push(Action::PluginSetting(id.clone(), key.clone(), value));
                }
            }
            if let Some(error) = &record.error {
                theme::text(
                    ui,
                    gettext(locale, "Plugin execution failed"),
                    theme::regular(12.0),
                    palette.danger,
                )
                .on_hover_text(error);
            }
        });
    }
    app.actions.extend(actions);
}

// Permission names must stay readable beside their small switches.
fn permission_switch(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    label: &str,
    allowed: &mut bool,
) -> egui::Response {
    let response = ui
        .horizontal(|ui| {
            let label_width = (ui.available_width() - 40.0 - ui.spacing().item_spacing.x).max(0.0);
            ui.allocate_ui_with_layout(
                egui::vec2(label_width, 22.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    let galley = crate::bidi::layout(
                        ui.painter(),
                        label,
                        theme::medium(14.0),
                        palette.text,
                        label_width,
                        usize::MAX,
                        None,
                    );
                    ui.add(egui::Label::new(galley).selectable(false));
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                widgets::switch(ui, palette, label, allowed)
            })
            .inner
        })
        .inner;
    ui.add_space(10.0);
    response
}

fn value_label(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}

// Match native soft buttons while keeping untrusted captions inside their slot.
fn command_button(ui: &mut egui::Ui, palette: &theme::Palette, label: &str) -> egui::Response {
    let width = ui.available_width().max(0.0);
    let galley = crate::bidi::layout(
        ui.painter(),
        label,
        theme::medium(13.0),
        palette.text,
        (width - 24.0).max(0.0),
        1,
        Some(crate::bidi::ELLIPSIS),
    );
    let size = egui::vec2((galley.size().x + 24.0).min(width), galley.size().y + 14.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    if ui.is_rect_visible(rect) {
        let fill = if response.hovered() {
            palette.surface_hover
        } else {
            palette.surface
        };
        ui.painter().rect_filled(rect, rect.height() / 2.0, fill);
        let text_rect = rect.shrink2(egui::vec2(12.0, 7.0));
        let position = crate::bidi::galley_pos(text_rect, &galley);
        ui.painter()
            .with_clip_rect(rect)
            .galley(position, galley, palette.text);
    }
    theme::focus_ring(ui, &response);
    response
}

pub fn slot(app: &mut App, ui: &mut egui::Ui, slot: &str) {
    let mut actions = Vec::new();
    for result in app.plugins.outputs.values() {
        if !app.plugins.permits(result, Permission::UiWrite) {
            continue;
        }
        let Ok(output) = &result.result else {
            continue;
        };
        ui.push_id(&result.id, |ui| {
            for node in output
                .ui_slots
                .get(slot)
                .into_iter()
                .flatten()
                .chain(output.ui.iter().filter(|_| slot == "page_end"))
            {
                draw_node(app, ui, result, node, &mut actions);
            }
        });
    }
    app.actions.extend(actions);
}

fn draw_node(
    app: &App,
    ui: &mut egui::Ui,
    result: &RuntimeResult,
    node: &UiNode,
    actions: &mut Vec<Action>,
) {
    match node {
        UiNode::Label { text } => {
            theme::text(
                ui,
                text.resolve(app.locale.tag()),
                theme::regular(14.0),
                app.palette.text,
            );
        }
        UiNode::Button { text, command } => {
            if command_button(ui, &app.palette, text.resolve(app.locale.tag())).clicked() {
                actions.push(Action::PluginCommand {
                    id: result.id.clone(),
                    digest: result.digest.clone(),
                    revision: result.revision,
                    command: command.clone(),
                });
            }
        }
        UiNode::Row { children } => {
            ui.horizontal_wrapped(|ui| {
                for node in children {
                    draw_node(app, ui, result, node, actions);
                }
            });
        }
        UiNode::Column { children } => {
            ui.vertical(|ui| {
                for node in children {
                    draw_node(app, ui, result, node, actions);
                }
            });
        }
    }
}

pub fn replaces_page(app: &App) -> bool {
    !matches!(app.page(), crate::model::Page::Settings)
        && app.plugins.outputs.values().any(|result| {
            app.plugins.permits(result, Permission::UiWrite)
                && result
                    .result
                    .as_ref()
                    .is_ok_and(|output| output.ui_replace.contains("page"))
        })
}

#[cfg(test)]
mod tests {
    use super::SettingText;
    use egui::{TextBuffer, text::CharIndex};

    #[test]
    fn plugin_text_edits_keep_complete_characters_within_the_byte_limit() {
        let mut value = "я".repeat(2047);
        let mut text = SettingText(&mut value);
        assert_eq!(text.insert_text("я界", CharIndex(100)), 1);
        assert_eq!(text.as_str().len(), 4096);
        assert_eq!(text.insert_text("x", CharIndex(0)), 0);
        text.delete_char_range(CharIndex(0)..CharIndex(2));
        assert_eq!(text.insert_text("界я", CharIndex(0)), 1);
        assert_eq!(text.as_str().len(), 4095);
        assert!(text.as_str().starts_with('界'));
        assert_eq!(text.insert_text("x", CharIndex(0)), 1);
        assert_eq!(text.as_str().len(), 4096);
    }

    #[test]
    fn plugin_permission_labels_remain_readable_at_narrow_widths() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for palette in [
            crate::theme::Palette::light(),
            crate::theme::Palette::dark(),
        ] {
            crate::theme::apply(&ctx, &palette);
            for width in [320.0, 440.0, 960.0] {
                for label in [
                    "Чтение воспроизведения и треков",
                    "السماح بتغيير عناصر الواجهة English 123",
                ] {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 200.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            let switch = super::permission_switch(ui, &palette, label, &mut false);
                            assert!(switch.rect.right() <= width);
                        },
                    );
                    output.textures_delta.clear();
                    let text = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Text(text) => Some(text),
                            _ => None,
                        })
                        .expect("permission label is drawn");
                    assert!(!text.galley.elided, "truncated permission at {width}");
                    assert_eq!(
                        text.galley.text().split_whitespace().collect::<Vec<_>>(),
                        label.split_whitespace().collect::<Vec<_>>(),
                    );
                    assert!(text.pos.x + text.galley.rect.right() <= width);
                }
            }
        }
    }

    #[test]
    fn plugin_command_captions_stay_inside_the_slot() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        for palette in [
            crate::theme::Palette::light(),
            crate::theme::Palette::dark(),
        ] {
            crate::theme::apply(&ctx, &palette);
            for width in [80.0, 320.0, 960.0] {
                for text in ["Long caption ".repeat(60), "اختبار English 123 ".repeat(30)] {
                    let mut output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 200.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            let bounds = super::command_button(ui, &palette, &text).rect;
                            assert!(
                                bounds.left() >= 0.0 && bounds.right() <= width,
                                "{bounds:?} at {width}"
                            );
                        },
                    );
                    output.textures_delta.clear();
                }
            }
        }
    }
}
