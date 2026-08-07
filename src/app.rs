use std::{
    ffi::OsStr,
    fs::{OpenOptions, create_dir_all},
    io::{self, Write},
    path::PathBuf,
    str::FromStr,
};

use anyhow::{Context, Result, bail};
use clap::builder::StyledStr;
use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};
use etcetera::{AppStrategy, AppStrategyArgs, choose_app_strategy};
use url::Url;

use crate::{
    brew::{BrewData, BrewDevice, BrewMethod, DeviceLabel},
    db::Database,
    edit::{Builder, edit_with_builder},
    label::{CoffeeType, Label},
    print::{render_json, render_svg, render_text, render_toml},
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
    /// Add a new coffee.
    Add {
        /// Label category for the new coffee.
        #[arg(value_enum)]
        kind: CoffeeType,
    },
    /// Edit an existing coffee.
    Edit {
        /// Coffee label, such as F3.
        #[arg(add = ArgValueCompleter::new(complete_labels))]
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
    /// Get a human-readable description of an existing coffee.
    Get {
        /// Coffee label, such as F3.
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
        /// Output format.
        #[arg(short, long, value_enum, default_value_t)]
        format: OutputFormat,
        /// Write the output to a file instead of standard output.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Add and list coffee brews.
    Brew {
        #[command(subcommand)]
        command: BrewCommand,
    },
}

#[derive(Debug, Subcommand)]
enum BrewCommand {
    /// Add a brew using an editor.
    Add {
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
        #[arg(add = ArgValueCompleter::new(complete_devices))]
        device: DeviceLabel,
    },
    /// List brews for a coffee, newest first.
    List {
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        let path = database_path(self.database)?;
        if let Some(parent) = path.parent() {
            create_dir_all(parent)
                .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
        }
        let mut database = Database::open(&path)?;

        match self.command {
            Command::Add { kind } => add(&mut database, kind),
            Command::Edit { label } => edit(&mut database, label),
            Command::List { template, flavours } => list(&database, &template, flavours),
            Command::Get { label, format, out } => get(&database, label, format, out),
            Command::Brew { command } => match command {
                BrewCommand::Add { label, device } => add_brew(&mut database, label, device),
                BrewCommand::List { label } => list_brews(&database, label),
            },
        }
    }
}

fn complete_devices(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Some(path) = completion_database_path() else {
        return Vec::new();
    };
    let Ok(database) = Database::open_read_only(&path) else {
        return Vec::new();
    };
    database
        .list_devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.label.as_str().starts_with(current))
        .map(|d| {
            CompletionCandidate::new(d.label.to_string())
                .help(Some(StyledStr::from(format!("{} ({})", d.name, d.method))))
        })
        .collect()
}

fn add_brew(database: &mut Database, coffee: Label, requested: DeviceLabel) -> Result<()> {
    if database.get(coffee)?.is_none() {
        bail!("coffee '{coffee}' does not exist");
    }
    let (initial_method, pending) = match database.device(&requested)? {
        Some(device) => (device.method, None),
        None => {
            eprintln!("Device '{requested}' is not registered.");
            let name = loop {
                let value = prompt("Descriptive name")?;
                if !value.is_empty() {
                    break value;
                }
                eprintln!("Device name must not be empty.");
            };
            let method = loop {
                let value = prompt("Method (pourover, immersion, or espresso)")?;
                match value.parse() {
                    Ok(method) => break method,
                    Err(error) => eprintln!("Invalid method: {error}"),
                }
            };
            let device = BrewDevice {
                label: requested.clone(),
                name,
                method,
            };
            if !confirm(
                &format!(
                    "Register device '{}' as '{}' ({})?",
                    device.label, device.name, device.method
                ),
                false,
            )? {
                bail!("device registration cancelled");
            }
            (method, Some(device))
        }
    };
    let mut source = BrewData::draft(&requested, initial_method);
    loop {
        let Some((data, resolved_pending)) =
            edit_brew_record(database, &source, initial_method, pending.as_ref())?
        else {
            bail!("no brew was added");
        };
        if !database.grinder_exists(&data.grinder)?
            && !confirm(&format!("Register grinder '{}'?", data.grinder), false)?
        {
            source = data.to_toml()?;
            continue;
        }
        let register_grinder = !database.grinder_exists(&data.grinder)?;
        let brew = database.add_brew(coffee, data, resolved_pending, register_grinder)?;
        print!("{}", brew.to_toml()?);
        return Ok(());
    }
}

fn edit_brew_record<'a>(
    database: &Database,
    source: &str,
    initial_method: BrewMethod,
    pending: Option<&'a BrewDevice>,
) -> Result<Option<(BrewData, Option<&'a BrewDevice>)>> {
    let mut builder = Builder::new();
    builder.suffix(".toml");
    let mut response = source.to_owned();
    loop {
        let user_text = edit_with_builder(&response, &builder).context("failed to edit brew")?;
        if user_text == response {
            return Ok(None);
        }
        let attempt = (|| -> Result<(BrewData, Option<&BrewDevice>)> {
            let table: toml::Table = toml::from_str(&user_text)?;
            let raw = table
                .get("device")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("'device' must be a string"))?;
            let label: DeviceLabel = raw.parse()?;
            let (method, registration) = if pending.is_some_and(|d| d.label == label) {
                (initial_method, pending)
            } else {
                let device = database.device(&label)?.ok_or_else(|| {
                    anyhow::anyhow!("replacement device '{label}' is not registered")
                })?;
                (device.method, None)
            };
            let data = BrewData::parse_for_method(&user_text, method)
                .with_context(|| format!("metadata does not match device method '{method}'"))?;
            Ok((data, registration))
        })();
        match attempt {
            Ok(value) => return Ok(Some(value)),
            Err(error) => eprintln!("Contents invalid: {error:#}"),
        }
        if !confirm("Continue editing?", true)? {
            return Ok(None);
        }
        response = user_text;
    }
}

fn list_brews(database: &Database, coffee: Label) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    for brew in database.list_brews(coffee)? {
        writeln!(output, "[[brews]]")?;
        writeln!(output, "{}", brew.to_toml()?.trim_end())?;
    }
    Ok(())
}

fn default_database_path() -> Result<PathBuf> {
    let strategy = choose_app_strategy(AppStrategyArgs {
        top_level_domain: "org".into(),
        author: env!("CARGO_PKG_NAME").into(),
        app_name: env!("CARGO_PKG_NAME").into(),
    })?;
    Ok(strategy.data_dir().join("coffeedb.db"))
}

fn database_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    explicit.map_or_else(default_database_path, Ok)
}

fn completion_database_path() -> Option<PathBuf> {
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "-D" || argument == "--database" {
            return arguments.next().map(PathBuf::from);
        }
        let Some(argument) = argument.to_str() else {
            continue;
        };
        if let Some(path) = argument.strip_prefix("--database=") {
            return Some(path.into());
        }
        if let Some(path) = argument.strip_prefix("-D").filter(|path| !path.is_empty()) {
            return Some(path.into());
        }
    }

    std::env::var_os("COFFEEDB_DATABASE_PATH")
        .map(PathBuf::from)
        .or_else(|| default_database_path().ok())
}

fn complete_labels(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Some(path) = completion_database_path() else {
        return Vec::new();
    };
    let Ok(database) = Database::open_read_only(&path) else {
        return Vec::new();
    };
    let Ok(template) = CoffeeTemplate::from_str(r#"{%name} (by {%roaster})"#) else {
        return Vec::new();
    };
    let Ok(coffees) = database.list() else {
        return Vec::new();
    };

    coffees
        .into_iter()
        .filter(|coffee| coffee.label.to_string().starts_with(current))
        .filter_map(|coffee| {
            let help = template.render(&coffee).ok()?;
            Some(
                CompletionCandidate::new(coffee.label.to_string())
                    .help(Some(StyledStr::from(help))),
            )
        })
        .collect()
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
    for coffee in database.list()? {
        writeln!(output, "{}", template.render(&coffee)?.trim_end())?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Text,
    Svg,
    Toml,
    Json,
}

fn get(
    database: &Database,
    label: Label,
    format: OutputFormat,
    out: Option<PathBuf>,
) -> Result<()> {
    let Some(coffee) = database.get(label)? else {
        bail!("coffee '{label}' does not exist");
    };
    let rendered = match format {
        OutputFormat::Text => render_text(&coffee),
        OutputFormat::Svg => render_svg(&coffee),
        OutputFormat::Toml => render_toml(&coffee)?,
        OutputFormat::Json => render_json(&coffee)?,
    };
    if let Some(path) = out {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("failed to create output file '{}'", path.display()))?;
        write!(output, "{rendered}")
            .with_context(|| format!("failed to write output file '{}'", path.display()))?;
    } else {
        let stdout = io::stdout();
        write!(stdout.lock(), "{rendered}")?;
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
