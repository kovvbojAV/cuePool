//! Cue list — the main view replacing WPF's DataGrid.

use crate::app::{AppCommand, CueType, SharedStateHandle};
use crate::{colour_to_egui, cue_type_label};
use cuepool_core::Cue;
use egui::{Color32, RichText};
use rust_decimal::Decimal;

/// Which cell of a row is open for editing, if any.
///
/// The Q# and Name cells used to be live text fields, so a single click on a row
/// put a caret in one and the arrow keys then walked the caret rather than the
/// standby playhead. They are labels now; the editor opens on a double click, or
/// on Enter for the selected row so the list stays usable from the keyboard.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum EditCell {
    #[default]
    Qid,
    Name,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct Editing {
    qid: Decimal,
    cell: EditCell,
    /// Set for the one frame between opening the editor and the field existing
    /// to take focus. Without it the field's first frame — which legitimately
    /// has no focus yet — reads as "focus lost" and closes the editor at once.
    arm_focus: bool,
}

const EDITING_ID: &str = "cue_list_editing_cell";

fn editing_id() -> egui::Id {
    egui::Id::new(EDITING_ID)
}

/// Stable per-cell widget id. `id_salt` would hash in the parent `Ui`, which the
/// focus request cannot reproduce, so the field sets its id outright.
fn cell_id(qid: Decimal, cell: EditCell) -> egui::Id {
    egui::Id::new(("cue_cell", qid, matches!(cell, EditCell::Name)))
}

fn editing_now(editing: Option<Editing>, qid: Decimal, cell: EditCell) -> bool {
    editing
        == Some(Editing {
            qid,
            cell,
            arm_focus: true,
        })
        || editing
            == Some(Editing {
                qid,
                cell,
                arm_focus: false,
            })
}

fn open_editor(ui: &egui::Ui, qid: Decimal, cell: EditCell) {
    ui.data_mut(|d| {
        d.insert_temp(
            editing_id(),
            Editing {
                qid,
                cell,
                arm_focus: true,
            },
        )
    });
}

fn close_editor(ui: &egui::Ui) {
    ui.data_mut(|d| d.remove_temp::<Editing>(editing_id()));
}

/// True once, on the frame the field first appears, so it can claim focus.
fn take_focus_arm(ui: &egui::Ui, editing: Option<Editing>) -> bool {
    match editing {
        Some(state) if state.arm_focus => {
            ui.data_mut(|d| {
                d.insert_temp(
                    editing_id(),
                    Editing {
                        arm_focus: false,
                        ..state
                    },
                )
            });
            true
        }
        _ => false,
    }
}

/// Every cue type that can be added, in toolbar order. Shared with the row
/// context menu so the two cannot drift.
const CUE_TYPES: [(&str, &str, CueType); 13] = [
    ("🎵", "Sound", CueType::Sound),
    ("🎬", "Video", CueType::Video),
    ("⏹", "Stop", CueType::Stop),
    ("🔉", "Volume", CueType::Volume),
    ("📁", "Group", CueType::Group),
    ("▢", "Dummy", CueType::Dummy),
    ("📡", "Network", CueType::Network),
    ("🗛", "Text", CueType::Text),
    ("🖼", "Image", CueType::Image),
    ("↪", "Goto", CueType::Goto),
    ("💡", "Lighting", CueType::Lighting),
    ("🎞", "DMX Show", CueType::DmxShow),
    ("⊞", "Pixel Map", CueType::PixelMap),
];

/// Height of every cue-list cell, header and body alike.
const CELL_HEIGHT: f32 = 18.0;

/// Draws one cue-list cell: an exact `width` x [`CELL_HEIGHT`] rect, laid out
/// with `layout` and clipped to that rect.
///
/// The cells used to be `ui.add_sized`, which centres a widget in the size it
/// is given and lets a widget that does not fit extend past it. The row then
/// continued after the overflow, so a long name, or the Trigger combo showing
/// "After last", moved every later column on that row. A cell always advances
/// the row by exactly `width`, and content that does not fit is cut off at the
/// cell's edge.
fn cell<R>(
    ui: &mut egui::Ui,
    width: f32,
    layout: egui::Layout,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let (_, rect) = ui.allocate_space(egui::vec2(width, CELL_HEIGHT));
    let mut cell_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
    // Intersects with the parent's clip rect, so the scroll area still clips
    // rows scrolled out of view.
    cell_ui.shrink_clip_rect(rect);
    add_contents(&mut cell_ui)
}

pub fn show(ui: &mut egui::Ui, state: &SharedStateHandle) {
    let (cues, selected_id, show_mode, active_positions, tc_fps) = {
        let Ok(state) = state.lock() else { return };
        let active_positions: std::collections::HashMap<
            rust_decimal::Decimal,
            (f32, Option<f32>, bool),
        > = state
            .active_cues
            .iter()
            .map(|ac| (ac.qid, (ac.position_secs, ac.length_secs, ac.paused)))
            .collect();
        (
            state.show_file.cues.clone(),
            state.selected_cue_id,
            state.show_mode,
            active_positions,
            state.show_file.show_settings.timecode_fps,
        )
    };

    ui.heading(format!("Cues ({})", cues.len()));
    ui.separator();

    // Toolbar — icon-per-cue-type; glyphs limited to egui's bundled
    // NotoEmoji / emoji-icon fonts. Wrapped so a narrow panel flows to a
    // second row instead of clipping.
    if show_mode == crate::app::ShowMode::Edit {
        ui.horizontal_wrapped(|ui| {
            for (icon, name, cue_type) in CUE_TYPES {
                if ui
                    .button(RichText::new(icon).size(16.0))
                    .on_hover_text(format!("Add {name} cue"))
                    .clicked()
                {
                    queue_cmd(state, AppCommand::AddCue { cue_type });
                }
            }
        });
        ui.separator();
    }

    // Header row — use fixed column widths so headers align with body cells.
    const COL_PLAYHEAD: f32 = 18.0;
    const COL_DRAG: f32 = 20.0;
    const COL_QID: f32 = 48.0;
    // Name's minimum width. Above it, Name takes what the other columns leave;
    // it is low so a narrow panel shortens the name before the row overflows.
    const COL_NAME: f32 = 80.0;
    // Fits the combo showing "After last", the longest trigger label.
    const COL_TRIGGER: f32 = 80.0;
    const COL_FIRE: f32 = 96.0;
    const COL_DURATION: f32 = 60.0;
    // Fits the "Loop" header (29px), which the clipped cell would otherwise cut.
    const COL_LOOP: f32 = 32.0;
    // Fits the "Type" header and every badge. The 8px it gave up went to Loop,
    // so the list still fits the default 1280px window without Name at its
    // minimum.
    const COL_TYPE: f32 = 32.0;
    const COL_COLOUR: f32 = 16.0;
    // Show mode's play/pause marker, in front of the cue number.
    const PLAY_MARKER_SLOT: f32 = 14.0;
    // Group members indent inside the row (not via frame margin) so every
    // column after Name stays aligned with the header and top-level rows.
    const GROUP_INDENT: f32 = 22.0;
    // Row frames have a 4px left inner margin; match it so headers line up.
    const ROW_MARGIN: f32 = 4.0;

    // Show mode has no drag handle, and gives the play marker its own slot in
    // the Q# cell so the number keeps the room it has in Edit mode.
    let qid_w = if show_mode == crate::app::ShowMode::Edit {
        COL_QID
    } else {
        COL_QID + PLAY_MARKER_SLOT
    };
    // Name takes the width the fixed columns leave, so a wider panel shows more
    // of a long name and a narrower one truncates it sooner. At COL_NAME it
    // stops shrinking, and the row runs past the panel's right edge instead.
    // Each fixed cell brings one item_spacing gap, and the row frames have
    // ROW_MARGIN on both sides.
    let spacing = ui.spacing().item_spacing.x;
    let drag_col: &[f32] = if show_mode == crate::app::ShowMode::Edit {
        &[COL_DRAG]
    } else {
        &[]
    };
    let fixed_cols = [
        COL_PLAYHEAD,
        qid_w,
        COL_TRIGGER,
        COL_FIRE,
        COL_DURATION,
        COL_LOOP,
        COL_TYPE,
        COL_COLOUR,
    ];
    let fixed_w: f32 = fixed_cols.iter().chain(drag_col).map(|w| w + spacing).sum();
    let name_w = (ui.available_width() - fixed_w - 2.0 * ROW_MARGIN)
        .max(COL_NAME)
        .floor();

    // Q#, Name, Trigger and Fire read from the cell's left edge, Duration from
    // its right, and Loop and Type are centred. Justified, so a label or a
    // selectable fills its cell: the whole cell takes the click and shows the
    // hover text, as it did before cells were clipped.
    let left = egui::Layout::left_to_right(egui::Align::Center)
        .with_main_align(egui::Align::Min)
        .with_main_justify(true);
    let right = egui::Layout::right_to_left(egui::Align::Center)
        .with_main_align(egui::Align::Max)
        .with_main_justify(true);
    let centred = egui::Layout::centered_and_justified(egui::Direction::LeftToRight);

    ui.horizontal(|ui| {
        ui.add_space(ROW_MARGIN);
        cell(ui, COL_PLAYHEAD, left, |_| {});
        if show_mode == crate::app::ShowMode::Edit {
            cell(ui, COL_DRAG, left, |_| {});
        }
        // The Q# and Name cells are selectable buttons, and Edit mode's Trigger
        // cell a combo, so their text starts button_padding in from the cell
        // edge. The headers do the same so they sit over the text.
        let text_inset = ui.spacing().button_padding.x;
        cell(ui, qid_w, left, |ui| {
            let mut inset = text_inset;
            if show_mode == crate::app::ShowMode::Show {
                // Past the play marker's slot and the gap the button puts
                // after it.
                inset += PLAY_MARKER_SLOT + ui.spacing().icon_spacing;
            }
            ui.add_space(inset);
            ui.label(RichText::new("#").strong())
        });
        cell(ui, name_w, left, |ui| {
            ui.add_space(text_inset);
            ui.label(RichText::new("Name").strong())
        });
        cell(ui, COL_TRIGGER, left, |ui| {
            if show_mode == crate::app::ShowMode::Edit {
                ui.add_space(text_inset);
            }
            ui.label(RichText::new("Trigger").strong())
        });
        cell(ui, COL_FIRE, left, |ui| ui.label(RichText::new("Fire").strong()))
            .on_hover_text("Alternate firing methods (hotkey / MIDI / wall clock / timecode) — edit in the inspector's Triggers tab");
        cell(ui, COL_DURATION, right, |ui| ui.label(RichText::new("Duration").strong()));
        cell(ui, COL_LOOP, centred, |ui| ui.label(RichText::new("Loop").strong()));
        cell(ui, COL_TYPE, centred, |ui| ui.label(RichText::new("Type").strong()));
        cell(ui, COL_COLOUR, centred, |_| {});
    });
    ui.separator();

    // Remember the observed selection in egui temp memory so changes scroll only
    // enough to reveal the row, without jerking an already-visible selection.
    let scroll_memory_id = egui::Id::new("cue_list_last_scrolled_selection");
    let selection_changed = ui.data_mut(|data| {
        let changed = data.get_temp::<Option<Decimal>>(scroll_memory_id) != Some(selected_id);
        data.insert_temp(scroll_memory_id, selected_id);
        changed
    });

    // Enter opens the selected row's name for editing: double click is the mouse
    // route in, and without this there is no keyboard one. Only when nothing
    // already holds the keys, so Enter committing a field cannot reopen it.
    let mut editing = ui.data(|d| d.get_temp::<Editing>(editing_id()));
    if show_mode == crate::app::ShowMode::Edit
        && editing.is_none()
        && !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
        && let Some(selected) = selected_id
    {
        open_editor(ui, selected, EditCell::Name);
        editing = ui.data(|d| d.get_temp::<Editing>(editing_id()));
    }

    egui::ScrollArea::vertical().show(ui, |ui| {
        // Group membership is the `parent` field: a member points at its Group
        // cue and is drawn indented. Drag a cue onto a group (or a member) to join
        // it; drag to the strip below the list to free it.
        for (idx, cue) in cues.iter().enumerate() {
            let base = cue.base();
            let qid = base.qid;
            let is_selected = selected_id == Some(qid);
            let name = &base.name;
            let cue_type = cue_type_label(cue);
            let colour = colour_to_egui(base.colour);

            let is_group = matches!(cue, Cue::Group { .. });
            // Groups are always top-level, so they never render indented.
            let in_group = base.parent.is_some() && !is_group;
            // The group a cue dropped onto this row should join.
            let row_group = if is_group { Some(qid) } else { base.parent };
            let is_active = active_positions.contains_key(&qid);
            let is_paused = active_positions.get(&qid).is_some_and(|(_, _, p)| *p);

            let bg = if is_paused {
                paused_row_fill(ui.visuals())
            } else if is_active {
                active_row_fill(ui.visuals())
            } else if is_group {
                ui.visuals().widgets.active.weak_bg_fill
            } else if is_selected {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().panel_fill
            };

            let frame = egui::Frame::new()
                .inner_margin(egui::Margin::symmetric(ROW_MARGIN as i8, 4));

            let row_content = |ui: &mut egui::Ui| {
                ui.horizontal(|ui| {
                    ui.set_min_height(20.0);

                    // Painted rather than font-backed so the standby marker is
                    // distinct from the green playing glyph and always renders.
                    cell(ui, COL_PLAYHEAD, centred, |ui| {
                        let (marker_rect, marker_response) = ui.allocate_exact_size(
                            egui::vec2(COL_PLAYHEAD, CELL_HEIGHT),
                            egui::Sense::hover(),
                        );
                        if is_selected {
                            let marker_color = ui.visuals().selection.stroke.color;
                            let stroke = egui::Stroke::new(2.5_f32, marker_color);
                            let x = marker_rect.center().x;
                            let y = marker_rect.center().y;
                            ui.painter().line_segment(
                                [egui::pos2(x - 3.0, y - 5.0), egui::pos2(x + 2.0, y)],
                                stroke,
                            );
                            ui.painter().line_segment(
                                [egui::pos2(x + 2.0, y), egui::pos2(x - 3.0, y + 5.0)],
                                stroke,
                            );
                            marker_response.on_hover_text("Standby: Go fires this cue");
                        }
                    });

                    if in_group {
                        ui.add_space(GROUP_INDENT);
                    }
                    // Absorb the indent in the Name column so later columns align.
                    let name_w = if in_group { name_w - GROUP_INDENT } else { name_w };

                    // Drag handle (only in edit mode)
                    if show_mode == crate::app::ShowMode::Edit {
                        let drag_id = ui.auto_id_with(("drag", idx));
                        ui.dnd_drag_source(drag_id, idx, |ui| {
                            cell(ui, COL_DRAG, centred, |ui| {
                                ui.label(egui::RichText::new("≡").monospace().size(14.0))
                            });
                        })
                        .response
                        .on_hover_text("Drag to reorder — drop on a Group cue to join it, on the strip below to ungroup");
                    }

                    // Q# column. A single click selects the row; the editor opens
                    // on a double click, or Enter on the selected row. Leaving the
                    // cells as live text fields meant one click put a caret in
                    // them, and the arrow keys then walked the caret instead of
                    // the standby playhead until you pressed Escape.
                    if show_mode == crate::app::ShowMode::Edit {
                        if editing_now(editing, qid, EditCell::Qid) {
                            // The buffer is rebuilt from the model every frame, so
                            // in-progress typing lives in egui temp memory and
                            // only commits (parses) once focus leaves the field.
                            let edit_id = cell_id(qid, EditCell::Qid);
                            let pending = ui.data_mut(|d| d.get_temp::<String>(edit_id));
                            let mut qid_str = pending.clone().unwrap_or_else(|| qid.to_string());
                            let response = cell(ui, qid_w, left, |ui| {
                                // Frameless so the row highlight (selected/active)
                                // shows through the cell.
                                ui.add(
                                    egui::TextEdit::singleline(&mut qid_str)
                                        .id(edit_id)
                                        .frame(egui::Frame::NONE)
                                        .font(egui::TextStyle::Monospace)
                                        .desired_width(f32::INFINITY),
                                )
                            });
                            // A focused TextEdit only surrenders focus on
                            // Enter/Tab/Esc; blur it on a click anywhere else so
                            // the edit ends.
                            if response.has_focus() && response.clicked_elsewhere() {
                                ui.memory_mut(|mem| mem.surrender_focus(response.id));
                            }
                            if take_focus_arm(ui, editing) {
                                ui.memory_mut(|mem| mem.request_focus(edit_id));
                            } else if response.has_focus() {
                                ui.data_mut(|d| d.insert_temp(edit_id, qid_str.clone()));
                            } else {
                                // Commit the pending edit. lost_focus() can't be
                                // used — a text cell rendered later in the same
                                // frame steals focus after this one was processed,
                                // so the transition is never observed here.
                                ui.data_mut(|d| d.remove_temp::<String>(edit_id));
                                close_editor(ui);
                                let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if !cancelled
                                    && let Ok(new_qid) = qid_str.parse::<rust_decimal::Decimal>()
                                    && new_qid != qid
                                {
                                    queue_cmd(state, AppCommand::UpdateCueQid { qid, new_qid });
                                }
                            }
                        } else {
                            let response = cell(ui, qid_w, left, |ui| {
                                ui.add(
                                    egui::Button::selectable(
                                        is_selected,
                                        RichText::new(qid.to_string()).monospace(),
                                    )
                                    .truncate(),
                                )
                            });
                            if response.double_clicked() {
                                open_editor(ui, qid, EditCell::Qid);
                            } else if response.clicked() {
                                queue_select(state, qid);
                            }
                        }
                    } else {
                        // QLab-style play marker: green ▶ for a playing cue
                        // (amber when paused) in front of the cue number, which
                        // takes the same colour. The marker has a slot of its
                        // own on every row, so the number stays put when the cue
                        // starts; it used to be a "▶ " prefix on the number.
                        let play_colour = is_active.then(|| {
                            if is_paused {
                                Color32::from_rgb(224, 172, 60)
                            } else {
                                Color32::from_rgb(86, 200, 120)
                            }
                        });
                        let response = cell(ui, qid_w, left, |ui| {
                            let marker_id = egui::Id::new("play_marker");
                            let mut number = RichText::new(qid.to_string());
                            if let Some(colour) = play_colour {
                                number = number.color(colour);
                            }
                            let button = egui::Button::selectable(
                                is_selected,
                                (
                                    egui::Atom::custom(
                                        marker_id,
                                        egui::vec2(PLAY_MARKER_SLOT, 0.0),
                                    ),
                                    number,
                                ),
                            )
                            .truncate()
                            .atom_ui(ui);
                            if let (Some(colour), Some(slot)) =
                                (play_colour, button.rect(marker_id))
                            {
                                ui.painter().text(
                                    slot.left_center(),
                                    egui::Align2::LEFT_CENTER,
                                    "▶",
                                    egui::TextStyle::Button.resolve(ui.style()),
                                    colour,
                                );
                            }
                            button.response
                        });
                        if response.clicked() {
                            queue_select(state, qid);
                        }
                    }

                    // Name column. Commits on every keystroke — the model feeds
                    // the buffer next frame, and per-key undo snapshots merge via
                    // the merge key. (Committing only on lost_focus never fired,
                    // since changed() and lost_focus() happen on different frames.)
                    if show_mode == crate::app::ShowMode::Edit
                        && editing_now(editing, qid, EditCell::Name)
                    {
                        let edit_id = cell_id(qid, EditCell::Name);
                        let mut name_str = name.clone();
                        let response = cell(ui, name_w, left, |ui| {
                            // Frameless so the row highlight shows through.
                            ui.add(
                                egui::TextEdit::singleline(&mut name_str)
                                    .id(edit_id)
                                    .frame(egui::Frame::NONE)
                                    .font(egui::TextStyle::Body)
                                    .desired_width(f32::INFINITY),
                            )
                        });
                        if response.has_focus() && response.clicked_elsewhere() {
                            ui.memory_mut(|mem| mem.surrender_focus(response.id));
                        }
                        if take_focus_arm(ui, editing) {
                            ui.memory_mut(|mem| mem.request_focus(edit_id));
                        } else if !response.has_focus() {
                            close_editor(ui);
                        }
                        if response.changed() {
                            queue_cmd(state, AppCommand::UpdateCueName { qid, name: name_str });
                        }
                        response.on_hover_text(name);
                    } else {
                        let response = cell(ui, name_w, left, |ui| {
                            ui.add(egui::Button::selectable(is_selected, name.as_str()).truncate())
                        });
                        if show_mode == crate::app::ShowMode::Edit && response.double_clicked() {
                            open_editor(ui, qid, EditCell::Name);
                        } else if response.clicked() {
                            queue_select(state, qid);
                        }
                        // A name wider than the column is cut to fit with an
                        // ellipsis, so the full name is on hover.
                        response.on_hover_text(name);
                    }

                    // Trigger column. `ComboBox::width` is a minimum, not a cap,
                    // so the combo used to grow with its label. The label now
                    // truncates to the cell, and no truncated label needs more
                    // than the whole cell, so every combo is the cell's width.
                    if show_mode == crate::app::ShowMode::Edit {
                        let mut trigger = base.trigger;
                        let response = cell(ui, COL_TRIGGER, left, |ui| {
                            egui::ComboBox::from_id_salt(egui::Id::new(("trigger", qid)))
                                .selected_text(trigger_label(trigger))
                                .width(COL_TRIGGER)
                                .truncate()
                                .show_ui(ui, |ui| {
                                    for mode in [
                                        cuepool_core::TriggerMode::Go,
                                        cuepool_core::TriggerMode::WithLast,
                                        cuepool_core::TriggerMode::AfterLast,
                                    ] {
                                        if ui.selectable_label(trigger == mode, trigger_label(mode)).clicked() {
                                            trigger = mode;
                                        }
                                    }
                                })
                                .response
                        });
                        response.on_hover_text(trigger_help(base.trigger));
                        if trigger != base.trigger {
                            queue_cmd(state, AppCommand::UpdateCueTrigger { qid, trigger });
                        }
                    } else {
                        cell(ui, COL_TRIGGER, left, |ui| {
                            ui.label(
                                RichText::new(trigger_label(base.trigger))
                                    .monospace()
                                    .size(10.0),
                            )
                        })
                        .on_hover_text(trigger_help(base.trigger));
                    }

                    // Fire column — badges for the cue's alternate triggers
                    // (hotkey / MIDI / wall clock / timecode), edited in the
                    // inspector's Triggers tab. Empty when none are configured.
                    {
                        let badges = trigger_badges(&base.triggers);
                        cell(ui, COL_FIRE, left, |ui| {
                            ui.label(RichText::new(badges.join(" ")).monospace().size(10.0).weak())
                        })
                        .on_hover_text(describe_triggers(&base.triggers, tc_fps));
                    }

                    // Duration / Progress column
                    let duration_str = match cue {
                        cuepool_core::Cue::Sound { duration, .. }
                        | cuepool_core::Cue::Video { duration, .. }
                        | cuepool_core::Cue::TimeCode { duration, .. } => {
                            if duration.as_secs_f64() > 0.0 {
                                format_duration(duration)
                            } else {
                                "—".to_string()
                            }
                        }
                        _ => "—".to_string(),
                    };
                    cell(ui, COL_DURATION, right, |ui| {
                        if let Some((pos, len, _paused)) = active_positions.get(&qid)
                            && let Some(len) = len
                            && *len > 0.0
                        {
                            let progress = (pos / len).clamp(0.0, 1.0);
                            let bar_width = COL_DURATION - 4.0;
                            let bar_height = 6.0;
                            let (rect, _response) = ui.allocate_exact_size(
                                egui::vec2(bar_width, bar_height),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(40, 40, 40));
                            let fill_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::vec2(bar_width * progress, bar_height),
                            );
                            ui.painter().rect_filled(fill_rect, 2.0, Color32::from_rgb(100, 180, 100));
                        } else {
                            ui.label(RichText::new(&duration_str).monospace().size(10.0));
                        }
                    });

                    // Loop column
                    let (loop_short, loop_desc) = match base.loop_mode {
                        cuepool_core::LoopMode::OneShot => ("1".to_string(), "Plays once".to_string()),
                        cuepool_core::LoopMode::Looped => {
                            (format!("{}", base.loop_count), format!("Loops {}×", base.loop_count))
                        }
                        cuepool_core::LoopMode::LoopedInfinite => ("∞".to_string(), "Loops forever".to_string()),
                        cuepool_core::LoopMode::HoldLast => {
                            ("H".to_string(), "Holds the last frame/value when it ends".to_string())
                        }
                    };
                    cell(ui, COL_LOOP, centred, |ui| {
                        ui.label(RichText::new(loop_short).monospace().size(10.0))
                    })
                    .on_hover_text(loop_desc);

                    // Type column
                    cell(ui, COL_TYPE, centred, |ui| {
                        ui.label(RichText::new(cue_type).monospace().size(10.0))
                    })
                    .on_hover_text(format!("{} cue", cue_type_name(cue)));

                    // Colour swatch
                    cell(ui, COL_COLOUR, centred, |ui| {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(COL_COLOUR, 16.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().rect_filled(rect, 4.0, colour);
                        response
                    })
                    .on_hover_text("Cue colour tag");
                });
            };
            let (drop_response, dropped_payload) = ui
                .scope(|ui| {
                    ui.visuals_mut().widgets.inactive.bg_fill = bg;
                    ui.dnd_drop_zone::<usize, ()>(frame, row_content)
                })
                .inner;

            if is_selected {
                ui.painter().rect_stroke(
                    drop_response.response.rect,
                    0.0,
                    egui::Stroke::new(2.0_f32, ui.visuals().selection.stroke.color),
                    egui::StrokeKind::Inside,
                );
                if selection_changed {
                    // Instant, not egui's default 0.1–0.3s animated scroll: arrow-key
                    // navigation repeats faster than the animation, so the viewport
                    // would permanently chase the playhead.
                    drop_response
                        .response
                        .scroll_to_me_animation(None, egui::style::ScrollAnimation::none());
                }
            }

            // Select on a click anywhere over the row, and open a context menu on
            // right-click — both read the pointer state directly rather than adding
            // a click-sensing widget over the row. A click-sensing row widget ties
            // with the inline cells in egui's hit-test (ties go to the last-added
            // widget = the row), which stole the cells' clicks (no selection) and
            // wedged the pointer state (text highlighting on hover).
            if show_mode == crate::app::ShowMode::Edit {
                let over_row = drop_response.response.contains_pointer();
                let (primary, secondary) = ui.input(|i| {
                    (i.pointer.primary_clicked(), i.pointer.secondary_clicked())
                });
                if over_row && (primary || secondary) {
                    queue_select(state, qid);
                }
                // Open on right-click only; the menu's own close-on-click handles
                // the rest. (Force-closing on any primary click closed the popup
                // before its buttons could see the click, so Delete etc. never
                // fired.)
                // Right-clicking any row closes every other row's menu. Each row
                // owns a popup id, so without this a menu left open over a stale
                // selection sat there while another opened.
                let menu_open = if secondary {
                    Some(egui::SetOpenCommand::Bool(over_row))
                } else {
                    None
                };
                egui::Popup::menu(&drop_response.response)
                    .id(ui.make_persistent_id(("row_menu", qid)))
                    .at_pointer_fixed()
                    .open_memory(menu_open)
                    .show(|ui| {
                        // Every item names this row and acts on it. They used to
                        // issue selection-scoped commands, which agreed with the
                        // label only because right-click selects first.
                        if ui.button(format!("Move Q{qid} up")).clicked() {
                            queue_cmd(state, AppCommand::MoveCueUp { qid: Some(qid) });
                            ui.close();
                        }
                        if ui.button(format!("Move Q{qid} down")).clicked() {
                            queue_cmd(state, AppCommand::MoveCueDown { qid: Some(qid) });
                            ui.close();
                        }
                        if in_group && ui.button(format!("Remove Q{qid} from group")).clicked() {
                            queue_cmd(state, AppCommand::UngroupCue { qid });
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(format!("Duplicate Q{qid}")).clicked() {
                            queue_cmd(state, AppCommand::DuplicateCue { qid: Some(qid) });
                            ui.close();
                        }
                        if ui.button(format!("Delete Q{qid}")).clicked() {
                            queue_cmd(state, AppCommand::DeleteCue { qid: Some(qid) });
                            ui.close();
                        }
                        ui.separator();
                        // Same list as the toolbar. The menu used to offer seven
                        // of the thirteen, with no rule about which.
                        ui.menu_button("Add cue", |ui| {
                            for (icon, name, cue_type) in CUE_TYPES {
                                if ui.button(format!("{icon}  {name}")).clicked() {
                                    queue_cmd(state, AppCommand::AddCue { cue_type });
                                    ui.close();
                                }
                            }
                        });
                    });
            }

            // Handle dropped payload for reordering. Dropping onto a row joins
            // that row's group; dropping onto a group header makes it the first
            // member (inserted just after the header).
            if show_mode == crate::app::ShowMode::Edit
                && let Some(source_idx) = dropped_payload {
                    let source = *source_idx;
                    if source != idx {
                        let to_idx = if is_group { idx + 1 } else { idx };
                        queue_cmd(
                            state,
                            AppCommand::MoveCue { from_idx: source, to_idx, parent: row_group },
                        );
                    }
                }
        }

        // Trailing drop strip: drop a cue here to move it to the end and free it
        // from any group. Lets you place cues after a group (which can't be done
        // by reordering alone, since membership is the parent field).
        if show_mode == crate::app::ShowMode::Edit {
            let bg = ui.visuals().faint_bg_color;
            let frame = egui::Frame::new()
                .inner_margin(egui::Margin::same(8));
            let (_resp, payload) = ui
                .scope(|ui| {
                    ui.visuals_mut().widgets.inactive.bg_fill = bg;
                    ui.dnd_drop_zone::<usize, ()>(frame, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.weak("⤓  drop here to ungroup / move to end");
                    })
                })
                .inner;
            if let Some(source_idx) = payload {
                queue_cmd(
                    state,
                    AppCommand::MoveCue {
                        from_idx: *source_idx,
                        to_idx: cues.len(),
                        parent: None,
                    },
                );
            }
        }
    });

    if show_mode == crate::app::ShowMode::Show {
        ui.horizontal(|ui| {
            ui.colored_label(Color32::YELLOW, "• SHOW MODE");
            ui.label("Editing disabled");
        });
    }
}

fn queue_select(state: &SharedStateHandle, qid: Decimal) {
    if let Ok(mut state) = state.lock() {
        state.command_queue.push(AppCommand::SelectCue(qid));
    }
}

fn queue_cmd(state: &SharedStateHandle, cmd: AppCommand) {
    if let Ok(mut state) = state.lock() {
        state.command_queue.push(cmd);
    }
}

/// Trigger-column text. `{:?}` on `TriggerMode` reads "WithLast", and Show mode
/// used to cut that to "Wit".
fn trigger_label(mode: cuepool_core::TriggerMode) -> &'static str {
    match mode {
        cuepool_core::TriggerMode::Go => "Go",
        cuepool_core::TriggerMode::WithLast => "With last",
        cuepool_core::TriggerMode::AfterLast => "After last",
    }
}

/// What the cue's own trigger mode means, for the column tooltip.
fn trigger_help(mode: cuepool_core::TriggerMode) -> &'static str {
    match mode {
        cuepool_core::TriggerMode::Go => "Fires when you press Go",
        cuepool_core::TriggerMode::WithLast => "Fires with the cue above it",
        cuepool_core::TriggerMode::AfterLast => "Fires when the cue above it ends",
    }
}

/// Full cue-type name for the Type-column badge tooltip.
fn cue_type_name(cue: &Cue) -> &'static str {
    match cue {
        Cue::Group { .. } => "Group",
        Cue::Sound { .. } => "Sound",
        Cue::Video { .. } => "Video",
        Cue::Stop { .. } => "Stop",
        Cue::Volume { .. } => "Volume",
        Cue::Dummy { .. } => "Dummy",
        Cue::TimeCode { .. } => "TimeCode",
        Cue::Network { .. } => "Network",
        Cue::Text { .. } => "Text",
        Cue::Image { .. } => "Image",
        Cue::Goto { .. } => "Goto",
        Cue::Lighting { .. } => "Lighting",
        Cue::DmxShow { .. } => "DMX Show",
        Cue::PixelMap { .. } => "Pixel Map",
    }
}

/// Fire-column badges, one per configured alternate trigger.
fn trigger_badges(triggers: &cuepool_core::CueTriggers) -> Vec<&'static str> {
    let mut badges = Vec::new();
    if triggers.hotkey.is_some() {
        badges.push("key");
    }
    if triggers.midi.is_some() {
        badges.push("midi");
    }
    if triggers.wall_clock.is_some() {
        badges.push("clk");
    }
    if triggers.timecode.is_some() {
        badges.push("tc");
    }
    badges
}

/// Fire-column tooltip: the full config of every configured trigger.
fn describe_triggers(triggers: &cuepool_core::CueTriggers, tc_fps: f32) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(hotkey) = &triggers.hotkey {
        parts.push(format!("Hotkey '{}'", hotkey.key));
    }
    if let Some(midi) = &triggers.midi {
        let what = match midi.kind {
            cuepool_core::MidiTriggerKind::NoteOn | cuepool_core::MidiTriggerKind::NoteOff => {
                "note"
            }
            cuepool_core::MidiTriggerKind::CC => "CC",
        };
        let mut s = format!(
            "MIDI {:?} ch{} {} {}",
            midi.kind, midi.channel, what, midi.note_or_cc
        );
        if matches!(midi.kind, cuepool_core::MidiTriggerKind::NoteOn) {
            s.push_str(&format!(" vel ≥ {}", midi.velocity_min));
        }
        parts.push(s);
    }
    if let Some(clock) = &triggers.wall_clock {
        let mut s = format!("Wall clock {}", clock.time);
        let mut opts: Vec<&str> = Vec::new();
        if matches!(clock.mode, cuepool_core::ClockMode::TwelveHour) {
            opts.push("12h");
        }
        if matches!(clock.repeat, cuepool_core::RepeatMode::Once) {
            opts.push("once");
        }
        if !opts.is_empty() {
            s.push_str(&format!(" ({})", opts.join(", ")));
        }
        parts.push(s);
    }
    if let Some(tc) = &triggers.timecode {
        parts.push(format!(
            "Timecode ≥ {}",
            crate::transport::format_timecode(tc.time.as_secs_f64(), tc_fps)
        ));
    }
    if parts.is_empty() {
        "No alternate triggers — set them in the inspector's Triggers tab".to_string()
    } else {
        parts.join(" · ")
    }
}

/// Row fill for a currently playing cue: a clear green tint (QLab-style) so
/// live cues stand out in the list without fighting the selection colour.
fn active_row_fill(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(33, 96, 55)
    } else {
        Color32::from_rgb(168, 224, 186)
    }
}

/// Row fill for a paused cue: amber, matching the Active Cues panel.
fn paused_row_fill(visuals: &egui::Visuals) -> Color32 {
    if visuals.dark_mode {
        Color32::from_rgb(102, 78, 26)
    } else {
        Color32::from_rgb(240, 214, 140)
    }
}

fn format_duration(d: &cuepool_core::Timespan) -> String {
    let secs = d.as_secs_f64();
    let mins = (secs / 60.0) as u64;
    let rem_secs = secs % 60.0;
    if mins > 0 {
        format!("{}:{:05.2}", mins, rem_secs)
    } else {
        format!("{:.2}s", secs)
    }
}
