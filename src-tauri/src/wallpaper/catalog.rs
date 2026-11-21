use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::schema::{Language, ThemeId, WallpaperKind, WallpaperMood};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct WallpaperItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub kind: WallpaperKind,
    pub moods: Vec<WallpaperMood>,
    pub recommended_theme: ThemeId,
    pub accent_hex: String,
    pub estimated_luma: f32,
    pub thumb_path: String,
    pub is_custom: bool,
    pub is_cached: bool,
    pub resolution_label: String,
}

#[derive(Debug, Clone, Copy)]
pub struct BuiltinShaderSpec {
    pub id: &'static str,
    pub title_da: &'static str,
    pub title_en: &'static str,
    pub sub_da: &'static str,
    pub sub_en: &'static str,
    pub moods: &'static [WallpaperMood],
    pub recommended_theme: ThemeId,
    pub accent_hex: &'static str,
    pub estimated_luma: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct CuratedPhotoSpec {
    pub id: &'static str,
    pub title_da: &'static str,
    pub title_en: &'static str,
    pub sub_da: &'static str,
    pub sub_en: &'static str,
    pub base_url: &'static str,
    pub moods: &'static [WallpaperMood],
    pub recommended_theme: ThemeId,
    pub accent_hex: &'static str,
    pub estimated_luma: f32,
}

pub const BUILTIN_SHADERS: &[BuiltinShaderSpec] = &[
    BuiltinShaderSpec {
        id: "scene-nordic-aurora",
        title_da: "Nordlys over Fjord",
        title_en: "Nordic Aurora Fjord",
        sub_da: "Volumetrisk arktisk nordlys over spejlblank fjord",
        sub_en: "Volumetric arctic aurora curtains over still fjord waters",
        moods: &[WallpaperMood::Dark, WallpaperMood::Nature],
        recommended_theme: ThemeId::Aurora,
        accent_hex: "#7FE3C1",
        estimated_luma: 0.14,
    },
    BuiltinShaderSpec {
        id: "scene-obsidian-silk",
        title_da: "Obsidian Silke",
        title_en: "Obsidian Silk",
        sub_da: "Flydende mørke silkebølger med bløde sølvkanter",
        sub_en: "Liquid dark silk folds with subtle silver rim highlights",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#B8C7D9",
        estimated_luma: 0.10,
    },
    BuiltinShaderSpec {
        id: "scene-solar-horizon",
        title_da: "Gylden Horisont",
        title_en: "Amber Horizon",
        sub_da: "Dyb skumring med varm rav-glød i horisonten",
        sub_en: "Deep twilight sky melting into warm amber horizon haze",
        moods: &[WallpaperMood::Warm, WallpaperMood::Minimal],
        recommended_theme: ThemeId::Aurora,
        accent_hex: "#F6B733",
        estimated_luma: 0.24,
    },
    BuiltinShaderSpec {
        id: "scene-alpine-mist",
        title_da: "Alpetåge",
        title_en: "Alpine Mist",
        sub_da: "Lagdelte bjergkæder i blå time med langsom daltåge",
        sub_en: "Layered mountain silhouettes in blue hour with drifting valley mist",
        moods: &[WallpaperMood::Dark, WallpaperMood::Nature],
        recommended_theme: ThemeId::Aurora,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.16,
    },
    BuiltinShaderSpec {
        id: "scene-monolith-mesh",
        title_da: "Monolit",
        title_en: "Monolith Mesh",
        sub_da: "Arkitektonisk højdekort i mat grafit med rolige bølger",
        sub_en: "Architectural matte-graphite heightfield with crisp contour lines",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal, WallpaperMood::Architecture],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#9AA2AE",
        estimated_luma: 0.12,
    },
    BuiltinShaderSpec {
        id: "scene-deep-ocean",
        title_da: "Dybhavslys",
        title_en: "Abyssal Light",
        sub_da: "Stille petrol-dybhav med svævende lysstråler",
        sub_en: "Calm petrol ocean depths with gently drifting sun shafts",
        moods: &[WallpaperMood::Dark, WallpaperMood::Nature],
        recommended_theme: ThemeId::Aurora,
        accent_hex: "#5EC8C2",
        estimated_luma: 0.15,
    },
    BuiltinShaderSpec {
        id: "scene-amber-dunes",
        title_da: "Ravklitter",
        title_en: "Amber Dunes",
        sub_da: "Skulpturelle sandklitter badet i varmt morgenlys",
        sub_en: "Sculpted desert sand dunes in warm directional morning light",
        moods: &[WallpaperMood::Warm, WallpaperMood::Minimal, WallpaperMood::Light],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#E3A14F",
        estimated_luma: 0.58,
    },
    BuiltinShaderSpec {
        id: "scene-quiet-field",
        title_da: "Quiet Field",
        title_en: "Quiet Field",
        sub_da: "Mørkeblå bund med langsomt drivende farvefelter",
        sub_en: "Near-black base with slowly drifting soft colour fields",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#5E8FA8",
        estimated_luma: 0.09,
    },
];

pub const CURATED_PHOTOS: &[CuratedPhotoSpec] = &[
    CuratedPhotoSpec {
        id: "photo-obsidian-waves",
        title_da: "Stille Bølger",
        title_en: "Calm Ocean Swell",
        sub_da: "Dyb nordisk havoverflade i dæmpet lys",
        sub_en: "Minimal dark ocean surface under muted light",
        base_url: "https://images.unsplash.com/photo-1518837695005-2083093ee35b",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal, WallpaperMood::Nature],
        recommended_theme: ThemeId::Aurora,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.28,
    },

    CuratedPhotoSpec {
        id: "photo-golden-dunes",
        title_da: "Gyldne Klitter",
        title_en: "Golden Dunes",
        sub_da: "Minimalistisk ørkenhorisont med varme toner",
        sub_en: "Clean minimalist sand dunes with warm tones",
        base_url: "https://images.unsplash.com/photo-1542401886-65d6c61db217",
        moods: &[WallpaperMood::Warm, WallpaperMood::Minimal, WallpaperMood::Light],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#F6B733",
        estimated_luma: 0.68,
    },

    CuratedPhotoSpec {
        id: "photo-deep-space",
        title_da: "Nattehimmel (OLED)",
        title_en: "Deep Night Sky (OLED)",
        sub_da: "Ægte dybsort stjernehimmel til OLED-skærme",
        sub_en: "Pitch-dark starry sky tailored for OLED panels",
        base_url: "https://images.unsplash.com/photo-1475274047050-1d0c0975c63e",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#FFFFFF",
        estimated_luma: 0.05,
    },
    CuratedPhotoSpec {
        id: "photo-glass-towers",
        title_da: "Glas & Stål",
        title_en: "Monolith Towers",
        sub_da: "Stram geometrisk arkitektur i køligt stållys",
        sub_en: "Crisp architectural geometry in cool steel light",
        base_url: "https://images.unsplash.com/photo-1486406146926-c627a92ad1ab",
        moods: &[WallpaperMood::Dark, WallpaperMood::Architecture],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.32,
    },
    CuratedPhotoSpec {
        id: "photo-ceramic-geo",
        title_da: "Hvid Arkitektur",
        title_en: "Ceramic Geometry",
        sub_da: "Rene lyse facader og bløde dagslysskygger",
        sub_en: "Clean white facades with soft architectural shadows",
        base_url: "https://images.unsplash.com/photo-1591192536912-24ebd86c82d3",
        moods: &[WallpaperMood::Light, WallpaperMood::Minimal, WallpaperMood::Architecture],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#F6B733",
        estimated_luma: 0.78,
    },
    CuratedPhotoSpec {
        id: "photo-alpine-valley",
        title_da: "Alpedal",
        title_en: "Alpine Valley",
        sub_da: "Skarpe bjergsider og krystalklar højlandsluft",
        sub_en: "Sharp mountain ridges and crystal-clear alpine air",
        base_url: "https://images.unsplash.com/photo-1464822759023-fed622ff2c3b",
        moods: &[WallpaperMood::Nature, WallpaperMood::Light],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.42,
    },
    CuratedPhotoSpec {
        id: "photo-petrol-coast",
        title_da: "Kystlinje fra Luften",
        title_en: "Aerial Coastline",
        sub_da: "Blågrøn brænding mod lys sandstrand ovenfra",
        sub_en: "Petrol waves meeting pale coastal sand from above",
        base_url: "https://images.unsplash.com/photo-1505118380757-91f5f5632de0",
        moods: &[WallpaperMood::Minimal, WallpaperMood::Nature],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#7FE3C1",
        estimated_luma: 0.48,
    },
    CuratedPhotoSpec {
        id: "photo-ice-peak",
        title_da: "Istinde",
        title_en: "Glacier Summit",
        sub_da: "Klar højfjeldssne mod kølig lyseblå himmel",
        sub_en: "Crisp snow peak beneath a bright alpine sky",
        base_url: "https://images.unsplash.com/photo-1454496522488-7a8e488e8606",
        moods: &[WallpaperMood::Nature, WallpaperMood::Light],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.72,
    },
    CuratedPhotoSpec {
        id: "photo-arch-curve",
        title_da: "Røde Buers",
        title_en: "Curved Facade",
        sub_da: "Bølgede terracotta-facader i hårdt sollys",
        sub_en: "Undulating terracotta facades in hard sunlight",
        base_url: "https://images.unsplash.com/photo-1486718448742-163732cd1544",
        moods: &[WallpaperMood::Architecture, WallpaperMood::Warm],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#E2603C",
        estimated_luma: 0.45,
    },
    CuratedPhotoSpec {
        id: "photo-city-dusk",
        title_da: "Manhattan i Skumring",
        title_en: "Manhattan Dusk",
        sub_da: "Renslig skyline over stille havne-vand",
        sub_en: "Clean skyline over still harbour water",
        base_url: "https://images.unsplash.com/photo-1514565131-fce0801e5785",
        moods: &[WallpaperMood::Architecture, WallpaperMood::Dark],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.38,
    },
    CuratedPhotoSpec {
        id: "photo-earth-night",
        title_da: "Jorden om Natten",
        title_en: "Earth at Night",
        sub_da: "Byens lys set fra rummet på dybsort baggrund",
        sub_en: "City lights seen from orbit on a pitch-black canvas",
        base_url: "https://images.unsplash.com/photo-1451187580459-43490279c0fa",
        moods: &[WallpaperMood::Dark, WallpaperMood::Minimal],
        recommended_theme: ThemeId::Grafit,
        accent_hex: "#F6B733",
        estimated_luma: 0.08,
    },
    CuratedPhotoSpec {
        id: "photo-blue-gradient",
        title_da: "Blå Gradient",
        title_en: "Blue Gradient",
        sub_da: "Ren kølig gradient uden støj eller motiver",
        sub_en: "Clean cool gradient without noise or motifs",
        base_url: "https://images.unsplash.com/photo-1557683316-973673baf926",
        moods: &[WallpaperMood::Minimal, WallpaperMood::Light],
        recommended_theme: ThemeId::Laerk,
        accent_hex: "#8CCBFF",
        estimated_luma: 0.50,
    },
];

pub fn find_shader(id: &str) -> Option<&'static BuiltinShaderSpec> {
    BUILTIN_SHADERS.iter().find(|s| s.id == id)
}

pub fn find_photo(id: &str) -> Option<&'static CuratedPhotoSpec> {
    CURATED_PHOTOS.iter().find(|p| p.id == id)
}

pub fn photo_thumb_url(spec: &CuratedPhotoSpec) -> String {
    format!("{}?auto=format&fit=crop&w=640&h=360&q=85", spec.base_url)
}

pub fn photo_full_url(spec: &CuratedPhotoSpec) -> String {
    format!("{}?auto=format&w=3840&q=95", spec.base_url)
}

pub fn localized_shader_text(spec: &BuiltinShaderSpec, lang: Language) -> (&'static str, &'static str) {
    match lang {
        Language::Da => (spec.title_da, spec.sub_da),
        Language::En => (spec.title_en, spec.sub_en),
    }
}

pub fn localized_photo_text(spec: &CuratedPhotoSpec, lang: Language) -> (&'static str, &'static str) {
    match lang {
        Language::Da => (spec.title_da, spec.sub_da),
        Language::En => (spec.title_en, spec.sub_en),
    }
}
