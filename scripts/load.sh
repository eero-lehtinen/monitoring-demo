#!/usr/bin/env bash
# Generates traffic so the dashboard has something to draw.
set -euo pipefail
HOST="${HOST:-http://localhost:8080}"

while true; do
  curl -sS -o /dev/null "$HOST/hello"
  curl -sS -o /dev/null "$HOST/slow"
  curl -sS -o /dev/null "$HOST/flaky" || true
  sleep 0.2
done
