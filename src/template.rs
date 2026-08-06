use std::{convert::Infallible, fmt, str::FromStr};

use anyhow::{Result, anyhow};
use mufmt::{Ast, Manifest};

use crate::record::Coffee;

pub const DEFAULT_TEMPLATE: &str = "{label}: {name} — {roaster} {origin}";

#[derive(Debug, Clone, Copy)]
enum Key {
    Label,
    Added,
    Name,
    RoastDate,
    Roaster,
    Origin,
    Producer,
    Farm,
    Elevation,
    Variety,
    Process,
    Flavours,
}

impl Ast<'_> for Key {
    type Error = anyhow::Error;

    fn from_expr(expression: &str) -> Result<Self> {
        match expression {
            "label" => Ok(Self::Label),
            "added" => Ok(Self::Added),
            "name" => Ok(Self::Name),
            "roast_date" => Ok(Self::RoastDate),
            "roaster" => Ok(Self::Roaster),
            "origin" => Ok(Self::Origin),
            "producer" => Ok(Self::Producer),
            "farm" => Ok(Self::Farm),
            "elevation" => Ok(Self::Elevation),
            "variety" => Ok(Self::Variety),
            "process" => Ok(Self::Process),
            "%flavours" => Ok(Self::Flavours),
            _ => Err(anyhow!("unknown Coffee column '{expression}'")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CoffeeTemplate {
    inner: mufmt::Template<String, Key>,
}

impl CoffeeTemplate {
    pub fn render(&self, coffee: &Coffee) -> Result<String> {
        Ok(self.inner.render(&CoffeeManifest(coffee))?)
    }
}

impl FromStr for CoffeeTemplate {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let inner = mufmt::Template::<String, Key>::compile(value)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self { inner })
    }
}

struct CoffeeManifest<'a>(&'a Coffee);

enum Value<'a> {
    Text(&'a str),
    Label(crate::label::Label),
    Date(Option<chrono::NaiveDate>),
    Timestamp(&'a chrono::DateTime<chrono::Local>),
    Flavours(&'a [String]),
}

impl fmt::Display for Value<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(value) => formatter.write_str(value),
            Self::Label(value) => value.fmt(formatter),
            Self::Date(Some(value)) => value.fmt(formatter),
            Self::Date(None) => Ok(()),
            Self::Timestamp(value) => value.fmt(formatter),
            Self::Flavours(flavours) => {
                let mut flavours = flavours.iter();
                if let Some(first) = flavours.next() {
                    formatter.write_str(first)?;
                    for flavour in flavours {
                        write!(formatter, ", {flavour}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Manifest<Key> for CoffeeManifest<'_> {
    type Error = Infallible;

    fn manifest(&self, key: &Key) -> Result<impl fmt::Display, Self::Error> {
        let coffee = self.0;
        let value = match key {
            Key::Label => Value::Label(coffee.label),
            Key::Added => Value::Timestamp(&coffee.added),
            Key::Name => Value::Text(&coffee.data.name),
            Key::RoastDate => Value::Date(coffee.data.roast_date),
            Key::Roaster => Value::Text(&coffee.data.roaster),
            Key::Origin => Value::Text(coffee.data.origin.as_deref().unwrap_or("")),
            Key::Producer => Value::Text(coffee.data.producer.as_deref().unwrap_or("")),
            Key::Farm => Value::Text(coffee.data.farm.as_deref().unwrap_or("")),
            Key::Elevation => Value::Text(coffee.data.elevation.as_deref().unwrap_or("")),
            Key::Variety => Value::Text(coffee.data.variety.as_deref().unwrap_or("")),
            Key::Process => Value::Text(coffee.data.process.as_deref().unwrap_or("")),
            Key::Flavours => Value::Flavours(&coffee.data.flavours),
        };
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use crate::{label::CoffeeType, record::CoffeeData};

    use super::*;

    #[test]
    fn renders_fields_and_empty_options() {
        let coffee = Coffee {
            label: crate::label::Label::new(CoffeeType::Filter, NonZero::new(1).unwrap()),
            added: chrono::Local::now(),
            data: CoffeeData {
                name: "Lot One".into(),
                roast_date: None,
                roaster: "Roaster".into(),
                origin: None,
                producer: None,
                farm: None,
                elevation: None,
                variety: None,
                process: None,
                flavours: vec!["apple".into(), "peach".into(), "mango".into()],
            },
        };
        let template: CoffeeTemplate = "{label} {name} {origin} {added} {%flavours}"
            .parse()
            .unwrap();
        let rendered = template.render(&coffee).unwrap();
        assert!(rendered.starts_with("F1 Lot One  "));
        assert!(rendered.ends_with("apple, peach, mango"));
        assert!("{flavours}".parse::<CoffeeTemplate>().is_err());
    }
}
