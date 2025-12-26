use common::{validation, Shape, ShapeType};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use uuid::Uuid;

fn generate_shape(shape_type: ShapeType) -> Shape {
    Shape {
        id: Uuid::new_v4(),
        name: "Benchmark Shape".to_string(),
        shape_type,
        x: 100.0,
        y: 100.0,
        width: 200.0,
        height: 150.0,
        rotation: Some(45.0),
        transform: None,
        transform_inverse: None,
        parent_id: None,
        frame_id: None,
        fills: None,
        strokes: None,
        opacity: Some(0.8),
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

fn generate_shapes(count: usize) -> Vec<Shape> {
    (0..count).map(|_| generate_shape(ShapeType::Rect)).collect()
}

fn benchmark_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("shape_validation");
    
    for size in [1, 10, 100, 1000, 10000].iter() {
        let shapes = generate_shapes(*size);
        
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &shapes,
            |b, shapes| {
                b.iter(|| validation::validate_shapes_batch(black_box(shapes)))
            },
        );
    }
    
    group.finish();
}

fn benchmark_single_shape_types(c: &mut Criterion) {
    let mut group = c.benchmark_group("single_shape_validation");
    
    let shape_types = [
        ("rect", ShapeType::Rect),
        ("circle", ShapeType::Circle),
        ("frame", ShapeType::Frame),
        ("group", ShapeType::Group),
    ];
    
    for (name, shape_type) in shape_types {
        let shape = generate_shape(shape_type);
        
        group.bench_with_input(
            BenchmarkId::from_parameter(name),
            &shape,
            |b, shape| {
                b.iter(|| validation::validate_shape(black_box(shape)))
            },
        );
    }
    
    group.finish();
}

criterion_group!(benches, benchmark_validation, benchmark_single_shape_types);
criterion_main!(benches);
