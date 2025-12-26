# Penpot High-Performance Rust Services

This directory contains Rust microservices designed to replace performance-critical parts of Penpot's Clojure backend.

## 🏗️ Architecture

```
┌─────────────────────────────────────────────────────┐
│              Frontend (Existing)                    │
│         ClojureScript + React + WebSocket           │
└──────────────────┬──────────────────────────────────┘
                   │
           ┌───────▼───────┐
           │  API Gateway  │
           │     :8080     │
           └───────┬───────┘
        ┌──────────┼──────────┬──────────────┐
        │          │          │              │
┌───────▼──┐ ┌────▼─────┐ ┌──▼─────────┐ ┌──▼────────┐
│  Shape   │ │ Realtime │ │  Render    │ │  Clojure  │
│Validator │ │   Sync   │ │  Service   │ │  Backend  │
│  :8081   │ │  :8082   │ │   :8083    │ │ (existing)│
└──────────┘ └──────────┘ └────────────┘ └───────────┘
        │          │          │              │
        └──────────┴──────────┴──────────────┘
                   │
           ┌───────▼───────┐
           │  Prometheus   │
           │    Metrics    │
           └───────────────┘
```

## 📦 Services

| Service | Port | Description | Status |
|---------|------|-------------|--------|
| `api-gateway` | 8080 | Central routing & caching | ✅ Ready |
| `shape-validator` | 8081 | Fast shape validation | ✅ Ready |
| `realtime-sync` | 8082 | WebSocket collaboration | ✅ Ready |
| `render-service` | 8083 | Server-side rendering with resvg | ✅ Ready |

### Service Features

- **API Gateway**: Request routing, caching with TTL, service health aggregation
- **Shape Validator**: Batch validation, concurrent processing, Prometheus metrics
- **Realtime Sync**: WebSocket rooms, presence tracking, cursor sync
- **Render Service**: SVG → PNG rendering, thumbnails, font support

## 🚀 Quick Start

### Prerequisites

- Rust 1.83+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`)
- Docker & Docker Compose

### Build & Run

```bash
# Build all services
cargo build --release

# Run individually
cargo run -p api-gateway
cargo run -p shape-validator
cargo run -p realtime-sync
cargo run -p render-service

# Or use Docker
docker compose -f docker-compose.hybrid.yml up -d
```

### Test API Gateway

```bash
# Health check (includes all service status)
curl http://localhost:8080/health

# Cache stats
curl http://localhost:8080/cache/stats

# Metrics
curl http://localhost:8080/metrics
```

### Test Shape Validator

```bash
# Health check
curl http://localhost:8081/health

# Validate shapes
curl -X POST http://localhost:8081/validate \
  -H "Content-Type: application/json" \
  -d '{
    "shapes": [{
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Rectangle",
      "type": "rect",
      "x": 0, "y": 0,
      "width": 100, "height": 100
    }]
  }'
```

### Test Render Service

```bash
# Health check
curl http://localhost:8083/health

# Render SVG to PNG (base64 encoded response)
curl -X POST http://localhost:8083/render \
  -H "Content-Type: application/json" \
  -d '{
    "svg": "<svg width=\"100\" height=\"100\"><rect fill=\"red\" width=\"100\" height=\"100\"/></svg>",
    "format": "png"
  }'

# Generate thumbnail
curl -X POST http://localhost:8083/thumbnail \
  -H "Content-Type: application/json" \
  -d '{
    "svg": "<svg width=\"1000\" height=\"1000\"><circle cx=\"500\" cy=\"500\" r=\"400\" fill=\"blue\"/></svg>",
    "max_width": 128,
    "max_height": 128
  }'
```

### Test Real-time Sync

```bash
# Health check
curl http://localhost:8082/health

# Stats
curl http://localhost:8082/stats

# WebSocket connection (use wscat or browser)
wscat -c ws://localhost:8082/ws/550e8400-e29b-41d4-a716-446655440000
```

## 📁 Structure

```
rust-services/
├── Cargo.toml              # Workspace manifest
├── docker-compose.hybrid.yml
├── common/                 # Shared types & utilities
│   ├── src/
│   │   ├── lib.rs
│   │   ├── types.rs        # Penpot data types
│   │   ├── validation.rs   # Shape validation logic
│   │   └── error.rs        # Error types
├── api-gateway/            # Central routing service
│   └── src/main.rs
├── shape-validator/        # Shape validation service
│   └── src/main.rs
├── realtime-sync/          # WebSocket service
│   └── src/main.rs
├── render-service/         # SVG rendering service
│   └── src/main.rs
├── benchmarks/             # Performance testing
│   ├── src/
│   └── scripts/
└── docker/                 # Docker configurations
    ├── Dockerfile.*
    └── prometheus.yml
```

## 🧪 Testing

```bash
# Run unit tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific package tests
cargo test -p common
cargo test -p shape-validator

# Run integration tests (requires services to be running)
# First start all services:
./scripts/dev.sh start

# Then run integration tests:
cargo test -p integration-tests -- --ignored --test-threads=1
# or
./scripts/dev.sh integration
```

### Test Coverage

| Test Type | Count | Description |
|-----------|-------|-------------|
| Unit Tests | 5 | Common library validation |
| Integration Tests | 16 | End-to-end service tests |
| Performance Tests | 2 | Latency benchmarks |

## 📁 Development Script

Use the included dev script for common operations:

```bash
./scripts/dev.sh help

# Common commands:
./scripts/dev.sh build       # Build all services (release)
./scripts/dev.sh start       # Start all services locally
./scripts/dev.sh stop        # Stop all services
./scripts/dev.sh health      # Check service health
./scripts/dev.sh test        # Run unit tests
./scripts/dev.sh integration # Run integration tests
./scripts/dev.sh smoke       # Quick smoke tests
./scripts/dev.sh check       # Full check (fmt, lint, test)
```

## 📊 Benchmarking

```bash
# Install wrk
sudo apt install wrk

# Benchmark shape validator
wrk -t12 -c400 -d30s -s benchmarks/scripts/validate.lua http://localhost:8081/validate

# Run Criterion benchmarks
cargo bench
```

## 🔧 Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `RUST_LOG` | `info` | Log level (trace, debug, info, warn, error) |
| `VALIDATOR_URL` | `http://localhost:8081` | Shape validator URL |
| `REALTIME_URL` | `http://localhost:8082` | Realtime sync URL |
| `RENDER_URL` | `http://localhost:8083` | Render service URL |
| `BACKEND_URL` | `http://localhost:6060` | Clojure backend URL |
| `CACHE_TTL_SECS` | `60` | Cache TTL in seconds |
| `REDIS_URL` | - | Redis/Valkey connection URL |

## 📈 Monitoring

All services expose Prometheus metrics at `/metrics`:

```bash
# API Gateway metrics
curl http://localhost:8080/metrics

# Shape Validator metrics
curl http://localhost:8081/metrics

# Realtime Sync metrics
curl http://localhost:8082/metrics

# Render Service metrics
curl http://localhost:8083/metrics
```

### Key Metrics

- `*_requests_total` - Total request count
- `*_processing_seconds` - Request latency histogram
- `*_errors_total` - Error count
- `ws_active_connections` - Active WebSocket connections
- `gateway_cache_hits_total` / `gateway_cache_misses_total` - Cache performance

## 🤝 Integration with Penpot

These services are designed to work alongside the existing Penpot backend:

1. **API Gateway**: Central entry point, routes to appropriate service
2. **Shape Validator**: Clojure backend calls `POST /validate` before saving shapes
3. **Real-time Sync**: Frontend connects directly for WebSocket collaboration
4. **Render Service**: Export operations are routed to this service

See [IMPLEMENTATION_PLAN.md](../IMPLEMENTATION_PLAN.md) for full integration details.

## 📈 Performance Targets

| Metric | Clojure | Rust Target | Improvement |
|--------|---------|-------------|-------------|
| Validation (100 shapes) | ~50ms | <1ms | 50x |
| WebSocket latency | ~50ms | <5ms | 10x |
| Memory per connection | ~1MB | <10KB | 100x |
| Cold start | ~30s | <200ms | 150x |
| SVG rendering | ~500ms | <50ms | 10x |

## 📝 License

MPL-2.0 (same as Penpot)
