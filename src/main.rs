mod app;
mod db;
mod edit;
mod label;
mod record;
mod template;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    app::Cli::parse().run()
}
