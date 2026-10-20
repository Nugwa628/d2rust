//! `report-service`: runs the bug report intake service.
//!
//! Settings come from the environment (`report_service::Config::from_env`),
//! plus `REPORT_SERVICE_ADDR` for the listen address (default
//! `127.0.0.1:8787`). It is meant to sit behind an HTTPS front end; it
//! speaks plain HTTP itself and holds no secrets in the repository.

use std::net::SocketAddr;

use report_service::{system_clock, Config, Service};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let addr: SocketAddr = std::env::var("REPORT_SERVICE_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()?;
    let app = Service::open(config, system_clock())?.router();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("report-service: listening on {addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
