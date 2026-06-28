#![allow(clippy::too_many_arguments)]

use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("sherion=info".parse()?))
        .init();

    let config = sherion::config::Config::load()?;
    sherion::app::App::run(config)
}
