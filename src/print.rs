use std::io::{self, Write};

use anyhow::Result;
use serde::Serialize;

use crate::record::Coffee;

const DIVIDER_PADDING_ABOVE: usize = 14;
const DIVIDER_PADDING_BELOW: usize = 22;
const PARAGRAPH_PADDING: usize = 32;
const HORIZONTAL_PADDING: usize = 30;
const MINIMUM_WIDTH: usize = 180;

#[derive(Serialize)]
struct SerializableCoffee<'a> {
    label: String,
    #[serde(flatten)]
    data: &'a crate::record::CoffeeData,
}

impl<'a> From<&'a Coffee> for SerializableCoffee<'a> {
    fn from(coffee: &'a Coffee) -> Self {
        Self {
            label: coffee.label.to_string(),
            data: &coffee.data,
        }
    }
}

pub fn render_toml(coffee: &Coffee) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(&SerializableCoffee::from(coffee))
}

pub fn render_json<W: Write>(mut writer: W, coffee: &Coffee) -> Result<()> {
    serde_json::to_writer_pretty(&mut writer, &SerializableCoffee::from(coffee))?;
    writeln!(writer)?;
    Ok(())
}

pub fn render_text<W: Write>(mut writer: W, coffee: &Coffee) -> io::Result<()> {
    writeln!(writer, "{} #{}", coffee.data.name, coffee.label)?;

    if !coffee.data.flavours.is_empty() {
        writeln!(writer, "{}", coffee.data.flavours.join(", "))?;
    }
    writeln!(writer)?;

    if let Some(first_value) = coffee
        .data
        .process
        .as_deref()
        .or(coffee.data.variety.as_deref())
    {
        write!(writer, "{} ", indefinite_article(first_value))?;
        if let Some(process) = coffee.data.process.as_deref() {
            write!(writer, "{} process", process.to_lowercase())?;
        }
        if let Some(variety) = coffee.data.variety.as_deref() {
            if coffee.data.process.is_some() {
                write!(writer, " ")?;
            }
            write!(writer, "{}", variety.to_lowercase())?;
        }
        writeln!(writer)?;
    }

    match (coffee.data.producer.as_deref(), coffee.data.farm.as_deref()) {
        (Some(producer), Some(farm)) => writeln!(writer, "produced by {producer} @ {farm}")?,
        (Some(producer), None) => writeln!(writer, "produced by {producer}")?,
        (None, Some(farm)) => writeln!(writer, "produced at {farm}")?,
        (None, None) => {}
    }
    writeln!(writer, "roasted by {}", coffee.data.roaster)?;

    match (coffee.data.origin.as_deref(), coffee.data.elevation) {
        (Some(origin), Some(elevation)) => writeln!(writer, "\n{origin}  {elevation}")?,
        (Some(origin), None) => writeln!(writer, "\n{origin}")?,
        (None, Some(elevation)) => writeln!(writer, "\n{elevation}")?,
        (None, None) => {}
    }

    Ok(())
}

pub fn render_svg<W: Write>(mut writer: W, coffee: &Coffee) -> io::Result<()> {
    let width = estimated_svg_width(coffee);
    let right_x = width - HORIZONTAL_PADDING;
    let has_production = coffee.data.producer.is_some() || coffee.data.farm.is_some();
    let has_processing = coffee.data.variety.is_some() || coffee.data.process.is_some();
    let has_location = coffee.data.origin.is_some() || coffee.data.elevation.is_some();
    let has_notes = !coffee.data.flavours.is_empty();
    let body_start = if has_notes { 72 } else { 48 };
    let processing_y = body_start + PARAGRAPH_PADDING;
    let production_y = processing_y + usize::from(has_processing) * 22;
    let roasted_y = production_y + usize::from(has_production) * 22;
    let content_end = roasted_y;
    let bottom_divider_y = content_end + DIVIDER_PADDING_ABOVE;
    let footer_y = bottom_divider_y + DIVIDER_PADDING_BELOW;
    let footer_end = if has_location { footer_y } else { content_end };
    let height = footer_end + if has_location { 16 } else { 24 };

    writeln!(
        writer,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="coffee label">"#
    )?;
    writeln!(
        writer,
        r##"  <rect width="{width}" height="{height}" rx="16" fill="#faf9f6"/>"##
    )?;
    writer.write_all(
        "  <g fill=\"#1c1b19\" font-family=\"Palatino, 'Palatino Linotype', 'Book Antiqua', serif\">\n\
         \x20   <text x=\"30\" y=\"48\" font-size=\"27\" font-weight=\"600\" font-variant=\"small-caps\" letter-spacing=\"-0.5\">"
            .as_bytes(),
    )?;
    write_xml(&mut writer, &coffee.data.name)?;
    writeln!(
        writer,
        "<tspan font-weight=\"400\" fill=\"#aaa59c\" letter-spacing=\"1.2\"> #{}</tspan></text>",
        coffee.label
    )?;

    if has_notes {
        writer.write_all(b"    <text x=\"30\" y=\"72\" font-size=\"15\" font-style=\"italic\" fill=\"#5f5c56\" style=\"text-transform:lowercase\">")?;
        for (index, flavour) in coffee.data.flavours.iter().enumerate() {
            if index > 0 {
                writer.write_all(b", ")?;
            }
            write_xml(&mut writer, flavour)?;
        }
        writer.write_all(b"</text>\n")?;
    }

    if let Some(first_value) = coffee
        .data
        .process
        .as_deref()
        .or(coffee.data.variety.as_deref())
    {
        write!(
            writer,
            "    <text x=\"30\" y=\"{processing_y}\" font-size=\"14\" font-style=\"italic\">{} ",
            indefinite_article(first_value)
        )?;
        if let Some(process) = coffee.data.process.as_deref() {
            writer.write_all(b"<tspan font-weight=\"700\">")?;
            write_xml_lowercase(&mut writer, process)?;
            writer.write_all(b"</tspan> process")?;
        }
        if let Some(variety) = coffee.data.variety.as_deref() {
            if coffee.data.process.is_some() {
                writer.write_all(b" ")?;
            }
            writer.write_all(b"<tspan font-weight=\"700\">")?;
            write_xml_lowercase(&mut writer, variety)?;
            writer.write_all(b"</tspan>")?;
        }
        writer.write_all(b"</text>\n")?;
    }

    match (coffee.data.producer.as_deref(), coffee.data.farm.as_deref()) {
        (None, None) => {}
        production => {
            write!(
                writer,
                "    <text x=\"30\" y=\"{production_y}\" font-size=\"14\" font-style=\"italic\">produced "
            )?;
            match production {
                (Some(producer), Some(farm)) => {
                    writer.write_all(b"by <tspan font-weight=\"700\">")?;
                    write_xml(&mut writer, producer)?;
                    writer.write_all(b"</tspan> @ <tspan font-weight=\"700\">")?;
                    write_xml(&mut writer, farm)?;
                    writer.write_all(b"</tspan>")?;
                }
                (Some(producer), None) => {
                    writer.write_all(b"by <tspan font-weight=\"700\">")?;
                    write_xml(&mut writer, producer)?;
                    writer.write_all(b"</tspan>")?;
                }
                (None, Some(farm)) => {
                    writer.write_all(b"at <tspan font-weight=\"700\">")?;
                    write_xml(&mut writer, farm)?;
                    writer.write_all(b"</tspan>")?;
                }
                (None, None) => {}
            }
            writer.write_all(b"</text>\n")?;
        }
    }

    write!(
        writer,
        "    <text x=\"30\" y=\"{roasted_y}\" font-size=\"14\" font-style=\"italic\">roasted by <tspan font-weight=\"700\">"
    )?;
    write_xml(&mut writer, &coffee.data.roaster)?;
    writer.write_all(b"</tspan></text>\n")?;

    if has_location {
        writeln!(
            writer,
            "    <line x1=\"30\" y1=\"{bottom_divider_y}\" x2=\"{right_x}\" y2=\"{bottom_divider_y}\" stroke=\"#d8d5ce\" stroke-width=\"1\" />"
        )?;
    }

    if let Some(origin) = coffee.data.origin.as_deref() {
        write!(
            writer,
            "    <text x=\"30\" y=\"{footer_y}\" font-size=\"14\">"
        )?;
        write_xml(&mut writer, origin)?;
        writer.write_all(b"</text>\n")?;
    }
    if let Some(elevation) = coffee.data.elevation {
        writeln!(
            writer,
            "    <text x=\"{right_x}\" y=\"{footer_y}\" text-anchor=\"end\" font-size=\"14\">{elevation}</text>"
        )?;
    }
    writer.write_all(b"  </g>\n</svg>\n")?;
    Ok(())
}

fn estimated_svg_width(coffee: &Coffee) -> usize {
    fn text(value: &str, size: f32, bold: bool) -> f32 {
        let weight = if bold { 1.06 } else { 1.0 };
        value
            .chars()
            .map(|character| {
                let em = match character {
                    ' ' => 0.28,
                    'i' | 'l' | 'I' | '!' | '|' | '.' | ',' | ':' | ';' | '\'' => 0.27,
                    'f' | 'j' | 'r' | 't' | '(' | ')' | '-' => 0.36,
                    'm' | 'w' | 'M' | 'W' | '@' | '%' => 0.82,
                    'A'..='Z' => 0.68,
                    '0'..='9' => 0.55,
                    _ => 0.5,
                };
                em * size * weight
            })
            .sum()
    }

    fn spaced_text(value: &str, size: f32, bold: bool, letter_spacing: f32) -> f32 {
        text(value, size, bold) + value.chars().count().saturating_sub(1) as f32 * letter_spacing
    }

    let label = format!(" #{}", coffee.label);
    let mut widest =
        spaced_text(&coffee.data.name, 27.0, true, -0.5) + spaced_text(&label, 27.0, false, 1.2);

    if !coffee.data.flavours.is_empty() {
        widest = widest.max(text(&coffee.data.flavours.join(", "), 15.0, false));
    }

    if let Some(first) = coffee
        .data
        .process
        .as_deref()
        .or(coffee.data.variety.as_deref())
    {
        let mut line = text(indefinite_article(first), 14.0, false) + text(" ", 14.0, false);
        if let Some(process) = coffee.data.process.as_deref() {
            line += text(process, 14.0, true) + text(" process", 14.0, false);
        }
        if let Some(variety) = coffee.data.variety.as_deref() {
            if coffee.data.process.is_some() {
                line += text(" ", 14.0, false);
            }
            line += text(variety, 14.0, true);
        }
        widest = widest.max(line);
    }

    if let production @ (Some(_), _) | production @ (_, Some(_)) =
        (coffee.data.producer.as_deref(), coffee.data.farm.as_deref())
    {
        let mut line = text("produced ", 14.0, false);
        match production {
            (Some(producer), Some(farm)) => {
                line += text("by ", 14.0, false)
                    + text(producer, 14.0, true)
                    + text(" @ ", 14.0, false)
                    + text(farm, 14.0, true);
            }
            (Some(producer), None) => {
                line += text("by ", 14.0, false) + text(producer, 14.0, true);
            }
            (None, Some(farm)) => {
                line += text("at ", 14.0, false) + text(farm, 14.0, true);
            }
            (None, None) => {}
        }
        widest = widest.max(line);
    }

    widest = widest.max(text("roasted by ", 14.0, false) + text(&coffee.data.roaster, 14.0, true));

    let footer = coffee
        .data
        .origin
        .as_deref()
        .map_or(0.0, |origin| text(origin, 14.0, false))
        + match coffee.data.elevation {
            Some(elevation) => {
                let gap = if coffee.data.origin.is_some() {
                    30.0
                } else {
                    0.0
                };
                gap + text(&elevation.to_string(), 14.0, false)
            }
            None => 0.0,
        };
    widest = widest.max(footer);

    (widest.ceil() as usize + HORIZONTAL_PADDING * 2).max(MINIMUM_WIDTH)
}

fn indefinite_article(value: &str) -> &'static str {
    match value
        .chars()
        .find(|character| character.is_alphabetic())
        .map(|character| character.to_ascii_lowercase())
    {
        Some('a' | 'e' | 'i' | 'o' | 'u') => "an",
        _ => "a",
    }
}

fn write_xml_lowercase<W: Write>(writer: W, value: &str) -> io::Result<()> {
    write_xml(writer, &value.to_lowercase())
}

fn write_xml<W: Write>(mut writer: W, value: &str) -> io::Result<()> {
    for character in value.chars() {
        writer.write_all(
            match character {
                '&' => "&amp;",
                '<' => "&lt;",
                '>' => "&gt;",
                '"' => "&quot;",
                '\'' => "&apos;",
                _ => {
                    write!(writer, "{character}")?;
                    continue;
                }
            }
            .as_bytes(),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;

    use chrono::Local;

    use super::*;
    use crate::{
        db::CoffeeElevation,
        label::{CoffeeType, Label},
        record::CoffeeData,
    };

    fn rendered_svg(coffee: &Coffee) -> String {
        let mut output = Vec::new();
        render_svg(&mut output, coffee).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn coffee() -> Coffee {
        Coffee {
            label: Label::new(CoffeeType::Filter, NonZero::new(1).unwrap()),
            added: Local::now(),
            archived: false,
            data: CoffeeData {
                name: "House Coffee".into(),
                roaster: "Dak".into(),
                roast_date: None,
                origin: None,
                producer: None,
                farm: None,
                elevation: None,
                variety: None,
                process: None,
                flavours: vec![],
            },
        }
    }

    #[test]
    fn height_tracks_visible_optional_rows() {
        let mut coffee = coffee();
        assert!(rendered_svg(&coffee).contains(r#"height="104""#));

        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.process = Some("washed".into());
        assert!(rendered_svg(&coffee).contains(r#"height="154""#));

        coffee.data.flavours.push("peach".into());
        assert!(rendered_svg(&coffee).contains(r#"height="178""#));
    }

    #[test]
    fn omits_empty_tasting_notes() {
        let mut coffee = coffee();
        assert!(!rendered_svg(&coffee).contains(r#"x="30" y="72" font-size="15""#));

        coffee.data.flavours.push("peach".into());
        let svg = rendered_svg(&coffee);
        assert!(svg.contains(r#"x="30" y="72" font-size="15""#));
        assert!(svg.contains(">peach</text>"));
        assert!(!svg.contains(">notes"));
    }

    #[test]
    fn formats_production_byline() {
        let mut coffee = coffee();
        coffee.data.producer = Some("Producer & Sons".into());
        let svg = rendered_svg(&coffee);
        assert!(svg.contains("produced by <tspan font-weight=\"700\">Producer &amp; Sons</tspan>"));

        coffee.data.farm = Some("The Farm".into());
        let svg = rendered_svg(&coffee);
        assert!(
            svg.contains(
                "Producer &amp; Sons</tspan> @ <tspan font-weight=\"700\">The Farm</tspan>"
            )
        );

        coffee.data.producer = None;
        let svg = rendered_svg(&coffee);
        assert!(svg.contains("produced at <tspan font-weight=\"700\">The Farm</tspan>"));
    }

    #[test]
    fn puts_origin_below_attribution_without_a_label() {
        let mut coffee = coffee();
        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.producer = Some("Producer".into());
        let svg = rendered_svg(&coffee);
        let roasted = svg.find("roasted by").unwrap();
        let produced = svg.find("produced by").unwrap();
        let origin = svg.find(">Ethiopia</text>").unwrap();
        assert!(produced < roasted && roasted < origin);
        assert!(!svg.contains(">origin</tspan>"));
        assert!(svg.contains(r#"<line x1="30""#));
    }

    #[test]
    fn formats_variety_and_process_as_a_sentence() {
        let mut coffee = coffee();
        coffee.data.variety = Some("Bourbon".into());
        let svg = rendered_svg(&coffee);
        assert!(
            svg.contains(
                "font-style=\"italic\">a <tspan font-weight=\"700\">bourbon</tspan></text>"
            )
        );
        assert!(!svg.contains(" process"));

        coffee.data.process = Some("Washed".into());
        let svg = rendered_svg(&coffee);
        assert!(svg.contains(
            "font-style=\"italic\">a <tspan font-weight=\"700\">washed</tspan> process <tspan font-weight=\"700\">bourbon</tspan>"
        ));

        coffee.data.variety = None;
        let svg = rendered_svg(&coffee);
        assert!(svg.contains(
            "font-style=\"italic\">a <tspan font-weight=\"700\">washed</tspan> process</text>"
        ));

        coffee.data.process = Some("Anaerobic".into());
        assert!(rendered_svg(&coffee).contains(
            "font-style=\"italic\">an <tspan font-weight=\"700\">anaerobic</tspan> process</text>"
        ));
    }

    #[test]
    fn puts_processing_below_tasting_notes() {
        let mut coffee = coffee();
        coffee.data.flavours = vec!["peach".into()];
        coffee.data.process = Some("Washed".into());
        let svg = rendered_svg(&coffee);
        let notes = svg.find(">peach</text>").unwrap();
        let processing = svg.find(">washed</tspan> process").unwrap();
        let roasted = svg.find("roasted by").unwrap();
        assert!(notes < processing && processing < roasted);
        assert!(svg.contains(r#"x="30" y="104" font-size="14" font-style="italic">a "#));
    }

    #[test]
    fn spaces_processing_producer_and_roaster_evenly() {
        let mut coffee = coffee();
        coffee.data.flavours = vec!["peach".into()];
        coffee.data.process = Some("Washed".into());
        coffee.data.producer = Some("Producer".into());
        let svg = rendered_svg(&coffee);
        assert!(svg.contains(r#"y="104" font-size="14" font-style="italic">a"#));
        assert!(svg.contains(r#"y="126" font-size="14" font-style="italic">produced"#));
        assert!(svg.contains(r#"y="148" font-size="14" font-style="italic">roasted"#));
    }

    #[test]
    fn puts_elevation_in_the_bottom_right() {
        let mut coffee = coffee();
        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.elevation = Some(CoffeeElevation::Range(1000, 1200));
        let svg = rendered_svg(&coffee);
        let width = estimated_svg_width(&coffee);
        let right_x = width - HORIZONTAL_PADDING;
        assert!(width < 500);
        assert!(svg.contains(r#"height="132""#));
        assert!(svg.contains(&format!(r#"y1="94" x2="{right_x}" y2="94""#)));
        assert!(svg.contains(r#"x="30" y="116" font-size="14">Ethiopia"#));
        assert!(svg.contains(&format!(
            r#"x="{right_x}" y="116" text-anchor="end" font-size="14">1000 - 1200 masl"#
        )));
    }

    #[test]
    fn width_tracks_the_widest_text_line() {
        let short = coffee();
        let mut long = coffee();
        long.data.name = "An Exceptionally Long Coffee Name".into();
        assert!(estimated_svg_width(&short) < estimated_svg_width(&long));

        let svg = rendered_svg(&short);
        let width = estimated_svg_width(&short);
        assert!(svg.contains(&format!(r#"width="{width}""#)));
        assert!(width < 500);
    }

    #[test]
    fn escapes_xml_text() {
        let mut coffee = coffee();
        coffee.data.name = "A & B <Coffee>".into();
        assert!(rendered_svg(&coffee).contains("A &amp; B &lt;Coffee&gt;"));
    }
}
