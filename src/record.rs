use std::{collections::HashSet, fmt, str::FromStr};

use anyhow::{Result, bail};
use chrono::{DateTime, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::label::Label;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Coffee {
    pub label: Label,
    pub added: DateTime<Local>,
    pub data: CoffeeData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoffeeData {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roast_date: Option<NaiveDate>,
    pub roaster: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub producer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub farm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variety: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<String>,
    #[serde(default)]
    pub flavours: Vec<String>,
}

impl CoffeeData {
    pub fn validate(mut self) -> Result<Self> {
        if self.name.is_empty() {
            bail!("'name' must not be empty");
        }
        if self.roaster.is_empty() {
            bail!("'roaster' must not be empty");
        }
        for (name, value) in [
            ("origin", &self.origin),
            ("producer", &self.producer),
            ("farm", &self.farm),
            ("elevation", &self.elevation),
            ("variety", &self.variety),
            ("process", &self.process),
        ] {
            if value.as_deref() == Some("") {
                bail!("'{name}' is empty; delete fields whose values are missing");
            }
        }

        let mut seen = HashSet::new();
        let mut flavours = Vec::with_capacity(self.flavours.len());
        for raw in self.flavours {
            let flavour = raw.trim_matches(' ');
            if flavour.is_empty() {
                bail!("flavours must not be empty");
            }
            if !flavour
                .bytes()
                .all(|byte| byte == b' ' || byte.is_ascii_alphabetic())
            {
                bail!("flavour '{flavour}' may contain only ASCII letters and spaces");
            }
            let flavour = flavour.to_ascii_lowercase();
            if seen.insert(flavour.clone()) {
                flavours.push(flavour);
            }
        }
        self.flavours = flavours;
        Ok(self)
    }

    pub fn draft() -> &'static str {
        r#"# Fill in required fields. Delete optional fields whose values are missing.
# A label will be assigned after this record is saved.
name = ""
roast_date = ""
roaster = ""
origin = ""
producer = ""
farm = ""
elevation = ""
variety = ""
process = ""
flavours = []
"#
    }

    pub fn to_toml(&self, label: Option<Label>) -> Result<String> {
        let mut output = String::new();
        if let Some(label) = label {
            output.push_str(&format!("# label = \"{label}\"\n"));
        }
        output.push_str(&toml::to_string_pretty(self)?);
        Ok(output)
    }
}

impl FromStr for CoffeeData {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        toml::from_str::<Self>(value)?.validate()
    }
}

impl fmt::Display for Coffee {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.data.to_toml(Some(self.label)) {
            Ok(toml) => f.write_str(&toml),
            Err(_) => Err(fmt::Error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_and_deduplicates_flavours() {
        let mut data: CoffeeData = r#"
name = "Coffee"
roaster = "Roaster"
flavours = ["  Apple ", "apple", "Brown Sugar"]
"#
        .parse()
        .unwrap();
        assert_eq!(data.flavours, ["apple", "brown sugar"]);
        data.flavours.push("cocoa-nib".into());
        assert!(data.validate().is_err());
    }

    #[test]
    fn rejects_non_ascii_letters_and_whitespace() {
        for flavour in ["café", "brown\tsugar", "cocoa 70"] {
            let data = CoffeeData {
                name: "Coffee".into(),
                roast_date: None,
                roaster: "Roaster".into(),
                origin: None,
                producer: None,
                farm: None,
                elevation: None,
                variety: None,
                process: None,
                flavours: vec![flavour.into()],
            };
            assert!(data.validate().is_err(), "accepted {flavour}");
        }
    }

    #[test]
    fn rejects_empty_optional_fields() {
        assert!(
            r#"name = "Coffee"
roaster = "Roaster"
origin = ""
"#
            .parse::<CoffeeData>()
            .is_err()
        );
    }
}
