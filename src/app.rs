use std::{
    fs::create_dir_all,
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use url::Url;

use crate::{
    db::Database,
    edit::{Builder, edit_with_builder},
    label::{CoffeeType, Label},
    record::CoffeeData,
    template::{CoffeeTemplate, DEFAULT_TEMPLATE},
};

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Use a specific SQLite database file.
    #[arg(
        short = 'D',
        long,
        env = "COFFEEDB_DATABASE_PATH",
        global = true,
        value_name = "PATH"
    )]
    database: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add a purchased coffee.
    Add {
        /// Label category for the new coffee.
        #[arg(value_enum)]
        kind: CoffeeType,
    },
    /// Edit an existing coffee.
    Edit {
        /// Coffee label, such as F3.
        label: Label,
    },
    /// List all coffees.
    List {
        /// Format template using Coffee column names as keys.
        #[arg(short, long, default_value = DEFAULT_TEMPLATE, conflicts_with = "flavours")]
        template: CoffeeTemplate,
        /// List registered flavours instead of coffees.
        #[arg(long)]
        flavours: bool,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        let path = match self.database {
            Some(path) => path,
            None => {
                let strategy = choose_app_strategy(AppStrategyArgs {
                    top_level_domain: "org".into(),
                    author: env!("CARGO_PKG_NAME").into(),
                    app_name: env!("CARGO_PKG_NAME").into(),
                })?;
                strategy.data_dir().join("coffeedb.db")
            }
        };
        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
        }
        let mut database = Database::open(&path)?;

        match self.command {
            Command::Add { kind } => add(&mut database, kind),
            Command::Edit { label } => edit(&mut database, label),
            Command::List { template, flavours } => list(&database, &template, flavours),
        }
    }
}

fn add(database: &mut Database, kind: CoffeeType) -> Result<()> {
    let Some(mut data) = edit_record(CoffeeData::draft(), None)? else {
        bail!("no data was added");
    };
    loop {
        match resolve_roaster(database, &data.roaster)? {
            RoasterChoice::Accept(url) => {
                let coffee = database.add(kind, data, url.as_ref())?;
                print!("{coffee}");
                return Ok(());
            }
            RoasterChoice::Edit => {
                let source = data.to_toml(None)?;
                let Some(edited) = edit_record(&source, Some(&data))? else {
                    bail!("no data was added");
                };
                data = edited;
            }
        }
    }
}

fn edit(database: &mut Database, label: Label) -> Result<()> {
    let Some(current) = database.get(label)? else {
        bail!("coffee '{label}' does not exist");
    };
    let source = current.data.to_toml(Some(label))?;
    let Some(mut data) = edit_record(&source, Some(&current.data))? else {
        bail!("no data was changed");
    };
    loop {
        match resolve_roaster(database, &data.roaster)? {
            RoasterChoice::Accept(url) => {
                let coffee = database.update(label, data, url.as_ref())?;
                print!("{coffee}");
                return Ok(());
            }
            RoasterChoice::Edit => {
                let source = data.to_toml(Some(label))?;
                let Some(edited) = edit_record(&source, Some(&data))? else {
                    bail!("no data was changed");
                };
                data = edited;
            }
        }
    }
}

fn list(database: &Database, template: &CoffeeTemplate, flavours: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if flavours {
        for flavour in database.list_flavours()? {
            writeln!(output, "{flavour}")?;
        }
        return Ok(());
    }
    for coffee in database.list(template.uses_flavours())? {
        writeln!(output, "{}", template.render(&coffee)?.trim_end())?;
    }
    Ok(())
}

fn edit_record(source: &str, original: Option<&CoffeeData>) -> Result<Option<CoffeeData>> {
    let mut builder = Builder::new();
    builder.suffix(".toml");
    let mut response = source.to_owned();
    loop {
        let user_text = edit_with_builder(&response, &builder).context("failed to edit record")?;
        if user_text == response {
            return Ok(None);
        }
        match CoffeeData::from_str(&user_text) {
            Ok(data) if original == Some(&data) => {
                eprintln!("Text edited but contents unchanged.");
            }
            Ok(data) => return Ok(Some(data)),
            Err(error) => eprintln!("Contents invalid: {error:#}"),
        }
        if !confirm("Continue editing?", true)? {
            return Ok(None);
        }
        response = user_text;
    }
}

enum RoasterChoice {
    Accept(Option<Url>),
    Edit,
}

fn resolve_roaster(database: &Database, roaster: &str) -> Result<RoasterChoice> {
    if database.roaster_url(roaster)?.is_some() {
        return Ok(RoasterChoice::Accept(None));
    }
    eprintln!("Roaster '{roaster}' does not exist and will be created.");
    loop {
        let input = prompt("URL (blank for none, or 'edit' to edit the coffee again)")?;
        if matches!(input.as_str(), "e" | "edit") {
            return Ok(RoasterChoice::Edit);
        }
        if input.is_empty() {
            return Ok(RoasterChoice::Accept(None));
        }
        match Url::parse(&input) {
            Ok(url) => return Ok(RoasterChoice::Accept(Some(url))),
            Err(error) => eprintln!("Invalid URL: {error}"),
        }
    }
}

fn prompt(message: &str) -> Result<String> {
    eprint!("{message}: ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    input.truncate(input.trim_end().len());
    Ok(input)
}

fn confirm(message: &str, default: bool) -> Result<bool> {
    let suffix = if default { "[Y]/n" } else { "y/[N]" };
    let response = prompt(&format!("{message} {suffix}"))?;
    Ok(match response.as_str() {
        "y" | "Y" => true,
        "n" | "N" => false,
        "" => default,
        _ => false,
    })
}
