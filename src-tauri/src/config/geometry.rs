use std::sync::LazyLock;

use serde::Deserialize;

use super::schema::{RadiusPreset, WidgetSize};

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SizeSpec {
    width: f64,
    height: f64,
    #[allow(dead_code)]
    padding_x: f64,
    #[allow(dead_code)]
    padding_y: f64,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct SizeTable {
    pill: SizeSpec,
    s: SizeSpec,
    m: SizeSpec,
    l: SizeSpec,
}

impl SizeTable {
    fn get(&self, size: WidgetSize) -> SizeSpec {
        match size {
            WidgetSize::Pill => self.pill,
            WidgetSize::S => self.s,
            WidgetSize::M => self.m,
            WidgetSize::L => self.l,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct RadiusTable {
    soft: SizeRadii,
    round: SizeRadii,
    extra: SizeRadii,
}

impl RadiusTable {
    fn get(&self, radius: RadiusPreset) -> SizeRadii {
        match radius {
            RadiusPreset::Soft => self.soft,
            RadiusPreset::Round => self.round,
            RadiusPreset::Extra => self.extra,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct SizeRadii {
    pill: f32,
    s: f32,
    m: f32,
    l: f32,
}

impl SizeRadii {
    fn get(&self, size: WidgetSize) -> f32 {
        match size {
            WidgetSize::Pill => self.pill,
            WidgetSize::S => self.s,
            WidgetSize::M => self.m,
            WidgetSize::L => self.l,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct ClusterRadii {
    soft: f32,
    round: f32,
    extra: f32,
}

impl ClusterRadii {
    fn get(&self, radius: RadiusPreset) -> f32 {
        match radius {
            RadiusPreset::Soft => self.soft,
            RadiusPreset::Round => self.round,
            RadiusPreset::Extra => self.extra,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetGeometry {
    sizes: SizeTable,
    radii: RadiusTable,
    cluster_radii: ClusterRadii,
    laerk_radius_multiplier: f32,
    squircle_multiplier: f32,
}

impl Default for WidgetGeometry {
    fn default() -> Self {
        Self {
            sizes: SizeTable {
                pill: SizeSpec { width: 204.0, height: 48.0, padding_x: 12.0, padding_y: 8.0 },
                s: SizeSpec { width: 132.0, height: 132.0, padding_x: 14.0, padding_y: 14.0 },
                m: SizeSpec { width: 276.0, height: 132.0, padding_x: 14.0, padding_y: 14.0 },
                l: SizeSpec { width: 276.0, height: 276.0, padding_x: 16.0, padding_y: 16.0 },
            },
            radii: RadiusTable {
                soft: SizeRadii { pill: 8.0, s: 10.0, m: 10.0, l: 12.0 },
                round: SizeRadii { pill: 14.0, s: 22.0, m: 22.0, l: 26.0 },
                extra: SizeRadii { pill: 24.0, s: 36.0, m: 36.0, l: 42.0 },
            },
            cluster_radii: ClusterRadii { soft: 10.0, round: 22.0, extra: 34.0 },
            laerk_radius_multiplier: 0.72,
            squircle_multiplier: 1.5,
        }
    }
}

static GEOMETRY: LazyLock<WidgetGeometry> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../src/shared/widgetGeometry.json"))
        .unwrap_or_default()
});

pub fn widget_base_size(size: WidgetSize) -> (f64, f64) {
    let spec = GEOMETRY.sizes.get(size);
    (spec.width, spec.height)
}

pub fn widget_radius_base(radius: RadiusPreset, size: WidgetSize) -> f32 {
    GEOMETRY.radii.get(radius).get(size)
}

pub fn cluster_radius_base(radius: RadiusPreset) -> f32 {
    GEOMETRY.cluster_radii.get(radius)
}

pub fn laerk_radius_multiplier() -> f32 {
    GEOMETRY.laerk_radius_multiplier
}

pub fn squircle_multiplier() -> f32 {
    GEOMETRY.squircle_multiplier
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_widget_geometry_matches_authoritative_values() {
        assert_eq!(widget_base_size(WidgetSize::Pill), (204.0, 48.0));
        assert_eq!(widget_base_size(WidgetSize::S), (132.0, 132.0));
        assert_eq!(widget_base_size(WidgetSize::M), (276.0, 132.0));
        assert_eq!(widget_base_size(WidgetSize::L), (276.0, 276.0));
        assert_eq!(widget_radius_base(RadiusPreset::Soft, WidgetSize::Pill), 8.0);
        assert_eq!(widget_radius_base(RadiusPreset::Soft, WidgetSize::S), 10.0);
        assert_eq!(widget_radius_base(RadiusPreset::Soft, WidgetSize::M), 10.0);
        assert_eq!(widget_radius_base(RadiusPreset::Soft, WidgetSize::L), 12.0);
        assert_eq!(widget_radius_base(RadiusPreset::Round, WidgetSize::Pill), 14.0);
        assert_eq!(widget_radius_base(RadiusPreset::Round, WidgetSize::S), 22.0);
        assert_eq!(widget_radius_base(RadiusPreset::Round, WidgetSize::M), 22.0);
        assert_eq!(widget_radius_base(RadiusPreset::Round, WidgetSize::L), 26.0);
        assert_eq!(widget_radius_base(RadiusPreset::Extra, WidgetSize::Pill), 24.0);
        assert_eq!(widget_radius_base(RadiusPreset::Extra, WidgetSize::S), 36.0);
        assert_eq!(widget_radius_base(RadiusPreset::Extra, WidgetSize::M), 36.0);
        assert_eq!(widget_radius_base(RadiusPreset::Extra, WidgetSize::L), 42.0);
        assert_eq!(cluster_radius_base(RadiusPreset::Soft), 10.0);
        assert_eq!(cluster_radius_base(RadiusPreset::Round), 22.0);
        assert_eq!(cluster_radius_base(RadiusPreset::Extra), 34.0);
        assert_eq!(laerk_radius_multiplier(), 0.72);
        assert_eq!(squircle_multiplier(), 1.5);
    }
}
