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
use serde::{
    Serialize, Serializer,
    ser::{SerializeSeq, SerializeStruct},
};
use url::Url;

use crate::{
    brew::{BrewData, BrewDevice, BrewMethod, DeviceLabel, Grinder},
    db::{Database, MapBrewsError},
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

    /// Use a specific coffeedb database file.
    #[arg(short = 'D', long, env = "COFFEEDB_DATABASE_PATH", value_name = "PATH")]
    database: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Add, inspect, edit, and list coffee beans.
    Bean {
        #[command(subcommand)]
        command: BeanCommand,
    },
    /// Add and list coffee brews.
    Brew {
        #[command(subcommand)]
        command: BrewCommand,
    },
}

#[derive(Debug, Subcommand)]
enum BeanCommand {
    /// Add a new coffee.
    Add {
        /// The type of coffee.
        #[arg(value_enum)]
        kind: CoffeeType,
    },
    /// Edit an existing coffee.
    Edit {
        /// Coffee label.
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
        /// Exclude archived coffees.
        #[arg(long, conflicts_with = "flavours")]
        active: bool,
    },
    /// Get a human-readable description of an existing coffee.
    Get {
        /// Coffee label.
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
        /// Output format.
        #[arg(short, long, value_enum, default_value_t)]
        format: OutputFormat,
        /// Write the output to a file instead of standard output.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Archive a coffee.
    Archive {
        /// Coffee label.
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
        /// Restore the coffee to active status.
        #[arg(long)]
        unarchive: bool,
    },
}

#[derive(Debug, Subcommand)]
enum BrewCommand {
    /// Add a brew using an editor.
    Add {
        /// The bean used in the brew.
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Label,
        /// The device used for brewing.
        #[arg(add = ArgValueCompleter::new(complete_devices))]
        device: DeviceLabel,
    },
    /// Edit an existing brew.
    Edit {
        /// Brew number as shown by `brew list`.
        #[arg(add = ArgValueCompleter::new(complete_brews))]
        id: i64,
    },
    /// List brews.
    List {
        #[arg(add = ArgValueCompleter::new(complete_labels))]
        label: Option<Label>,
        /// Output format.
        #[arg(short, long, value_enum, default_value_t)]
        format: BrewListFormat,
        /// Show only the N most recent brews.
        #[arg(long, value_name = "N")]
        limit: Option<u32>,
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
            Command::Bean { command } => match command {
                BeanCommand::Add { kind } => add(&mut database, kind),
                BeanCommand::Edit { label } => edit(&mut database, label),
                BeanCommand::List {
                    template,
                    flavours,
                    active,
                } => list(&database, &template, flavours, active),
                BeanCommand::Get { label, format, out } => get(&database, label, format, out),
                BeanCommand::Archive { label, unarchive } => {
                    database.set_archived(label, !unarchive)
                }
            },
            Command::Brew { command } => match command {
                BrewCommand::Add { label, device } => add_brew(&mut database, label, device),
                BrewCommand::Edit { id } => edit_brew(&mut database, id),
                BrewCommand::List {
                    label,
                    format,
                    limit,
                } => list_brews(&database, label, format, limit.map(i64::from).unwrap_or(-1)),
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

fn complete_brews(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Some(path) = completion_database_path() else {
        return Vec::new();
    };
    let Ok(database) = Database::open_read_only(&path) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    if database
        .map_brews(None, -1, |brew| {
            let id = brew.id.to_string();
            if id.starts_with(current) {
                let method = brew_method_title(brew.data.metadata.method());
                let help = format!(
                    "{} {method} brew for #{}",
                    brew.data.occurred.format("%Y-%m-%d"),
                    brew.coffee
                );
                candidates.push(CompletionCandidate::new(id).help(Some(StyledStr::from(help))));
            }
            Ok::<(), std::convert::Infallible>(())
        })
        .is_err()
    {
        return Vec::new();
    }
    candidates
}

fn add_brew(database: &mut Database, coffee: Label, requested: DeviceLabel) -> Result<()> {
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
                true,
            )? {
                bail!("device registration cancelled");
            }
            (method, Some(device))
        }
    };
    let mut source = BrewData::draft(&requested, initial_method);
    'editing: loop {
        let Some((data, resolved_pending)) =
            edit_brew_record(database, &source, initial_method, pending.as_ref(), None)?
        else {
            bail!("no brew was added");
        };
        let new_grinder = if database.grinder(&data.grinder)?.is_none() {
            eprintln!("Grinder '{}' is not registered.", data.grinder);
            loop {
                let value = prompt("Descriptive name (or 'edit' to edit the brew again)")?;
                if matches!(value.as_str(), "e" | "edit") {
                    source = data.to_toml()?;
                    continue 'editing;
                }
                if !value.is_empty() {
                    let grinder = Grinder {
                        label: data.grinder.clone(),
                        name: value,
                    };
                    if confirm(
                        &format!(
                            "Register grinder '{}' as '{}'?",
                            grinder.label, grinder.name
                        ),
                        true,
                    )? {
                        break Some(grinder);
                    }
                    source = data.to_toml()?;
                    continue 'editing;
                }
                eprintln!("Grinder name must not be empty.");
            }
        } else {
            None
        };
        let brew = database.add_brew(coffee, data, resolved_pending, new_grinder.as_ref())?;
        let stdout = io::stdout();
        render_brew_text(stdout.lock(), database, &brew)?;
        return Ok(());
    }
}

fn edit_brew(database: &mut Database, id: i64) -> Result<()> {
    let Some(current) = database.brew(id)? else {
        bail!("brew '{id}' does not exist");
    };
    let source = current.data.to_toml()?;
    let initial_method = current.data.metadata.method();
    let Some((data, _)) =
        edit_brew_record(database, &source, initial_method, None, Some(&current.data))?
    else {
        bail!("no data was changed");
    };
    let brew = database.update_brew(id, data)?;
    let stdout = io::stdout();
    render_brew_text(stdout.lock(), database, &brew)?;
    Ok(())
}

fn edit_brew_record<'a>(
    database: &Database,
    source: &str,
    initial_method: BrewMethod,
    pending: Option<&'a BrewDevice>,
    original: Option<&BrewData>,
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
            if original.is_some() && database.grinder(&data.grinder)?.is_none() {
                bail!("replacement grinder '{}' is not registered", data.grinder);
            }
            Ok((data, registration))
        })();
        match attempt {
            Ok(value) if original == Some(&value.0) => {
                eprintln!("Text edited but contents unchanged.");
            }
            Ok(value) => return Ok(Some(value)),
            Err(error) => eprintln!("Contents invalid: {error:#}"),
        }
        if !confirm("Continue editing?", true)? {
            return Ok(None);
        }
        response = user_text;
    }
}

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum BrewListFormat {
    #[default]
    Text,
    Toml,
    Json,
}

fn list_brews(
    database: &Database,
    coffee: Option<Label>,
    format: BrewListFormat,
    limit: i64,
) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    let log = BrewLog {
        database,
        coffee,
        limit,
    };
    match format {
        BrewListFormat::Text => {
            let mut first = true;
            database
                .map_brews(coffee, limit, |brew| {
                    if !first {
                        writeln!(output)?;
                    }
                    first = false;
                    render_brew_text(&mut output, database, &brew)
                })
                .map_err(|error| match error {
                    MapBrewsError::Callback(error) | MapBrewsError::Other(error) => error,
                })?;
        }
        BrewListFormat::Toml => {
            let mut buffer = toml::ser::Buffer::new();
            log.serialize(toml::Serializer::pretty(&mut buffer))?;
            write!(output, "{buffer}")?;
        }
        BrewListFormat::Json => {
            serde_json::to_writer_pretty(&mut output, &JsonBrewLog(&log))?;
            writeln!(output)?;
        }
    }
    Ok(())
}

fn render_brew_text<W: io::Write>(
    mut writer: W,
    database: &Database,
    brew: &crate::brew::Brew,
) -> Result<()> {
    use crate::brew::BrewMetadata;

    let device = database
        .device(&brew.data.device)?
        .ok_or_else(|| anyhow::anyhow!("device '{}' does not exist", brew.data.device))?;
    let grinder = database
        .grinder(&brew.data.grinder)?
        .ok_or_else(|| anyhow::anyhow!("grinder '{}' does not exist", brew.data.grinder))?;
    let method = brew_method_title(brew.data.metadata.method());
    writeln!(writer, "> {}", brew.data.occurred.format("%B %-d, %Y"))?;
    writeln!(
        writer,
        "= {method} brew for #{} (brew {})",
        brew.coffee, brew.id
    )?;
    match &brew.data.metadata {
        BrewMetadata::Immersion(metadata) => writeln!(
            writer,
            "· {:.1}g / {:.0}ml in a {} @ {:.0}°C",
            brew.data.coffee_grams, metadata.volume_ml, device.name, brew.data.temperature_c
        )?,
        _ => writeln!(
            writer,
            "· {:.1}g in a {} @ {:.0}°C",
            brew.data.coffee_grams, device.name, brew.data.temperature_c
        )?,
    }
    writeln!(writer, "· {} @ {}", grinder.name, brew.data.grind_setting)?;
    match &brew.data.metadata {
        BrewMetadata::Espresso(metadata) => writeln!(
            writer,
            "· {:.1}g out (ratio {:.1}) in {:.0}s",
            metadata.beverage_grams,
            metadata.beverage_grams / brew.data.coffee_grams,
            metadata.duration_seconds
        )?,
        BrewMetadata::Immersion(metadata) => writeln!(
            writer,
            "· {:.0}s brew, {:.0}s drawdown",
            metadata.immersion_seconds, metadata.drawdown_seconds
        )?,
        BrewMetadata::Pourover(metadata) => {
            write!(
                writer,
                "· {} {}: ",
                metadata.pours.len(),
                if metadata.pours.len() == 1 {
                    "Pour"
                } else {
                    "Pours"
                }
            )?;
            for (index, &(elapsed, volume)) in metadata.pours.iter().enumerate() {
                if index != 0 {
                    write!(writer, ", ")?;
                }
                write!(writer, "{volume:.0}ml @ {elapsed:.0}s")?;
            }
            write!(writer, "\n")?;
        }
    }
    if let Some(notes) = &brew.data.notes {
        for line in notes.trim().lines() {
            writeln!(writer, "| {}", line.trim())?;
        }
    }
    Ok(())
}

fn brew_method_title(method: BrewMethod) -> &'static str {
    match method {
        BrewMethod::Pourover => "Pourover",
        BrewMethod::Immersion => "Immersion",
        BrewMethod::Espresso => "Espresso",
    }
}

struct BrewLog<'a> {
    database: &'a Database,
    coffee: Option<Label>,
    limit: i64,
}

struct BrewSequence<'a>(&'a BrewLog<'a>);
struct JsonBrewLog<'a>(&'a BrewLog<'a>);

impl Serialize for BrewLog<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("BrewLog", 1)?;
        state.serialize_field("brews", &BrewSequence(self))?;
        state.end()
    }
}

impl Serialize for BrewSequence<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut sequence = serializer.serialize_seq(None)?;
        self.0
            .database
            .map_brews(self.0.coffee, self.0.limit, |brew| {
                sequence.serialize_element(&brew)?;
                Ok::<(), S::Error>(())
            })
            .map_err(serde::ser::Error::custom)?;
        sequence.end()
    }
}

impl Serialize for JsonBrewLog<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use crate::brew::JsonBrew;

        struct JsonSequence<'a>(&'a BrewLog<'a>);
        impl Serialize for JsonSequence<'_> {
            fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                let mut sequence = serializer.serialize_seq(None)?;
                self.0
                    .database
                    .map_brews(self.0.coffee, self.0.limit, |brew| {
                        sequence.serialize_element(&JsonBrew(&brew))?;
                        Ok::<(), S::Error>(())
                    })
                    .map_err(serde::ser::Error::custom)?;
                sequence.end()
            }
        }

        let mut state = serializer.serialize_struct("BrewLog", 1)?;
        state.serialize_field("brews", &JsonSequence(self.0))?;
        state.end()
    }
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
    let Ok(coffees) = database.list_active() else {
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

fn list(
    database: &Database,
    template: &CoffeeTemplate,
    flavours: bool,
    active: bool,
) -> Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if flavours {
        for flavour in database.list_flavours()? {
            writeln!(output, "{flavour}")?;
        }
        return Ok(());
    }
    let coffees = if active {
        database.list_active()?
    } else {
        database.list()?
    };
    for coffee in coffees {
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
    if let Some(path) = out {
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| format!("failed to create output file '{}'", path.display()))?;
        render_output(&mut output, &coffee, format)
            .with_context(|| format!("failed to write output file '{}'", path.display()))?;
    } else {
        let stdout = io::stdout();
        render_output(stdout.lock(), &coffee, format)?;
    }
    Ok(())
}

fn render_output<W: io::Write>(
    mut writer: W,
    coffee: &crate::record::Coffee,
    format: OutputFormat,
) -> Result<()> {
    match format {
        OutputFormat::Text => render_text(writer, coffee)?,
        OutputFormat::Svg => render_svg(writer, coffee)?,
        OutputFormat::Toml => write!(writer, "{}", render_toml(coffee)?)?,
        OutputFormat::Json => render_json(writer, coffee)?,
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
