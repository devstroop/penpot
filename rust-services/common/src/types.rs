//! Penpot data types
//!
//! These types mirror the Clojure spec definitions for compatibility.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for Penpot objects
pub type PenpotId = Uuid;

/// 2D point
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// RGBA color (0.0 - 1.0 range)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Default for Color {
    fn default() -> Self {
        Self {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
    }
}

/// Bounding box / selection rect
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// 2D transformation matrix (affine)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Matrix {
    fn default() -> Self {
        Self::identity()
    }
}

impl Matrix {
    pub fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
}

/// Shape types supported by Penpot
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShapeType {
    Frame,
    Group,
    Rect,
    Circle,
    Path,
    Text,
    Image,
    Svg,
    Bool,
}

/// Fill type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "fill-type", rename_all = "kebab-case")]
pub enum Fill {
    Solid {
        fill_color: Color,
        fill_opacity: Option<f64>,
    },
    LinearGradient {
        start_x: f64,
        start_y: f64,
        end_x: f64,
        end_y: f64,
        stops: Vec<GradientStop>,
    },
    RadialGradient {
        center_x: f64,
        center_y: f64,
        radius: f64,
        stops: Vec<GradientStop>,
    },
    Image {
        #[serde(rename = "fill-image")]
        fill_image: Uuid,
    },
}

/// Gradient color stop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f64,
    pub color: Color,
}

/// Stroke configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stroke {
    pub stroke_color: Option<Color>,
    pub stroke_opacity: Option<f64>,
    pub stroke_width: Option<f64>,
    pub stroke_alignment: Option<StrokeAlignment>,
    pub stroke_cap_start: Option<StrokeCap>,
    pub stroke_cap_end: Option<StrokeCap>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrokeAlignment {
    Inner,
    Center,
    Outer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StrokeCap {
    Round,
    Square,
    LineArrow,
    TriangleArrow,
    SquareMarker,
    CircleMarker,
    DiamondMarker,
}

/// Shadow effect
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shadow {
    pub id: Option<Uuid>,
    pub style: ShadowStyle,
    pub color: Color,
    pub offset_x: f64,
    pub offset_y: f64,
    pub blur: f64,
    pub spread: f64,
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShadowStyle {
    DropShadow,
    InnerShadow,
}

/// Blur effect
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blur {
    pub id: Option<Uuid>,
    #[serde(rename = "type")]
    pub blur_type: BlurType,
    pub value: f64,
    pub hidden: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlurType {
    LayerBlur,
    BackgroundBlur,
}

/// Complete shape definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shape {
    pub id: PenpotId,
    pub name: String,
    #[serde(rename = "type")]
    pub shape_type: ShapeType,

    // Geometry
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub rotation: Option<f64>,
    pub transform: Option<Matrix>,
    pub transform_inverse: Option<Matrix>,

    // Hierarchy
    pub parent_id: Option<PenpotId>,
    pub frame_id: Option<PenpotId>,

    // Appearance
    pub fills: Option<Vec<Fill>>,
    pub strokes: Option<Vec<Stroke>>,
    pub opacity: Option<f64>,
    pub blend_mode: Option<BlendMode>,
    pub hidden: Option<bool>,
    pub blocked: Option<bool>,
    pub locked: Option<bool>,

    // Effects
    pub shadow: Option<Vec<Shadow>>,
    pub blur: Option<Blur>,

    // Constraints
    pub constraints_h: Option<Constraint>,
    pub constraints_v: Option<Constraint>,

    // For path shapes
    pub content: Option<PathContent>,

    // For text shapes
    pub text_content: Option<serde_json::Value>,

    // For image shapes
    pub metadata: Option<ImageMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
    Normal,
    Darken,
    Multiply,
    ColorBurn,
    Lighten,
    Screen,
    ColorDodge,
    Overlay,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Constraint {
    Start,
    End,
    Center,
    Scale,
    Fixed,
}

/// Path content (SVG-like path commands)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathContent {
    pub segments: Vec<PathSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum PathSegment {
    MoveTo { x: f64, y: f64 },
    LineTo { x: f64, y: f64 },
    CurveTo { c1x: f64, c1y: f64, c2x: f64, c2y: f64, x: f64, y: f64 },
    Close,
}

/// Image metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMetadata {
    pub id: Uuid,
    pub width: u32,
    pub height: u32,
    pub mtype: String,
}

/// File representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct File {
    pub id: PenpotId,
    pub name: String,
    pub project_id: PenpotId,
    pub created_at: String,
    pub modified_at: String,
    pub is_shared: bool,
}

/// Page representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: PenpotId,
    pub name: String,
    pub file_id: PenpotId,
    pub ordering: i32,
}

/// Project representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: PenpotId,
    pub name: String,
    pub team_id: PenpotId,
    pub created_at: String,
    pub modified_at: String,
    pub is_default: bool,
}
