use anyhow::Result;
use tracing_subscriber::EnvFilter;

fn parse_args() -> Option<String> {
    let mut args = std::env::args().skip(1);
    let mut profile = None;
    while let Some(arg) = args.next() {
        if arg == "--profile" {
            profile = args.next();
        }
    }
    profile
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("sherion=info".parse()?))
        .init();

    let profile = parse_args();
    let config = sherion::config::Config::load_profile(profile.as_deref())?;
    sherion::app::App::run(config)
}
