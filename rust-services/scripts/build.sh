#!/bin/bash
# Quick start script for Rust services development

set -e

echo "🦀 Penpot Rust Services - Quick Start"
echo "======================================"

# Check Rust installation
if ! command -v cargo &> /dev/null; then
    echo "❌ Rust not found. Installing..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "✅ Rust $(rustc --version)"

cd "$(dirname "$0")"

# Build
echo ""
echo "📦 Building services..."
cargo build --release

echo ""
echo "✅ Build complete!"
echo ""
echo "Available commands:"
echo "  cargo run -p shape-validator   # Start shape validator on :8081"
echo "  cargo run -p realtime-sync     # Start realtime sync on :8082"
echo "  cargo run -p render-service    # Start render service on :8083"
echo ""
echo "Or run all with Docker:"
echo "  docker compose -f ../docker-compose.hybrid.yml up rust-services"
