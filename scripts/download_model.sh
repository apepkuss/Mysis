#!/bin/bash
# 下载 bge-small-zh-v1.5 嵌入模型到 ~/.mysis/models/
set -e

MODEL_DIR="$HOME/.mysis/models/bge-small-zh-v1.5"
REPO="BAAI/bge-small-zh-v1.5"
BASE_URL="https://huggingface.co/${REPO}/resolve/main"

mkdir -p "$MODEL_DIR"

echo "Downloading bge-small-zh-v1.5 to $MODEL_DIR ..."

# tokenizer.json (~400KB)
if [ ! -f "$MODEL_DIR/tokenizer.json" ]; then
    echo "  -> tokenizer.json"
    curl -L -o "$MODEL_DIR/tokenizer.json" "${BASE_URL}/tokenizer.json"
else
    echo "  -> tokenizer.json (already exists, skipping)"
fi

# ONNX model (~90MB)
if [ ! -f "$MODEL_DIR/model.onnx" ]; then
    echo "  -> model.onnx (this may take a moment)"
    curl -L -o "$MODEL_DIR/model.onnx" "${BASE_URL}/onnx/model.onnx"
else
    echo "  -> model.onnx (already exists, skipping)"
fi

echo "Done. Model files:"
ls -lh "$MODEL_DIR"
