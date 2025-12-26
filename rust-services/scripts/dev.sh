#!/bin/bash
# Development script for Penpot Rust Services
# Usage: ./scripts/dev.sh [command]

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
cd "$ROOT_DIR"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() { echo -e "${BLUE}[INFO]${NC} $1"; }
log_success() { echo -e "${GREEN}[OK]${NC} $1"; }
log_warn() { echo -e "${YELLOW}[WARN]${NC} $1"; }
log_error() { echo -e "${RED}[ERROR]${NC} $1"; }

# Commands
cmd_build() {
    log_info "Building all services in release mode..."
    cargo build --release
    log_success "Build complete!"
}

cmd_test() {
    log_info "Running unit tests..."
    cargo test
    log_success "Tests passed!"
}

cmd_integration() {
    log_info "Running integration tests (services must be running)..."
    cargo test -p integration-tests -- --ignored --test-threads=1
    log_success "Integration tests passed!"
}

cmd_start() {
    log_info "Starting all services..."
    
    # Kill any existing processes
    pkill -f "shape-validator|render-service|realtime-sync|api-gateway" 2>/dev/null || true
    sleep 1

    # Start services in background
    log_info "Starting Shape Validator on :8081"
    ./target/release/shape-validator &
    
    log_info "Starting Realtime Sync on :8082"
    ./target/release/realtime-sync &
    
    log_info "Starting Render Service on :8083"
    ./target/release/render-service &
    
    log_info "Starting API Gateway on :8080"
    ./target/release/api-gateway &
    
    sleep 2
    log_success "All services started!"
    
    # Health check
    cmd_health
}

cmd_stop() {
    log_info "Stopping all services..."
    pkill -f "shape-validator|render-service|realtime-sync|api-gateway" 2>/dev/null || true
    log_success "Services stopped"
}

cmd_health() {
    log_info "Checking service health..."
    
    check_health() {
        local name=$1
        local port=$2
        if curl -s "http://localhost:$port/health" > /dev/null 2>&1; then
            log_success "$name (:$port) - healthy"
            return 0
        else
            log_error "$name (:$port) - unavailable"
            return 1
        fi
    }
    
    check_health "API Gateway" 8080 || true
    check_health "Shape Validator" 8081 || true
    check_health "Realtime Sync" 8082 || true
    check_health "Render Service" 8083 || true
}

cmd_watch() {
    log_info "Starting in watch mode (requires cargo-watch)..."
    log_warn "Install with: cargo install cargo-watch"
    
    # Pick a service to watch
    local service="${1:-shape-validator}"
    cargo watch -x "run -p $service"
}

cmd_bench() {
    log_info "Running benchmarks..."
    cargo bench
}

cmd_load() {
    log_info "Running load tests (requires wrk)..."
    
    if ! command -v wrk &> /dev/null; then
        log_error "wrk not found. Install with: sudo apt install wrk"
        exit 1
    fi
    
    log_info "Load testing shape validator..."
    wrk -t4 -c100 -d10s -s benchmarks/load-tests/benchmark.lua http://localhost:8081/validate
}

cmd_docker() {
    log_info "Building Docker images..."
    docker compose -f docker-compose.hybrid.yml build
}

cmd_docker_up() {
    log_info "Starting Docker services..."
    docker compose -f docker-compose.hybrid.yml up -d
}

cmd_docker_down() {
    log_info "Stopping Docker services..."
    docker compose -f docker-compose.hybrid.yml down
}

cmd_logs() {
    local service="${1:-all}"
    if [[ "$service" == "all" ]]; then
        docker compose -f docker-compose.hybrid.yml logs -f
    else
        docker compose -f docker-compose.hybrid.yml logs -f "$service"
    fi
}

cmd_clean() {
    log_info "Cleaning build artifacts..."
    cargo clean
    log_success "Clean complete"
}

cmd_fmt() {
    log_info "Formatting code..."
    cargo fmt
    log_success "Format complete"
}

cmd_lint() {
    log_info "Running clippy..."
    cargo clippy --all-targets --all-features -- -D warnings
    log_success "Lint complete"
}

cmd_check() {
    log_info "Running full check (fmt, lint, test)..."
    cargo fmt -- --check
    cargo clippy --all-targets -- -D warnings
    cargo test
    log_success "All checks passed!"
}

cmd_smoke() {
    log_info "Running smoke tests..."
    
    # Health checks
    echo "Testing health endpoints..."
    curl -s http://localhost:8081/health | jq .status
    curl -s http://localhost:8082/health | jq .status
    curl -s http://localhost:8083/health | jq .status
    curl -s http://localhost:8080/health | jq .status
    
    # Validation test
    echo "Testing shape validation..."
    curl -s -X POST http://localhost:8081/validate \
        -H "Content-Type: application/json" \
        -d '{"shapes":[{"id":"550e8400-e29b-41d4-a716-446655440000","name":"Test","type":"rect","x":0,"y":0,"width":100,"height":100}]}' | jq .valid
    
    # Render test
    echo "Testing render..."
    curl -s -X POST http://localhost:8083/render \
        -H "Content-Type: application/json" \
        -d '{"svg":"<svg width=\"100\" height=\"100\"><rect fill=\"red\" width=\"100\" height=\"100\"/></svg>","format":"png"}' | jq .success
    
    log_success "Smoke tests complete!"
}

cmd_help() {
    echo "Penpot Rust Services Development Script"
    echo ""
    echo "Usage: ./scripts/dev.sh [command]"
    echo ""
    echo "Commands:"
    echo "  build       Build all services (release mode)"
    echo "  test        Run unit tests"
    echo "  integration Run integration tests (services must be running)"
    echo "  start       Start all services locally"
    echo "  stop        Stop all services"
    echo "  health      Check service health"
    echo "  watch [svc] Watch mode for a service (default: shape-validator)"
    echo "  bench       Run benchmarks"
    echo "  load        Run load tests (requires wrk)"
    echo "  smoke       Run smoke tests"
    echo ""
    echo "Docker:"
    echo "  docker      Build Docker images"
    echo "  docker-up   Start Docker services"
    echo "  docker-down Stop Docker services"
    echo "  logs [svc]  View service logs"
    echo ""
    echo "Code Quality:"
    echo "  fmt         Format code"
    echo "  lint        Run clippy"
    echo "  check       Full check (fmt, lint, test)"
    echo "  clean       Clean build artifacts"
    echo ""
    echo "Examples:"
    echo "  ./scripts/dev.sh build && ./scripts/dev.sh start"
    echo "  ./scripts/dev.sh smoke"
    echo "  ./scripts/dev.sh watch realtime-sync"
}

# Main
case "${1:-help}" in
    build)       cmd_build ;;
    test)        cmd_test ;;
    integration) cmd_integration ;;
    start)       cmd_start ;;
    stop)        cmd_stop ;;
    health)      cmd_health ;;
    watch)       cmd_watch "$2" ;;
    bench)       cmd_bench ;;
    load)        cmd_load ;;
    smoke)       cmd_smoke ;;
    docker)      cmd_docker ;;
    docker-up)   cmd_docker_up ;;
    docker-down) cmd_docker_down ;;
    logs)        cmd_logs "$2" ;;
    fmt)         cmd_fmt ;;
    lint)        cmd_lint ;;
    check)       cmd_check ;;
    clean)       cmd_clean ;;
    help|*)      cmd_help ;;
esac
