use crate::model::{Error, Result};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
};

/// Tiles render at most 48 logical pixels, so 128 covers 200%+ DPI without carrying a
/// full-size bitmap per entry in memory. Older databases still hold 256px blobs.
const STORE_MAX: u32 = 128;

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SOURCE_EDGE: u32 = 4096;
const MAX_SOURCE_ALLOC: u64 = 64 * 1024 * 1024;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_SOURCE_ALLOC);
    limits
}

fn downscale_rgba(image: image::RgbaImage, max_edge: u32) -> image::RgbaImage {
    let (width, height) = (image.width(), image.height());
    if width <= max_edge && height <= max_edge {
        return image;
    }
    let scale = max_edge as f64 / width.max(height) as f64;
    let new_width = ((width as f64 * scale).round() as u32).clamp(1, max_edge);
    let new_height = ((height as f64 * scale).round() as u32).clamp(1, max_edge);
    image::imageops::resize(
        &image,
        new_width,
        new_height,
        image::imageops::FilterType::Lanczos3,
    )
}

pub fn encode(image: image::RgbaImage) -> Result<Vec<u8>> {
    let image = downscale_rgba(image, STORE_MAX);
    let mut data = Cursor::new(Vec::new());
    image
        .write_to(&mut data, image::ImageFormat::Png)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    Ok(data.into_inner())
}

/// Windows `SHIL_JUMBO` often draws legacy 32/48px icons in the top-left corner of a 256px canvas.
/// Trim that corner box back to its natural standard bucket so downscaling keeps the icon crisp.
pub fn trim_jumbo_corner_padding(image: image::RgbaImage) -> image::RgbaImage {
    let (width, height) = (image.width(), image.height());
    if width < 128 || height < 128 {
        return image;
    }
    let mut max_x = None;
    let mut max_y = 0u32;
    for (x, y, px) in image.enumerate_pixels() {
        if px[3] > 8 {
            max_x = Some(max_x.map_or(x, |cur: u32| cur.max(x)));
            max_y = max_y.max(y);
        }
    }
    let Some(max_x) = max_x else {
        return image;
    };
    if max_x < 64 && max_y < 64 {
        let extent = (max_x + 1).max(max_y + 1);
        let side = [32u32, 48, 64]
            .into_iter()
            .find(|&bucket| extent <= bucket)
            .unwrap_or(64)
            .min(width.min(height));
        return image::imageops::crop_imm(&image, 0, 0, side, side).to_image();
    }
    image
}

fn icon_rank(
    width: u8,
    height: u8,
    color_count: u8,
    bit_count: u16,
    bytes_in_res: u32,
) -> (u32, u32, u16, u32) {
    let rw = if width == 0 { 256 } else { u32::from(width) };
    let rh = if height == 0 { 256 } else { u32::from(height) };
    let effective_bpp = if bit_count > 0 {
        bit_count
    } else if color_count == 0 {
        32
    } else {
        8
    };
    (rw.max(rh), rw.min(rh), effective_bpp, bytes_in_res)
}

fn decode_icon_frame(
    raw: &[u8],
    width: u8,
    height: u8,
    color_count: u8,
    planes: u16,
    bit_count: u16,
) -> Option<image::RgbaImage> {
    let res_len = u32::try_from(raw.len()).ok()?;
    if res_len < 8 || u64::from(res_len) > MAX_SOURCE_BYTES {
        return None;
    }
    if raw.starts_with(&PNG_SIGNATURE) {
        let mut reader = image::ImageReader::with_format(Cursor::new(raw), image::ImageFormat::Png);
        reader.limits(decode_limits());
        return reader.decode().ok().map(|img| img.to_rgba8());
    }
    let mut wrapped = Vec::with_capacity(22 + raw.len());
    wrapped.extend_from_slice(&[0, 0, 1, 0, 1, 0]);
    wrapped.push(width);
    wrapped.push(height);
    wrapped.push(color_count);
    wrapped.push(0);
    wrapped.extend_from_slice(&planes.to_le_bytes());
    wrapped.extend_from_slice(&bit_count.to_le_bytes());
    wrapped.extend_from_slice(&res_len.to_le_bytes());
    wrapped.extend_from_slice(&22u32.to_le_bytes());
    wrapped.extend_from_slice(raw);
    let mut reader = image::ImageReader::with_format(Cursor::new(wrapped), image::ImageFormat::Ico);
    reader.limits(decode_limits());
    reader.decode().ok().map(|img| img.to_rgba8())
}

pub fn decode_best_ico(bytes: &[u8]) -> Option<image::RgbaImage> {
    if bytes.len() < 6 {
        return None;
    }
    let reserved = u16::from_le_bytes([bytes[0], bytes[1]]);
    let kind = u16::from_le_bytes([bytes[2], bytes[3]]);
    let count = usize::from(u16::from_le_bytes([bytes[4], bytes[5]]));
    if reserved != 0 || kind != 1 || count == 0 || count > 1024 {
        return None;
    }
    let dir_end = 6usize.checked_add(count.checked_mul(16)?)?;
    if dir_end > bytes.len() {
        return None;
    }
    let mut candidates = Vec::with_capacity(count);
    for entry in bytes[6..dir_end].as_chunks::<16>().0 {
        let width = entry[0];
        let height = entry[1];
        let color_count = entry[2];
        let planes = u16::from_le_bytes([entry[4], entry[5]]);
        let bit_count = u16::from_le_bytes([entry[6], entry[7]]);
        let bytes_in_res = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
        let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
        let start = usize::try_from(offset).ok()?;
        let len = usize::try_from(bytes_in_res).ok()?;
        let Some(end) = start.checked_add(len) else {
            continue;
        };
        if len < 8 || end > bytes.len() {
            continue;
        }
        candidates.push((
            icon_rank(width, height, color_count, bit_count, bytes_in_res),
            width,
            height,
            color_count,
            planes,
            bit_count,
            start,
            end,
        ));
    }
    candidates.sort_by_key(|c| std::cmp::Reverse(c.0));
    candidates.into_iter().find_map(|c| {
        decode_icon_frame(&bytes[c.6..c.7], c.1, c.2, c.3, c.4, c.5)
            .filter(|img| img.pixels().any(|p| p[3] > 0))
    })
}

struct PeSection {
    virtual_address: u32,
    size_of_raw_data: u32,
    pointer_to_raw_data: u32,
}

fn read_exact_at<R: Read + Seek>(reader: &mut R, offset: u64, buf: &mut [u8]) -> Option<()> {
    reader.seek(SeekFrom::Start(offset)).ok()?;
    reader.read_exact(buf).ok()
}

fn u16_le(bytes: &[u8], at: usize) -> Option<u16> {
    let slice = bytes.get(at..at + 2)?;
    Some(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_le(bytes: &[u8], at: usize) -> Option<u32> {
    let slice = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn rva_to_offset(sections: &[PeSection], rva: u32) -> Option<u64> {
    for section in sections {
        if section.pointer_to_raw_data == 0 || section.size_of_raw_data == 0 {
            continue;
        }
        if let Some(rel) = rva.checked_sub(section.virtual_address)
            && rel < section.size_of_raw_data
        {
            return u64::from(section.pointer_to_raw_data).checked_add(u64::from(rel));
        }
    }
    None
}

fn read_rva_bytes<R: Read + Seek>(
    reader: &mut R,
    sections: &[PeSection],
    rva: u32,
    size: u32,
    max_size: usize,
) -> Option<Vec<u8>> {
    let len = usize::try_from(size).ok()?;
    if len == 0 || len > max_size {
        return None;
    }
    let section = sections.iter().find(|s| {
        s.pointer_to_raw_data > 0
            && s.size_of_raw_data > 0
            && rva.checked_sub(s.virtual_address).is_some_and(|rel| {
                u64::from(rel) + u64::from(size) <= u64::from(s.size_of_raw_data)
            })
    })?;
    let rel = rva - section.virtual_address;
    let offset = u64::from(section.pointer_to_raw_data) + u64::from(rel);
    let mut out = vec![0u8; len];
    read_exact_at(reader, offset, &mut out)?;
    Some(out)
}

#[derive(Clone, Copy)]
struct ResourceEntry {
    is_named: bool,
    id: u32,
    offset_to_data: u32,
}

fn read_resource_dir<R: Read + Seek>(
    reader: &mut R,
    sections: &[PeSection],
    rsrc_rva: u32,
    sub_offset: u32,
) -> Option<Vec<ResourceEntry>> {
    let dir_rva = rsrc_rva.checked_add(sub_offset & 0x7FFF_FFFF)?;
    let dir_off = rva_to_offset(sections, dir_rva)?;
    let mut header = [0u8; 16];
    read_exact_at(reader, dir_off, &mut header)?;
    let named = usize::from(u16_le(&header, 12)?);
    let id_count = usize::from(u16_le(&header, 14)?);
    let total = named.checked_add(id_count)?;
    if total == 0 || total > 4096 {
        return None;
    }
    let mut raw = vec![0u8; total * 8];
    read_exact_at(reader, dir_off + 16, &mut raw)?;
    let mut entries = Vec::with_capacity(total);
    for (idx, chunk) in raw.as_chunks::<8>().0.iter().enumerate() {
        entries.push(ResourceEntry {
            is_named: idx < named,
            id: u32_le(chunk, 0)? & 0x7FFF_FFFF,
            offset_to_data: u32_le(chunk, 4)?,
        });
    }
    Some(entries)
}

fn first_resource_data<R: Read + Seek>(
    reader: &mut R,
    sections: &[PeSection],
    rsrc_rva: u32,
    offset_to_data: u32,
    depth: u8,
) -> Option<(u32, u32)> {
    if offset_to_data & 0x8000_0000 == 0 {
        let leaf_rva = rsrc_rva.checked_add(offset_to_data)?;
        let leaf_off = rva_to_offset(sections, leaf_rva)?;
        let mut entry = [0u8; 16];
        read_exact_at(reader, leaf_off, &mut entry)?;
        return Some((u32_le(&entry, 0)?, u32_le(&entry, 4)?));
    }
    if depth >= 4 {
        return None;
    }
    let children = read_resource_dir(reader, sections, rsrc_rva, offset_to_data)?;
    for child in children {
        if let Some(found) =
            first_resource_data(reader, sections, rsrc_rva, child.offset_to_data, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

pub fn extract_pe_icon(path: &Path, icon_index: i32) -> Option<image::RgbaImage> {
    let mut file = std::fs::File::open(path).ok()?;
    extract_pe_icon_from_reader(&mut file, icon_index)
}

fn extract_pe_icon_from_reader<R: Read + Seek>(
    reader: &mut R,
    icon_index: i32,
) -> Option<image::RgbaImage> {
    let mut dos = [0u8; 64];
    read_exact_at(reader, 0, &mut dos)?;
    if &dos[0..2] != b"MZ" {
        return None;
    }
    let e_lfanew = u64::from(u32_le(&dos, 0x3C)?);
    let mut coff = [0u8; 24];
    read_exact_at(reader, e_lfanew, &mut coff)?;
    if &coff[0..4] != b"PE\0\0" {
        return None;
    }
    let section_count = usize::from(u16_le(&coff, 6)?);
    let opt_size = u16_le(&coff, 20)?;
    if !(1..=256).contains(&section_count) || !(120..=4096).contains(&opt_size) {
        return None;
    }
    let opt_off = e_lfanew + 24;
    let mut opt = vec![0u8; usize::from(opt_size)];
    read_exact_at(reader, opt_off, &mut opt)?;
    let (rsrc_dir_at, num_dirs_at) = match u16_le(&opt, 0)? {
        0x010B => (112usize, 92usize),
        0x020B => (128usize, 108usize),
        _ => return None,
    };
    if u32_le(&opt, num_dirs_at)? <= 2 {
        return None;
    }
    let rsrc_rva = u32_le(&opt, rsrc_dir_at)?;
    let rsrc_size = u32_le(&opt, rsrc_dir_at + 4)?;
    if rsrc_rva == 0 || rsrc_size == 0 {
        return None;
    }

    let mut raw_sections = vec![0u8; section_count * 40];
    read_exact_at(reader, opt_off + u64::from(opt_size), &mut raw_sections)?;
    let mut sections = Vec::with_capacity(section_count);
    for chunk in raw_sections.as_chunks::<40>().0 {
        sections.push(PeSection {
            virtual_address: u32_le(chunk, 12)?,
            size_of_raw_data: u32_le(chunk, 16)?,
            pointer_to_raw_data: u32_le(chunk, 20)?,
        });
    }

    let root = read_resource_dir(reader, &sections, rsrc_rva, 0)?;
    let group_type = root
        .iter()
        .find(|e| !e.is_named && e.id == 14 && (e.offset_to_data & 0x8000_0000 != 0))?;
    let icon_type = root
        .iter()
        .find(|e| !e.is_named && e.id == 3 && (e.offset_to_data & 0x8000_0000 != 0))?;

    let groups = read_resource_dir(reader, &sections, rsrc_rva, group_type.offset_to_data)?;
    let chosen_group = if icon_index < 0 {
        let target_id = icon_index.unsigned_abs();
        groups.iter().find(|e| !e.is_named && e.id == target_id)?
    } else {
        groups.get(usize::try_from(icon_index).ok()?)?
    };
    let (grp_rva, grp_size) =
        first_resource_data(reader, &sections, rsrc_rva, chosen_group.offset_to_data, 1)?;
    let grp_bytes = read_rva_bytes(reader, &sections, grp_rva, grp_size, 64 * 1024)?;
    if grp_bytes.len() < 6 {
        return None;
    }
    let reserved = u16_le(&grp_bytes, 0)?;
    let kind = u16_le(&grp_bytes, 2)?;
    let count = usize::from(u16_le(&grp_bytes, 4)?);
    if reserved != 0
        || kind != 1
        || count == 0
        || 6usize.checked_add(count.checked_mul(14)?)? > grp_bytes.len()
    {
        return None;
    }

    let mut frames = Vec::with_capacity(count);
    for chunk in grp_bytes[6..6 + count * 14].as_chunks::<14>().0 {
        let width = chunk[0];
        let height = chunk[1];
        let color_count = chunk[2];
        let planes = u16_le(chunk, 4)?;
        let bit_count = u16_le(chunk, 6)?;
        let bytes_in_res = u32_le(chunk, 8)?;
        let res_id = u32::from(u16_le(chunk, 12)?);
        frames.push((
            icon_rank(width, height, color_count, bit_count, bytes_in_res),
            width,
            height,
            color_count,
            planes,
            bit_count,
            res_id,
        ));
    }
    frames.sort_by_key(|f| std::cmp::Reverse(f.0));

    let rt_icons = read_resource_dir(reader, &sections, rsrc_rva, icon_type.offset_to_data)?;
    for frame in frames {
        let Some(icon_entry) = rt_icons.iter().find(|e| !e.is_named && e.id == frame.6) else {
            continue;
        };
        let Some((data_rva, data_size)) =
            first_resource_data(reader, &sections, rsrc_rva, icon_entry.offset_to_data, 1)
        else {
            continue;
        };
        let Some(raw) = read_rva_bytes(reader, &sections, data_rva, data_size, 4 * 1024 * 1024)
        else {
            continue;
        };
        if let Some(decoded) = decode_icon_frame(&raw, frame.1, frame.2, frame.3, frame.4, frame.5)
            .filter(|img| img.pixels().any(|p| p[3] > 0))
        {
            return Some(decoded);
        }
    }
    None
}

pub fn from_file(path: &Path) -> Result<Vec<u8>> {
    if std::fs::metadata(path)?.len() > MAX_SOURCE_BYTES {
        return Err(Error::Invalid(
            crate::i18n::t("图标文件不能超过 16 MB", "Icon file must not exceed 16 MB.").into(),
        ));
    }
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"))
        && let Ok(bytes) = std::fs::read(path)
        && let Some(image) = decode_best_ico(&bytes)
    {
        return encode(image);
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    reader.limits(decode_limits());
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
    notified: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pending: std::collections::HashSet<(i64, u64)>,
    entries: std::collections::HashMap<i64, Cached>,
    clock: u64,
    bytes: usize,
}
impl Cache {
    pub fn new(notify: impl Fn() + Send + 'static) -> Self {
        let (sender, jobs) = std::sync::mpsc::sync_channel::<Job>(64);
        let (output, receiver) = std::sync::mpsc::channel();
        let notified = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_notified = std::sync::Arc::clone(&notified);
        let worker = std::thread::spawn(move || {
            while let Ok((id, key, data)) = jobs.recv() {
                let image = decode(&data).map(|image| downscale_rgba(image, STORE_MAX));
                let ghost = image.as_ref().map(|i| {
                    image::imageops::resize(i, 36, 36, image::imageops::FilterType::Triangle)
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
                if !worker_notified.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    notify();
                }
            }
        });
        Self {
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            notified,
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
        if bytes.is_empty() || self.pending.len() >= 64 || self.pending.contains(&(id, key)) {
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
        self.notified
            .store(false, std::sync::atomic::Ordering::SeqCst);
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

    fn encode_png_rgba(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([0, 120, 212, 255]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn encode_png_rgb(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(width, height, image::Rgb([16, 124, 16]));
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn write_dir_entry(buf: &mut [u8], is_named: bool, id: u32, offset_to_data: u32) {
        let name_or_id = if is_named { id | 0x8000_0000 } else { id };
        buf[0..4].copy_from_slice(&name_or_id.to_le_bytes());
        buf[4..8].copy_from_slice(&offset_to_data.to_le_bytes());
    }

    fn write_grp_icon_entry(buf: &mut [u8], edge: u8, bytes_len: u32, res_id: u16) {
        buf[0] = edge;
        buf[1] = edge;
        buf[2] = 0;
        buf[3] = 0;
        buf[4..6].copy_from_slice(&1u16.to_le_bytes());
        buf[6..8].copy_from_slice(&32u16.to_le_bytes());
        buf[8..12].copy_from_slice(&bytes_len.to_le_bytes());
        buf[12..14].copy_from_slice(&res_id.to_le_bytes());
    }

    #[test]
    fn encode_downscales_with_lanczos3_preserving_aspect_ratio() {
        let wide = image::RgbaImage::from_pixel(256, 128, image::Rgba([10, 20, 30, 255]));
        let png = encode(wide).unwrap();
        let decoded = decode(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (128, 64));
    }

    #[test]
    fn decode_best_ico_prefers_highest_resolution_and_handles_rgb_png() {
        let small = encode_png_rgba(16, 16);
        let large = encode_png_rgb(256, 256);
        let offset1 = 6 + 2 * 16;
        let offset2 = offset1 + small.len();

        let mut ico = Vec::new();
        ico.extend_from_slice(&[0, 0, 1, 0, 2, 0]);
        for (edge, len, off) in [(16u8, small.len(), offset1), (0u8, large.len(), offset2)] {
            ico.extend_from_slice(&[edge, edge, 0, 0]);
            ico.extend_from_slice(&1u16.to_le_bytes());
            ico.extend_from_slice(&32u16.to_le_bytes());
            ico.extend_from_slice(&(len as u32).to_le_bytes());
            ico.extend_from_slice(&(off as u32).to_le_bytes());
        }
        ico.extend_from_slice(&small);
        ico.extend_from_slice(&large);

        let decoded = decode_best_ico(&ico).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (256, 256));
    }

    #[test]
    fn extract_pe_icon_selects_highest_resolution_by_index_and_negative_id() {
        let png16 = encode_png_rgba(16, 16);
        let png256 = encode_png_rgba(256, 256);
        let png48 = encode_png_rgba(48, 48);

        // Layout a minimal PE32+ image with a single `.rsrc` section at RVA 0x2000, file 0x200.
        let rsrc_rva = 0x2000u32;
        let rsrc_file = 0x200u32;
        let mut rsrc = vec![0u8; 0x2000];

        // Directory table at rel 0x000 (root, 2 entries: RT_ICON=3, RT_GROUP_ICON=14)
        // rel 0x030: RT_GROUP_ICON dir (2 entries: ID 1 -> rel 0x060, ID 101 -> rel 0x080)
        // rel 0x060..0x0C0: language dirs + data entries
        rsrc[14..16].copy_from_slice(&2u16.to_le_bytes());
        write_dir_entry(&mut rsrc[16..24], false, 3, 0x8000_0050);
        write_dir_entry(&mut rsrc[24..32], false, 14, 0x8000_0030);

        // RT_GROUP_ICON dir at 0x30 (2 id entries)
        rsrc[0x30 + 14..0x30 + 16].copy_from_slice(&2u16.to_le_bytes());
        write_dir_entry(&mut rsrc[0x40..0x48], false, 1, 0x8000_00A0);
        write_dir_entry(&mut rsrc[0x48..0x50], false, 101, 0x8000_00C0);

        // RT_ICON dir at 0x50 (3 id entries: 1, 2, 3)
        rsrc[0x50 + 14..0x50 + 16].copy_from_slice(&3u16.to_le_bytes());
        write_dir_entry(&mut rsrc[0x60..0x68], false, 1, 0x8000_00E0);
        write_dir_entry(&mut rsrc[0x68..0x70], false, 2, 0x8000_0100);
        write_dir_entry(&mut rsrc[0x70..0x78], false, 3, 0x8000_0120);

        // Language dirs (1 id entry each) pointing directly to 16-byte data leaves.
        for (dir_rel, leaf_rel) in [
            (0x0A0usize, 0x160usize),
            (0x0C0, 0x170),
            (0x0E0, 0x180),
            (0x100, 0x190),
            (0x120, 0x1A0),
        ] {
            rsrc[dir_rel + 14..dir_rel + 16].copy_from_slice(&1u16.to_le_bytes());
            write_dir_entry(
                &mut rsrc[dir_rel + 16..dir_rel + 24],
                false,
                1033,
                leaf_rel as u32,
            );
        }

        // Write payloads starting at rel 0x200 inside .rsrc.
        let mut payload_cursor = 0x200usize;
        let mut push_payload = |rsrc: &mut [u8], leaf_rel: usize, bytes: &[u8]| {
            let end = payload_cursor + bytes.len();
            rsrc[payload_cursor..end].copy_from_slice(bytes);
            rsrc[leaf_rel..leaf_rel + 4]
                .copy_from_slice(&(rsrc_rva + payload_cursor as u32).to_le_bytes());
            rsrc[leaf_rel + 4..leaf_rel + 8].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            payload_cursor = (end + 3) & !3;
        };

        let mut grp0 = vec![0u8; 6 + 2 * 14];
        grp0[2..6].copy_from_slice(&[1, 0, 2, 0]);
        write_grp_icon_entry(&mut grp0[6..20], 16, png16.len() as u32, 1);
        write_grp_icon_entry(&mut grp0[20..34], 0, png256.len() as u32, 2);

        let mut grp1 = vec![0u8; 6 + 14];
        grp1[2..6].copy_from_slice(&[1, 0, 1, 0]);
        write_grp_icon_entry(&mut grp1[6..20], 48, png48.len() as u32, 3);

        push_payload(&mut rsrc, 0x160, &grp0);
        push_payload(&mut rsrc, 0x170, &grp1);
        push_payload(&mut rsrc, 0x180, &png16);
        push_payload(&mut rsrc, 0x190, &png256);
        push_payload(&mut rsrc, 0x1A0, &png48);

        let pe_off = 0x80usize;
        let opt_size = 240u16;
        let section_off = pe_off + 24 + usize::from(opt_size);
        let rsrc_len = payload_cursor;
        let mut pe = vec![0u8; rsrc_file as usize + rsrc_len];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
        pe[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
        pe[pe_off + 6..pe_off + 8].copy_from_slice(&1u16.to_le_bytes());
        pe[pe_off + 20..pe_off + 22].copy_from_slice(&opt_size.to_le_bytes());

        let opt_off = pe_off + 24;
        pe[opt_off..opt_off + 2].copy_from_slice(&0x020Bu16.to_le_bytes());
        pe[opt_off + 108..opt_off + 112].copy_from_slice(&16u32.to_le_bytes());
        pe[opt_off + 128..opt_off + 132].copy_from_slice(&rsrc_rva.to_le_bytes());
        pe[opt_off + 132..opt_off + 136].copy_from_slice(&(rsrc_len as u32).to_le_bytes());

        pe[section_off..section_off + 5].copy_from_slice(b".rsrc");
        pe[section_off + 8..section_off + 12].copy_from_slice(&(rsrc_len as u32).to_le_bytes());
        pe[section_off + 12..section_off + 16].copy_from_slice(&rsrc_rva.to_le_bytes());
        pe[section_off + 16..section_off + 20].copy_from_slice(&(rsrc_len as u32).to_le_bytes());
        pe[section_off + 20..section_off + 24].copy_from_slice(&rsrc_file.to_le_bytes());
        pe[rsrc_file as usize..].copy_from_slice(&rsrc[..rsrc_len]);

        let img0 = extract_pe_icon_from_reader(&mut Cursor::new(&pe), 0).unwrap();
        assert_eq!((img0.width(), img0.height()), (256, 256));
        let img1 = extract_pe_icon_from_reader(&mut Cursor::new(&pe), 1).unwrap();
        assert_eq!((img1.width(), img1.height()), (48, 48));
        let img_neg = extract_pe_icon_from_reader(&mut Cursor::new(&pe), -101).unwrap();
        assert_eq!((img_neg.width(), img_neg.height()), (48, 48));
    }

    #[test]
    fn trim_jumbo_corner_padding_crops_small_corner_icon_but_preserves_full_canvas() {
        let mut corner = image::RgbaImage::from_pixel(256, 256, image::Rgba([0, 0, 0, 0]));
        for y in 0..32 {
            for x in 0..32 {
                corner.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
            }
        }
        let cropped = trim_jumbo_corner_padding(corner);
        assert_eq!((cropped.width(), cropped.height()), (32, 32));

        let mut full = image::RgbaImage::from_pixel(256, 256, image::Rgba([0, 0, 0, 0]));
        for y in 16..240 {
            for x in 16..240 {
                full.put_pixel(x, y, image::Rgba([0, 255, 0, 255]));
            }
        }
        let kept = trim_jumbo_corner_padding(full);
        assert_eq!((kept.width(), kept.height()), (256, 256));
    }
}
