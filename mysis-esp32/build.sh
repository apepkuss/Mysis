#!/usr/bin/env bash
set -euo pipefail

CHIP="${1:-esp32s3}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

case "$CHIP" in
    esp32s3)
        cp "$SCRIPT_DIR/.cargo/config.toml" "$SCRIPT_DIR/.cargo/config.toml.bak" 2>/dev/null || true
        cat > "$SCRIPT_DIR/.cargo/config.toml" <<'EOF'
[build]
target = "xtensa-esp32s3-espidf"

[target.xtensa-esp32s3-espidf]
linker = "ldproxy"
EOF
        ;;
    esp32c3)
        cp "$SCRIPT_DIR/.cargo/config.toml" "$SCRIPT_DIR/.cargo/config.toml.bak" 2>/dev/null || true
        cp "$SCRIPT_DIR/.cargo/config_esp32c3.toml" "$SCRIPT_DIR/.cargo/config.toml"
        ;;
    esp32c6)
        cp "$SCRIPT_DIR/.cargo/config.toml" "$SCRIPT_DIR/.cargo/config.toml.bak" 2>/dev/null || true
        cp "$SCRIPT_DIR/.cargo/config_esp32c6.toml" "$SCRIPT_DIR/.cargo/config.toml"
        ;;
    *)
        echo "Usage: $0 [esp32s3|esp32c3|esp32c6]"
        exit 1
        ;;
esac

echo "Building for $CHIP..."
cd "$SCRIPT_DIR"
cargo build --release --no-default-features --features "$CHIP"
echo "Build complete for $CHIP."
