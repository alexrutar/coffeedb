use std::{convert::Infallible, fmt, str::FromStr};

use anyhow::{Result, anyhow, bail};
use mufmt::{Ast, Manifest};

use crate::record::Coffee;

pub const DEFAULT_TEMPLATE: &str = r#"{%label}: {%name} — {%roaster}{=origin " "}{origin?}"#;

#[derive(Debug, Clone, Copy)]
enum Field {
    RoastDate,
    Origin,
    Producer,
    Farm,
    Elevation,
    Variety,
    Process,
}

impl FromStr for Field {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "roast_date" => Ok(Self::RoastDate),
            "origin" => Ok(Self::Origin),
            "producer" => Ok(Self::Producer),
            "farm" => Ok(Self::Farm),
            "elevation" => Ok(Self::Elevation),
            "variety" => Ok(Self::Variety),
            "process" => Ok(Self::Process),
            _ => Err(anyhow!("unknown Coffee column '{value}'")),
        }
    }
}

impl Field {
    fn is_defined(self, coffee: &Coffee) -> bool {
        match self {
            Self::RoastDate => coffee.data.roast_date.is_some(),
            Self::Origin => coffee.data.origin.is_some(),
            Self::Producer => coffee.data.producer.is_some(),
            Self::Farm => coffee.data.farm.is_some(),
            Self::Elevation => coffee.data.elevation.is_some(),
            Self::Variety => coffee.data.variety.is_some(),
            Self::Process => coffee.data.process.is_some(),
        }
    }

    fn render(self, coffee: &Coffee) -> String {
        match self {
            Self::RoastDate => coffee
                .data
                .roast_date
                .map(|date| date.to_string())
                .unwrap_or_default(),
            Self::Origin => coffee.data.origin.clone().unwrap_or_default(),
            Self::Producer => coffee.data.producer.clone().unwrap_or_default(),
            Self::Farm => coffee.data.farm.clone().unwrap_or_default(),
            Self::Elevation => coffee.data.elevation.clone().unwrap_or_default(),
            Self::Variety => coffee.data.variety.clone().unwrap_or_default(),
            Self::Process => coffee.data.process.clone().unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Meta {
    Label,
    Added,
    Name,
    Roaster,
    Flavours,
}

impl FromStr for Meta {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "label" => Ok(Self::Label),
            "added" => Ok(Self::Added),
            "name" => Ok(Self::Name),
            "roaster" => Ok(Self::Roaster),
            "flavours" => Ok(Self::Flavours),
            _ => Err(anyhow!("unknown template meta key '%{value}'")),
        }
    }
}

#[derive(Debug, Clone)]
enum Atom {
    Field(Field),
    String(String),
    Meta(Meta),
}

impl Atom {
    fn parse(value: &str) -> Result<Self> {
        if value.starts_with('"') {
            return Ok(Self::String(serde_json::from_str(value)?));
        }
        if let Some(meta) = value.strip_prefix('%') {
            return Ok(Self::Meta(meta.parse()?));
        }

        let field = value.strip_suffix('?').unwrap_or(value).parse()?;
        Ok(Self::Field(field))
    }

    fn render(&self, coffee: &Coffee) -> String {
        match self {
            Self::Field(field) => field.render(coffee),
            Self::String(value) => value.clone(),
            Self::Meta(Meta::Label) => coffee.label.to_string(),
            Self::Meta(Meta::Added) => coffee.added.to_string(),
            Self::Meta(Meta::Name) => coffee.data.name.clone(),
            Self::Meta(Meta::Roaster) => coffee.data.roaster.clone(),
            Self::Meta(Meta::Flavours) => coffee.data.flavours.join(", "),
        }
    }
}

#[derive(Debug, Clone)]
enum Expression {
    Bare(Atom),
    IfDefined(Field, Atom),
    IfUndefined(Field, Atom),
}

impl Ast<'_> for Expression {
    type Error = anyhow::Error;

    fn from_expr(value: &str) -> Result<Self> {
        let Some(condition) = value
            .as_bytes()
            .first()
            .copied()
            .filter(|byte| matches!(byte, b'=' | b'!'))
        else {
            return Ok(Self::Bare(Atom::parse(value)?));
        };

        let rest = &value[1..];
        let Some(separator) = rest.find(char::is_whitespace) else {
            bail!("conditional expression requires a field followed by whitespace and a value");
        };
        let field = rest[..separator].parse::<Field>().map_err(|_| {
            anyhow!(
                "filter key '{}' is not a nullable Coffee column",
                &rest[..separator]
            )
        })?;
        let atom = rest[separator..].trim_start();
        if atom.is_empty() {
            bail!("conditional expression requires a value after the field");
        }
        let atom = Atom::parse(atom)?;
        Ok(if condition == b'=' {
            Self::IfDefined(field, atom)
        } else {
            Self::IfUndefined(field, atom)
        })
    }
}

#[derive(Debug, Clone)]
pub struct CoffeeTemplate {
    inner: mufmt::Template<String, Expression>,
}

impl CoffeeTemplate {
    pub fn render(&self, coffee: &Coffee) -> Result<String> {
        Ok(self.inner.render(&CoffeeManifest(coffee))?)
    }
}

impl FromStr for CoffeeTemplate {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        let inner = mufmt::Template::<String, Expression>::compile(value)
            .map_err(|error| anyhow!(error.to_string()))?;
        Ok(Self { inner })
    }
}

struct CoffeeManifest<'a>(&'a Coffee);

impl Manifest<Expression> for CoffeeManifest<'_> {
    type Error = Infallible;

    fn manifest(&self, expression: &Expression) -> Result<impl fmt::Display, Self::Error> {
        let rendered = match expression {
            Expression::Bare(atom) => atom.render(self.0),
            Expression::IfDefined(field, atom) if field.is_defined(self.0) => atom.render(self.0),
            Expression::IfUndefined(field, atom) if !field.is_defined(self.0) => {
                atom.render(self.0)
            }
            Expression::IfDefined(..) | Expression::IfUndefined(..) => String::new(),
        };
        Ok(rendered)
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use crate::{label::CoffeeType, record::CoffeeData};

    use super::*;

    fn coffee(origin: Option<&str>) -> Coffee {
        Coffee {
            label: crate::label::Label::new(CoffeeType::Filter, NonZero::new(1).unwrap()),
            added: chrono::Local::now(),
            data: CoffeeData {
                name: "Lot One".into(),
                roast_date: None,
                roaster: "Roaster".into(),
                origin: origin.map(str::to_owned),
                producer: None,
                farm: None,
                elevation: None,
                variety: None,
                process: None,
                flavours: vec!["apple".into(), "peach".into(), "mango".into()],
            },
        }
    }

    #[test]
    fn renders_fields_and_meta_keys() {
        let coffee = coffee(None);
        let template: CoffeeTemplate = "{%label} {%name} {origin} {%added} {%flavours}"
            .parse()
            .unwrap();
        let rendered = template.render(&coffee).unwrap();
        assert!(rendered.starts_with("F1 Lot One  "));
        assert!(rendered.ends_with("apple, peach, mango"));
        assert!("{flavours}".parse::<CoffeeTemplate>().is_err());
    }

    #[test]
    fn conditionally_renders_defined_and_undefined_fields() {
        let template: CoffeeTemplate =
            r#"{%name}{=origin ": "}{origin?}{!origin " (unknown origin)"}"#
                .parse()
                .unwrap();
        assert_eq!(
            template.render(&coffee(Some("Ethiopia"))).unwrap(),
            "Lot One: Ethiopia"
        );
        assert_eq!(
            template.render(&coffee(None)).unwrap(),
            "Lot One (unknown origin)"
        );
    }

    #[test]
    fn rejects_invalid_filter_syntax() {
        for template in [
            "{=origin}",
            "{=unknown value}",
            "{=label value}",
            "{=added value}",
            "{=roaster value}",
            "{=name value}",
            "{%unknown}",
            "{label}",
            "{added}",
            "{name}",
            "{roaster}",
            "{flavours}",
        ] {
            assert!(
                template.parse::<CoffeeTemplate>().is_err(),
                "accepted {template}"
            );
        }
    }
}
