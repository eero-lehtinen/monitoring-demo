mod telemetry;

use std::sync::LazyLock;
use std::time::Duration;

use axum::Router;
use axum::extract::Query;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum_otel_metrics::HttpMetricsLayerBuilder;
use axum_tracing_opentelemetry::middleware::{OtelAxumLayer, OtelInResponseLayer};
use opentelemetry::global;
use opentelemetry::metrics::Counter;
use rand::RngExt;
use serde::Deserialize;

#[tokio::main]
async fn main() {
    let providers = telemetry::init();

    let app = Router::new()
        .route("/hello", get(hello))
        .route("/slow", get(slow))
        .route("/flaky", get(flaky))
        // http.server.request.duration, .request.body.size, .response.body.size
        // and .active_requests, labelled per the OTel semantic conventions.
        .layer(HttpMetricsLayerBuilder::new().build())
        // Puts the trace id in the response headers, so a caller can look up the
        // trace for a request they just made.
        .layer(OtelInResponseLayer)
        // Outermost: reads any incoming traceparent and opens the server span
        // that every log line and child span below it attaches to.
        .layer(OtelAxumLayer::default());

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .expect("port 8080 is free");
    tracing::info!("listening on http://0.0.0.0:8080");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("server error");

    providers.shutdown();
}

/// Fast and always 200. The baseline every other endpoint is compared against.
async fn hello() -> &'static str {
    tracing::info!("hello");
    "hello\n"
}

#[derive(Deserialize)]
struct SlowParams {
    ms: Option<u64>,
}

/// Sleeps for a random or given time, so the latency histogram has a spread
/// instead of a single spike.
async fn slow(Query(params): Query<SlowParams>) -> String {
    let ms = params
        .ms
        .unwrap_or_else(|| rand::rng().random_range(10..1500));
    work(ms).await;
    tracing::info!(ms, "slow response");
    format!("slept {ms}ms\n")
}

/// A nested set of spans that makes the trace waterfall useful to inspect.
#[tracing::instrument]
async fn work(ms: u64) {
    let edge_ms = ms / 5;
    let execute_ms = ms - edge_ms * 2;

    prepare_work(edge_ms).await;
    execute_work(execute_ms).await;
    finalize_work(edge_ms).await;
}

#[tracing::instrument(name = "work.prepare")]
async fn prepare_work(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tracing::instrument(name = "work.execute")]
async fn execute_work(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tracing::instrument(name = "work.finalize")]
async fn finalize_work(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// A metric defined by the application rather than the HTTP middleware. Built
/// once: creating an instrument per request would allocate on every call.
static FLAKY_FAILURES: LazyLock<Counter<u64>> = LazyLock::new(|| {
    global::meter("monitoring-demo")
        .u64_counter("demo.flaky.failures")
        .with_description("Requests to /flaky that were failed on purpose.")
        .build()
});

#[derive(Deserialize)]
struct FlakyParams {
    fail_rate: Option<f64>,
}

/// Fails a share of requests, so error rates and failed traces have something
/// to show.
async fn flaky(Query(params): Query<FlakyParams>) -> Response {
    let fail_rate = params.fail_rate.unwrap_or(0.3).clamp(0.0, 1.0);
    if rand::rng().random_bool(fail_rate) {
        FLAKY_FAILURES.add(1, &[]);
        tracing::warn!(fail_rate, "flaky failure");
        return (StatusCode::INTERNAL_SERVER_ERROR, "boom\n").into_response();
    }
    "ok\n".into_response()
}
