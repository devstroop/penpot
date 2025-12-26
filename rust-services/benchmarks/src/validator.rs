//! Benchmark: Shape Validator Performance
//!
//! Measures validation throughput and latency.

use common::{Shape, ShapeType};
use indicatif::{ProgressBar, ProgressStyle};
use rand::Rng;
use std::time::{Duration, Instant};
use tabled::{Table, Tabled};
use uuid::Uuid;

#[derive(Tabled)]
struct BenchmarkResult {
    #[tabled(rename = "Test")]
    name: String,
    #[tabled(rename = "Shapes")]
    shape_count: usize,
    #[tabled(rename = "Total (ms)")]
    total_ms: f64,
    #[tabled(rename = "Per Shape (µs)")]
    per_shape_us: f64,
    #[tabled(rename = "Throughput (shapes/s)")]
    throughput: String,
}

fn generate_random_shape(shape_type: ShapeType) -> Shape {
    let mut rng = rand::thread_rng();
    
    Shape {
        id: Uuid::new_v4(),
        name: format!("Shape-{}", rng.gen::<u32>()),
        shape_type,
        x: rng.gen_range(-1000.0..1000.0),
        y: rng.gen_range(-1000.0..1000.0),
        width: rng.gen_range(10.0..500.0),
        height: rng.gen_range(10.0..500.0),
        rotation: Some(rng.gen_range(0.0..360.0)),
        transform: None,
        transform_inverse: None,
        parent_id: None,
        frame_id: None,
        fills: None,
        strokes: None,
        opacity: Some(rng.gen_range(0.0..1.0)),
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
    let shape_types = [
        ShapeType::Rect,
        ShapeType::Circle,
        ShapeType::Frame,
        ShapeType::Group,
    ];
    
    let mut rng = rand::thread_rng();
    (0..count)
        .map(|_| generate_random_shape(shape_types[rng.gen_range(0..shape_types.len())]))
        .collect()
}

fn benchmark_validation(name: &str, shapes: &[Shape]) -> BenchmarkResult {
    let iterations = 100;
    let mut total_duration = Duration::ZERO;
    
    // Warm up
    for _ in 0..10 {
        let _ = common::validation::validate_shapes_batch(shapes);
    }
    
    // Actual benchmark
    for _ in 0..iterations {
        let start = Instant::now();
        let _ = common::validation::validate_shapes_batch(shapes);
        total_duration += start.elapsed();
    }
    
    let avg_duration = total_duration / iterations;
    let total_ms = avg_duration.as_secs_f64() * 1000.0;
    let per_shape_us = (avg_duration.as_nanos() as f64 / shapes.len() as f64) / 1000.0;
    let throughput = shapes.len() as f64 / avg_duration.as_secs_f64();
    
    BenchmarkResult {
        name: name.to_string(),
        shape_count: shapes.len(),
        total_ms,
        per_shape_us,
        throughput: format!("{:.0}", throughput),
    }
}

#[tokio::main]
async fn main() {
    println!("🚀 Penpot Shape Validator Benchmark");
    println!("====================================\n");
    
    let test_sizes = [1, 10, 100, 1000, 10_000];
    let mut results = Vec::new();
    
    let pb = ProgressBar::new(test_sizes.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({msg})")
            .unwrap()
    );
    
    for &size in &test_sizes {
        pb.set_message(format!("Testing {} shapes", size));
        let shapes = generate_shapes(size);
        let result = benchmark_validation(&format!("{} shapes", size), &shapes);
        results.push(result);
        pb.inc(1);
    }
    
    pb.finish_with_message("Done!");
    
    println!("\n📊 Results:\n");
    let table = Table::new(&results).to_string();
    println!("{}", table);
    
    println!("\n✅ Benchmark complete!");
    println!("\n📝 Notes:");
    println!("  - All times are averaged over 100 iterations");
    println!("  - Warm-up phase: 10 iterations (excluded from results)");
    println!("  - Lower per-shape time = better performance");
}
