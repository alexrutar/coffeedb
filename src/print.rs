use std::fmt::{self, Write as _};

use crate::record::Coffee;

const DIVIDER_PADDING_ABOVE: usize = 14;
const DIVIDER_PADDING_BELOW: usize = 22;
const PARAGRAPH_PADDING: usize = 32;
const HORIZONTAL_PADDING: usize = 30;
const MINIMUM_WIDTH: usize = 180;

pub fn render_svg(coffee: &Coffee) -> String {
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

    let mut output = String::new();
    writeln!(
        output,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="coffee label">"#
    )
    .unwrap();
    writeln!(
        output,
        r##"  <rect width="{width}" height="{height}" rx="16" fill="#faf9f6"/>"##
    )
    .unwrap();
    output.push_str(
        "  <g fill=\"#1c1b19\" font-family=\"Palatino, 'Palatino Linotype', 'Book Antiqua', serif\">\n\
         \x20   <text x=\"30\" y=\"48\" font-size=\"27\" font-weight=\"600\" font-variant=\"small-caps\" letter-spacing=\"-0.5\">",
    );
    write_xml(&mut output, &coffee.data.name).unwrap();
    writeln!(
        output,
        "<tspan font-weight=\"400\" fill=\"#aaa59c\" letter-spacing=\"1.2\"> #{}</tspan></text>",
        coffee.label
    )
    .unwrap();

    if has_notes {
        output.push_str("    <text x=\"30\" y=\"72\" font-size=\"15\" font-style=\"italic\" fill=\"#5f5c56\" style=\"text-transform:lowercase\">");
        for (index, flavour) in coffee.data.flavours.iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            write_xml(&mut output, flavour).unwrap();
        }
        output.push_str("</text>\n");
    }

    if has_processing {
        let first_value = coffee
            .data
            .process
            .as_deref()
            .or(coffee.data.variety.as_deref())
            .unwrap();
        write!(
            output,
            "    <text x=\"30\" y=\"{processing_y}\" font-size=\"14\" font-style=\"italic\">{} ",
            indefinite_article(first_value)
        )
        .unwrap();
        if let Some(process) = coffee.data.process.as_deref() {
            output.push_str("<tspan font-weight=\"700\">");
            write_xml_lowercase(&mut output, process).unwrap();
            output.push_str("</tspan> process");
        }
        if let Some(variety) = coffee.data.variety.as_deref() {
            if coffee.data.process.is_some() {
                output.push(' ');
            }
            output.push_str("<tspan font-weight=\"700\">");
            write_xml_lowercase(&mut output, variety).unwrap();
            output.push_str("</tspan>");
        }
        output.push_str("</text>\n");
    }

    if has_production {
        write!(
            output,
            "    <text x=\"30\" y=\"{production_y}\" font-size=\"14\" font-style=\"italic\">produced "
        )
        .unwrap();
        match (coffee.data.producer.as_deref(), coffee.data.farm.as_deref()) {
            (Some(producer), Some(farm)) => {
                output.push_str("by <tspan font-weight=\"700\">");
                write_xml(&mut output, producer).unwrap();
                output.push_str("</tspan> @ <tspan font-weight=\"700\">");
                write_xml(&mut output, farm).unwrap();
                output.push_str("</tspan>");
            }
            (Some(producer), None) => {
                output.push_str("by <tspan font-weight=\"700\">");
                write_xml(&mut output, producer).unwrap();
                output.push_str("</tspan>");
            }
            (None, Some(farm)) => {
                output.push_str("at <tspan font-weight=\"700\">");
                write_xml(&mut output, farm).unwrap();
                output.push_str("</tspan>");
            }
            (None, None) => unreachable!(),
        }
        output.push_str("</text>\n");
    }

    write!(
        output,
        "    <text x=\"30\" y=\"{roasted_y}\" font-size=\"14\" font-style=\"italic\">roasted by <tspan font-weight=\"700\">"
    )
    .unwrap();
    write_xml(&mut output, &coffee.data.roaster).unwrap();
    output.push_str("</tspan></text>\n");

    if has_location {
        writeln!(
            output,
            "    <line x1=\"30\" y1=\"{bottom_divider_y}\" x2=\"{right_x}\" y2=\"{bottom_divider_y}\" stroke=\"#d8d5ce\" stroke-width=\"1\" />"
        )
        .unwrap();
    }

    if let Some(origin) = coffee.data.origin.as_deref() {
        write!(
            output,
            "    <text x=\"30\" y=\"{footer_y}\" font-size=\"14\">"
        )
        .unwrap();
        write_xml(&mut output, origin).unwrap();
        output.push_str("</text>\n");
    }
    if let Some(elevation) = coffee.data.elevation {
        writeln!(
            output,
            "    <text x=\"{right_x}\" y=\"{footer_y}\" text-anchor=\"end\" font-size=\"14\">{elevation}</text>"
        )
        .unwrap();
    }
    output.push_str("  </g>\n</svg>\n");
    output
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

    if coffee.data.process.is_some() || coffee.data.variety.is_some() {
        let first = coffee
            .data
            .process
            .as_deref()
            .or(coffee.data.variety.as_deref())
            .unwrap();
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

    if coffee.data.producer.is_some() || coffee.data.farm.is_some() {
        let mut line = text("produced ", 14.0, false);
        match (coffee.data.producer.as_deref(), coffee.data.farm.as_deref()) {
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
            (None, None) => unreachable!(),
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

fn write_xml_lowercase(output: &mut String, value: &str) -> fmt::Result {
    write_xml(output, &value.to_lowercase())
}

fn write_xml(output: &mut String, value: &str) -> fmt::Result {
    for character in value.chars() {
        output.write_str(match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&apos;",
            _ => {
                output.write_char(character)?;
                continue;
            }
        })?;
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

    fn coffee() -> Coffee {
        Coffee {
            label: Label::new(CoffeeType::Filter, NonZero::new(1).unwrap()),
            added: Local::now(),
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
        assert!(render_svg(&coffee).contains(r#"height="104""#));

        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.process = Some("washed".into());
        assert!(render_svg(&coffee).contains(r#"height="154""#));

        coffee.data.flavours.push("peach".into());
        assert!(render_svg(&coffee).contains(r#"height="178""#));
    }

    #[test]
    fn omits_empty_tasting_notes() {
        let mut coffee = coffee();
        assert!(!render_svg(&coffee).contains(r#"x="30" y="72" font-size="15""#));

        coffee.data.flavours.push("peach".into());
        let svg = render_svg(&coffee);
        assert!(svg.contains(r#"x="30" y="72" font-size="15""#));
        assert!(svg.contains(">peach</text>"));
        assert!(!svg.contains(">notes"));
    }

    #[test]
    fn formats_production_byline() {
        let mut coffee = coffee();
        coffee.data.producer = Some("Producer & Sons".into());
        let svg = render_svg(&coffee);
        assert!(svg.contains("produced by <tspan font-weight=\"700\">Producer &amp; Sons</tspan>"));

        coffee.data.farm = Some("The Farm".into());
        let svg = render_svg(&coffee);
        assert!(
            svg.contains(
                "Producer &amp; Sons</tspan> @ <tspan font-weight=\"700\">The Farm</tspan>"
            )
        );

        coffee.data.producer = None;
        let svg = render_svg(&coffee);
        assert!(svg.contains("produced at <tspan font-weight=\"700\">The Farm</tspan>"));
    }

    #[test]
    fn puts_origin_below_attribution_without_a_label() {
        let mut coffee = coffee();
        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.producer = Some("Producer".into());
        let svg = render_svg(&coffee);
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
        let svg = render_svg(&coffee);
        assert!(
            svg.contains(
                "font-style=\"italic\">a <tspan font-weight=\"700\">bourbon</tspan></text>"
            )
        );
        assert!(!svg.contains(" process"));

        coffee.data.process = Some("Washed".into());
        let svg = render_svg(&coffee);
        assert!(svg.contains(
            "font-style=\"italic\">a <tspan font-weight=\"700\">washed</tspan> process <tspan font-weight=\"700\">bourbon</tspan>"
        ));

        coffee.data.variety = None;
        let svg = render_svg(&coffee);
        assert!(svg.contains(
            "font-style=\"italic\">a <tspan font-weight=\"700\">washed</tspan> process</text>"
        ));

        coffee.data.process = Some("Anaerobic".into());
        assert!(render_svg(&coffee).contains(
            "font-style=\"italic\">an <tspan font-weight=\"700\">anaerobic</tspan> process</text>"
        ));
    }

    #[test]
    fn puts_processing_below_tasting_notes() {
        let mut coffee = coffee();
        coffee.data.flavours = vec!["peach".into()];
        coffee.data.process = Some("Washed".into());
        let svg = render_svg(&coffee);
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
        let svg = render_svg(&coffee);
        assert!(svg.contains(r#"y="104" font-size="14" font-style="italic">a"#));
        assert!(svg.contains(r#"y="126" font-size="14" font-style="italic">produced"#));
        assert!(svg.contains(r#"y="148" font-size="14" font-style="italic">roasted"#));
    }

    #[test]
    fn puts_elevation_in_the_bottom_right() {
        let mut coffee = coffee();
        coffee.data.origin = Some("Ethiopia".into());
        coffee.data.elevation = Some(CoffeeElevation::Range(1000, 1200));
        let svg = render_svg(&coffee);
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

        let svg = render_svg(&short);
        let width = estimated_svg_width(&short);
        assert!(svg.contains(&format!(r#"width="{width}""#)));
        assert!(width < 500);
    }

    #[test]
    fn escapes_xml_text() {
        let mut coffee = coffee();
        coffee.data.name = "A & B <Coffee>".into();
        assert!(render_svg(&coffee).contains("A &amp; B &lt;Coffee&gt;"));
    }
}
