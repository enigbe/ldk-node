use std::pin::Pin;
use std::sync::Arc;
use std::{collections::HashMap, future::Future};

use opentelemetry::propagation::Injector;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{SpanExporter as OtlpExporter, WithExportConfig};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider, Resource};
use opentelemetry_stdout::SpanExporter as StdoutExporter;

use tracing::{debug, error, info, level_filters::LevelFilter, trace, warn};
use tracing_opentelemetry::{OpenTelemetryLayer, OpenTelemetrySpanExt};
use tracing_subscriber::{filter::Targets, fmt, layer::SubscriberExt, util::SubscriberInitExt};
use vss_client::headers::{VssHeaderProvider, VssHeaderProviderError};

use crate::logger::{LogRecord, LogWriter};

/// Adapter that bridges our `Logger` to the `tracing` ecosystem.
///
/// This allows existing `log_*!` macros to emit events as tracing spans/events
/// without requiring a migration to tracing-specific macros.
pub struct TracingLogWriter {}

impl LogWriter for TracingLogWriter {
	fn log(&self, record: LogRecord) {
		match record.level {
			lightning::util::logger::Level::Gossip => {
				trace!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
			lightning::util::logger::Level::Trace => {
				trace!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
			lightning::util::logger::Level::Debug => {
				debug!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
			lightning::util::logger::Level::Info => {
				info!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
			lightning::util::logger::Level::Warn => {
				warn!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
			lightning::util::logger::Level::Error => {
				error!(target: "ldk_node", module = record.module_path, line = record.line, "{}", record.args)
			},
		}
	}
}

pub(crate) struct TracingHeaderProvider {
	inner: Arc<dyn VssHeaderProvider>,
	propagator: TraceContextPropagator,
}

impl TracingHeaderProvider {
	pub fn new(inner: Arc<dyn VssHeaderProvider>) -> Self {
		Self { inner, propagator: TraceContextPropagator::new() }
	}
}

impl VssHeaderProvider for TracingHeaderProvider {
	fn get_headers<'life0, 'life1, 'async_trait>(
		&'life0 self, request: &'life1 [u8],
	) -> Pin<
		Box<
			dyn Future<Output = Result<HashMap<String, String>, VssHeaderProviderError>>
				+ Send
				+ 'async_trait,
		>,
	>
	where
		'life0: 'async_trait,
		'life1: 'async_trait,
		Self: 'async_trait,
	{
		let inner = Arc::clone(&self.inner);
		let request = request.to_vec();
		let propagator = self.propagator.clone();

		Box::pin(async move {
			let mut headers = inner.get_headers(&request).await?;

			let cx = tracing::Span::current().context();
			propagator.inject_context(&cx, &mut HeaderInjector(&mut headers));

			Ok(headers)
		})
	}
}

struct HeaderInjector<'a>(&'a mut HashMap<String, String>);

impl Injector for HeaderInjector<'_> {
	fn set(&mut self, key: &str, value: String) {
		self.0.insert(key.to_string(), value);
	}
}

/// Initialize tracing subscriber for Jaeger and `stdout` backends.
pub fn configure_tracer() {
	let otlp_jaeger_exporter = OtlpExporter::builder()
		.with_tonic()
		.with_endpoint("http://localhost:4317")
		.build()
		.expect("Failed to create OTLP exporter");
	let stdout_exporter = StdoutExporter::default();

	let tracer_provider = SdkTracerProvider::builder()
		.with_batch_exporter(otlp_jaeger_exporter)
		.with_batch_exporter(stdout_exporter)
		.with_resource(Resource::builder().with_service_name("ldk-node").build())
		.build();

	let tracer = tracer_provider.tracer("ldk_node");

	tracing_subscriber::registry()
		.with(
			Targets::new()
				.with_default(LevelFilter::WARN)
				.with_target("ldk_node", LevelFilter::INFO),
		)
		.with(fmt::layer().json())
		.with(OpenTelemetryLayer::new(tracer))
		.init();
}
