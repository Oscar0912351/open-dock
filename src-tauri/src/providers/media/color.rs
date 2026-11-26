use image::{imageops::FilterType, RgbImage};

pub fn extract_ambient_color(rgb: &RgbImage) -> Option<[u8; 3]> {
    let small = image::imageops::resize(rgb, 32, 32, FilterType::Triangle);
    let mut r_sum: f64 = 0.0;
    let mut g_sum: f64 = 0.0;
    let mut b_sum: f64 = 0.0;
    let mut count: u32 = 0;

    for px in small.pixels() {
        let (_, s, l) = rgb_to_hsl(px[0], px[1], px[2]);
        if s > 0.25 && (0.2..=0.85).contains(&l) {
            r_sum += px[0] as f64;
            g_sum += px[1] as f64;
            b_sum += px[2] as f64;
            count += 1;
        }
    }

    if count == 0 {
        return None;
    }

    let avg_r = (r_sum / count as f64).round().clamp(0.0, 255.0) as u8;
    let avg_g = (g_sum / count as f64).round().clamp(0.0, 255.0) as u8;
    let avg_b = (b_sum / count as f64).round().clamp(0.0, 255.0) as u8;

    let (h, s, _) = rgb_to_hsl(avg_r, avg_g, avg_b);
    let clamped_s = s.min(0.55);
    Some(hsl_to_rgb(h, clamped_s, 0.45))
}

fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) * 0.5;
    let d = max - min;
    if d < 1e-5 {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let mut h = if (max - rf).abs() < 1e-5 {
        (gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }
    } else if (max - gf).abs() < 1e-5 {
        (bf - rf) / d + 2.0
    } else {
        (rf - gf) / d + 4.0
    };
    h /= 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    if s < 1e-5 {
        let v = (l * 255.0).round().clamp(0.0, 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let b = hue_to_rgb(p, q, h - 1.0 / 3.0);
    [
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}
