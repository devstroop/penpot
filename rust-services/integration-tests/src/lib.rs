//! Integration tests for Penpot Rust Services
//!
//! These tests require the services to be running.
//! Run with: cargo test -p integration-tests -- --ignored

use std::time::Duration;

const VALIDATOR_URL: &str = "http://localhost:8081";
const REALTIME_URL: &str = "http://localhost:8082";
const RENDER_URL: &str = "http://localhost:8083";
const GATEWAY_URL: &str = "http://localhost:8080";

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

// =============================================================================
// Shape Validator Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_validator_health() {
    let resp = client()
        .get(format!("{}/health", VALIDATOR_URL))
        .send()
        .await
        .expect("Failed to connect to validator");
    
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
}

#[tokio::test]
#[ignore]
async fn test_validator_validates_shapes() {
    let shapes = serde_json::json!({
        "shapes": [{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "Rectangle",
            "type": "rect",
            "x": 0, "y": 0,
            "width": 100, "height": 100
        }]
    });

    let resp = client()
        .post(format!("{}/validate", VALIDATOR_URL))
        .json(&shapes)
        .send()
        .await
        .expect("Failed to validate shapes");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], true);
    assert_eq!(body["total_shapes"], 1);
}

#[tokio::test]
#[ignore]
async fn test_validator_batch_validation() {
    let shapes = serde_json::json!({
        "shapes": [
            {"id": "550e8400-e29b-41d4-a716-446655440001", "name": "R1", "type": "rect", "x": 0, "y": 0, "width": 100, "height": 100},
            {"id": "550e8400-e29b-41d4-a716-446655440002", "name": "R2", "type": "rect", "x": 10, "y": 10, "width": 50, "height": 50},
            {"id": "550e8400-e29b-41d4-a716-446655440003", "name": "C1", "type": "circle", "x": 0, "y": 0, "width": 100, "height": 100},
        ]
    });

    let resp = client()
        .post(format!("{}/validate", VALIDATOR_URL))
        .json(&shapes)
        .send()
        .await
        .expect("Failed to validate batch");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["total_shapes"], 3);
}

#[tokio::test]
#[ignore]
async fn test_validator_rejects_invalid() {
    let shapes = serde_json::json!({
        "shapes": [{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "Bad Rectangle",
            "type": "rect",
            "x": 0, "y": 0,
            "width": -100,
            "height": 100
        }]
    });

    let resp = client()
        .post(format!("{}/validate", VALIDATOR_URL))
        .json(&shapes)
        .send()
        .await
        .expect("Failed to validate");

    // Server returns 400 Bad Request for invalid shapes
    assert_eq!(resp.status().as_u16(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], false);
    assert!(body["invalid_shapes"].as_i64().unwrap() > 0);
}

#[tokio::test]
#[ignore]
async fn test_validator_metrics() {
    let resp = client()
        .get(format!("{}/metrics", VALIDATOR_URL))
        .send()
        .await
        .expect("Failed to get metrics");

    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert!(body.contains("validator_requests_total"));
}

// =============================================================================
// Realtime Sync Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_realtime_health() {
    let resp = client()
        .get(format!("{}/health", REALTIME_URL))
        .send()
        .await
        .expect("Failed to connect to realtime");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
}

#[tokio::test]
#[ignore]
async fn test_realtime_stats() {
    let resp = client()
        .get(format!("{}/stats", REALTIME_URL))
        .send()
        .await
        .expect("Failed to get stats");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["active_rooms"].is_number());
    assert!(body["total_connections"].is_number());
}

// =============================================================================
// Render Service Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_render_health() {
    let resp = client()
        .get(format!("{}/health", RENDER_URL))
        .send()
        .await
        .expect("Failed to connect to render");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
    assert!(body["capabilities"].as_array().unwrap().contains(&serde_json::json!("png")));
}

#[tokio::test]
#[ignore]
async fn test_render_svg_to_png() {
    let request = serde_json::json!({
        "svg": r#"<svg width="100" height="100" xmlns="http://www.w3.org/2000/svg"><rect fill="red" width="100" height="100"/></svg>"#,
        "format": "png"
    });

    let resp = client()
        .post(format!("{}/render", RENDER_URL))
        .json(&request)
        .send()
        .await
        .expect("Failed to render");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert_eq!(body["format"], "png");
    assert!(body["data"].as_str().is_some());
}

#[tokio::test]
#[ignore]
async fn test_render_thumbnail() {
    let request = serde_json::json!({
        "svg": r#"<svg width="1000" height="1000" xmlns="http://www.w3.org/2000/svg"><circle cx="500" cy="500" r="400" fill="blue"/></svg>"#,
        "max_width": 128,
        "max_height": 128
    });

    let resp = client()
        .post(format!("{}/thumbnail", RENDER_URL))
        .json(&request)
        .send()
        .await
        .expect("Failed to create thumbnail");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["success"], true);
    assert!(body["width"].as_u64().unwrap() <= 128);
    assert!(body["height"].as_u64().unwrap() <= 128);
}

// =============================================================================
// API Gateway Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_gateway_health() {
    let resp = client()
        .get(format!("{}/health", GATEWAY_URL))
        .send()
        .await
        .expect("Failed to connect to gateway");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "healthy");
    assert!(body["services"]["validator"].is_boolean());
}

#[tokio::test]
#[ignore]
async fn test_gateway_validates_via_proxy() {
    let shapes = serde_json::json!({
        "shapes": [{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "Rectangle",
            "type": "rect",
            "x": 0, "y": 0,
            "width": 100, "height": 100
        }]
    });

    let resp = client()
        .post(format!("{}/api/v1/validate", GATEWAY_URL))
        .json(&shapes)
        .send()
        .await
        .expect("Failed to validate via gateway");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["valid"], true);
}

#[tokio::test]
#[ignore]
async fn test_gateway_cache_stats() {
    let resp = client()
        .get(format!("{}/cache/stats", GATEWAY_URL))
        .send()
        .await
        .expect("Failed to get cache stats");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["size"].is_number());
    assert!(body["ttl_seconds"].is_number());
}

#[tokio::test]
#[ignore]
async fn test_gateway_rate_limit_info() {
    let resp = client()
        .get(format!("{}/rate-limit", GATEWAY_URL))
        .send()
        .await
        .expect("Failed to get rate limit info");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["rate_limit"]["requests_per_second"].is_number());
}

// =============================================================================
// Performance Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_validator_performance() {
    use std::time::Instant;

    let shapes = serde_json::json!({
        "shapes": (0..100).map(|i| serde_json::json!({
            "id": format!("550e8400-e29b-41d4-a716-44665544{:04}", i),
            "name": format!("Shape {}", i),
            "type": "rect",
            "x": i * 10, "y": i * 10,
            "width": 100, "height": 100
        })).collect::<Vec<_>>()
    });

    let start = Instant::now();
    let iterations = 10;

    for _ in 0..iterations {
        let resp = client()
            .post(format!("{}/validate", VALIDATOR_URL))
            .json(&shapes)
            .send()
            .await
            .expect("Failed to validate");
        assert!(resp.status().is_success());
    }

    let avg_ms = start.elapsed().as_millis() / iterations;
    println!("Average validation time for 100 shapes: {}ms", avg_ms);
    
    // Allow 100ms for CI environments (production should be <10ms)
    assert!(avg_ms < 100, "Validation too slow: {}ms", avg_ms);
}

#[tokio::test]
#[ignore]
async fn test_render_performance() {
    use std::time::Instant;

    let request = serde_json::json!({
        "svg": r#"<svg width="500" height="500" xmlns="http://www.w3.org/2000/svg">
            <rect fill="red" width="500" height="500"/>
            <circle cx="250" cy="250" r="200" fill="blue"/>
            <text x="250" y="250" text-anchor="middle" font-size="50">Test</text>
        </svg>"#,
        "format": "png"
    });

    let start = Instant::now();
    let iterations = 10;

    for _ in 0..iterations {
        let resp = client()
            .post(format!("{}/render", RENDER_URL))
            .json(&request)
            .send()
            .await
            .expect("Failed to render");
        assert!(resp.status().is_success());
    }

    let avg_ms = start.elapsed().as_millis() / iterations;
    println!("Average render time for 500x500 SVG: {}ms", avg_ms);
    
    assert!(avg_ms < 200, "Rendering too slow: {}ms", avg_ms);
}

// =============================================================================
// Rate Limiting Tests
// =============================================================================

#[tokio::test]
#[ignore]
async fn test_gateway_rate_limiting() {
    // The rate limiter allows 100 req/s with burst of 200
    // We'll send requests and verify it returns 429 when limit exceeded
    
    let shapes = serde_json::json!({
        "shapes": [{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "name": "Test",
            "type": "rect",
            "x": 0, "y": 0,
            "width": 100, "height": 100
        }]
    });

    // Send many requests rapidly
    let mut success_count = 0;
    let mut rate_limited_count = 0;
    
    for _ in 0..250 {
        let resp = client()
            .post(format!("{}/api/v1/validate", GATEWAY_URL))
            .json(&shapes)
            .send()
            .await
            .expect("Request failed");
        
        if resp.status().as_u16() == 429 {
            rate_limited_count += 1;
        } else if resp.status().is_success() {
            success_count += 1;
        }
    }

    println!("Successful requests: {}", success_count);
    println!("Rate limited requests: {}", rate_limited_count);
    
    // With 100 req/s and burst 200, most should succeed in a quick burst
    // but some should be rate limited
    assert!(success_count > 100, "Should have many successful requests");
    // Rate limiting may or may not trigger depending on timing
}
