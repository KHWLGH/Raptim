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
    prepare_tooltips(window);
    use i_slint_core::{
        lengths::{LogicalPoint, LogicalRect, LogicalSize},
        window::WindowInner,
    };
    let size = window.size().to_logical(window.scale_factor());
    let region = LogicalRect::new(
        LogicalPoint::new(0., 0.),
        LogicalSize::new(size.width, size.height),
    );
    WindowInner::from_pub(window)
        .window_adapter()
        .renderer()
        .mark_dirty_region(region.into());
}

/// Slint 1.18 still wraps custom Tooltip content in the styled ToolTipImpl.
/// Remove only that first outline before painting; HintSurface owns the surface.
/// Kept with the other version-pinned rendering bridges until upstream fixes it.
pub fn prepare_tooltips(window: &slint::Window) {
    use i_slint_core::{
        item_tree::{ItemVisitorResult, visit_items},
        items::BorderRectangle,
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
                if let Some(outline) =
                    i_slint_core::items::ItemRef::downcast_pin::<BorderRectangle>(item)
                {
                    BorderRectangle::FIELD_OFFSETS
                        .background()
                        .apply_pin(outline)
                        .set(Default::default());
                    BorderRectangle::FIELD_OFFSETS
                        .border_width()
                        .apply_pin(outline)
                        .set(Default::default());
                    return ItemVisitorResult::Abort;
                }
                ItemVisitorResult::Continue(())
            },
            (),
        );
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
    if let Some((rect, work)) = crate::platform::geometry(window) {
        let size = window.size();
        if rect.width > work.width || rect.height > work.height {
            window.set_size(slint::PhysicalSize::new(
                size.width.min(work.width.max(1) as u32),
                size.height.min(work.height.max(1) as u32),
            ));
        }
    }
    if existed {
        if !crate::platform::request_redraw_now(window) {
            window.request_redraw();
        }
    } else {
        invalidate(window);
    }
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

pub fn show_child(
    child: &slint::Window,
    parent: &slint::Window,
) -> Result<(), slint::PlatformError> {
    crate::platform::center_child(child, parent);
    crate::platform::set_owner(child, parent);
    show(child)?;
    // The first show creates the native handle and can change DPI / constrained size.
    crate::platform::set_owner(child, parent);
    crate::platform::center_child(child, parent);
    crate::platform::focus(child);
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
}
