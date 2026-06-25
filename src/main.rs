mod app;
mod clipboard;
mod config;
mod event;
mod input;
mod pty;
mod render;
mod tabs;

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("sherion=info".parse()?))
        .init();

    let config = config::Config::load()?;
    app::App::run(config)
}
