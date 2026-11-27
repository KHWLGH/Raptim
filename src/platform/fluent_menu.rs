//! Fluent owner drawing with Windows menu tracking (keyboard, capture and cascades).
use super::*;
use crate::menus::MenuItem;
use windows::Win32::UI::Controls::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

thread_local! {
    static ACTIVE_PAINT: std::cell::Cell<*const Paint> = const { std::cell::Cell::new(std::ptr::null()) };
    static POPUPS: std::cell::RefCell<Vec<HWND>> = const { std::cell::RefCell::new(Vec::new()) };
}

// User32 paints its selected cascade arrow after WM_DRAWITEM using system
// highlight text. Repaint just the arrow gutter after the popup's own paint.
unsafe extern "system" fn arrow_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCPAINT {
            // User32's legacy bevel does not follow the compositor's rounded
            // outline. Fill only the non-client gutter; DWM owns the outer stroke.
            paint_frame(hwnd, &*(data as *const Paint));
            return LRESULT(0);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(arrow_proc), id);
        } else if msg == WM_PAINT || msg == WM_PRINTCLIENT || msg == WM_APP + 44 {
            let paint = &*(data as *const Paint);
            let menu = HMENU(SendMessageW(hwnd, 0x01e1, None, None).0 as *mut _);
            if !menu.is_invalid() {
                let dc = if msg == WM_PRINTCLIENT {
                    HDC(wp.0 as *mut _)
                } else {
                    GetDC(Some(hwnd))
                };
                let saved = SaveDC(dc);
                for i in 0..GetMenuItemCount(Some(menu)).max(0) as u32 {
                    let mut item = MENUITEMINFOW {
                        cbSize: size_of::<MENUITEMINFOW>() as u32,
                        fMask: MIIM_SUBMENU | MIIM_STATE,
                        ..Default::default()
                    };
                    if GetMenuItemInfoW(menu, i, true, &mut item).is_err()
                        || item.hSubMenu.is_invalid()
                    {
                        continue;
                    }
                    let mut r = RECT::default();
                    if GetMenuItemRect(Some(hwnd), menu, i, &mut r).is_err() {
                        continue;
                    }
                    let mut origin = POINT {
                        x: r.left,
                        y: r.top,
                    };
                    let _ = ScreenToClient(hwnd, &mut origin);
                    r = RECT {
                        left: origin.x,
                        top: origin.y,
                        right: origin.x + r.right - r.left,
                        bottom: origin.y + r.bottom - r.top,
                    };
                    r.left = r.right - paint.px(20);
                    r.right -= paint.px(4);
                    r.top += paint.px(3);
                    r.bottom -= paint.px(3);
                    let selected = item.fState.0 & MFS_HILITE.0 != 0;
                    FillRect(
                        dc,
                        &r,
                        if selected {
                            paint.hover
                        } else {
                            paint.background
                        },
                    );
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(
                        dc,
                        if item.fState.0 & MFS_DISABLED.0 != 0 {
                            paint.muted
                        } else {
                            paint.foreground
                        },
                    );
                    SelectObject(dc, paint.icons.into());
                    let mut arrow: Vec<_> = "\u{e76c}".encode_utf16().collect();
                    DrawTextW(
                        dc,
                        &mut arrow,
                        &mut r,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                }
                let _ = RestoreDC(dc, saved);
                if msg != WM_PRINTCLIENT {
                    ReleaseDC(Some(hwnd), dc);
                }
            }
        }
        result
    }
}

unsafe fn paint_frame(hwnd: HWND, paint: &Paint) {
    unsafe {
        let mut outer = RECT::default();
        let mut client = RECT::default();
        if GetWindowRect(hwnd, &mut outer).is_err() || GetClientRect(hwnd, &mut client).is_err() {
            return;
        }
        let mut origin = POINT::default();
        if !ClientToScreen(hwnd, &mut origin).as_bool() {
            return;
        }
        let dc = GetWindowDC(Some(hwnd));
        if dc.is_invalid() {
            return;
        }
        let saved = SaveDC(dc);
        ExcludeClipRect(
            dc,
            origin.x - outer.left,
            origin.y - outer.top,
            origin.x - outer.left + client.right,
            origin.y - outer.top + client.bottom,
        );
        let bounds = RECT {
            left: 0,
            top: 0,
            right: outer.right - outer.left,
            bottom: outer.bottom - outer.top,
        };
        FillRect(dc, &bounds, paint.background);
        if GetPropW(hwnd, w!("Raptim.MenuComposed")).is_invalid() {
            let pen = CreatePen(PS_SOLID, 1, paint.stroke);
            let old_pen = SelectObject(dc, pen.into());
            let old_brush = SelectObject(dc, GetStockObject(NULL_BRUSH));
            let _ = RoundRect(
                dc,
                0,
                0,
                bounds.right,
                bounds.bottom,
                paint.px(16),
                paint.px(16),
            );
            SelectObject(dc, old_brush);
            SelectObject(dc, old_pen);
            let _ = DeleteObject(pen.into());
        }
        let _ = RestoreDC(dc, saved);
        ReleaseDC(Some(hwnd), dc);
    }
}

// Menus are transient windows made by TrackPopupMenu. Style only menus created
// on this thread while our Fluent menu is open; the Shell menu stays untouched.
unsafe extern "system" fn menu_created(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if code >= 0 {
            let event = &*(lp.0 as *const CWPSTRUCT);
            if event.message == WM_WINDOWPOSCHANGED {
                let mut class = [0u16; 16];
                let len = GetClassNameW(event.hwnd, &mut class);
                if String::from_utf16_lossy(&class[..len as usize]) == "#32768" {
                    ACTIVE_PAINT.with(|p| {
                        if !p.get().is_null() {
                            let _ = SetWindowSubclass(
                                event.hwnd,
                                Some(arrow_proc),
                                0xD144,
                                p.get() as usize,
                            );
                            POPUPS.with(|windows| {
                                let mut windows = windows.borrow_mut();
                                if !windows.contains(&event.hwnd) {
                                    windows.push(event.hwnd);
                                }
                            });
                        }
                    });
                    let mut r = RECT::default();
                    if GetWindowRect(event.hwnd, &mut r).is_ok() {
                        let stamp =
                            (((r.right - r.left) as usize) << 32) | (r.bottom - r.top) as usize;
                        if GetPropW(event.hwnd, w!("Raptim.MenuSize")).0 as usize == stamp {
                            return CallNextHookEx(None, code, wp, lp);
                        }
                        let _ = SetPropW(
                            event.hwnd,
                            w!("Raptim.MenuSize"),
                            Some(HANDLE(stamp as *mut _)),
                        );
                        let corner = DWMWCP_ROUND;
                        let composed = DwmSetWindowAttribute(
                            event.hwnd,
                            DWMWA_WINDOW_CORNER_PREFERENCE,
                            &corner as *const _ as _,
                            size_of_val(&corner) as u32,
                        )
                        .is_ok();
                        let color = ACTIVE_PAINT.with(|p| (*p.get()).stroke);
                        let _ = DwmSetWindowAttribute(
                            event.hwnd,
                            DWMWA_BORDER_COLOR,
                            &color as *const _ as _,
                            4,
                        );
                        if composed {
                            // An HRGN disables DWM anti-aliasing and clips the
                            // native border at a different radius. Never combine them.
                            let _ = SetPropW(
                                event.hwnd,
                                w!("Raptim.MenuComposed"),
                                Some(HANDLE(1usize as _)),
                            );
                            let _ = SetWindowRgn(event.hwnd, None, true);
                        } else {
                            let scale = GetDpiForWindow(event.hwnd) as f32 / 96.;
                            let diameter = (16. * scale).round() as i32;
                            let region = CreateRoundRectRgn(
                                0,
                                0,
                                r.right - r.left + 1,
                                r.bottom - r.top + 1,
                                diameter,
                                diameter,
                            );
                            if SetWindowRgn(event.hwnd, Some(region), true) == 0 {
                                let _ = DeleteObject(region.into());
                            }
                        }
                    }
                }
            }
        }
        CallNextHookEx(None, code, wp, lp)
    }
}

struct Paint {
    scale: f32,
    background: HBRUSH,
    hover: HBRUSH,
    divider: HBRUSH,
    foreground: COLORREF,
    muted: COLORREF,
    stroke: COLORREF,
    font: HFONT,
    icons: HFONT,
}
impl Paint {
    fn px(&self, n: i32) -> i32 {
        (n as f32 * self.scale).round() as i32
    }
}
impl Drop for Paint {
    fn drop(&mut self) {
        unsafe {
            for object in [
                self.background.into(),
                self.hover.into(),
                self.divider.into(),
                self.font.into(),
                self.icons.into(),
            ] {
                let _ = DeleteObject(object);
            }
        }
    }
}

fn new_paint(scale: f32, dark: bool) -> Paint {
    unsafe {
        let font = |name, size| {
            CreateFontW(
                -(size as f32 * scale).round() as i32,
                0,
                0,
                0,
                400,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                0,
                name,
            )
        };
        Paint {
            scale,
            background: CreateSolidBrush(COLORREF(if dark { 0x282828 } else { 0xfcfcfc })),
            hover: CreateSolidBrush(COLORREF(if dark { 0x3c3c3c } else { 0xeeeeee })),
            divider: CreateSolidBrush(COLORREF(if dark { 0x3f3f3f } else { 0xe6e6e6 })),
            foreground: COLORREF(if dark { 0xf4f4f4 } else { 0x242424 }),
            muted: COLORREF(if dark { 0x888888 } else { 0xa0a0a0 }),
            stroke: COLORREF(if dark { 0x505050 } else { 0xd0d0d0 }),
            font: font(
                if crate::i18n::is_en() {
                    w!("Segoe UI")
                } else {
                    w!("Microsoft YaHei UI")
                },
                13,
            ),
            icons: font(w!("Segoe Fluent Icons"), 16),
        }
    }
}

/// Measured in physical pixels: `(width, height)`. A section rule is a nine-logical-pixel band with
/// a single line through it, so adding a section to a menu costs 18 physical pixels at 200% DPI.
fn measure_item(paint: &Paint, item: &MenuItem, label_px: i32) -> (i32, i32) {
    if item.separator {
        return (paint.px(206), paint.px(9));
    }
    (
        (label_px + paint.px(60)).clamp(paint.px(206), paint.px(340)),
        paint.px(34),
    )
}

/// One row, into whatever DC User32 hands us — or, in the probes, a memory bitmap.
unsafe fn draw_item(dc: HDC, paint: &Paint, r: &RECT, item: &MenuItem, selected: bool) {
    unsafe {
        FillRect(dc, r, paint.background);
        if item.separator {
            let y = (r.top + r.bottom) / 2;
            let line = RECT {
                left: r.left + paint.px(10),
                top: y - paint.px(1) / 2,
                right: r.right - paint.px(10),
                bottom: y + paint.px(1) - paint.px(1) / 2,
            };
            FillRect(dc, &line, paint.divider);
            return;
        }
        if selected && item.enabled {
            let old_brush = SelectObject(dc, paint.hover.into());
            let old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
            let _ = RoundRect(
                dc,
                r.left + paint.px(3),
                r.top + paint.px(2),
                r.right - paint.px(3),
                r.bottom - paint.px(2),
                paint.px(8),
                paint.px(8),
            );
            SelectObject(dc, old_pen);
            SelectObject(dc, old_brush);
        }
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(
            dc,
            if item.enabled {
                paint.foreground
            } else {
                paint.muted
            },
        );
        let old = SelectObject(dc, paint.font.into());
        let mut label: Vec<_> = item.label.encode_utf16().collect();
        let mut text_rect = RECT {
            left: r.left + paint.px(38),
            right: r.right - paint.px(22),
            ..*r
        };
        DrawTextW(
            dc,
            &mut label,
            &mut text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, paint.icons.into());
        let mut glyph: Vec<_> = item.icon.to_string().encode_utf16().collect();
        let mut icon_rect = RECT {
            left: r.left + paint.px(11),
            right: r.left + paint.px(29),
            ..*r
        };
        DrawTextW(
            dc,
            &mut glyph,
            &mut icon_rect,
            DT_SINGLELINE | DT_VCENTER | DT_CENTER,
        );
        SelectObject(dc, old);
    }
}

unsafe fn label_width(paint: &Paint, item: &MenuItem) -> i32 {
    unsafe {
        let dc = GetDC(None);
        let old = SelectObject(dc, paint.font.into());
        let text: Vec<_> = item.label.encode_utf16().collect();
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(dc, &text, &mut size);
        SelectObject(dc, old);
        ReleaseDC(None, dc);
        size.cx
    }
}

unsafe extern "system" fn paint_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let paint = &*(data as *const Paint);
        if msg == WM_MEASUREITEM {
            let measure = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
            if measure.CtlType == ODT_MENU && measure.itemData != 0 {
                let item = &*(measure.itemData as *const MenuItem);
                let (width, height) = measure_item(paint, item, label_width(paint, item));
                measure.itemWidth = width as u32;
                measure.itemHeight = height as u32;
                return LRESULT(1);
            }
        }
        if msg == WM_DRAWITEM {
            let draw = &*(lp.0 as *const DRAWITEMSTRUCT);
            if draw.CtlType == ODT_MENU && draw.itemData != 0 {
                let item = &*(draw.itemData as *const MenuItem);
                draw_item(
                    draw.hDC,
                    paint,
                    &draw.rcItem,
                    item,
                    (draw.itemState.0 & ODS_SELECTED.0) != 0,
                );
                // Highlight changes can draw directly without WM_PAINT.
                POPUPS.with(|windows| {
                    for hwnd in windows.borrow().iter() {
                        let _ = PostMessageW(Some(*hwnd), WM_APP + 44, WPARAM(0), LPARAM(0));
                    }
                });
                return LRESULT(1);
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
unsafe fn build(items: &[MenuItem], paint: &Paint) -> windows::core::Result<HMENU> {
    unsafe {
        let menu = CreatePopupMenu()?;
        let result = (|| {
            SetMenuInfo(
                menu,
                &MENUINFO {
                    cbSize: size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND | MIM_STYLE,
                    hbrBack: paint.background,
                    dwStyle: MNS_NOCHECK,
                    ..Default::default()
                },
            )?;
            for (index, item) in items.iter().enumerate() {
                let child = if item.children.is_empty() {
                    HMENU::default()
                } else {
                    build(&item.children, paint)?
                };
                let mut label = wide(&item.label.replace('&', "&&"));
                let info = MENUITEMINFOW {
                    cbSize: size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE
                        | MIIM_STRING
                        | MIIM_ID
                        | MIIM_STATE
                        | MIIM_DATA
                        | MIIM_SUBMENU,
                    fType: MFT_OWNERDRAW,
                    fState: if item.enabled {
                        MFS_ENABLED
                    } else {
                        MFS_DISABLED
                    },
                    wID: item.command,
                    hSubMenu: child,
                    dwItemData: item as *const MenuItem as usize,
                    dwTypeData: PWSTR(label.as_mut_ptr()),
                    ..Default::default()
                };
                if let Err(e) = InsertMenuItemW(menu, index as u32, true, &info) {
                    if !child.is_invalid() {
                        let _ = DestroyMenu(child);
                    }
                    return Err(e);
                }
            }
            Ok(menu)
        })();
        if result.is_err() {
            let _ = DestroyMenu(menu);
        }
        result
    }
}
pub fn show(window: &slint::Window, items: &[MenuItem], background: slint::Color) -> Option<u32> {
    let hwnd = native_handle(window)?;
    let dark =
        u32::from(background.red()) + u32::from(background.green()) + u32::from(background.blue())
            < 384;
    let scale = window.scale_factor();
    unsafe {
        let paint = new_paint(scale, dark);
        let menu = build(items, &paint).ok()?;
        if !SetWindowSubclass(
            hwnd,
            Some(paint_proc),
            0xD143,
            &paint as *const Paint as usize,
        )
        .as_bool()
        {
            let _ = DestroyMenu(menu);
            return None;
        }
        let (x, y) = cursor();
        ACTIVE_PAINT.with(|p| p.set(&paint));
        let hook = SetWindowsHookExW(
            WH_CALLWNDPROC,
            Some(menu_created),
            None,
            GetCurrentThreadId(),
        )
        .ok();
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenuEx(
            menu,
            (TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN).0,
            x,
            y,
            hwnd,
            None,
        )
        .0 as u32;
        let _ = RemoveWindowSubclass(hwnd, Some(paint_proc), 0xD143);
        if let Some(hook) = hook {
            let _ = UnhookWindowsHookEx(hook);
        }
        POPUPS.with(|windows| {
            for hwnd in windows.borrow_mut().drain(..) {
                let _ = RemoveWindowSubclass(hwnd, Some(arrow_proc), 0xD144);
            }
        });
        let _ = DestroyMenu(menu);
        ACTIVE_PAINT.with(|p| p.set(std::ptr::null()));
        (command != 0).then_some(command)
    }
}
