//! Window lifecycle shared by every application window.
//!
//! Slint 1.18 / winit software rendering can reuse a cleared Windows surface after
//! hide/show without a dirty region. request_redraw alone then presents no pixels.
//! Keep this small bridge pinned to the exact Slint version, and remove it once
//! upstream guarantees a full redraw on remapping (see the native regression case).
pub fn invalidate(window: &slint::Window) {
    mark_full_frame(window);
    if !crate::platform::request_redraw_now(window) {
        window.request_redraw();
    }
}

/// Call before a software frame is rendered when Windows may have replaced the backing
/// surface (resize, scale-factor and theme changes); a partial dirty region can otherwise
/// retain old tiles. This deliberately does not request another frame.
pub fn mark_full_frame(window: &slint::Window) {
    let size = crate::platform::surface_size(window);
    mark_physical_rect(window, 0, 0, size.width as i32, size.height as i32);
}

pub fn mark_physical_rect(window: &slint::Window, left: i32, top: i32, right: i32, bottom: i32) {
    prepare_tooltips(window);
    let Some((x, y, width, height)) =
        physical_rect_to_logical(left, top, right, bottom, window.scale_factor())
    else {
        return;
    };
    use i_slint_core::{
        lengths::{LogicalPoint, LogicalRect, LogicalSize},
        window::WindowInner,
    };
    let region = LogicalRect::new(LogicalPoint::new(x, y), LogicalSize::new(width, height));
    WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(region.into());
}

fn physical_rect_to_logical(
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
    scale_factor: f32,
) -> Option<(f32, f32, f32, f32)> {
    if right <= left || bottom <= top || right <= 0 || bottom <= 0 {
        return None;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0. {
        scale_factor
    } else {
        1.
    };
    let x0 = (left.max(0) as f32 / scale).floor();
    let y0 = (top.max(0) as f32 / scale).floor();
    let x1 = (right as f32 / scale).ceil();
    let y1 = (bottom as f32 / scale).ceil();
    Some((x0, y0, (x1 - x0).max(1.), (y1 - y0).max(1.)))
}

fn clear_outline_brushes(
    background: core::pin::Pin<&i_slint_core::Property<i_slint_core::Brush>>,
    border_color: core::pin::Pin<&i_slint_core::Property<i_slint_core::Brush>>,
) {
    // Clear only the non-constant brush bindings so the software renderer skips painting without
    // mutating ToolTipImpl's constant border_width/border_radius properties.
    background.set(Default::default());
    border_color.set(Default::default());
}

fn strip_tooltip_outline(item: core::pin::Pin<i_slint_core::items::ItemRef<'_>>) -> bool {
    use i_slint_core::items::{BasicBorderRectangle, BorderRectangle, ItemRef};
    if let Some(outline) = ItemRef::downcast_pin::<BasicBorderRectangle>(item) {
        clear_outline_brushes(
            BasicBorderRectangle::FIELD_OFFSETS
                .background()
                .apply_pin(outline),
            BasicBorderRectangle::FIELD_OFFSETS
                .border_color()
                .apply_pin(outline),
        );
        return true;
    }
    if let Some(outline) = ItemRef::downcast_pin::<BorderRectangle>(item) {
        clear_outline_brushes(
            BorderRectangle::FIELD_OFFSETS
                .background()
                .apply_pin(outline),
            BorderRectangle::FIELD_OFFSETS
                .border_color()
                .apply_pin(outline),
        );
        return true;
    }
    false
}

/// Slint 1.18 wraps custom Tooltip content in ToolTipImpl, which lowers uniform rounded borders
/// into BasicBorderRectangle with constant border_width/border_radius. Clear only that first
/// outer wrapper's brushes before painting so HintSurface owns the card surface without corners
/// peeking through or mutating constant properties.
pub fn prepare_tooltips(window: &slint::Window) {
    use i_slint_core::{
        item_tree::{ItemVisitorResult, visit_items},
        window::{WindowInner, WindowKind},
    };
    for popup in WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .filter(|p| matches!(p.window_kind, WindowKind::ToolTip))
    {
        visit_items(
            &popup.component,
            i_slint_core::item_tree::TraversalOrder::BackToFront,
            |_, item, _, &()| {
                if strip_tooltip_outline(item) {
                    ItemVisitorResult::Abort
                } else {
                    ItemVisitorResult::Continue(())
                }
            },
            (),
        );
    }
}

fn clamp_to_work_area(window: &slint::Window) {
    if let Some((rect, work)) = crate::platform::geometry(window) {
        let size = window.size();
        if rect.width > work.width || rect.height > work.height {
            window.set_size(slint::PhysicalSize::new(
                size.width.min(work.width.max(1) as u32),
                size.height.min(work.height.max(1) as u32),
            ));
        }
    }
}

pub fn show(window: &slint::Window) -> Result<(), slint::PlatformError> {
    let existed = crate::platform::native_ready(window);
    crate::platform::skip_taskbar(window);
    if existed {
        // Prepare frame attributes and full dirty region before ShowWindow so the very first
        // WM_PAINT presents complete pixels with DWMWCP_ROUND intact and zero style thrash.
        crate::platform::decorate(window);
        mark_full_frame(window);
    }
    window.show()?;
    crate::platform::decorate(window);
    clamp_to_work_area(window);
    invalidate(window);
    Ok(())
}
pub fn has_popup(window: &slint::Window) -> bool {
    i_slint_core::window::WindowInner::from_pub(window)
        .active_popups()
        .iter()
        .any(|p| !matches!(p.window_kind, i_slint_core::window::WindowKind::ToolTip))
}
pub fn close_popups(window: &slint::Window) {
    i_slint_core::window::WindowInner::from_pub(window).close_all_popups();
}

/// Returns true only when `(x, y)` (logical coordinates) hits an interactive item inside an
/// enabled `DragArea` — never scrollbars, text inputs, overlays, popups, or blank areas.
pub fn hit_test_drag_area(window: &slint::Window, x: f32, y: f32) -> bool {
    if has_popup(window) {
        return false;
    }
    use i_slint_core::{
        item_tree::{ItemRc, ItemVisitorResult, TraversalOrder, visit_items},
        items::{DragArea, ItemRef, TextInput, TouchArea, WindowMoveArea},
        lengths::LogicalPoint,
        window::WindowInner,
    };
    let Some(component) = WindowInner::from_pub(window).try_component() else {
        return false;
    };
    let mut hit_drag = false;
    visit_items(
        &component,
        TraversalOrder::FrontToBack,
        |item_tree, item, index, &(pos, in_drag_area)| {
            let item_rc = ItemRc::new(item_tree.clone(), index);
            let geom = item_rc.geometry();
            let contains = geom.contains(pos);
            if !contains && item.as_ref().clips_children() {
                return ItemVisitorResult::SkipChildren;
            }
            let next_pos = pos - geom.origin.to_vector();
            if contains {
                if let Some(touch) = ItemRef::downcast_pin::<TouchArea>(item)
                    && touch.enabled()
                {
                    hit_drag = in_drag_area;
                    return ItemVisitorResult::Abort;
                }
                if let Some(input) = ItemRef::downcast_pin::<TextInput>(item)
                    && input.enabled()
                {
                    hit_drag = false;
                    return ItemVisitorResult::Abort;
                }
                if let Some(move_area) = ItemRef::downcast_pin::<WindowMoveArea>(item)
                    && move_area.enabled()
                {
                    hit_drag = false;
                    return ItemVisitorResult::Abort;
                }
            }
            let next_drag = in_drag_area
                || (contains
                    && ItemRef::downcast_pin::<DragArea>(item).is_some_and(|drag| drag.enabled()));
            ItemVisitorResult::Continue((next_pos, next_drag))
        },
        (LogicalPoint::new(x, y), false),
    );
    hit_drag
}

fn finish_child_show(child: &slint::Window, parent: &slint::Window) {
    crate::platform::skip_taskbar(child);
    crate::platform::decorate(child);
    clamp_to_work_area(child);
    crate::platform::set_owner(child, parent);
    crate::platform::center_child(child, parent);
    crate::platform::focus(child);
    invalidate(child);
}

fn schedule_child_finish(
    child_adapter: std::rc::Weak<dyn i_slint_core::window::WindowAdapter>,
    parent_adapter: std::rc::Weak<dyn i_slint_core::window::WindowAdapter>,
    delay: std::time::Duration,
    attempts_left: u8,
    on_async_ready: impl FnOnce() + 'static,
) {
    slint::Timer::single_shot(delay, move || {
        let (Some(child_adapter_rc), Some(parent_adapter_rc)) =
            (child_adapter.upgrade(), parent_adapter.upgrade())
        else {
            return;
        };
        let child = child_adapter_rc.window();
        let parent = parent_adapter_rc.window();
        if !child.is_visible() {
            return;
        }
        if crate::platform::native_ready(child) {
            finish_child_show(child, parent);
            on_async_ready();
        } else if attempts_left > 0 {
            schedule_child_finish(
                child_adapter,
                parent_adapter,
                std::time::Duration::from_millis(16),
                attempts_left - 1,
                on_async_ready,
            );
        }
    });
}

pub fn show_child_with(
    child: &slint::Window,
    parent: &slint::Window,
    on_async_ready: impl FnOnce() + 'static,
) -> Result<(), slint::PlatformError> {
    crate::platform::center_child(child, parent);
    crate::platform::set_owner(child, parent);
    show(child)?;
    if crate::platform::native_ready(child) {
        crate::platform::set_owner(child, parent);
        crate::platform::center_child(child, parent);
        crate::platform::focus(child);
        return Ok(());
    }
    // Slint's winit backend defers HWND creation until the event loop's next
    // about_to_wait pass, so owner, decoration and foreground must run once it exists.
    use i_slint_core::window::WindowInner;
    let child_adapter = std::rc::Rc::downgrade(&WindowInner::from_pub(child).window_adapter());
    let parent_adapter = std::rc::Rc::downgrade(&WindowInner::from_pub(parent).window_adapter());
    schedule_child_finish(
        child_adapter,
        parent_adapter,
        std::time::Duration::ZERO,
        20,
        on_async_ready,
    );
    Ok(())
}

/// Hand an active child's foreground back before hiding it. Otherwise Windows
/// may activate an unrelated window, leaving the owner behind it (and making
/// the next focus/fullscreen policy tick treat closing the child as app loss).
/// Never recall a hidden owner or steal focus when the child is in background.
pub fn close_child(
    child: &slint::Window,
    parent: &slint::Window,
) -> Result<(), slint::PlatformError> {
    let return_focus = parent.is_visible() && crate::platform::window_active(child);
    if return_focus {
        crate::platform::focus(parent);
    }
    child.hide()?;
    if return_focus {
        // Complete the handoff after the native hide has processed activation.
        crate::platform::focus(parent);
        invalidate(parent);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Physical screen coordinates throughout, including negative monitor origins.
pub fn centered_position(parent: Rect, child: (i32, i32), work: Rect) -> (i32, i32) {
    let x = parent.x + (parent.width - child.0) / 2;
    let y = parent.y + (parent.height - child.1) / 2;
    (
        x.clamp(work.x, work.x + (work.width - child.0).max(0)),
        y.clamp(work.y, work.y + (work.height - child.1).max(0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centers_on_main_window_instead_of_primary_screen() {
        assert_eq!(
            centered_position(
                Rect {
                    x: -1500,
                    y: 180,
                    width: 600,
                    height: 960
                },
                (690, 750),
                Rect {
                    x: -1920,
                    y: 0,
                    width: 1920,
                    height: 1200
                }
            ),
            (-1545, 285)
        );
    }
    #[test]
    fn clamps_to_work_area_near_taskbar_and_removed_screen() {
        assert_eq!(
            centered_position(
                Rect {
                    x: 1800,
                    y: 900,
                    width: 400,
                    height: 640
                },
                (520, 600),
                Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1040
                }
            ),
            (1400, 440)
        );
    }

    #[test]
    fn converts_physical_update_rects_outward_to_logical_bounds() {
        assert_eq!(physical_rect_to_logical(0, 0, 0, 100, 1.5), None);
        assert_eq!(
            physical_rect_to_logical(1, 0, 50, 200, 1.5),
            Some((0., 0., 34., 134.))
        );
        assert_eq!(
            physical_rect_to_logical(-5, -3, 10, 10, 1.25),
            Some((0., 0., 8., 8.))
        );
    }

    #[test]
    fn strips_basic_border_rectangle_tooltip_outline_without_touching_constants() {
        use i_slint_core::{
            Brush, Color, Property, items::BasicBorderRectangle, lengths::LogicalLength,
        };
        use slint::private_unstable_api::re_exports::VRef;

        let theme_bg = std::rc::Rc::pin(Property::new(Brush::SolidColor(Color::from_argb_u8(
            255, 243, 243, 243,
        ))));
        let outline = std::pin::pin!(BasicBorderRectangle::default());
        BasicBorderRectangle::FIELD_OFFSETS
            .background()
            .apply_pin(outline.as_ref())
            .set_binding({
                let theme_bg = theme_bg.clone();
                move || theme_bg.as_ref().get()
            });
        BasicBorderRectangle::FIELD_OFFSETS
            .border_color()
            .apply_pin(outline.as_ref())
            .set(Brush::SolidColor(Color::from_argb_u8(255, 180, 180, 180)));

        let border_width = BasicBorderRectangle::FIELD_OFFSETS
            .border_width()
            .apply_pin(outline.as_ref());
        border_width.set(LogicalLength::new(1.0));
        border_width.set_constant();

        let border_radius = BasicBorderRectangle::FIELD_OFFSETS
            .border_radius()
            .apply_pin(outline.as_ref());
        border_radius.set(LogicalLength::new(4.0));
        border_radius.set_constant();

        let item = VRef::new_pin(outline.as_ref());
        assert!(strip_tooltip_outline(item));
        assert!(strip_tooltip_outline(item));

        // Reactive palette changes must not revive the cleared outline.
        theme_bg
            .as_ref()
            .set(Brush::SolidColor(Color::from_argb_u8(255, 32, 32, 32)));
        assert_eq!(outline.as_ref().background(), Brush::default());
        assert_eq!(outline.as_ref().border_color(), Brush::default());
    }

    #[test]
    fn strips_border_rectangle_tooltip_outline_without_touching_constants() {
        use i_slint_core::{Brush, Color, items::BorderRectangle, lengths::LogicalLength};
        use slint::private_unstable_api::re_exports::VRef;

        let outline = std::pin::pin!(BorderRectangle::default());
        BorderRectangle::FIELD_OFFSETS
            .background()
            .apply_pin(outline.as_ref())
            .set(Brush::SolidColor(Color::from_argb_u8(255, 240, 240, 240)));
        BorderRectangle::FIELD_OFFSETS
            .border_color()
            .apply_pin(outline.as_ref())
            .set(Brush::SolidColor(Color::from_argb_u8(255, 190, 190, 190)));

        let border_width = BorderRectangle::FIELD_OFFSETS
            .border_width()
            .apply_pin(outline.as_ref());
        border_width.set(LogicalLength::new(1.0));
        border_width.set_constant();

        let item = VRef::new_pin(outline.as_ref());
        assert!(strip_tooltip_outline(item));
        assert!(strip_tooltip_outline(item));
        assert_eq!(outline.as_ref().background(), Brush::default());
        assert_eq!(outline.as_ref().border_color(), Brush::default());
    }
}
