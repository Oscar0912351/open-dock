use std::{
    collections::HashSet,
    fs::{self, File},
    io::BufWriter,
    path::{Path, PathBuf},
};

use image::{
    codecs::jpeg::JpegEncoder,
    imageops::{crop_imm, overlay, resize, FilterType},
    DynamicImage, Rgb, RgbImage,
};

use crate::{config::schema::RectF, error::AppError};

pub const LUMA_COLS: usize = 64;
pub const LUMA_ROWS: usize = 36;
const PLATE_JPEG_QUALITY: u8 = 82;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WallpaperPosition {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
    Span,
}

impl WallpaperPosition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Fit => "fit",
            Self::Stretch => "stretch",
            Self::Center => "center",
            Self::Tile => "tile",
            Self::Span => "span",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlateSource {
    File(PathBuf),
    Solid([u8; 3]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RectPx {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropF {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateOp {
    /// Crop `(src)` from the source image and resize to `(dst)` on the output canvas.
    Blit { src: IntRect, dst: IntRect },
    /// Repeat source image tiles in monitor-pixel space onto the output canvas.
    Tile {
        tile_w: u32,
        tile_h: u32,
        monitor_w: u32,
        monitor_h: u32,
    },
}

#[derive(Debug, Clone)]
pub struct ComposeSpec {
    pub monitor: RectPx,
    pub virtual_bounds: RectPx,
    pub position: WallpaperPosition,
    pub bg: [u8; 3],
    pub output_width: u32,
    pub output_height: u32,
}

#[derive(Debug, Clone)]
pub struct LumaGrid {
    pub cols: usize,
    pub rows: usize,
    pub values: Vec<f32>,
}

impl Default for LumaGrid {
    fn default() -> Self {
        Self::uniform(0.15)
    }
}

impl LumaGrid {
    pub fn uniform(value: f32) -> Self {
        let clamped = value.clamp(0.0, 1.0);
        Self {
            cols: LUMA_COLS,
            rows: LUMA_ROWS,
            values: vec![clamped; LUMA_COLS * LUMA_ROWS],
        }
    }

    pub fn from_rgb_image(img: &RgbImage) -> Self {
        let width = img.width().max(1);
        let height = img.height().max(1);
        let mut values = Vec::with_capacity(LUMA_COLS * LUMA_ROWS);

        for row in 0..LUMA_ROWS {
            let y0 = (row as u64 * height as u64 / LUMA_ROWS as u64) as u32;
            let y1 = (((row + 1) as u64 * height as u64) / LUMA_ROWS as u64)
                .max(y0 as u64 + 1)
                .min(height as u64) as u32;

            for col in 0..LUMA_COLS {
                let x0 = (col as u64 * width as u64 / LUMA_COLS as u64) as u32;
                let x1 = (((col + 1) as u64 * width as u64) / LUMA_COLS as u64)
                    .max(x0 as u64 + 1)
                    .min(width as u64) as u32;

                let mut sum = 0.0_f64;
                let mut count = 0_u64;
                for py in y0..y1 {
                    for px in x0..x1 {
                        let p = img.get_pixel(px, py);
                        sum += srgb_to_luma(p[0], p[1], p[2]) as f64;
                        count += 1;
                    }
                }
                let avg = if count > 0 {
                    (sum / count as f64) as f32
                } else {
                    0.0
                };
                values.push(avg.clamp(0.0, 1.0));
            }
        }

        Self {
            cols: LUMA_COLS,
            rows: LUMA_ROWS,
            values,
        }
    }

    pub fn average_for_rect(&self, monitor_logical_w: f64, monitor_logical_h: f64, rect: RectF) -> f32 {
        if self.values.is_empty() || monitor_logical_w <= 0.0 || monitor_logical_h <= 0.0 {
            return 0.15;
        }
        let x0 = (rect.x / monitor_logical_w * self.cols as f64).floor() as isize;
        let x1 = ((rect.x + rect.width) / monitor_logical_w * self.cols as f64).ceil() as isize;
        let y0 = (rect.y / monitor_logical_h * self.rows as f64).floor() as isize;
        let y1 = ((rect.y + rect.height) / monitor_logical_h * self.rows as f64).ceil() as isize;

        let cx0 = x0.clamp(0, self.cols as isize - 1) as usize;
        let cx1 = x1.clamp(cx0 as isize + 1, self.cols as isize) as usize;
        let cy0 = y0.clamp(0, self.rows as isize - 1) as usize;
        let cy1 = y1.clamp(cy0 as isize + 1, self.rows as isize) as usize;

        let mut sum = 0.0_f64;
        let mut count = 0_usize;
        for row in cy0..cy1 {
            for col in cx0..cx1 {
                sum += self.values[row * self.cols + col] as f64;
                count += 1;
            }
        }
        if count > 0 {
            (sum / count as f64).clamp(0.0, 1.0) as f32
        } else {
            0.15
        }
    }

    pub fn average(&self) -> f32 {
        if self.values.is_empty() {
            return 0.15;
        }
        let sum: f64 = self.values.iter().map(|&v| v as f64).sum();
        (sum / self.values.len() as f64).clamp(0.0, 1.0) as f32
    }
}

pub fn srgb_to_luma(r: u8, g: u8, b: u8) -> f32 {
    fn linearize(channel: u8) -> f32 {
        let c = channel as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    (0.2126 * linearize(r) + 0.7152 * linearize(g) + 0.0722 * linearize(b)).clamp(0.0, 1.0)
}

pub fn output_dimensions(monitor_w: u32, monitor_h: u32) -> (u32, u32) {
    let mw = monitor_w.max(1);
    let mh = monitor_h.max(1);
    let target_w = ((mw as f64 * 0.5).round() as u32).max(960).min(mw);
    let target_h = ((target_w as f64 * mh as f64 / mw as f64).round() as u32).max(1);
    (target_w, target_h)
}

pub fn compute_fill_crop(img_w: u32, img_h: u32, target_w: u32, target_h: u32) -> CropF {
    let iw = img_w.max(1) as f64;
    let ih = img_h.max(1) as f64;
    let tw = target_w.max(1) as f64;
    let th = target_h.max(1) as f64;
    let scale = (tw / iw).max(th / ih);
    let src_w = (tw / scale).min(iw);
    let src_h = (th / scale).min(ih);
    let src_x = ((iw - src_w) * 0.5).max(0.0);
    let src_y = ((ih - src_h) * 0.5).max(0.0);
    CropF {
        x: src_x,
        y: src_y,
        width: src_w,
        height: src_h,
    }
}

pub fn compute_span_crop(
    img_w: u32,
    img_h: u32,
    virtual_bounds: RectPx,
    monitor: RectPx,
) -> CropF {
    let iw = img_w.max(1) as f64;
    let ih = img_h.max(1) as f64;
    let vw = virtual_bounds.width.max(1) as f64;
    let vh = virtual_bounds.height.max(1) as f64;
    let scale = (vw / iw).max(vh / ih);

    let virt_src_w = vw / scale;
    let virt_src_h = vh / scale;
    let virt_src_x = (iw - virt_src_w) * 0.5;
    let virt_src_y = (ih - virt_src_h) * 0.5;

    let ox = (monitor.x - virtual_bounds.x) as f64;
    let oy = (monitor.y - virtual_bounds.y) as f64;
    let src_x = (virt_src_x + ox / scale).clamp(0.0, iw - 1.0);
    let src_y = (virt_src_y + oy / scale).clamp(0.0, ih - 1.0);
    let src_w = (monitor.width.max(1) as f64 / scale).min(iw - src_x).max(1.0);
    let src_h = (monitor.height.max(1) as f64 / scale).min(ih - src_y).max(1.0);

    CropF {
        x: src_x,
        y: src_y,
        width: src_w,
        height: src_h,
    }
}

pub fn plan_plate_op(img_w: u32, img_h: u32, spec: &ComposeSpec) -> Option<PlateOp> {
    let iw = img_w.max(1);
    let ih = img_h.max(1);
    let mw = spec.monitor.width.max(1);
    let mh = spec.monitor.height.max(1);
    let ow = spec.output_width.max(1);
    let oh = spec.output_height.max(1);
    let out_sx = ow as f64 / mw as f64;
    let out_sy = oh as f64 / mh as f64;

    match spec.position {
        WallpaperPosition::Fill => {
            let crop = compute_fill_crop(iw, ih, mw, mh);
            Some(PlateOp::Blit {
                src: crop_to_int_rect(crop, iw, ih),
                dst: IntRect { x: 0, y: 0, width: ow, height: oh },
            })
        }
        WallpaperPosition::Span => {
            let crop = compute_span_crop(iw, ih, spec.virtual_bounds, spec.monitor);
            Some(PlateOp::Blit {
                src: crop_to_int_rect(crop, iw, ih),
                dst: IntRect { x: 0, y: 0, width: ow, height: oh },
            })
        }
        WallpaperPosition::Stretch => Some(PlateOp::Blit {
            src: IntRect { x: 0, y: 0, width: iw, height: ih },
            dst: IntRect { x: 0, y: 0, width: ow, height: oh },
        }),
        WallpaperPosition::Fit => {
            let scale = (mw as f64 / iw as f64).min(mh as f64 / ih as f64);
            let fit_w = iw as f64 * scale;
            let fit_h = ih as f64 * scale;
            let mon_x = (mw as f64 - fit_w) * 0.5;
            let mon_y = (mh as f64 - fit_h) * 0.5;
            let dst_x = (mon_x * out_sx).round() as i32;
            let dst_y = (mon_y * out_sy).round() as i32;
            let dst_w = (fit_w * out_sx).round().max(1.0) as u32;
            let dst_h = (fit_h * out_sy).round().max(1.0) as u32;
            Some(PlateOp::Blit {
                src: IntRect { x: 0, y: 0, width: iw, height: ih },
                dst: IntRect {
                    x: dst_x.clamp(0, ow as i32 - 1),
                    y: dst_y.clamp(0, oh as i32 - 1),
                    width: dst_w.min(ow),
                    height: dst_h.min(oh),
                },
            })
        }
        WallpaperPosition::Center => {
            let mon_x = (mw as i32 - iw as i32) / 2;
            let mon_y = (mh as i32 - ih as i32) / 2;
            let vis_x0 = mon_x.max(0);
            let vis_y0 = mon_y.max(0);
            let vis_x1 = (mon_x + iw as i32).min(mw as i32);
            let vis_y1 = (mon_y + ih as i32).min(mh as i32);
            if vis_x1 <= vis_x0 || vis_y1 <= vis_y0 {
                return None;
            }
            let src_x = (vis_x0 - mon_x) as u32;
            let src_y = (vis_y0 - mon_y) as u32;
            let src_w = (vis_x1 - vis_x0) as u32;
            let src_h = (vis_y1 - vis_y0) as u32;

            let dst_x = (vis_x0 as f64 * out_sx).round() as i32;
            let dst_y = (vis_y0 as f64 * out_sy).round() as i32;
            let dst_w = (src_w as f64 * out_sx).round().max(1.0) as u32;
            let dst_h = (src_h as f64 * out_sy).round().max(1.0) as u32;
            Some(PlateOp::Blit {
                src: IntRect {
                    x: src_x as i32,
                    y: src_y as i32,
                    width: src_w,
                    height: src_h,
                },
                dst: IntRect {
                    x: dst_x.clamp(0, ow as i32 - 1),
                    y: dst_y.clamp(0, oh as i32 - 1),
                    width: dst_w.min(ow),
                    height: dst_h.min(oh),
                },
            })
        }
        WallpaperPosition::Tile => Some(PlateOp::Tile {
            tile_w: iw,
            tile_h: ih,
            monitor_w: mw,
            monitor_h: mh,
        }),
    }
}

fn crop_to_int_rect(crop: CropF, max_w: u32, max_h: u32) -> IntRect {
    let x = (crop.x.round() as i32).clamp(0, max_w.saturating_sub(1) as i32);
    let y = (crop.y.round() as i32).clamp(0, max_h.saturating_sub(1) as i32);
    let width = (crop.width.round() as u32).clamp(1, max_w - x as u32);
    let height = (crop.height.round() as u32).clamp(1, max_h - y as u32);
    IntRect { x, y, width, height }
}

pub fn compose_plate_image(source_img: Option<&DynamicImage>, spec: &ComposeSpec) -> RgbImage {
    let ow = spec.output_width.max(1);
    let oh = spec.output_height.max(1);
    let mut canvas = RgbImage::from_pixel(ow, oh, Rgb(spec.bg));

    let Some(img) = source_img else {
        return canvas;
    };
    let rgb = img.to_rgb8();
    let (iw, ih) = rgb.dimensions();
    if iw == 0 || ih == 0 {
        return canvas;
    }

    let Some(op) = plan_plate_op(iw, ih, spec) else {
        return canvas;
    };

    match op {
        PlateOp::Blit { src, dst } => {
            let cropped = crop_imm(
                &rgb,
                src.x.max(0) as u32,
                src.y.max(0) as u32,
                src.width,
                src.height,
            )
            .to_image();
            let scaled = resize(&cropped, dst.width.max(1), dst.height.max(1), FilterType::Triangle);
            overlay(&mut canvas, &scaled, dst.x as i64, dst.y as i64);
        }
        PlateOp::Tile {
            tile_w,
            tile_h,
            monitor_w,
            monitor_h,
        } => {
            let out_sx = ow as f64 / monitor_w.max(1) as f64;
            let out_sy = oh as f64 / monitor_h.max(1) as f64;
            let scaled_tile_w = ((tile_w as f64 * out_sx).round() as u32).max(1);
            let scaled_tile_h = ((tile_h as f64 * out_sy).round() as u32).max(1);
            let scaled_tile = resize(&rgb, scaled_tile_w, scaled_tile_h, FilterType::Triangle);

            let mut ty = 0_u32;
            while ty < monitor_h {
                let dst_y0 = (ty as f64 * out_sy).round() as u32;
                let next_ty = (ty + tile_h).min(monitor_h);
                let dst_y1 = ((next_ty as f64 * out_sy).round() as u32).min(oh);
                let copy_h = dst_y1.saturating_sub(dst_y0).min(scaled_tile_h);

                let mut tx = 0_u32;
                while tx < monitor_w {
                    let dst_x0 = (tx as f64 * out_sx).round() as u32;
                    let next_tx = (tx + tile_w).min(monitor_w);
                    let dst_x1 = ((next_tx as f64 * out_sx).round() as u32).min(ow);
                    let copy_w = dst_x1.saturating_sub(dst_x0).min(scaled_tile_w);

                    if copy_w > 0 && copy_h > 0 {
                        let patch = crop_imm(&scaled_tile, 0, 0, copy_w, copy_h).to_image();
                        overlay(&mut canvas, &patch, dst_x0 as i64, dst_y0 as i64);
                    }
                    tx += tile_w;
                }
                ty += tile_h;
            }
        }
    }

    canvas
}

pub fn plate_hash(
    source_tag: &str,
    mtime_nanos: u128,
    monitor: RectPx,
    virtual_bounds: RectPx,
    position: WallpaperPosition,
    bg: [u8; 3],
) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };

    feed(source_tag.as_bytes());
    feed(&mtime_nanos.to_le_bytes());
    feed(&monitor.x.to_le_bytes());
    feed(&monitor.y.to_le_bytes());
    feed(&monitor.width.to_le_bytes());
    feed(&monitor.height.to_le_bytes());
    feed(&virtual_bounds.x.to_le_bytes());
    feed(&virtual_bounds.y.to_le_bytes());
    feed(&virtual_bounds.width.to_le_bytes());
    feed(&virtual_bounds.height.to_le_bytes());
    feed(position.as_str().as_bytes());
    feed(&bg);

    format!("{h:016x}")
}

pub fn save_jpeg_82(img: &RgbImage, path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(AppError::ConfigWrite)?;
    }
    let tmp = path.with_extension("jpg.tmp");
    {
        let file = File::create(&tmp).map_err(AppError::ConfigWrite)?;
        let mut writer = BufWriter::new(file);
        let mut encoder = JpegEncoder::new_with_quality(&mut writer, PLATE_JPEG_QUALITY);
        encoder
            .encode_image(img)
            .map_err(|e| AppError::Other(format!("Kunne ikke kode plade-JPEG: {e}")))?;
    }
    fs::rename(&tmp, path).map_err(AppError::ConfigWrite)?;
    Ok(())
}

pub fn prune_old_plates(dir: &Path, keep_names: &HashSet<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("plate-") && name.ends_with(".jpg") && !keep_names.contains(name) {
            if let Err(error) = fs::remove_file(&path) {
                log::warn!("Kunne ikke slette gammel plade {name}: {error}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(pos: WallpaperPosition, mon: RectPx, virt: RectPx) -> ComposeSpec {
        let (ow, oh) = output_dimensions(mon.width, mon.height);
        ComposeSpec {
            monitor: mon,
            virtual_bounds: virt,
            position: pos,
            bg: [10, 20, 30],
            output_width: ow,
            output_height: oh,
        }
    }

    #[test]
    fn output_dimensions_follow_half_size_with_960_floor() {
        assert_eq!(output_dimensions(1920, 1080), (960, 540));
        assert_eq!(output_dimensions(2560, 1440), (1280, 720));
        assert_eq!(output_dimensions(3840, 2160), (1920, 1080));
        assert_eq!(output_dimensions(1280, 720), (960, 540));
        assert_eq!(output_dimensions(800, 600), (800, 600));
    }

    #[test]
    fn fill_crops_horizontally_or_vertically_centered() {
        let mon = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        // Square 1000x1000 image on 16:9 monitor -> crop vertical middle (1000 x 562.5)
        let op = plan_plate_op(1000, 1000, &spec(WallpaperPosition::Fill, mon, mon)).unwrap();
        match op {
            PlateOp::Blit { src, dst } => {
                assert_eq!(src.x, 0);
                assert_eq!(src.width, 1000);
                assert!((src.y as i32 - 219).abs() <= 1);
                assert!((src.height as i32 - 563).abs() <= 1);
                assert_eq!(dst, IntRect { x: 0, y: 0, width: 960, height: 540 });
            }
            _ => panic!("Expected Blit for Fill"),
        }
    }

    #[test]
    fn fit_letterboxes_with_centered_destination() {
        let mon = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        // 1000x1000 image on 1920x1080 -> fits height (540 in 960x540 output), x centered at (960-540)/2 = 210
        let op = plan_plate_op(1000, 1000, &spec(WallpaperPosition::Fit, mon, mon)).unwrap();
        match op {
            PlateOp::Blit { src, dst } => {
                assert_eq!(src, IntRect { x: 0, y: 0, width: 1000, height: 1000 });
                assert_eq!(dst, IntRect { x: 210, y: 0, width: 540, height: 540 });
            }
            _ => panic!("Expected Blit for Fit"),
        }
    }

    #[test]
    fn stretch_uses_full_source_and_full_output() {
        let mon = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        let op = plan_plate_op(800, 600, &spec(WallpaperPosition::Stretch, mon, mon)).unwrap();
        assert_eq!(
            op,
            PlateOp::Blit {
                src: IntRect { x: 0, y: 0, width: 800, height: 600 },
                dst: IntRect { x: 0, y: 0, width: 960, height: 540 },
            }
        );
    }

    #[test]
    fn center_keeps_native_size_and_crops_if_larger() {
        let mon = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        // Smaller 400x200 image -> centered at (760, 440) monitor coords -> (380, 220, 200, 100) output
        let small = plan_plate_op(400, 200, &spec(WallpaperPosition::Center, mon, mon)).unwrap();
        assert_eq!(
            small,
            PlateOp::Blit {
                src: IntRect { x: 0, y: 0, width: 400, height: 200 },
                dst: IntRect { x: 380, y: 220, width: 200, height: 100 },
            }
        );

        // Larger 2400x1400 image -> centered crop (240, 160, 1920, 1080) -> full output (0, 0, 960, 540)
        let large = plan_plate_op(2400, 1400, &spec(WallpaperPosition::Center, mon, mon)).unwrap();
        assert_eq!(
            large,
            PlateOp::Blit {
                src: IntRect { x: 240, y: 160, width: 1920, height: 1080 },
                dst: IntRect { x: 0, y: 0, width: 960, height: 540 },
            }
        );
    }

    #[test]
    fn span_splits_virtual_desktop_across_two_monitors() {
        let virt = RectPx { x: 0, y: 0, width: 3840, height: 1080 };
        let left = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        let right = RectPx { x: 1920, y: 0, width: 1920, height: 1080 };

        let op_left = plan_plate_op(3840, 2160, &spec(WallpaperPosition::Span, left, virt)).unwrap();
        let op_right = plan_plate_op(3840, 2160, &spec(WallpaperPosition::Span, right, virt)).unwrap();

        assert_eq!(
            op_left,
            PlateOp::Blit {
                src: IntRect { x: 0, y: 540, width: 1920, height: 1080 },
                dst: IntRect { x: 0, y: 0, width: 960, height: 540 },
            }
        );
        assert_eq!(
            op_right,
            PlateOp::Blit {
                src: IntRect { x: 1920, y: 540, width: 1920, height: 1080 },
                dst: IntRect { x: 0, y: 0, width: 960, height: 540 },
            }
        );
    }

    #[test]
    fn compose_tile_and_solid_and_luma_grid_work() {
        let mon = RectPx { x: 0, y: 0, width: 1920, height: 1080 };
        let s = spec(WallpaperPosition::Tile, mon, mon);

        // Two-tone 64x64 tile: top half black, bottom half white
        let mut tile = RgbImage::new(64, 64);
        for y in 0..64 {
            let color = if y < 32 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) };
            for x in 0..64 {
                tile.put_pixel(x, y, color);
            }
        }
        let dyn_img = DynamicImage::ImageRgb8(tile);
        let composed = compose_plate_image(Some(&dyn_img), &s);
        assert_eq!(composed.dimensions(), (960, 540));
        // Top-left pixel should be dark, 16px down (= 32px in monitor -> scaled 0.5 = 16px) should be bright
        assert!(composed.get_pixel(4, 4)[0] < 30);
        assert!(composed.get_pixel(4, 24)[0] > 220);

        // Solid fallback
        let solid = compose_plate_image(None, &s);
        assert_eq!(solid.get_pixel(100, 100), &Rgb([10, 20, 30]));

        // Luma grid on split image (left half black, right half white)
        let mut split = RgbImage::new(960, 540);
        for y in 0..540 {
            for x in 0..960 {
                split.put_pixel(x, y, if x < 480 { Rgb([0, 0, 0]) } else { Rgb([255, 255, 255]) });
            }
        }
        let grid = LumaGrid::from_rgb_image(&split);
        let dark_luma = grid.average_for_rect(1920.0, 1080.0, RectF { x: 40.0, y: 40.0, width: 200.0, height: 200.0 });
        let bright_luma = grid.average_for_rect(1920.0, 1080.0, RectF { x: 1400.0, y: 40.0, width: 200.0, height: 200.0 });
        assert!(dark_luma < 0.05);
        assert!(bright_luma > 0.95);
    }
}
