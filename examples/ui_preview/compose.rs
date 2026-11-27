use image::{Rgba, RgbaImage, imageops::FilterType};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStyle {
    Window,
    Card,
    Raw,
}

impl FrameStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Card => "card",
            Self::Raw => "raw",
        }
    }
}

fn rounded_box_sdf(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let qx = px.abs() - half_w + radius;
    let qy = py.abs() - half_h + radius;
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - radius
}

fn build_window_masks(w: u32, h: u32, scale: f32) -> (Vec<f32>, Vec<f32>) {
    let total = (w as usize) * (h as usize);
    let mut fill_mask = vec![0.0f32; total];
    let mut border_mask = vec![0.0f32; total];
    let half_w = w as f32 * 0.5;
    let half_h = h as f32 * 0.5;
    let radius = (8.0 * scale).min(half_w).min(half_h);
    let border_thickness = scale.max(1.0);

    for y in 0..h {
        let py = y as f32 + 0.5 - half_h;
        let row = (y as usize) * (w as usize);
        for x in 0..w {
            let px = x as f32 + 0.5 - half_w;
            let dist = rounded_box_sdf(px, py, half_w, half_h, radius);
            let outer = (0.5 - dist).clamp(0.0, 1.0);
            let inner = (0.5 - (dist + border_thickness)).clamp(0.0, 1.0);
            fill_mask[row + x as usize] = outer;
            border_mask[row + x as usize] = (outer - inner).max(0.0);
        }
    }
    (fill_mask, border_mask)
}

fn box_blur_horizontal(
    src: &[f32],
    dst: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    prefix: &mut [f32],
) {
    let denom = (2 * radius + 1) as f32;
    for y in 0..height {
        let row = y * width;
        prefix[0] = 0.0;
        for x in 0..width {
            prefix[x + 1] = prefix[x] + src[row + x];
        }
        for x in 0..width {
            let lo = x.saturating_sub(radius);
            let hi = (x + radius + 1).min(width);
            dst[row + x] = (prefix[hi] - prefix[lo]) / denom;
        }
    }
}

fn box_blur_vertical(
    src: &[f32],
    dst: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    prefix: &mut [f32],
) {
    let denom = (2 * radius + 1) as f32;
    for x in 0..width {
        prefix[0] = 0.0;
        for y in 0..height {
            prefix[y + 1] = prefix[y] + src[y * width + x];
        }
        for y in 0..height {
            let lo = y.saturating_sub(radius);
            let hi = (y + radius + 1).min(height);
            dst[y * width + x] = (prefix[hi] - prefix[lo]) / denom;
        }
    }
}

fn box_blur_3pass(buf: &mut [f32], width: usize, height: usize, radius: usize) {
    if radius == 0 || width == 0 || height == 0 {
        return;
    }
    let mut tmp = vec![0.0f32; buf.len()];
    let mut prefix = vec![0.0f32; width.max(height) + 1];
    for _ in 0..3 {
        box_blur_horizontal(buf, &mut tmp, width, height, radius, &mut prefix);
        box_blur_vertical(&tmp, buf, width, height, radius, &mut prefix);
    }
}

struct MaskGrid<'a> {
    data: &'a [f32],
    width: usize,
    height: usize,
}

fn stamp_mask(
    dst: &mut [f32],
    canvas_w: usize,
    canvas_h: usize,
    mask: MaskGrid<'_>,
    offset_x: usize,
    offset_y: usize,
) {
    for y in 0..mask.height {
        let cy = offset_y + y;
        if cy >= canvas_h {
            break;
        }
        let dst_row = cy * canvas_w + offset_x;
        let src_row = y * mask.width;
        let copy_w = mask.width.min(canvas_w.saturating_sub(offset_x));
        dst[dst_row..dst_row + copy_w].copy_from_slice(&mask.data[src_row..src_row + copy_w]);
    }
}

/// Frames a raw RGB window screenshot with optional Windows 11 rounded corners,
/// subtle hairline border, soft drop shadow, and optional presentation backdrop.
pub fn frame_screenshot(
    raw: &image::RgbImage,
    dark: bool,
    scale: f32,
    style: FrameStyle,
) -> RgbaImage {
    let w = raw.width();
    let h = raw.height();
    if style == FrameStyle::Raw {
        let mut out = RgbaImage::new(w, h);
        for (x, y, px) in raw.enumerate_pixels() {
            out.put_pixel(x, y, Rgba([px[0], px[1], px[2], 255]));
        }
        return out;
    }

    let scale = scale.max(0.5);
    let pad_x = (28.0 * scale).round() as usize;
    let pad_top = (20.0 * scale).round() as usize;
    let pad_bottom = (36.0 * scale).round() as usize;
    let out_w = w as usize + pad_x * 2;
    let out_h = h as usize + pad_top + pad_bottom;

    let (fill_mask, border_mask) = build_window_masks(w, h, scale);

    let ambient_dy = (2.0 * scale).round() as usize;
    let mut ambient = vec![0.0f32; out_w * out_h];
    stamp_mask(
        &mut ambient,
        out_w,
        out_h,
        MaskGrid {
            data: &fill_mask,
            width: w as usize,
            height: h as usize,
        },
        pad_x,
        pad_top + ambient_dy,
    );
    box_blur_3pass(
        &mut ambient,
        out_w,
        out_h,
        (4.0 * scale).round().max(1.0) as usize,
    );

    let key_dy = (8.0 * scale).round() as usize;
    let mut key_shadow = vec![0.0f32; out_w * out_h];
    stamp_mask(
        &mut key_shadow,
        out_w,
        out_h,
        MaskGrid {
            data: &fill_mask,
            width: w as usize,
            height: h as usize,
        },
        pad_x,
        pad_top + key_dy,
    );
    box_blur_3pass(
        &mut key_shadow,
        out_w,
        out_h,
        (16.0 * scale).round().max(2.0) as usize,
    );

    let (ambient_strength, key_strength) = if dark { (0.28, 0.44) } else { (0.14, 0.22) };
    let (border_rgb, border_alpha) = if dark {
        ((255.0f32, 255.0f32, 255.0f32), 34.0 / 255.0)
    } else {
        ((0.0f32, 0.0f32, 0.0f32), 28.0 / 255.0)
    };

    let mut out = RgbaImage::new(out_w as u32, out_h as u32);
    let raw_bytes = raw.as_raw();
    let w_usize = w as usize;
    let h_usize = h as usize;

    for cy in 0..out_h {
        let out_row = cy * out_w;
        for cx in 0..out_w {
            let idx = out_row + cx;
            // Combine ambient + key shadows into black shadow alpha.
            let sa =
                (ambient[idx] * ambient_strength + key_shadow[idx] * key_strength).clamp(0.0, 0.85);
            let mut pr = 0.0f32;
            let mut pg = 0.0f32;
            let mut pb = 0.0f32;
            let mut pa = sa;

            if cx >= pad_x && cx < pad_x + w_usize && cy >= pad_top && cy < pad_top + h_usize {
                let wx = cx - pad_x;
                let wy = cy - pad_top;
                let widx = wy * w_usize + wx;
                let wa = fill_mask[widx];
                if wa > 0.0 {
                    let p = widx * 3;
                    let r = f32::from(raw_bytes[p]);
                    let g = f32::from(raw_bytes[p + 1]);
                    let b = f32::from(raw_bytes[p + 2]);
                    let inv = 1.0 - wa;
                    pr = r * wa + pr * inv;
                    pg = g * wa + pg * inv;
                    pb = b * wa + pb * inv;
                    pa = wa + pa * inv;

                    let ba = border_mask[widx] * border_alpha;
                    if ba > 0.0 {
                        let inv_b = 1.0 - ba;
                        pr = border_rgb.0 * ba + pr * inv_b;
                        pg = border_rgb.1 * ba + pg * inv_b;
                        pb = border_rgb.2 * ba + pb * inv_b;
                        pa = ba + pa * inv_b;
                    }
                }
            }

            if pa > 0.0 {
                let inv_a = 1.0 / pa;
                out.put_pixel(
                    cx as u32,
                    cy as u32,
                    Rgba([
                        (pr * inv_a).round().clamp(0.0, 255.0) as u8,
                        (pg * inv_a).round().clamp(0.0, 255.0) as u8,
                        (pb * inv_a).round().clamp(0.0, 255.0) as u8,
                        (pa * 255.0).round().clamp(0.0, 255.0) as u8,
                    ]),
                );
            }
        }
    }

    if style == FrameStyle::Window {
        return out;
    }

    let margin = (24.0 * scale).round() as u32;
    let card_w = out.width() + margin * 2;
    let card_h = out.height() + margin * 2;
    let mut card = paint_backdrop(card_w, card_h, dark);
    alpha_blend_onto(&mut card, &out, margin, margin);
    card
}

fn paint_backdrop(width: u32, height: u32, dark: bool) -> RgbaImage {
    let mut canvas = RgbaImage::new(width, height);
    let (start, end) = if dark {
        ([17.0f32, 22.0, 32.0], [27.0f32, 34.0, 50.0])
    } else {
        ([238.0f32, 242.0, 247.0], [223.0f32, 231.0, 241.0])
    };
    let denom = (width.saturating_sub(1) + height.saturating_sub(1)).max(1) as f32;
    for y in 0..height {
        for x in 0..width {
            let t = (x + y) as f32 / denom;
            let r = (start[0] + (end[0] - start[0]) * t).round() as u8;
            let g = (start[1] + (end[1] - start[1]) * t).round() as u8;
            let b = (start[2] + (end[2] - start[2]) * t).round() as u8;
            canvas.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }
    canvas
}

fn alpha_blend_onto(dst: &mut RgbaImage, src: &RgbaImage, ox: u32, oy: u32) {
    let max_w = dst.width();
    let max_h = dst.height();
    for (x, y, sp) in src.enumerate_pixels() {
        let dx = ox + x;
        let dy = oy + y;
        if dx >= max_w || dy >= max_h || sp[3] == 0 {
            continue;
        }
        if sp[3] == 255 {
            dst.put_pixel(dx, dy, *sp);
            continue;
        }
        let dp = *dst.get_pixel(dx, dy);
        let sa = f32::from(sp[3]) / 255.0;
        let inv = 1.0 - sa;
        let r = (f32::from(sp[0]) * sa + f32::from(dp[0]) * inv).round() as u8;
        let g = (f32::from(sp[1]) * sa + f32::from(dp[1]) * inv).round() as u8;
        let b = (f32::from(sp[2]) * sa + f32::from(dp[2]) * inv).round() as u8;
        let a = ((sa + (f32::from(dp[3]) / 255.0) * inv) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
        dst.put_pixel(dx, dy, Rgba([r, g, b, a]));
    }
}

/// Composes up to 4 hero screenshots side-by-side onto a unified presentation canvas
/// while preserving each window's authentic relative proportions.
pub fn compose_overview(tiles: &[(&str, &RgbaImage)], dark: bool, scale: f32) -> Option<RgbaImage> {
    if tiles.is_empty() {
        return None;
    }
    let hero_order = [
        "main-grid",
        "main-search",
        "editor-program",
        "settings-general",
    ];
    let mut chosen: Vec<&RgbaImage> = Vec::with_capacity(4);
    for id in hero_order {
        if let Some((_, img)) = tiles.iter().find(|(tile_id, _)| *tile_id == id) {
            chosen.push(*img);
        }
    }
    for (_, img) in tiles {
        if chosen.len() >= 4 {
            break;
        }
        if !chosen.iter().any(|existing| std::ptr::eq(*existing, *img)) {
            chosen.push(*img);
        }
    }

    let natural_w: u32 = chosen.iter().map(|i| i.width()).sum();
    let max_tiles_w = 2400u32;
    let downscale = if natural_w > max_tiles_w {
        max_tiles_w as f32 / natural_w as f32
    } else {
        1.0
    };

    let scaled: Vec<RgbaImage> = chosen
        .into_iter()
        .map(|img| {
            if (downscale - 1.0).abs() < 0.01 {
                img.clone()
            } else {
                let nw = ((img.width() as f32 * downscale).round() as u32).max(1);
                let nh = ((img.height() as f32 * downscale).round() as u32).max(1);
                image::imageops::resize(img, nw, nh, FilterType::Lanczos3)
            }
        })
        .collect();

    let gap = (20.0 * scale * downscale).round().max(12.0) as u32;
    let pad_x = (36.0 * scale * downscale).round().max(24.0) as u32;
    let pad_y = (36.0 * scale * downscale).round().max(24.0) as u32;
    let row_w: u32 =
        scaled.iter().map(|i| i.width()).sum::<u32>() + gap * scaled.len().saturating_sub(1) as u32;
    let row_h: u32 = scaled.iter().map(|i| i.height()).max().unwrap_or(1);

    let mut canvas = paint_backdrop(row_w + pad_x * 2, row_h + pad_y * 2, dark);
    let mut cursor_x = pad_x;
    for img in &scaled {
        let y = pad_y + (row_h - img.height()) / 2;
        alpha_blend_onto(&mut canvas, img, cursor_x, y);
        cursor_x += img.width() + gap;
    }
    Some(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rgb(w: u32, h: u32) -> image::RgbImage {
        image::RgbImage::from_pixel(w, h, image::Rgb([245, 245, 248]))
    }

    #[test]
    fn raw_preserves_dimensions_and_opaque_alpha() {
        let src = sample_rgb(120, 80);
        let framed = frame_screenshot(&src, false, 1.0, FrameStyle::Raw);
        assert_eq!(framed.dimensions(), (120, 80));
        assert_eq!(framed.get_pixel(0, 0)[3], 255);
        assert_eq!(framed.get_pixel(60, 40)[0], 245);
    }

    #[test]
    fn window_rounds_corners_and_adds_shadow_padding() {
        let src = sample_rgb(160, 120);
        let framed = frame_screenshot(&src, false, 1.0, FrameStyle::Window);
        assert_eq!(framed.dimensions(), (160 + 56, 120 + 56));
        assert_eq!(framed.get_pixel(0, 0)[3], 0);
        // Inner window corner pixel sits inside shadow padding; its extreme corner is clipped.
        let corner = framed.get_pixel(28, 20);
        assert!(corner[3] < 200);
        // Center pixel of the window stays fully opaque.
        assert_eq!(framed.get_pixel(28 + 80, 20 + 60)[3], 255);
    }

    #[test]
    fn card_outputs_opaque_backdrop_pixels() {
        let src = sample_rgb(100, 80);
        let card = frame_screenshot(&src, true, 1.0, FrameStyle::Card);
        assert!(card.width() > 100 && card.height() > 80);
        assert_eq!(card.get_pixel(0, 0)[3], 255);
        assert_eq!(card.get_pixel(card.width() - 1, card.height() - 1)[3], 255);
    }

    #[test]
    fn box_blur_stays_bounded_and_spreads_edges() {
        let mut buf = vec![0.0f32; 32 * 32];
        for y in 10..22 {
            for x in 10..22 {
                buf[y * 32 + x] = 1.0;
            }
        }
        box_blur_3pass(&mut buf, 32, 32, 4);
        assert!(buf[8 * 32 + 16] > 0.0);
        assert!(buf.iter().all(|v| (0.0..=1.0).contains(v)));
    }
}
