//! One OLE target per application HWND. Only Drop reads data; hovering never imports.
use super::native_handle;
use crate::importing::ExternalDrop;
use std::{cell::Cell, path::PathBuf, rc::Rc};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            Com::*, DataExchange::RegisterClipboardFormatW, Memory::*, Ole::*,
            SystemServices::MODIFIERKEYS_FLAGS,
        },
        UI::Shell::*,
    },
    core::{PCWSTR, Ref, implement},
};

pub enum DropEvent {
    Hover(i32, i32),
    Leave,
    Data(ExternalDrop, i32, i32),
}
type Listener = Rc<dyn Fn(DropEvent)>;
#[implement(IDropTarget)]
struct Target {
    listener: Listener,
    accepted: Cell<bool>,
}
fn format(id: u16) -> FORMATETC {
    FORMATETC {
        cfFormat: id,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    }
}
fn registered(name: &str) -> u16 {
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) as u16 }
}
fn formats() -> &'static [u16; 5] {
    static FORMATS: std::sync::OnceLock<[u16; 5]> = std::sync::OnceLock::new();
    FORMATS.get_or_init(|| {
        [
            15,
            registered("text/x-moz-url"),
            registered("UniformResourceLocatorW"),
            13,
            registered("UniformResourceLocator"),
        ]
    })
}
fn accepts(data: &IDataObject) -> bool {
    formats()
        .iter()
        .any(|&id| id != 0 && unsafe { data.QueryGetData(&format(id)).is_ok() })
}
struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe {
            ReleaseStgMedium(&mut self.0);
        }
    }
}
fn read(data: &IDataObject) -> Option<ExternalDrop> {
    for (index, &id) in formats().iter().enumerate() {
        if id == 0 {
            continue;
        }
        let Ok(raw) = (unsafe { data.GetData(&format(id)) }) else {
            continue;
        };
        let medium = Medium(raw);
        if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
            continue;
        }
        let handle = unsafe { medium.0.u.hGlobal };
        if index == 0 {
            let drop = HDROP(handle.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            if count == 0 {
                return None;
            }
            if count > 500 {
                return Some(ExternalDrop::Files(vec![]));
            }
            let mut paths = Vec::with_capacity(count as usize);
            let mut buffer = Vec::new();
            for i in 0..count {
                let length = unsafe { DragQueryFileW(drop, i, None) };
                if length == 0 {
                    continue;
                }
                buffer.resize(length as usize + 1, 0);
                let written = unsafe { DragQueryFileW(drop, i, Some(&mut buffer)) } as usize;
                if written == 0 {
                    continue;
                }
                use std::os::windows::ffi::OsStringExt;
                paths.push(PathBuf::from(std::ffi::OsString::from_wide(
                    &buffer[..written.min(buffer.len())],
                )));
            }
            return (!paths.is_empty()).then_some(ExternalDrop::Files(paths));
        }
        let size = unsafe { GlobalSize(handle) };
        if size == 0 || size > 1024 * 1024 {
            continue;
        }
        let ptr = unsafe { GlobalLock(handle) };
        if ptr.is_null() {
            continue;
        }
        let text = if index == 4 {
            let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), size) };
            String::from_utf8_lossy(bytes.split(|b| *b == 0).next().unwrap_or_default())
                .into_owned()
        } else {
            let words = unsafe { std::slice::from_raw_parts(ptr.cast::<u16>(), size / 2) };
            String::from_utf16_lossy(words.split(|b| *b == 0).next().unwrap_or_default())
        };
        unsafe {
            let _ = GlobalUnlock(handle);
        }
        if !text.trim().is_empty() {
            return Some(ExternalDrop::Text(text));
        }
    }
    None
}
#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.accepted.set(data.as_ref().is_some_and(accepts));
        unsafe {
            *effect = if self.accepted.get() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if self.accepted.get() {
            (self.listener)(DropEvent::Hover(point.x, point.y));
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *effect = if self.accepted.get() {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        if self.accepted.get() {
            (self.listener)(DropEvent::Hover(point.x, point.y));
        }
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        self.accepted.set(false);
        (self.listener)(DropEvent::Leave);
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *effect = DROPEFFECT_NONE;
        }
        if self.accepted.get()
            && let Some(value) = data.as_ref().and_then(read)
        {
            (self.listener)(DropEvent::Data(value, point.x, point.y));
            unsafe {
                *effect = DROPEFFECT_COPY;
            }
        }
        self.accepted.set(false);
        Ok(())
    }
}
pub struct Registration {
    hwnd: HWND,
    _target: IDropTarget,
}
impl Registration {
    pub fn new(
        window: &slint::Window,
        listener: impl Fn(DropEvent) + 'static,
    ) -> windows::core::Result<Self> {
        let hwnd = native_handle(window).ok_or_else(|| windows::core::Error::from(E_HANDLE))?;
        unsafe {
            OleInitialize(None)?;
        }
        let target: IDropTarget = Target {
            listener: Rc::new(listener),
            accepted: Cell::new(false),
        }
        .into();
        if let Err(e) = unsafe { RegisterDragDrop(hwnd, &target) } {
            unsafe {
                OleUninitialize();
            }
            return Err(e);
        }
        Ok(Self {
            hwnd,
            _target: target,
        })
    }
}
impl Drop for Registration {
    fn drop(&mut self) {
        unsafe {
            let _ = RevokeDragDrop(self.hwnd);
            OleUninitialize();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[implement(IDataObject)]
    struct TestData(Vec<(u16, Vec<u16>)>);
    #[allow(non_snake_case)]
    impl IDataObject_Impl for TestData_Impl {
        fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
            let fmt = unsafe { (*f).cfFormat };
            let Some((_, text)) = self.0.iter().find(|(id, _)| *id == fmt) else {
                return Err(DV_E_FORMATETC.into());
            };
            let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, text.len() * 2)? };
            let ptr = unsafe { GlobalLock(handle) };
            if ptr.is_null() {
                unsafe {
                    let _ = GlobalFree(Some(handle));
                }
                return Err(E_OUTOFMEMORY.into());
            }
            unsafe {
                std::ptr::copy_nonoverlapping(text.as_ptr(), ptr.cast(), text.len());
                let _ = GlobalUnlock(handle);
            }
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: handle },
                pUnkForRelease: Default::default(),
            })
        }
        fn QueryGetData(&self, f: *const FORMATETC) -> windows::core::HRESULT {
            let fmt = unsafe { (*f).cfFormat };
            if self.0.iter().any(|(id, _)| *id == fmt) {
                S_OK
            } else {
                DV_E_FORMATETC
            }
        }
        fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn GetCanonicalFormatEtc(
            &self,
            _: *const FORMATETC,
            _: *mut FORMATETC,
        ) -> windows::core::HRESULT {
            E_NOTIMPL
        }
        fn SetData(
            &self,
            _: *const FORMATETC,
            _: *const STGMEDIUM,
            _: windows::core::BOOL,
        ) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn EnumFormatEtc(&self, _: u32) -> windows::core::Result<IEnumFORMATETC> {
            Err(E_NOTIMPL.into())
        }
        fn DAdvise(
            &self,
            _: *const FORMATETC,
            _: u32,
            _: Ref<'_, IAdviseSink>,
        ) -> windows::core::Result<u32> {
            Err(E_NOTIMPL.into())
        }
        fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
            Err(E_NOTIMPL.into())
        }
        fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
            Err(E_NOTIMPL.into())
        }
    }
    #[test]
    fn native_unicode_browser_data_and_cancel_do_not_import() {
        for format in [
            13,
            registered("UniformResourceLocatorW"),
            registered("text/x-moz-url"),
        ] {
            let data: IDataObject = TestData(vec![(
                format,
                "https://example.com/中文\n标题\0".encode_utf16().collect(),
            )])
            .into();
            assert!(accepts(&data));
            let Some(ExternalDrop::Text(text)) = read(&data) else {
                panic!("missing Unicode URL")
            };
            assert_eq!(
                crate::importing::url_entries(&text, 2).unwrap()[0].name,
                "标题"
            );
            let imported = Rc::new(Cell::new(0));
            let count = imported.clone();
            let target: IDropTarget = Target {
                listener: Rc::new(move |e| {
                    if matches!(e, DropEvent::Data(..)) {
                        count.set(count.get() + 1);
                    }
                }),
                accepted: Cell::new(false),
            }
            .into();
            let mut effect = DROPEFFECT_COPY;
            unsafe {
                target
                    .DragEnter(
                        &data,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL { x: 10, y: 10 },
                        &mut effect,
                    )
                    .unwrap();
                target.DragLeave().unwrap();
            }
            assert_eq!(imported.get(), 0);
            unsafe {
                target
                    .DragEnter(
                        &data,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL { x: 10, y: 10 },
                        &mut effect,
                    )
                    .unwrap();
                target
                    .Drop(
                        &data,
                        MODIFIERKEYS_FLAGS(0),
                        POINTL { x: 10, y: 10 },
                        &mut effect,
                    )
                    .unwrap();
            }
            assert_eq!(imported.get(), 1);
        }
    }

    #[test]
    fn prefers_moz_url_with_title_over_bare_uniform_resource_locator() {
        let data: IDataObject = TestData(vec![
            (
                registered("UniformResourceLocatorW"),
                "https://example.com/path\0".encode_utf16().collect(),
            ),
            (
                registered("text/x-moz-url"),
                "https://example.com/path\n网页标题\0"
                    .encode_utf16()
                    .collect(),
            ),
        ])
        .into();
        let Some(ExternalDrop::Text(text)) = read(&data) else {
            panic!("missing browser URL")
        };
        let entries = crate::importing::url_entries(&text, 2).unwrap();
        assert_eq!(entries[0].name, "网页标题");
        assert_eq!(entries[0].target.value(), "https://example.com/path");
    }
}
