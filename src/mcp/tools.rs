//! Strongly-typed wrappers and schema models for LightCraft 0.8.0 MCP tools.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ============================================================================
// Tool 1: import
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportMode {
    Add,
    Copy,
    Move,
}

impl Default for ImportMode {
    fn default() -> Self {
        ImportMode::Add
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnDeletedAction {
    Restore,
    Skip,
    Fresh,
}

/// Parameters for calling `import`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportParams {
    pub paths: Vec<String>,
    #[serde(default = "default_import_mode")]
    pub mode: ImportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destination: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_deleted: Option<OnDeletedAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organize: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename: Option<String>,
}

fn default_import_mode() -> ImportMode {
    ImportMode::Add
}

impl ImportParams {
    /// Creates a non-destructive import referencing files in place.
    pub fn new_add(paths: Vec<String>) -> Self {
        Self {
            paths,
            mode: ImportMode::Add,
            album: None,
            destination: None,
            on_deleted: None,
            organize: None,
            rename: None,
        }
    }
}

/// Raw result payload from LightCraft `import` tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRawResult {
    pub imported: Vec<i64>,
    pub files: usize,
    pub scanned: usize,
    #[serde(default)]
    pub sidecars: usize,
    #[serde(default)]
    pub duplicates: Vec<ImportDuplicate>,
    #[serde(default)]
    pub failed: Vec<ImportFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDuplicate {
    pub existing: i64,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFailure {
    pub path: String,
    pub error: String,
}

/// High-level strongly-typed response returned to application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportResponse {
    pub imported_count: usize,
    pub photos: Vec<i64>,
    pub duplicates: Vec<ImportDuplicate>,
    pub failed: Vec<ImportFailure>,
}

impl From<ImportRawResult> for ImportResponse {
    fn from(raw: ImportRawResult) -> Self {
        Self {
            imported_count: raw.imported.len(),
            photos: raw.imported,
            duplicates: raw.duplicates,
            failed: raw.failed,
        }
    }
}

// ============================================================================
// Tool 2: query_photos
// ============================================================================

/// Parameters for calling `query_photos`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QueryPhotosParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<CatalogFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<CatalogSort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_rating: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_rating: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogSort {
    pub field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub descending: Option<bool>,
}

/// Photo information record returned by `query_photos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoInfo {
    #[serde(default)]
    pub id: i64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub camera: Option<String>,
    #[serde(default)]
    pub captured: Option<String>,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub rating: i32,
    #[serde(default)]
    pub flag: String,
    #[serde(default)]
    pub edited: bool,
    #[serde(default)]
    pub copy_name: Option<String>,
    #[serde(default)]
    pub copy_of: Option<i64>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub preview_only: Option<bool>,
    #[serde(default)]
    pub title: String,
}

/// Raw result from `query_photos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryPhotosResult {
    pub photos: Vec<PhotoInfo>,
    pub total: usize,
}

// ============================================================================
// Tool 3: get_develop
// ============================================================================

/// Parameters for calling `get_develop`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDevelopParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
}

impl GetDevelopParams {
    pub fn for_numeric_id(id: i64) -> Self {
        Self {
            id: Some(id),
            photo_id: None,
        }
    }

    pub fn for_photo_id(photo_id: impl Into<String>) -> Self {
        Self {
            id: None,
            photo_id: Some(photo_id.into()),
        }
    }
}

/// Comprehensive non-destructive develop settings structure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevelopSettings {
    #[serde(default = "default_treatment")]
    pub treatment: String,
    #[serde(default)]
    pub process: u32,
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub orientation: String,
    #[serde(default)]
    pub profile: ProfileSettings,
    #[serde(default)]
    pub wb: WhiteBalanceSettings,
    #[serde(default)]
    pub light: LightSettings,
    #[serde(default)]
    pub color: ColorSettings,
    #[serde(default)]
    pub effects: EffectsSettings,
    #[serde(default)]
    pub curve: CurveSettings,
    #[serde(default)]
    pub mixer: MixerSettings,
    #[serde(default)]
    pub bw_mix: BwMixSettings,
    #[serde(default)]
    pub grading: GradingSettings,
    #[serde(default)]
    pub detail: DetailSettings,
    #[serde(default)]
    pub grain: GrainSettings,
    #[serde(default)]
    pub vignette: VignetteSettings,
    #[serde(default)]
    pub optics: OpticsSettings,
    #[serde(default)]
    pub geometry: GeometrySettings,
    #[serde(default)]
    pub crop: CropSettings,
    #[serde(default)]
    pub calibration: CalibrationSettings,
    #[serde(default)]
    pub enhance: Option<EnhanceSettings>,
    #[serde(default)]
    pub lens_blur: Option<LensBlurSettings>,
    #[serde(default)]
    pub disabled_sections: Vec<String>,
    #[serde(default)]
    pub masks: Vec<serde_json::Value>,
    #[serde(default)]
    pub spots: Vec<serde_json::Value>,
    #[serde(default)]
    pub red_eye: Vec<serde_json::Value>,
    #[serde(default)]
    pub point_colors: Vec<serde_json::Value>,
    /// Fallback dictionary of flat slider values (e.g. {"light.exposure": 0.0}).
    #[serde(default)]
    pub values: Option<HashMap<String, f64>>,
}

fn default_treatment() -> String {
    "color".to_string()
}

impl Default for DevelopSettings {
    fn default() -> Self {
        Self {
            treatment: default_treatment(),
            process: 1,
            version: 1,
            orientation: "normal".to_string(),
            profile: ProfileSettings::default(),
            wb: WhiteBalanceSettings::default(),
            light: LightSettings::default(),
            color: ColorSettings::default(),
            effects: EffectsSettings::default(),
            curve: CurveSettings::default(),
            mixer: MixerSettings::default(),
            bw_mix: BwMixSettings::default(),
            grading: GradingSettings::default(),
            detail: DetailSettings::default(),
            grain: GrainSettings::default(),
            vignette: VignetteSettings::default(),
            optics: OpticsSettings::default(),
            geometry: GeometrySettings::default(),
            crop: CropSettings::default(),
            calibration: CalibrationSettings::default(),
            enhance: None,
            lens_blur: None,
            disabled_sections: Vec::new(),
            masks: Vec::new(),
            spots: Vec::new(),
            red_eye: Vec::new(),
            point_colors: Vec::new(),
            values: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSettings {
    #[serde(default = "default_profile_id")]
    pub id: String,
    #[serde(default = "default_100")]
    pub amount: f32,
}

fn default_profile_id() -> String {
    "adobe.color".to_string()
}

fn default_100() -> f32 {
    100.0
}

impl Default for ProfileSettings {
    fn default() -> Self {
        Self {
            id: default_profile_id(),
            amount: 100.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhiteBalanceSettings {
    #[serde(default = "default_wb_mode")]
    pub mode: String,
    #[serde(default = "default_temp")]
    pub temp: f32,
    #[serde(default)]
    pub tint: f32,
}

fn default_wb_mode() -> String {
    "as_shot".to_string()
}

fn default_temp() -> f32 {
    5500.0
}

impl Default for WhiteBalanceSettings {
    fn default() -> Self {
        Self {
            mode: default_wb_mode(),
            temp: 5500.0,
            tint: 0.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LightSettings {
    #[serde(default)]
    pub exposure: f32,
    #[serde(default)]
    pub contrast: f32,
    #[serde(default)]
    pub highlights: f32,
    #[serde(default)]
    pub shadows: f32,
    #[serde(default)]
    pub whites: f32,
    #[serde(default)]
    pub blacks: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ColorSettings {
    #[serde(default)]
    pub saturation: f32,
    #[serde(default)]
    pub vibrance: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectsSettings {
    #[serde(default)]
    pub texture: f32,
    #[serde(default)]
    pub clarity: f32,
    #[serde(default)]
    pub dehaze: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CurveSettings {
    #[serde(default)]
    pub master: Vec<CurvePoint>,
    #[serde(default)]
    pub red: Vec<CurvePoint>,
    #[serde(default)]
    pub green: Vec<CurvePoint>,
    #[serde(default)]
    pub blue: Vec<CurvePoint>,
    #[serde(default)]
    pub highlights: f32,
    #[serde(default)]
    pub lights: f32,
    #[serde(default)]
    pub darks: f32,
    #[serde(default)]
    pub shadows: f32,
    #[serde(default)]
    pub refine_saturation: f32,
    #[serde(default)]
    pub split_highlights: f32,
    #[serde(default)]
    pub split_mid: f32,
    #[serde(default)]
    pub split_shadows: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HslAdjustment {
    #[serde(default)]
    pub hue: f32,
    #[serde(default)]
    pub sat: f32,
    #[serde(default)]
    pub lum: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MixerSettings {
    #[serde(default)]
    pub red: HslAdjustment,
    #[serde(default)]
    pub orange: HslAdjustment,
    #[serde(default)]
    pub yellow: HslAdjustment,
    #[serde(default)]
    pub green: HslAdjustment,
    #[serde(default)]
    pub aqua: HslAdjustment,
    #[serde(default)]
    pub blue: HslAdjustment,
    #[serde(default)]
    pub purple: HslAdjustment,
    #[serde(default)]
    pub magenta: HslAdjustment,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BwMixSettings {
    #[serde(default)]
    pub red: f32,
    #[serde(default)]
    pub orange: f32,
    #[serde(default)]
    pub yellow: f32,
    #[serde(default)]
    pub green: f32,
    #[serde(default)]
    pub aqua: f32,
    #[serde(default)]
    pub blue: f32,
    #[serde(default)]
    pub purple: f32,
    #[serde(default)]
    pub magenta: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GradingSettings {
    #[serde(default)]
    pub shadows: HslAdjustment,
    #[serde(default)]
    pub midtones: HslAdjustment,
    #[serde(default)]
    pub highlights: HslAdjustment,
    #[serde(default)]
    pub global: HslAdjustment,
    #[serde(default)]
    pub blending: f32,
    #[serde(default)]
    pub balance: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DetailSettings {
    #[serde(default)]
    pub sharpen_amount: f32,
    #[serde(default)]
    pub sharpen_radius: f32,
    #[serde(default)]
    pub sharpen_detail: f32,
    #[serde(default)]
    pub sharpen_masking: f32,
    #[serde(default)]
    pub nr_luminance: f32,
    #[serde(default)]
    pub nr_detail: f32,
    #[serde(default)]
    pub nr_contrast: f32,
    #[serde(default)]
    pub nr_color: f32,
    #[serde(default)]
    pub nr_color_detail: f32,
    #[serde(default)]
    pub nr_color_smoothness: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GrainSettings {
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub size: f32,
    #[serde(default)]
    pub roughness: f32,
    #[serde(default)]
    pub seed: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VignetteSettings {
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub midpoint: f32,
    #[serde(default)]
    pub roundness: f32,
    #[serde(default)]
    pub feather: f32,
    #[serde(default)]
    pub highlights: f32,
    #[serde(default)]
    pub style: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpticsSettings {
    #[serde(default)]
    pub distortion: f32,
    #[serde(default)]
    pub vignetting: f32,
    #[serde(default)]
    pub vignetting_midpoint: f32,
    #[serde(default)]
    pub remove_ca: bool,
    #[serde(default)]
    pub ca_red: f32,
    #[serde(default)]
    pub ca_blue: f32,
    #[serde(default)]
    pub lens_profile: bool,
    #[serde(default)]
    pub profile_distortion: f32,
    #[serde(default)]
    pub profile_vignetting: f32,
    #[serde(default)]
    pub defringe_purple_amount: f32,
    #[serde(default)]
    pub defringe_purple_hue_lo: f32,
    #[serde(default)]
    pub defringe_purple_hue_hi: f32,
    #[serde(default)]
    pub defringe_green_amount: f32,
    #[serde(default)]
    pub defringe_green_hue_lo: f32,
    #[serde(default)]
    pub defringe_green_hue_hi: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GeometrySettings {
    #[serde(default)]
    pub vertical: f32,
    #[serde(default)]
    pub horizontal: f32,
    #[serde(default)]
    pub rotate: f32,
    #[serde(default)]
    pub scale: f32,
    #[serde(default)]
    pub aspect: f32,
    #[serde(default)]
    pub offset_x: f32,
    #[serde(default)]
    pub offset_y: f32,
    #[serde(default)]
    pub upright: String,
    #[serde(default)]
    pub upright_transform: Option<serde_json::Value>,
    #[serde(default)]
    pub constrain_crop: bool,
    #[serde(default)]
    pub guides: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NormalizedRect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CropGeometry {
    #[serde(default)]
    pub angle: f32,
    #[serde(default)]
    pub rect: NormalizedRect,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CropSettings {
    #[serde(default)]
    pub aspect: Option<String>,
    #[serde(default)]
    pub geometry: CropGeometry,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSettings {
    #[serde(default)]
    pub shadows_tint: f32,
    #[serde(default)]
    pub red_hue: f32,
    #[serde(default)]
    pub red_sat: f32,
    #[serde(default)]
    pub green_hue: f32,
    #[serde(default)]
    pub green_sat: f32,
    #[serde(default)]
    pub blue_hue: f32,
    #[serde(default)]
    pub blue_sat: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EnhanceSettings {
    #[serde(default)]
    pub denoise: f32,
    #[serde(default)]
    pub raw_details: bool,
    #[serde(default)]
    pub super_resolution: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LensBlurSettings {
    #[serde(default)]
    pub amount: f32,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub focal_distance: f32,
    #[serde(default)]
    pub focal_range: f32,
}

// ============================================================================
// Tool 4: set_develop
// ============================================================================

/// Parameters for calling `set_develop`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDevelopParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub values: Option<HashMap<String, f64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Single control slider update returned in `set_develop` response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlValue {
    pub id: String,
    pub value: f64,
}

/// Result returned from `set_develop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetDevelopResult {
    pub ok: bool,
    #[serde(default)]
    pub controls: Vec<ControlValue>,
}

// ============================================================================
// Tool 5: apply_preset
// ============================================================================

/// Parameters for calling `apply_preset`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyPresetParams {
    pub preset: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
}

impl ApplyPresetParams {
    pub fn new(preset: impl Into<String>, amount_pct: f64, ids: Option<Vec<i64>>) -> Self {
        Self {
            preset: preset.into(),
            amount: Some(amount_pct),
            ids,
            photo_id: None,
        }
    }
}

// ============================================================================
// Tool 6: cmd_photo_virtualCopy
// ============================================================================

/// Parameters for `cmd_photo_virtualCopy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualCopyParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Raw result payload from `cmd_photo_virtualCopy`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualCopyRawResult {
    #[serde(default)]
    pub ids: Vec<i64>,
    #[serde(default)]
    pub virtual_copy_id: Option<String>,
}

impl VirtualCopyRawResult {
    /// Returns the newly allocated virtual copy photo ID as i64.
    pub fn primary_id(&self) -> Option<i64> {
        self.ids.first().copied().or_else(|| {
            self.virtual_copy_id
                .as_ref()
                .and_then(|s| s.parse::<i64>().ok())
        })
    }
}

// ============================================================================
// Tool 7: render_preview
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Png,
    Jpeg,
}

/// Parameters for calling `render_preview`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderPreviewParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_side: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ImageFormat>,
}

/// Metadata payload returned in text block of `render_preview`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderMeta {
    #[serde(default)]
    pub id: Option<i64>,
    pub width: u32,
    pub height: u32,
}

/// High-level decoded preview ready for graphics rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPreviewResult {
    pub id: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data_base64: String,
}

impl RenderPreviewResult {
    /// Decodes base64 payload into raw image bytes.
    pub fn decode_bytes(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&self.data_base64)
    }
}

// ============================================================================
// Tool 8: render_photo
// ============================================================================

/// Parameters for calling `render_photo`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderPhotoParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ImageFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderPhotoResult {
    pub id: Option<i64>,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
    pub data_base64: String,
    pub saved_path: Option<String>,
}

// ============================================================================
// Tool 9: export
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Jpeg,
    Png,
    Tiff,
    Psd,
    Webp,
    Avif,
    Jxl,
    Original,
    Dng,
    Rendereddng,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportColorSpace {
    Srgb,
    DisplayP3,
    AdobeRgb,
    ProPhoto,
    Rec2020,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    Unique,
    Overwrite,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataExportPolicy {
    All,
    AllExceptCamera,
    Copyright,
    None,
}

/// Parameters for calling `export`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ids: Option<Vec<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub photo_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subfolder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<ExportFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bit_depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color_space: Option<ExportColorSpace>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_edge: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub short_edge: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dont_enlarge: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub naming: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataExportPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watermark: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedFile {
    pub bytes: u64,
    pub path: String,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub sidecars: Vec<String>,
}

/// Result returned from `export`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub conflict_policy: Option<String>,
    #[serde(default)]
    pub success: Option<bool>,
    #[serde(default)]
    pub files: Vec<ExportedFile>,
}

// ============================================================================
// Presets, Profiles, and Develop Controls
// ============================================================================

/// Preset descriptor returned by `cmd_presets_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPresetInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub builtin: bool,
    #[serde(default)]
    pub favorite: bool,
}

/// Color profile descriptor returned by `cmd_profiles_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpProfileInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub favorite: bool,
}

/// Develop slider control descriptor returned by `list_controls`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DevelopControlInfo {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub section: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    #[serde(default)]
    pub value: f64,
    #[serde(default)]
    pub step: f64,
    #[serde(default)]
    pub decimals: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_import_params_defaults_to_add() {
        let params = ImportParams::new_add(vec!["./DRAW".to_string()]);
        assert_eq!(params.mode, ImportMode::Add);
        let val = serde_json::to_value(&params).unwrap();
        assert_eq!(val["mode"], "add");
    }

    #[test]
    fn test_develop_settings_defaults() {
        let ds = DevelopSettings::default();
        assert_eq!(ds.treatment, "color");
        assert_eq!(ds.light.exposure, 0.0);
        assert_eq!(ds.wb.temp, 5500.0);
    }

    #[test]
    fn test_export_conflict_policy_serialization() {
        let params = ExportParams {
            conflict: Some(ConflictPolicy::Unique),
            format: Some(ExportFormat::Jpeg),
            ..Default::default()
        };
        let val = serde_json::to_value(&params).unwrap();
        assert_eq!(val["conflict"], "unique");
        assert_eq!(val["format"], "jpeg");
    }
}
