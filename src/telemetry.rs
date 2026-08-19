use std::time::Duration;

use opentelemetry::global;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_appender_tracing::layer::OpenTelemetryTracingBridge;
use opentelemetry_otlp::{LogExporter, MetricExporter, SpanExporter};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::SdkLoggerProvider;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const SERVICE_NAME: &str = "monitoring-demo";

/// Owns the three signal providers. Dropping the process without calling
/// [`Providers::shutdown`] loses whatever is still sitting in the batch queues.
pub struct Providers {
    traces: SdkTracerProvider,
    metrics: SdkMeterProvider,
    logs: SdkLoggerProvider,
}

/// Wires up traces, metrics and logs over OTLP, plus stdout logging.
///
/// Everything goes to one collector endpoint, taken from
/// `OTEL_EXPORTER_OTLP_ENDPOINT` (default `http://localhost:4317`), so the
/// backend is a deployment decision rather than a code change.
pub fn init() -> Providers {
    // Attached to every span, metric and log, and how the backend tells services
    // apart.
    let resource = Resource::builder().with_service_name(SERVICE_NAME).build();

    let traces = SdkTracerProvider::builder()
        .with_resource(resource.clone())
        .with_batch_exporter(
            SpanExporter::builder()
                .with_tonic()
                .build()
                .expect("span exporter"),
        )
        .build();

    let metrics = SdkMeterProvider::builder()
        .with_resource(resource.clone())
        .with_reader(
            PeriodicReader::builder(
                MetricExporter::builder()
                    .with_tonic()
                    .build()
                    .expect("metric exporter"),
            )
            // Metrics are pushed on this interval instead of being scraped.
            // 5s is demo-fast; 30-60s is normal.
            .with_interval(Duration::from_secs(5))
            .build(),
        )
        .build();

    let logs = SdkLoggerProvider::builder()
        .with_resource(resource)
        .with_batch_exporter(
            LogExporter::builder()
                .with_tonic()
                .build()
                .expect("log exporter"),
        )
        .build();

    // Reads and writes W3C `traceparent` headers, so a trace survives a hop
    // between services. Without it the SDK propagates nothing.
    global::set_text_map_propagator(TraceContextPropagator::new());
    global::set_tracer_provider(traces.clone());
    global::set_meter_provider(metrics.clone());

    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                // otel::tracing=trace is required, not optional: the HTTP server
                // span is created at TRACE level on that target, and without the
                // directive every request is traced as SpanDisabled.
                .unwrap_or_else(|_| "info,monitoring_demo=debug,otel::tracing=trace".into()),
        )
        // Human-readable logs in the terminal.
        .with(tracing_subscriber::fmt::layer())
        // The same tracing spans, exported as OTLP traces.
        .with(tracing_opentelemetry::layer().with_tracer(traces.tracer(SERVICE_NAME)))
        // The same tracing events, exported as OTLP logs. This is what puts a
        // trace id on every log line without any extra work in the handlers.
        .with(OpenTelemetryTracingBridge::new(&logs))
        .init();

    Providers {
        traces,
        metrics,
        logs,
    }
}

impl Providers {
    /// Flushes anything still queued. Without this a short-lived process reports
    /// nothing at all.
    pub fn shutdown(&self) {
        let _ = self.traces.shutdown();
        let _ = self.metrics.shutdown();
        let _ = self.logs.shutdown();
    }
}
