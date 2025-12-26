//! Shape validation logic
//!
//! Provides fast, compile-time checked validation for Penpot shapes.
//! This is a drop-in replacement for Malli schema validation.

use crate::types::*;

/// Validation result for a single shape
#[derive(Debug, Clone, serde::Serialize)]
pub struct ShapeValidationResult {
    pub shape_id: uuid::Uuid,
    pub valid: bool,
    pub errors: Vec<ValidationError>,
}

/// Validation error details
#[derive(Debug, Clone, serde::Serialize)]
pub struct ValidationError {
    pub field: String,
    pub message: String,
    pub code: String,
}

/// Validates a single shape
pub fn validate_shape(shape: &Shape) -> ShapeValidationResult {
    let mut errors = Vec::new();

    // Custom validation rules
    errors.extend(validate_shape_geometry(shape));
    errors.extend(validate_shape_type_specific(shape));
    errors.extend(validate_shape_appearance(shape));

    ShapeValidationResult {
        shape_id: shape.id,
        valid: errors.is_empty(),
        errors,
    }
}

/// Validates a batch of shapes
pub fn validate_shapes(shapes: &[Shape]) -> Vec<ShapeValidationResult> {
    shapes.iter().map(validate_shape).collect()
}

/// Validates all shapes and returns combined result
pub fn validate_shapes_batch(shapes: &[Shape]) -> BatchValidationResult {
    let results = validate_shapes(shapes);
    let all_valid = results.iter().all(|r| r.valid);
    let total_errors: usize = results.iter().map(|r| r.errors.len()).sum();

    BatchValidationResult {
        valid: all_valid,
        total_shapes: shapes.len(),
        valid_shapes: results.iter().filter(|r| r.valid).count(),
        invalid_shapes: results.iter().filter(|r| !r.valid).count(),
        total_errors,
        results,
    }
}

/// Batch validation result
#[derive(Debug, Clone, serde::Serialize)]
pub struct BatchValidationResult {
    pub valid: bool,
    pub total_shapes: usize,
    pub valid_shapes: usize,
    pub invalid_shapes: usize,
    pub total_errors: usize,
    pub results: Vec<ShapeValidationResult>,
}

/// Validates shape geometry
fn validate_shape_geometry(shape: &Shape) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Width and height must be non-negative
    if shape.width < 0.0 {
        errors.push(ValidationError {
            field: "width".to_string(),
            message: "Width must be non-negative".to_string(),
            code: "range".to_string(),
        });
    }

    if shape.height < 0.0 {
        errors.push(ValidationError {
            field: "height".to_string(),
            message: "Height must be non-negative".to_string(),
            code: "range".to_string(),
        });
    }

    // Validate rotation is within bounds
    if let Some(rotation) = shape.rotation {
        if !rotation.is_finite() {
            errors.push(ValidationError {
                field: "rotation".to_string(),
                message: "Rotation must be a finite number".to_string(),
                code: "finite".to_string(),
            });
        }
    }

    // Validate transform matrix
    if let Some(ref transform) = shape.transform {
        if !is_valid_matrix(transform) {
            errors.push(ValidationError {
                field: "transform".to_string(),
                message: "Transform matrix contains invalid values".to_string(),
                code: "matrix".to_string(),
            });
        }
    }

    errors
}

/// Validates shape-type-specific rules
fn validate_shape_type_specific(shape: &Shape) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    match shape.shape_type {
        ShapeType::Frame => {
            // Frames should not have a frame_id pointing to themselves
            if shape.frame_id == Some(shape.id) {
                errors.push(ValidationError {
                    field: "frame_id".to_string(),
                    message: "Frame cannot be its own frame".to_string(),
                    code: "self_reference".to_string(),
                });
            }
        }
        ShapeType::Path => {
            // Path must have content
            if shape.content.is_none() {
                errors.push(ValidationError {
                    field: "content".to_string(),
                    message: "Path shape must have content".to_string(),
                    code: "required".to_string(),
                });
            }
        }
        ShapeType::Text => {
            // Text should have text_content
            if shape.text_content.is_none() {
                errors.push(ValidationError {
                    field: "text_content".to_string(),
                    message: "Text shape should have text_content".to_string(),
                    code: "required".to_string(),
                });
            }
        }
        ShapeType::Image => {
            // Image should have metadata
            if shape.metadata.is_none() {
                errors.push(ValidationError {
                    field: "metadata".to_string(),
                    message: "Image shape should have metadata".to_string(),
                    code: "required".to_string(),
                });
            }
        }
        _ => {}
    }

    errors
}

/// Validates a transformation matrix
fn is_valid_matrix(matrix: &Matrix) -> bool {
    matrix.a.is_finite()
        && matrix.b.is_finite()
        && matrix.c.is_finite()
        && matrix.d.is_finite()
        && matrix.e.is_finite()
        && matrix.f.is_finite()
}

/// Validates shape appearance (colors, opacity, etc.)
fn validate_shape_appearance(shape: &Shape) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    // Validate opacity range
    if let Some(opacity) = shape.opacity {
        if opacity < 0.0 || opacity > 1.0 {
            errors.push(ValidationError {
                field: "opacity".to_string(),
                message: "Opacity must be between 0.0 and 1.0".to_string(),
                code: "range".to_string(),
            });
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_test_shape(shape_type: ShapeType) -> Shape {
        Shape {
            id: Uuid::new_v4(),
            name: "Test Shape".to_string(),
            shape_type,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            rotation: None,
            transform: None,
            transform_inverse: None,
            parent_id: None,
            frame_id: None,
            fills: None,
            strokes: None,
            opacity: None,
            blend_mode: None,
            hidden: None,
            blocked: None,
            locked: None,
            shadow: None,
            blur: None,
            constraints_h: None,
            constraints_v: None,
            content: None,
            text_content: None,
            metadata: None,
        }
    }

    #[test]
    fn test_valid_rect() {
        let shape = create_test_shape(ShapeType::Rect);
        let result = validate_shape(&shape);
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_invalid_width() {
        let mut shape = create_test_shape(ShapeType::Rect);
        shape.width = -10.0;
        let result = validate_shape(&shape);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "width"));
    }

    #[test]
    fn test_path_without_content() {
        let shape = create_test_shape(ShapeType::Path);
        let result = validate_shape(&shape);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "content"));
    }

    #[test]
    fn test_batch_validation() {
        let shapes = vec![
            create_test_shape(ShapeType::Rect),
            create_test_shape(ShapeType::Circle),
        ];
        let result = validate_shapes_batch(&shapes);
        assert!(result.valid);
        assert_eq!(result.total_shapes, 2);
        assert_eq!(result.valid_shapes, 2);
    }
}
