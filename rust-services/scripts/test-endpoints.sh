#!/bin/bash
# Test all Rust services endpoints

set -e

echo "🧪 Testing Rust Services"
echo "========================"
echo ""

BASE_VALIDATOR="http://localhost:8081"
BASE_REALTIME="http://localhost:8082"
BASE_RENDER="http://localhost:8083"

# Test function
test_endpoint() {
    local name=$1
    local method=$2
    local url=$3
    local data=$4
    
    echo -n "  Testing $name... "
    
    if [ "$method" = "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "$url")
    else
        response=$(curl -s -w "\n%{http_code}" -X POST -H "Content-Type: application/json" -d "$data" "$url")
    fi
    
    status_code=$(echo "$response" | tail -n 1)
    body=$(echo "$response" | head -n -1)
    
    if [ "$status_code" -ge 200 ] && [ "$status_code" -lt 300 ]; then
        echo "✅ OK ($status_code)"
        return 0
    else
        echo "❌ FAILED ($status_code)"
        echo "    Response: $body"
        return 1
    fi
}

# Shape Validator tests
echo "📋 Shape Validator ($BASE_VALIDATOR)"
test_endpoint "Health check" GET "$BASE_VALIDATOR/health"

test_endpoint "Valid shape" POST "$BASE_VALIDATOR/validate" '{
    "shapes": [{
        "id": "550e8400-e29b-41d4-a716-446655440000",
        "name": "Test Rectangle",
        "type": "rect",
        "x": 0, "y": 0,
        "width": 100, "height": 100
    }]
}'

test_endpoint "Multiple shapes" POST "$BASE_VALIDATOR/validate" '{
    "shapes": [
        {"id": "550e8400-e29b-41d4-a716-446655440001", "name": "Rect", "type": "rect", "x": 0, "y": 0, "width": 100, "height": 100},
        {"id": "550e8400-e29b-41d4-a716-446655440002", "name": "Circle", "type": "circle", "x": 200, "y": 200, "width": 50, "height": 50}
    ]
}'

echo ""

# Real-time Sync tests
echo "📡 Real-time Sync ($BASE_REALTIME)"
test_endpoint "Health check" GET "$BASE_REALTIME/health"
test_endpoint "Stats" GET "$BASE_REALTIME/stats"

echo ""

# Render Service tests
echo "🎨 Render Service ($BASE_RENDER)"
test_endpoint "Health check" GET "$BASE_RENDER/health"

test_endpoint "Render request" POST "$BASE_RENDER/render" '{
    "file_id": "550e8400-e29b-41d4-a716-446655440000",
    "page_id": "550e8400-e29b-41d4-a716-446655440001",
    "format": "png"
}'

echo ""
echo "✅ All tests complete!"
