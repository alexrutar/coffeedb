mod app;
mod db;
mod edit;
mod label;
mod record;
mod template;

use anyhow::Result;
use clap::{CommandFactory, Parser};

fn main() -> Result<()> {
    clap_complete::CompleteEnv::with_factory(app::Cli::command).complete();
    app::Cli::parse().run()
}
