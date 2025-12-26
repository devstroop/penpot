//! Benchmark: Compare Rust vs Clojure Backend
//!
//! Sends requests to both backends and compares response times.

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tabled::{Table, Tabled};
use uuid::Uuid;

const RUST_URL: &str = "http://localhost:8081/validate";
const CLOJURE_URL: &str = "http://localhost:6060/api/rpc"; // Adjust as needed

#[derive(Serialize)]
struct ValidateRequest {
    shapes: Vec<TestShape>,
}

#[derive(Serialize, Clone)]
struct TestShape {
    id: Uuid,
    name: String,
    #[serde(rename = "type")]
    shape_type: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Deserialize)]
struct ValidateResponse {
    valid: bool,
    processing_time_us: Option<u64>,
}

#[derive(Tabled)]
struct ComparisonResult {
    #[tabled(rename = "Test")]
    name: String,
    #[tabled(rename = "Shapes")]
    shape_count: usize,
    #[tabled(rename = "Rust (ms)")]
    rust_ms: String,
    #[tabled(rename = "Clojure (ms)")]
    clojure_ms: String,
    #[tabled(rename = "Speedup")]
    speedup: String,
}

fn generate_shapes(count: usize) -> Vec<TestShape> {
    (0..count)
        .map(|i| TestShape {
            id: Uuid::new_v4(),
            name: format!("Shape-{}", i),
            shape_type: "rect".to_string(),
            x: (i as f64) * 10.0,
            y: (i as f64) * 10.0,
            width: 100.0,
            height: 100.0,
        })
        .collect()
}

async fn benchmark_rust(client: &Client, shapes: &[TestShape], iterations: u32) -> Option<Duration> {
    let request = ValidateRequest {
        shapes: shapes.to_vec(),
    };
    
    // Warm up
    for _ in 0..5 {
        let _ = client.post(RUST_URL).json(&request).send().await;
    }
    
    let mut total = Duration::ZERO;
    let mut success_count = 0;
    
    for _ in 0..iterations {
        let start = Instant::now();
        if let Ok(resp) = client.post(RUST_URL).json(&request).send().await {
            if resp.status().is_success() {
                total += start.elapsed();
                success_count += 1;
            }
        }
    }
    
    if success_count > 0 {
        Some(total / success_count)
    } else {
        None
    }
}

async fn benchmark_clojure(client: &Client, shapes: &[TestShape], iterations: u32) -> Option<Duration> {
    // Note: Adjust the request format to match Penpot's RPC format
    let request = serde_json::json!({
        "method": "validate-shapes",
        "params": {
            "shapes": shapes
        }
    });
    
    // Warm up
    for _ in 0..5 {
        let _ = client.post(CLOJURE_URL).json(&request).send().await;
    }
    
    let mut total = Duration::ZERO;
    let mut success_count = 0;
    
    for _ in 0..iterations {
        let start = Instant::now();
        if let Ok(resp) = client.post(CLOJURE_URL).json(&request).send().await {
            if resp.status().is_success() {
                total += start.elapsed();
                success_count += 1;
            }
        }
    }
    
    if success_count > 0 {
        Some(total / success_count)
    } else {
        None
    }
}

#[tokio::main]
async fn main() {
    println!("🏁 Penpot Backend Comparison Benchmark");
    println!("======================================\n");
    println!("Rust endpoint:    {}", RUST_URL);
    println!("Clojure endpoint: {}", CLOJURE_URL);
    println!();
    
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");
    
    // Check connectivity
    print!("Checking Rust service... ");
    match client.get("http://localhost:8081/health").send().await {
        Ok(resp) if resp.status().is_success() => println!("✅ OK"),
        _ => {
            println!("❌ Not available");
            println!("\nPlease start the Rust service: cargo run -p shape-validator");
            return;
        }
    }
    
    print!("Checking Clojure service... ");
    match client.get("http://localhost:6060/readyz").send().await {
        Ok(resp) if resp.status().is_success() => println!("✅ OK"),
        _ => println!("⚠️  Not available (will skip)"),
    }
    
    println!();
    
    let test_sizes = [1, 10, 100, 500, 1000];
    let iterations = 50;
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
        
        let rust_time = benchmark_rust(&client, &shapes, iterations).await;
        let clojure_time = benchmark_clojure(&client, &shapes, iterations).await;
        
        let (rust_ms, clojure_ms, speedup) = match (rust_time, clojure_time) {
            (Some(r), Some(c)) => {
                let r_ms = r.as_secs_f64() * 1000.0;
                let c_ms = c.as_secs_f64() * 1000.0;
                let speed = c_ms / r_ms;
                (format!("{:.2}", r_ms), format!("{:.2}", c_ms), format!("{:.1}x", speed))
            }
            (Some(r), None) => {
                (format!("{:.2}", r.as_secs_f64() * 1000.0), "N/A".to_string(), "N/A".to_string())
            }
            (None, Some(c)) => {
                ("N/A".to_string(), format!("{:.2}", c.as_secs_f64() * 1000.0), "N/A".to_string())
            }
            (None, None) => ("N/A".to_string(), "N/A".to_string(), "N/A".to_string()),
        };
        
        results.push(ComparisonResult {
            name: format!("{} shapes", size),
            shape_count: size,
            rust_ms,
            clojure_ms,
            speedup,
        });
        
        pb.inc(1);
    }
    
    pb.finish_with_message("Done!");
    
    println!("\n📊 Comparison Results:\n");
    let table = Table::new(&results).to_string();
    println!("{}", table);
    
    println!("\n✅ Benchmark complete!");
    println!("\n📝 Notes:");
    println!("  - Each test runs {} iterations (after 5 warm-up)", iterations);
    println!("  - Speedup = Clojure time / Rust time");
    println!("  - Higher speedup = Rust is faster");
}
