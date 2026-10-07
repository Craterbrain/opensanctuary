use crate::core::commands::ShowCommand;
use crate::core::events::ShowEvent;
use crate::core::models::{Schedule, ShowState, Slide};

/// Mirrors `schedule.items[item_index]` into `live_item`/`staged_item` if
/// either is currently showing that same item -- those are separate copies
/// (not shared references) so an in-place schedule edit doesn't otherwise
/// reach whatever's actually live/staged. Takes the index the caller
/// already has (every call site just edited `schedule.items[item_index]`)
/// rather than an id, so this is a direct slice index instead of an O(n)
/// `Schedule::items` scan, and callers don't need to clone the item's id
/// just to hand it back.
pub(super) fn sync_item_in_state(state: &mut ShowState, schedule: &Schedule, item_index: usize) {
    let Some(item) = schedule.items.get(item_index) else { return; };
    if let Some(ref mut live) = state.live_item {
        if live.id == item.id {
            *live = item.clone();
        }
    }
    if let Some(ref mut staged) = state.staged_item {
        if staged.id == item.id {
            *staged = item.clone();
        }
    }
}

fn apply_slide_edit_op(slide: &mut Slide, op: &crate::core::commands::SlideEditOp) {
    use crate::core::commands::SlideEditOp;
    match op {
        SlideEditOp::AddElement(el) => {
            slide.elements.push(el.clone());
            normalize_z_indices(&mut slide.elements);
        }
        SlideEditOp::RemoveElement { element_id } => {
            slide.elements.retain(|e| e.id() != element_id);
            normalize_z_indices(&mut slide.elements);
        }
        SlideEditOp::UpdateTransform { element_id, transform } => {
            if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                *el.transform_mut() = transform.clone().normalized();
            } else {
                tracing::warn!("SlideEditOp UpdateTransform: element_id {} not found on slide, dropped", element_id);
            }
        }
        SlideEditOp::UpdateTextBlockContent { element_id, runs, paragraph_style } => {
            if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                    block.runs = merge_contiguous_text_runs(runs);
                    if let Some(style) = paragraph_style {
                        block.paragraph_style = style.clone();
                    }
                } else {
                    tracing::warn!("SlideEditOp UpdateTextBlockContent: element_id {} is not a TextBlock, dropped", element_id);
                }
            } else {
                tracing::warn!("SlideEditOp UpdateTextBlockContent: element_id {} not found on slide, dropped", element_id);
            }
        }
        SlideEditOp::UpdateElementEffects { element_id, effects } => {
            if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                    block.effects = effects.clone();
                } else {
                    tracing::warn!("SlideEditOp UpdateElementEffects: element_id {} is not a TextBlock, dropped", element_id);
                }
            } else {
                tracing::warn!("SlideEditOp UpdateElementEffects: element_id {} not found on slide, dropped", element_id);
            }
        }
        SlideEditOp::ReorderElements { element_id, to_z } => {
            if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                el.transform_mut().z_index = *to_z;
            } else {
                tracing::warn!("SlideEditOp ReorderElements: element_id {} not found on slide, dropped", element_id);
            }
            normalize_z_indices(&mut slide.elements);
        }
        SlideEditOp::GroupElements { element_ids } => {
            group_elements(&mut slide.elements, element_ids);
        }
        SlideEditOp::UngroupElements { group_id } => {
            ungroup_elements(&mut slide.elements, group_id);
        }
        SlideEditOp::SetSpeakerNotes { notes } => {
            slide.speaker_notes = Some(notes.clone());
            slide.notes = Some(notes.clone());
        }
        SlideEditOp::SetCcliMetadata { metadata } => {
            slide.ccli_metadata = Some(metadata.clone());
        }
        SlideEditOp::SetBackground { background } => {
            slide.background = Some(format_background_v2_to_css(background));
            slide.background_v2 = Some(background.clone());
        }
        SlideEditOp::SetTransition { transition } => {
            slide.transition = Some(transition.clone());
        }
    }
}

fn normalize_z_indices(elements: &mut [crate::core::models::SlideElement]) {
    elements.sort_by_key(|e| e.transform().z_index);
    for (idx, el) in elements.iter_mut().enumerate() {
        el.transform_mut().z_index = idx as i32;
    }
}

fn group_elements(elements: &mut Vec<crate::core::models::SlideElement>, ids: &[String]) {
    let mut children = Vec::new();
    let mut min_x = 1.0;
    let mut min_y = 1.0;
    let mut max_x = 0.0;
    let mut max_y = 0.0;
    let mut max_z = 0;

    let mut i = 0;
    while i < elements.len() {
        if ids.contains(&elements[i].id().to_string()) {
            let el = elements.remove(i);
            let t = el.transform();
            if t.x < min_x { min_x = t.x; }
            if t.y < min_y { min_y = t.y; }
            if t.x + t.w > max_x { max_x = t.x + t.w; }
            if t.y + t.h > max_y { max_y = t.y + t.h; }
            if t.z_index > max_z { max_z = t.z_index; }
            children.push(el);
        } else {
            i += 1;
        }
    }

    if !children.is_empty() {
        let group_transform = crate::core::models::ElementTransform {
            x: min_x.clamp(0.0, 1.0),
            y: min_y.clamp(0.0, 1.0),
            w: (max_x - min_x).clamp(0.0, 1.0),
            h: (max_y - min_y).clamp(0.0, 1.0),
            rotation_deg: 0.0,
            z_index: max_z,
            locked: false,
            opacity: 1.0,
        };
        let group = crate::core::models::SlideElement::Group {
            id: format!("group_{}", uuid::Uuid::new_v4()),
            transform: group_transform,
            children,
        };
        elements.push(group);
        normalize_z_indices(elements);
    }
}

fn ungroup_elements(elements: &mut Vec<crate::core::models::SlideElement>, group_id: &str) {
    if let Some(pos) = elements.iter().position(|e| e.id() == group_id) {
        let el = elements.remove(pos);
        if let crate::core::models::SlideElement::Group { children, .. } = el {
            elements.extend(children);
            normalize_z_indices(elements);
        }
    }
}

fn merge_contiguous_text_runs(runs: &[crate::core::models::TextRun]) -> Vec<crate::core::models::TextRun> {
    let mut merged: Vec<crate::core::models::TextRun> = Vec::new();
    for run in runs {
        if run.text.is_empty() {
            continue;
        }
        if let Some(last) = merged.last_mut() {
            if last.bold == run.bold
                && last.italic == run.italic
                && last.underline == run.underline
                && last.strike == run.strike
                && last.color == run.color
                && last.font_family == run.font_family
                && (last.font_size_pt - run.font_size_pt).abs() < 0.01
                && (last.letter_spacing_px - run.letter_spacing_px).abs() < 0.01
                && last.baseline_shift == run.baseline_shift
            {
                last.text.push_str(&run.text);
                continue;
            }
        }
        merged.push(run.clone());
    }
    merged
}

fn format_background_v2_to_css(bg: &crate::core::models::SlideBackground) -> String {
    match bg {
        crate::core::models::SlideBackground::Solid(color) => color.clone(),
        crate::core::models::SlideBackground::Gradient { kind, stops, angle_deg } => {
            let stop_strs = stops.iter().map(|s| format!("{} {}%", s.color, (s.offset * 100.0) as u32)).collect::<Vec<_>>().join(", ");
            if kind == "radial" {
                format!("radial-gradient(circle, {})", stop_strs)
            } else {
                let angle = angle_deg.unwrap_or(135.0);
                format!("linear-gradient({}deg, {})", angle as u32, stop_strs)
            }
        }
        crate::core::models::SlideBackground::Image { file_path, .. } => format!("url('{}')", file_path),
        crate::core::models::SlideBackground::Video { file_path, .. } => file_path.clone(),
        crate::core::models::SlideBackground::Pattern { bg_color, .. } => bg_color.clone(),
    }
}

pub(super) fn plan_element_events(_state: &ShowState, _schedule: &Schedule, cmd: &ShowCommand) -> Vec<ShowEvent> {
    let mut events = Vec::new();
    match cmd {
        ShowCommand::AddSlideElement { item_index, slide_index, element, at_index } => {
            events.push(ShowEvent::SlideElementAdded {
                item_index: *item_index,
                slide_index: *slide_index,
                element: element.clone(),
                at_index: *at_index,
            });
        },
        ShowCommand::RemoveSlideElement { item_index, slide_index, element_id } => {
            events.push(ShowEvent::SlideElementRemoved {
                item_index: *item_index,
                slide_index: *slide_index,
                element_id: element_id.clone(),
            });
        },
        ShowCommand::UpdateSlideElementTransform { item_index, slide_index, element_id, transform } => {
            events.push(ShowEvent::SlideElementTransformUpdated {
                item_index: *item_index,
                slide_index: *slide_index,
                element_id: element_id.clone(),
                transform: transform.clone(),
            });
        },
        ShowCommand::UpdateTextBlockContent { item_index, slide_index, element_id, runs, paragraph_style } => {
            events.push(ShowEvent::TextBlockContentUpdated {
                item_index: *item_index,
                slide_index: *slide_index,
                element_id: element_id.clone(),
                runs: runs.clone(),
                paragraph_style: paragraph_style.clone(),
            });
        },
        ShowCommand::UpdateElementEffects { item_index, slide_index, element_id, effects } => {
            events.push(ShowEvent::ElementEffectsUpdated {
                item_index: *item_index,
                slide_index: *slide_index,
                element_id: element_id.clone(),
                effects: effects.clone(),
            });
        },
        ShowCommand::ReorderSlideElements { item_index, slide_index, element_id, to_z } => {
            events.push(ShowEvent::SlideElementsReordered {
                item_index: *item_index,
                slide_index: *slide_index,
                element_id: element_id.clone(),
                to_z: *to_z,
            });
        },
        ShowCommand::GroupSlideElements { item_index, slide_index, element_ids } => {
            events.push(ShowEvent::SlideElementsGrouped {
                item_index: *item_index,
                slide_index: *slide_index,
                element_ids: element_ids.clone(),
            });
        },
        ShowCommand::UngroupSlideElements { item_index, slide_index, group_id } => {
            events.push(ShowEvent::SlideElementsUngrouped {
                item_index: *item_index,
                slide_index: *slide_index,
                group_id: group_id.clone(),
            });
        },
        ShowCommand::SetSlideBackgroundV2 { item_index, slide_index, background } => {
            events.push(ShowEvent::SlideBackgroundV2Set {
                item_index: *item_index,
                slide_index: *slide_index,
                background: background.clone(),
            });
        },
        ShowCommand::SetSlideTransition { item_index, slide_index, transition } => {
            events.push(ShowEvent::SlideTransitionSet {
                item_index: *item_index,
                slide_index: *slide_index,
                transition: transition.clone(),
            });
        },
        ShowCommand::SetSlideSpeakerNotes { item_index, slide_index, notes } => {
            events.push(ShowEvent::SlideSpeakerNotesSet {
                item_index: *item_index,
                slide_index: *slide_index,
                notes: notes.clone(),
            });
        },
        ShowCommand::SetSlideCcliMetadata { item_index, slide_index, metadata } => {
            events.push(ShowEvent::SlideCcliMetadataSet {
                item_index: *item_index,
                slide_index: *slide_index,
                metadata: metadata.clone(),
            });
        },
        ShowCommand::BatchSlideEdit { item_index, slide_index, ops } => {
            events.push(ShowEvent::SlideEditedBatch {
                item_index: *item_index,
                slide_index: *slide_index,
                ops: ops.clone(),
            });
        },
        _ => unreachable!("plan_element_events called with a non-element command"),
    }
    events
}

pub(super) fn apply_element_event(state: &mut ShowState, schedule: &mut Schedule, event: &ShowEvent) {
    match event {
        ShowEvent::SlideElementAdded { item_index, slide_index, element, at_index } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    if let Some(idx) = at_index {
                        if *idx <= slide.elements.len() {
                            slide.elements.insert(*idx, element.clone());
                        } else {
                            slide.elements.push(element.clone());
                        }
                    } else {
                        slide.elements.push(element.clone());
                    }
                    normalize_z_indices(&mut slide.elements);
                    slide.project_text_from_elements();
                    slide.slide_document_version = 1;
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideElementRemoved { item_index, slide_index, element_id } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    slide.elements.retain(|e| e.id() != element_id);
                    normalize_z_indices(&mut slide.elements);
                    slide.project_text_from_elements();
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideElementTransformUpdated { item_index, slide_index, element_id, transform } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                        *el.transform_mut() = transform.clone().normalized();
                        schedule.is_modified = true;
                        sync_item_in_state(state, schedule, *item_index);
                    }
                }
            }
        }
        ShowEvent::TextBlockContentUpdated { item_index, slide_index, element_id, runs, paragraph_style } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                        if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                            block.runs = merge_contiguous_text_runs(runs);
                            if let Some(style) = paragraph_style {
                                block.paragraph_style = style.clone();
                            }
                            slide.project_text_from_elements();
                            schedule.is_modified = true;
                            sync_item_in_state(state, schedule, *item_index);
                        }
                    }
                }
            }
        }
        ShowEvent::ElementEffectsUpdated { item_index, slide_index, element_id, effects } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                        if let crate::core::models::SlideElement::TextBlock { block, .. } = el {
                            block.effects = effects.clone();
                            schedule.is_modified = true;
                            sync_item_in_state(state, schedule, *item_index);
                        }
                    }
                }
            }
        }
        ShowEvent::SlideElementsReordered { item_index, slide_index, element_id, to_z } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    if let Some(el) = slide.elements.iter_mut().find(|e| e.id() == element_id) {
                        el.transform_mut().z_index = *to_z;
                    }
                    normalize_z_indices(&mut slide.elements);
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideElementsGrouped { item_index, slide_index, element_ids } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    group_elements(&mut slide.elements, element_ids);
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideElementsUngrouped { item_index, slide_index, group_id } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    ungroup_elements(&mut slide.elements, group_id);
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideBackgroundV2Set { item_index, slide_index, background } => {
            if let Some(idx) = item_index {
                if let Some(item) = schedule.items.get_mut(*idx) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.background_v2 = Some(background.clone());
                        slide.background = Some(format_background_v2_to_css(background));
                        schedule.is_modified = true;
                        sync_item_in_state(state, schedule, *idx);
                    }
                }
            } else if let Some(ref mut live) = state.live_item {
                if let Some(slide) = live.slides.get_mut(*slide_index) {
                    slide.background_v2 = Some(background.clone());
                    slide.background = Some(format_background_v2_to_css(background));
                }
            }
        }
        ShowEvent::SlideTransitionSet { item_index, slide_index, transition } => {
            if let Some(idx) = item_index {
                if let Some(item) = schedule.items.get_mut(*idx) {
                    if let Some(slide) = item.slides.get_mut(*slide_index) {
                        slide.transition = Some(transition.clone());
                        schedule.is_modified = true;
                        sync_item_in_state(state, schedule, *idx);
                    }
                }
            }
        }
        ShowEvent::SlideSpeakerNotesSet { item_index, slide_index, notes } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    slide.speaker_notes = Some(notes.clone());
                    slide.notes = Some(notes.clone());
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideCcliMetadataSet { item_index, slide_index, metadata } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    slide.ccli_metadata = Some(metadata.clone());
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        ShowEvent::SlideEditedBatch { item_index, slide_index, ops } => {
            if let Some(item) = schedule.items.get_mut(*item_index) {
                if let Some(slide) = item.slides.get_mut(*slide_index) {
                    for op in ops {
                        apply_slide_edit_op(slide, op);
                    }
                    slide.project_text_from_elements();
                    slide.slide_document_version = 1;
                    schedule.is_modified = true;
                    sync_item_in_state(state, schedule, *item_index);
                }
            }
        }
        _ => unreachable!("apply_element_event called with a non-element event"),
    }
}
