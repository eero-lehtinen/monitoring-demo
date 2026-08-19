# Monitoring demo

This project is a small Rust HTTP service that sends metrics, logs, and traces
to a local Grafana observability stack through OpenTelemetry.

It has three endpoints with predictable behavior:

| Endpoint | Behavior |
| --- | --- |
| `GET /hello` | Returns `200` immediately. |
| `GET /slow?ms=800` | Waits for the requested number of milliseconds. Without `ms`, it waits for a random value between 10 and 1499 ms. Add `fail=true` to fail during the execute phase. |
| `GET /flaky?fail_rate=0.8` | Returns `500` at the requested rate. The default rate is `0.3`. |

## Run it

You need Docker, Rust, and `curl`.

```sh
docker compose up -d
RUST_LIB_BACKTRACE=1 cargo run
```

The complete service dashboard is available at
<http://localhost:3001/d/monitoring-demo/monitoring-demo>. The server listens
at <http://localhost:8080>.

## Try the endpoints

```sh
# Fast request that returns 200.
curl -i http://localhost:8080/hello

# Wait 500 ms before returning 200.
curl -i 'http://localhost:8080/slow?ms=500'

# Fail during work.execute, return 500, and report the error.
curl -i 'http://localhost:8080/slow?ms=500&fail=true'

# Always return 500 and increment the intentional-failure counter.
curl -i 'http://localhost:8080/flaky?fail_rate=1'
```

Every response includes a `traceparent` header. Print only that header with:

```sh
curl -sS -D - -o /dev/null 'http://localhost:8080/slow?ms=500' \
  | grep -i traceparent
```

Generate a steady mix of successful, slow, and failed requests in another
terminal:

```sh
./scripts/load.sh
```

Stop the load generator and server with Ctrl-C. Stop the monitoring stack with
`docker compose down`.

## Architecture

The Rust service runs on the host. Docker Compose starts one
`grafana/otel-lgtm` container with the OpenTelemetry Collector, Prometheus,
Loki, Tempo, and Grafana preconfigured to work together.

```text
                         OTLP/gRPC :4317
client ──HTTP──> Axum service ───────────────────> OTel Collector
                    │                                  │
                    │                       ┌──────────┼──────────┐
                    │                       v          v          v
                    │                  Prometheus     Loki      Tempo
                    │                       └──────────┼──────────┘
                    └── traceparent response header   v
                                               Grafana :3001
```

### What each part does

| Part | Purpose |
| --- | --- |
| `traceparent` response header | Gives the caller the trace ID for that request. Use it to find the exact request trace in Grafana when debugging a slow or failed call. |
| OpenTelemetry Collector | Receives all telemetry from the application, batches it, and sends each signal to the correct storage system. The application only needs to know one OTLP endpoint. |
| Prometheus | Stores numeric metrics such as request counts, error rates, latency, and counters. |
| Loki | Stores and searches application logs. Log records include trace IDs so they can be matched to requests. |
| Tempo | Stores traces. A trace shows the spans and timing for one request. |
| Grafana | Queries Prometheus, Loki, and Tempo and displays their data in one dashboard. Grafana does not store the telemetry itself. |

A request passes through three Axum middleware layers before it reaches a
handler:

1. `OtelAxumLayer` reads an incoming W3C `traceparent` header and creates the
   request span.
2. `OtelInResponseLayer` writes the current trace context to the response.
3. `HttpMetricsLayer` records request duration, body size, response size, and
   active requests. It labels measurements with the route template instead of
   each concrete URL. The duration histogram's count gives the request volume.

The handlers add log events, a `work` span with `work.prepare`, `work.execute`,
and `work.finalize` child spans, and application failure counters. A failed
execution creates an `anyhow::Error`. The error captures a Rust backtrace
because the run command enables `RUST_LIB_BACKTRACE`. The handler records that
backtrace as `exception.stacktrace`, marks `work.execute` as an error, and emits
a correlated ERROR log. `src/telemetry.rs` connects those signals to
OpenTelemetry:

- tracing events go to the terminal and to the OTLP log exporter;
- spans go to the OTLP trace exporter;
- HTTP and application metrics go to the OTLP metric exporter every five
  seconds.

All exporters use the same `monitoring-demo` service resource and send to
`OTEL_EXPORTER_OTLP_ENDPOINT`. The OpenTelemetry SDK defaults to
`http://localhost:4317`, which reaches the collector exposed by Compose. This
keeps the application independent of Prometheus, Loki, and Tempo. Only the
collector knows how each signal is stored.

The tracing and logging layers share the active span context, so exported log
records can be matched to the request trace. The response `traceparent` header
identifies the trace for that request.

## Dashboard

After running the load generator for a few seconds, open the
[Monitoring demo dashboard](http://localhost:3001/d/monitoring-demo/monitoring-demo).
Compose provisions it automatically. It includes:

- request rate, error rate, latency percentiles, and a duration heatmap;
- request totals, active requests, body sizes, and response throughput for
  each endpoint;
- the `demo.flaky.failures` application counter;
- application logs grouped by severity, plus a dedicated error stream with
  readable messages and trace IDs;
- failed and recent Tempo traces. Select one to inspect its spans.

To read a failure backtrace, select a failed trace, select the red
`work.execute` span, and open its exception event. Grafana shows the formatted
Rust stack in the `exception.stacktrace` attribute. The first application frame
points to the line that created the `anyhow::Error`.

OpenTelemetry uses dotted metric names. Prometheus converts them to its naming
format. For example, `demo.flaky.failures` becomes
`demo_flaky_failures_total`.

## Project layout

```text
src/main.rs       HTTP routes, middleware, handlers, and application metric
src/telemetry.rs  OpenTelemetry providers, exporters, logging, and shutdown
compose.yaml      Local collector, storage backends, and Grafana
grafana/           Provisioned dashboard and dashboard provider
scripts/load.sh   Repeating traffic generator
```

The service flushes all three OpenTelemetry providers during graceful shutdown.
Keep `otel::tracing=trace` enabled if you override `RUST_LOG`; the Axum tracing
middleware creates its request span at that level.
