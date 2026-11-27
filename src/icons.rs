use crate::model::{Error, Result};
use std::{io::Cursor, path::Path};
/// Tiles render at most 48 logical pixels, so 128 covers 200%+ DPI without carrying a
/// full-size bitmap per entry in memory. Older databases still hold 256px blobs.
pub const STORE_MAX: u32 = 128;
pub fn encode(image: image::RgbaImage) -> Result<Vec<u8>> {
    let image = image::DynamicImage::ImageRgba8(image);
    let image = if image.width() > STORE_MAX || image.height() > STORE_MAX {
        image.thumbnail(STORE_MAX, STORE_MAX)
    } else {
        image
    };
    let mut data = Cursor::new(Vec::new());
    image
        .write_to(&mut data, image::ImageFormat::Png)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(data.into_inner())
}
pub fn from_file(path: &Path) -> Result<Vec<u8>> {
    if std::fs::metadata(path)?.len() > 16 * 1024 * 1024 {
        return Err(Error::Invalid(
            crate::i18n::t("图标文件不能超过 16 MB", "Icon file must not exceed 16 MB.").into(),
        ));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| {
        Error::Invalid(format!(
            "{}{e}",
            crate::i18n::t("无法读取图标：", "Failed to read icon: ")
        ))
    })?;
    encode(image.to_rgba8())
}
fn decode(bytes: &[u8]) -> Option<image::RgbaImage> {
    if bytes.is_empty() {
        return None;
    }
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .unwrap();
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(256);
    limits.max_image_height = Some(256);
    limits.max_alloc = Some(1024 * 1024);
    reader.limits(limits);
    reader.decode().ok().map(|image| image.to_rgba8())
}

pub fn fingerprint(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// A blob's fingerprint plus its slot in the entry list. Scrolling asks for the same few dozen
/// icons over and over, so both are precomputed once per refresh instead of re-hashing the PNG
/// and rescanning the whole library on every viewport change.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct IconKey {
    pub fingerprint: u64,
    pub index: usize,
}

type Job = (i64, u64, Vec<u8>);
struct Decoded {
    id: i64,
    key: u64,
    image: Option<image::RgbaImage>,
    ghost: Option<image::RgbaImage>,
}
struct Cached {
    key: u64,
    image: slint::Image,
    ghost: slint::Image,
    size: usize,
    touched: u64,
}
pub struct Cache {
    sender: Option<std::sync::mpsc::SyncSender<Job>>,
    receiver: std::sync::mpsc::Receiver<Decoded>,
    worker: Option<std::thread::JoinHandle<()>>,
    pending: std::collections::HashSet<(i64, u64)>,
    entries: std::collections::HashMap<i64, Cached>,
    clock: u64,
    bytes: usize,
}
impl Cache {
    pub fn new(notify: impl Fn() + Send + 'static) -> Self {
        let (sender, jobs) = std::sync::mpsc::sync_channel::<Job>(64);
        let (output, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while let Ok((id, key, data)) = jobs.recv() {
                let image = decode(&data).map(|image| {
                    if image.width() > STORE_MAX || image.height() > STORE_MAX {
                        image::DynamicImage::ImageRgba8(image)
                            .thumbnail(STORE_MAX, STORE_MAX)
                            .to_rgba8()
                    } else {
                        image
                    }
                });
                let ghost = image.as_ref().map(|i| {
                    image::imageops::resize(i, 36, 36, image::imageops::FilterType::Lanczos3)
                });
                if output
                    .send(Decoded {
                        id,
                        key,
                        image,
                        ghost,
                    })
                    .is_err()
                {
                    break;
                }
                notify();
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            pending: Default::default(),
            entries: Default::default(),
            clock: 0,
            bytes: 0,
        }
    }
    pub fn request(&mut self, id: i64, key: u64, bytes: &[u8]) {
        self.clock += 1;
        if let Some(e) = self.entries.get_mut(&id)
            && e.key == key
        {
            e.touched = self.clock;
            return;
        }
        self.remove(&id);
        if bytes.is_empty() || self.pending.contains(&(id, key)) {
            return;
        }
        if self
            .sender
            .as_ref()
            .is_some_and(|s| s.try_send((id, key, bytes.to_vec())).is_ok())
        {
            self.pending.insert((id, key));
        }
    }
    pub fn get(&self, id: i64) -> (slint::Image, slint::Image) {
        self.entries
            .get(&id)
            .map(|e| (e.image.clone(), e.ghost.clone()))
            .unwrap_or_default()
    }
    pub fn remove(&mut self, id: &i64) {
        if let Some(e) = self.entries.remove(id) {
            self.bytes -= e.size;
        }
    }
    pub fn retain(&mut self, ids: &std::collections::HashSet<i64>) {
        self.entries.retain(|id, _| ids.contains(id));
        self.bytes = self.entries.values().map(|e| e.size).sum();
    }
    pub fn drain(
        &mut self,
        keys: &std::collections::HashMap<i64, IconKey>,
        visible: &std::collections::HashSet<i64>,
    ) -> Vec<i64> {
        let mut changed = Vec::new();
        while let Ok(result) = self.receiver.try_recv() {
            self.pending.remove(&(result.id, result.key));
            if keys.get(&result.id).map(|k| k.fingerprint) != Some(result.key) {
                continue;
            }
            let size = result.image.as_ref().map_or(0, |i| i.as_raw().len())
                + result.ghost.as_ref().map_or(0, |i| i.as_raw().len());
            let convert = |rgba: Option<image::RgbaImage>| {
                rgba.map(|i| {
                    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
                        i.as_raw(),
                        i.width(),
                        i.height(),
                    ))
                })
                .unwrap_or_default()
            };
            self.remove(&result.id);
            self.entries.insert(
                result.id,
                Cached {
                    key: result.key,
                    image: convert(result.image),
                    ghost: convert(result.ghost),
                    size,
                    touched: self.clock,
                },
            );
            self.bytes += size;
            changed.push(result.id);
        }
        // 224 visible tiles at 128px plus their 36px ghosts is ~16 MB; the rest is LRU headroom.
        while self.bytes > 24 * 1024 * 1024 {
            let oldest = self
                .entries
                .iter()
                .filter(|(id, _)| !visible.contains(id))
                .min_by_key(|(_, e)| e.touched)
                .map(|(id, _)| *id);
            let Some(id) = oldest else {
                break;
            };
            self.remove(&id);
        }
        changed
    }
}
impl Drop for Cache {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub fn display(bytes: &[u8]) -> slint::Image {
    let Some(rgba) = decode(bytes) else {
        return slint::Image::default();
    };
    slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
        rgba.as_raw(),
        rgba.width(),
        rgba.height(),
    ))
}
#[cfg(test)]
mod cache_tests {
    use super::*;
    #[test]
    fn stale_decodes_are_rejected_and_offscreen_entries_are_evicted() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut cache = Cache::new(move || {
            let _ = tx.send(());
        });
        cache.request(1, fingerprint(b"invalid old icon"), b"invalid old icon");
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        cache.drain(
            &std::collections::HashMap::from([(
                1,
                IconKey {
                    fingerprint: fingerprint(b"new icon"),
                    index: 0,
                },
            )]),
            &Default::default(),
        );
        assert!(cache.entries.is_empty());
        assert!(cache.pending.is_empty());
        for id in 1..=3 {
            cache.entries.insert(
                id,
                Cached {
                    key: 0,
                    image: Default::default(),
                    ghost: Default::default(),
                    size: 12 * 1024 * 1024,
                    touched: id as u64,
                },
            );
        }
        cache.bytes = 36 * 1024 * 1024;
        cache.drain(&Default::default(), &std::collections::HashSet::from([1]));
        assert!(cache.entries.contains_key(&1));
        assert!(!cache.entries.contains_key(&2));
        assert_eq!(cache.bytes, 24 * 1024 * 1024);
    }
}
