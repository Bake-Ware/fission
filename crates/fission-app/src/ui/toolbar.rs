use eframe::egui::{self, RichText, Stroke};
use super::commands::{self, Command, Entry, Panel};
use super::icons;
use super::theme;
use crate::app::FissionApp;

pub fn show(app: &mut FissionApp, ui: &mut egui::Ui) {
    egui::Panel::top("menu_bar")
        .frame(egui::Frame::NONE.fill(theme::panel()).inner_margin(egui::Margin::symmetric(6, 2)))
        .show(ui, |ui| menu_bar(app, ui));

    egui::Panel::top("toolbar")
        .frame(
            egui::Frame::NONE
                .fill(theme::toolbar())
                .stroke(Stroke::new(1.0, theme::border()))
                .inner_margin(egui::Margin { left: 8, right: 8, top: 2, bottom: 4 }),
        )
        .show(ui, |ui| if app.sketch.is_some() { sketch_tools(app, ui) } else { tools(app, ui) });
}

/// The SKETCH contextual tab, shown while editing a sketch (panel order
/// follows Fusion's SketchTab: CREATE, MODIFY, CONSTRAINTS, ... FINISH SKETCH).
fn sketch_tools(app: &mut FissionApp, ui: &mut egui::Ui) {
    use super::sketch::{ConstraintCmd, Tool};
    let active = app.sketch.as_ref().map(|s| s.tool);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("SKETCH").strong().color(theme::accent()).size(12.0));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(46.0, 2.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, theme::accent());
        });
        ui.add_space(10.0);

        let tool_button = |ui: &mut egui::Ui, tool: Tool, icon: icons::Icon| {
            let tip = match tool.hotkey() {
                Some(k) => format!("{} ({k})", tool.label()),
                None => tool.label().to_string(),
            };
            let (rect, response) = ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::click());
            if active == Some(tool) {
                ui.painter().rect_filled(rect, 3.0, theme::palette().pressed);
            } else if response.hovered() {
                ui.painter().rect_filled(rect, 3.0, theme::palette().hover);
            }
            icons::paint(ui.painter(), rect.shrink(4.0), icon, theme::icon());
            response.on_hover_text(tip).clicked()
        };

        let mut picked = None;
        let menu_pick = caption_menu(ui, "CREATE", |ui| {
            for (tool, icon) in [
                (Tool::Line, icons::Icon::Line),
                (Tool::Rectangle, icons::Icon::Rectangle),
                (Tool::Circle, icons::Icon::Circle),
                (Tool::Arc, icons::Icon::Arc),
                (Tool::Polygon, icons::Icon::Polygon),
                (Tool::Slot, icons::Icon::Slot),
                (Tool::Dimension, icons::Icon::Dimension),
            ] {
                if tool_button(ui, tool, icon) {
                    picked = Some(tool);
                }
            }
            // The polygon's side count, while drawing one.
            if let Some(session) = app.sketch.as_mut().filter(|s| s.tool == Tool::Polygon) {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Sides").size(10.0).color(theme::muted()));
                    ui.add(egui::DragValue::new(&mut session.polygon_sides).range(3..=64).speed(0.1));
                });
            }
        }, |ui| {
            // Fusion's CREATE dropdown, with the variants of each shape.
            let mut pick = None;
            let mut item = |ui: &mut egui::Ui, tool: Tool| {
                let mut b = egui::Button::new(tool.label());
                if let Some(k) = tool.hotkey() {
                    b = b.shortcut_text(k);
                }
                if ui.add(b).clicked() {
                    pick = Some(tool);
                    ui.close();
                }
            };
            item(ui, Tool::Line);
            ui.menu_button("Rectangle", |ui| {
                for t in [Tool::Rectangle, Tool::Rectangle3, Tool::CenterRectangle] {
                    item(ui, t);
                }
            });
            ui.menu_button("Circle", |ui| {
                for t in [Tool::Circle, Tool::Circle2, Tool::Circle3] {
                    item(ui, t);
                }
            });
            ui.menu_button("Arc", |ui| {
                for t in [Tool::Arc3, Tool::Arc] {
                    item(ui, t);
                }
            });
            item(ui, Tool::Polygon);
            item(ui, Tool::Slot);
            ui.separator();
            item(ui, Tool::Mirror);
            item(ui, Tool::Project);
            ui.separator();
            item(ui, Tool::Dimension);
            pick
        });
        picked = picked.or(menu_pick);
        separator(ui);
        let mut construction = false;
        caption_group(ui, "MODIFY", |ui| {
            let b = egui::Button::new(RichText::new("Construction").size(11.0)).min_size(egui::vec2(34.0, 34.0));
            construction = ui.add(b).on_hover_text("Toggle construction on selected curves (X)").clicked();
            let snap_on = app.sketch.as_ref().is_some_and(|s| s.grid_snap);
            let label = RichText::new("Grid snap").size(11.0);
            let b = egui::Button::new(label).selected(snap_on).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Snap to the sketch grid (points, midpoints and curves always snap)").clicked() {
                if let Some(s) = &mut app.sketch {
                    s.grid_snap = !s.grid_snap;
                }
            }
            let trim = app.sketch.as_ref().is_some_and(|s| s.tool == Tool::Trim);
            let b = egui::Button::new(RichText::new("Trim").size(11.0)).selected(trim).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Trim (T): click a curve to remove the piece between crossings").clicked() {
                picked = Some(Tool::Trim);
            }
            let extend = app.sketch.as_ref().is_some_and(|s| s.tool == Tool::Extend);
            let b = egui::Button::new(RichText::new("Extend").size(11.0)).selected(extend).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Extend: click near a free end to grow it to the next curve").clicked() {
                picked = Some(Tool::Extend);
            }
            let fillet = app.sketch.as_ref().is_some_and(|s| s.tool == Tool::Fillet);
            let b = egui::Button::new(RichText::new("Fillet").size(11.0)).selected(fillet).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Fillet: click a corner where two lines meet, then type the radius").clicked() {
                picked = Some(Tool::Fillet);
            }
            let offset = app.sketch.as_ref().is_some_and(|s| s.tool == Tool::Offset);
            let b = egui::Button::new(RichText::new("Offset").size(11.0)).selected(offset).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Offset (O): click a curve, then type the distance (negative goes inside)").clicked() {
                picked = Some(Tool::Offset);
            }
            let project = app.sketch.as_ref().is_some_and(|s| s.tool == Tool::Project);
            let b = egui::Button::new(RichText::new("Project").size(11.0)).selected(project).min_size(egui::vec2(34.0, 34.0));
            if ui.add(b).on_hover_text("Project (P): click model edges to copy them into the sketch, fixed").clicked() {
                picked = Some(Tool::Project);
            }
        });
        separator(ui);
        let mut constraint = None;
        caption_group(ui, "CONSTRAINTS", |ui| {
            for cmd in ConstraintCmd::ALL {
                if icons::button(ui, cmd.icon(), 28.0, cmd.label()).clicked() {
                    constraint = Some(cmd);
                }
            }
        });
        separator(ui);
        let mut finish = false;
        caption_group(ui, "FINISH SKETCH", |ui| {
            let (rect, response) = ui.allocate_exact_size(egui::vec2(40.0, 34.0), egui::Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 3.0, theme::palette().ok_hover);
            }
            icons::paint(ui.painter(), rect.shrink(5.0), icons::Icon::FinishSketch, theme::icon());
            finish = response.on_hover_text("Finish Sketch (Ctrl+Enter)").clicked();
        });

        if let Some(tool) = picked {
            app.set_sketch_tool(tool);
        }
        if construction {
            app.toggle_construction();
        }
        if let Some(cmd) = constraint {
            app.apply_constraint(cmd);
        }
        if finish {
            app.finish_sketch();
        }
    });
}

/// Buttons with a caption under them that opens a dropdown, like the
/// solid panels'. Returns what the dropdown picked.
fn caption_menu<T>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui), menu: impl FnOnce(&mut egui::Ui) -> Option<T>) -> Option<T> {
    ui.vertical(|ui| {
        let width = ui.horizontal(add).response.rect.width();
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 12.0), egui::Sense::click());
        let color = if response.hovered() { theme::accent() } else { theme::muted() };
        let text = ui.painter().text(rect.center() - egui::vec2(4.0, 0.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), color);
        let (x, y) = (text.max.x + 4.0, rect.center().y);
        ui.painter().add(egui::Shape::convex_polygon(
            vec![egui::pos2(x, y - 2.0), egui::pos2(x + 6.0, y - 2.0), egui::pos2(x + 3.0, y + 2.0)],
            color,
            Stroke::NONE,
        ));
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(190.0);
            menu(ui)
        })
        .and_then(|r| r.inner)
    })
    .inner
}

/// Buttons with a caption under them (no dropdown).
fn caption_group(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        let width = ui.horizontal(add).response.rect.width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 12.0), egui::Sense::hover());
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), theme::muted());
    });
}

fn menu_bar(app: &mut FissionApp, ui: &mut egui::Ui) {
    egui::containers::menu::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| {
            if ui.button("New Design").clicked() {
                app.new_document();
            }
            if ui.button("Open…").clicked() {
                app.open_dialog();
            }
            ui.separator();
            if ui.button("Save").clicked() {
                app.save(false);
            }
            if ui.button("Save As…").clicked() {
                app.save(true);
            }
            ui.separator();
            if ui.button("Import STEP / F3D…").clicked() {
                app.import_step_dialog();
            }
            if ui.button("Export STL…").clicked() {
                app.export_dialog("stl");
            }
            if ui.button("Export 3MF…").clicked() {
                app.export_dialog("3mf");
            }
            if ui.button("Export OBJ…").clicked() {
                app.export_dialog("obj");
            }
            if ui.button("Export STEP…").clicked() {
                app.export_dialog("step");
            }
        });
        ui.menu_button("Edit", |ui| {
            let undo = ui.add_enabled(app.history.can_undo(), egui::Button::new("Undo").shortcut_text("Ctrl+Z"));
            if undo.clicked() {
                app.finish_edit(true);
                let before = app.doc.clone();
                if app.history.undo(&mut app.doc) {
                    app.changed_from(&before);
                }
            }
            let redo = ui.add_enabled(app.history.can_redo(), egui::Button::new("Redo").shortcut_text("Ctrl+Y"));
            if redo.clicked() {
                app.finish_edit(true);
                let before = app.doc.clone();
                if app.history.redo(&mut app.doc) {
                    app.changed_from(&before);
                }
            }
        });
        ui.menu_button("View", |ui| {
            if ui.button("Fit All").clicked() {
                app.viewport.fit_all(&app.bodies);
            }
            for (label, view) in [
                ("Home", crate::viewport::StandardView::Home),
                ("Top", crate::viewport::StandardView::Top),
                ("Front", crate::viewport::StandardView::Front),
                ("Right", crate::viewport::StandardView::Right),
            ] {
                if ui.button(label).clicked() {
                    app.viewport.camera.set_view(view);
                    if view == crate::viewport::StandardView::Home {
                        app.viewport.fit_all(&app.bodies);
                    }
                }
            }
            ui.separator();
            let mut dark = theme::is_dark();
            if ui.checkbox(&mut dark, "Dark Mode").clicked() {
                theme::set_dark(ui.ctx(), dark);
            }
        });

        // Document tab, centered-ish like Fusion's.
        ui.add_space(24.0);
        let dirty = if app.is_dirty() { " •" } else { "" };
        egui::Frame::NONE
            .fill(theme::palette().surface)
            .stroke(Stroke::new(1.0, theme::border()))
            .corner_radius(egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 })
            .inner_margin(egui::Margin::symmetric(12, 2))
            .show(ui, |ui| {
                ui.label(RichText::new(format!("{}{dirty}", app.doc.name)).strong());
            });
    });
}

fn tools(app: &mut FissionApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        // Workspace tab strip.
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new("SOLID").strong().color(theme::accent()).size(12.0));
            let (rect, _) = ui.allocate_exact_size(egui::vec2(40.0, 2.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 0.0, theme::accent());
        });
        ui.add_space(10.0);

        for (n, panel) in commands::SOLID_PANELS.iter().enumerate() {
            if n > 0 {
                separator(ui);
            }
            let force_open = app.demo_menu.as_deref() == Some(panel.label);
            let clicked = group(ui, panel, force_open, |ui| {
                let mut clicked = None;
                for &cmd in panel.toolbar {
                    if tool(ui, cmd).clicked() {
                        clicked = Some(cmd);
                    }
                }
                clicked
            });
            if let Some(cmd) = clicked {
                cmd.run(app);
            }
        }
        // The popup remembers it's open; only force it on the first frame.
        app.demo_menu = None;
    });
}

/// Tool buttons with a caption underneath, sized to the buttons (not the
/// toolbar). Clicking the caption opens the panel's full dropdown.
fn group(
    ui: &mut egui::Ui,
    panel: &Panel,
    force_open: bool,
    add: impl FnOnce(&mut egui::Ui) -> Option<Command>,
) -> Option<Command> {
    ui.vertical(|ui| {
        // At least as wide as the caption and its chevron.
        let caption = ui.painter().layout_no_wrap(panel.label.to_string(), egui::FontId::proportional(10.0), theme::muted());
        let caption_w = caption.size().x + 16.0;
        let buttons = ui.horizontal(|ui| {
            let start = ui.cursor().min.x;
            let inner = ui.scope(add);
            let used = ui.cursor().min.x - start;
            if used < caption_w {
                ui.add_space(caption_w - used);
            }
            inner.inner
        });
        let mut clicked = buttons.inner;
        let width = buttons.response.rect.width().max(caption_w);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 12.0), egui::Sense::click());
        let color = if response.hovered() { theme::accent() } else { theme::muted() };
        let text = ui.painter().text(
            rect.center() - egui::vec2(4.0, 0.0),
            egui::Align2::CENTER_CENTER,
            panel.label,
            egui::FontId::proportional(10.0),
            color,
        );
        // Small dropdown chevron, drawn because the default font lacks ▾.
        let x = text.max.x + 4.0;
        let y = rect.center().y;
        ui.painter().add(egui::Shape::convex_polygon(
            vec![egui::pos2(x, y - 2.0), egui::pos2(x + 6.0, y - 2.0), egui::pos2(x + 3.0, y + 2.0)],
            color,
            Stroke::NONE,
        ));
        // Replacing Popup::menu's own open command, so keep its click toggle.
        let open = if force_open {
            Some(egui::SetOpenCommand::Bool(true))
        } else {
            response.clicked().then_some(egui::SetOpenCommand::Toggle)
        };
        egui::Popup::menu(&response).open_memory(open).show(|ui| {
            ui.set_min_width(190.0);
            dropdown(ui, panel.dropdown, &mut clicked);
        });
        clicked
    })
    .inner
}

fn dropdown(ui: &mut egui::Ui, entries: &[Entry], clicked: &mut Option<Command>) {
    for entry in entries {
        match entry {
            Entry::Cmd(cmd) => {
                let mut button = egui::Button::new(cmd.label());
                if let Some(key) = cmd.hotkey() {
                    button = button.shortcut_text(key);
                }
                if ui.add(button).on_hover_text(cmd.tooltip()).clicked() {
                    *clicked = Some(*cmd);
                }
            }
            Entry::Planned(label) => {
                ui.add_enabled(false, egui::Button::new(*label)).on_disabled_hover_text("Not in Fission yet");
            }
            Entry::Submenu(label, items) => {
                ui.menu_button(*label, |ui| dropdown(ui, items, clicked));
            }
            Entry::Separator => {
                ui.separator();
            }
        }
    }
}

fn tool(ui: &mut egui::Ui, cmd: Command) -> egui::Response {
    let tip = match cmd.hotkey() {
        Some(key) => format!("{} ({key})\n{}", cmd.label(), cmd.tooltip()),
        None => format!("{}\n{}", cmd.label(), cmd.tooltip()),
    };
    icons::button(ui, cmd.icon(), 34.0, &tip)
}

fn separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 48.0), egui::Sense::hover());
    ui.painter().vline(rect.center().x, rect.y_range(), Stroke::new(1.0, theme::border()));
}
