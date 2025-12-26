#!/bin/bash
# Load testing script for Rust services
# Requires: wrk (https://github.com/wg/wrk)

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RESULTS_DIR="${SCRIPT_DIR}/../results"
mkdir -p "$RESULTS_DIR"

# Configuration
VALIDATOR_URL="${VALIDATOR_URL:-http://localhost:8081}"
REALTIME_URL="${REALTIME_URL:-http://localhost:8082}"
RENDER_URL="${RENDER_URL:-http://localhost:8083}"

THREADS="${THREADS:-4}"
CONNECTIONS="${CONNECTIONS:-100}"
DURATION="${DURATION:-30s}"

echo "🚀 Penpot Rust Services Load Test"
echo "=================================="
echo ""
echo "Configuration:"
echo "  Threads:     $THREADS"
echo "  Connections: $CONNECTIONS"
echo "  Duration:    $DURATION"
echo ""

# Check if wrk is installed
if ! command -v wrk &> /dev/null; then
    echo "❌ wrk not found. Install it with:"
    echo "   Ubuntu/Debian: sudo apt install wrk"
    echo "   macOS: brew install wrk"
    exit 1
fi

# Check services
echo "Checking services..."

check_service() {
    local name=$1
    local url=$2
    if curl -s -f "${url}/health" > /dev/null 2>&1; then
        echo "  ✅ $name: OK"
        return 0
    else
        echo "  ⚠️  $name: Not available"
        return 1
    fi
}

VALIDATOR_OK=$(check_service "Shape Validator" "$VALIDATOR_URL" && echo 1 || echo 0)
REALTIME_OK=$(check_service "Real-time Sync" "$REALTIME_URL" && echo 1 || echo 0)
RENDER_OK=$(check_service "Render Service" "$RENDER_URL" && echo 1 || echo 0)

echo ""

# Run benchmarks
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

if [ "$VALIDATOR_OK" = "1" ]; then
    echo "📊 Benchmarking Shape Validator..."
    echo ""
    
    wrk -t"$THREADS" -c"$CONNECTIONS" -d"$DURATION" \
        -s "${SCRIPT_DIR}/benchmark.lua" \
        "${VALIDATOR_URL}/validate" \
        2>&1 | tee "${RESULTS_DIR}/validator_${TIMESTAMP}.txt"
    
    echo ""
fi

if [ "$REALTIME_OK" = "1" ]; then
    echo "📊 Benchmarking Real-time Sync (HTTP endpoints)..."
    echo ""
    
    wrk -t"$THREADS" -c"$CONNECTIONS" -d"$DURATION" \
        "${REALTIME_URL}/health" \
        2>&1 | tee "${RESULTS_DIR}/realtime_${TIMESTAMP}.txt"
    
    echo ""
fi

if [ "$RENDER_OK" = "1" ]; then
    echo "📊 Benchmarking Render Service..."
    echo ""
    
    wrk -t"$THREADS" -c"$CONNECTIONS" -d"$DURATION" \
        "${RENDER_URL}/health" \
        2>&1 | tee "${RESULTS_DIR}/render_${TIMESTAMP}.txt"
    
    echo ""
fi

echo "✅ Load tests complete!"
echo "Results saved to: ${RESULTS_DIR}"
