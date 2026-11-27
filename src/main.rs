use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing with both stdout and in-memory ring buffer for GUI logs
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lanio=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().without_time())
        .with(lanio::telemetry::MemoryLogLayer)
        .init();

    lanio::run(None).await
}
