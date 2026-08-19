#!/usr/bin/env bash
# Generates traffic so the dashboard has something to draw.
set -euo pipefail
HOST="${HOST:-http://localhost:8080}"
request_batch=0

while true; do
  curl -sS -o /dev/null "$HOST/hello"
  curl -sS -o /dev/null "$HOST/slow"
  curl -sS -o /dev/null "$HOST/flaky" || true
  if ((request_batch % 10 == 0)); then
    curl -sS -o /dev/null "$HOST/slow?ms=300&fail=true" || true
  fi
  request_batch=$((request_batch + 1))
  sleep 0.2
done
