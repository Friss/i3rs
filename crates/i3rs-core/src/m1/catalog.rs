//! Bounded metadata extraction from the TuneConfig XML embedded in M1 logs.
//!
//! Channel words are identified by their address base and offset. Explicit
//! DefaultLocale units use the bounded affine conversions verified against a
//! Tune-produced recording. Unknown pairs retain physical units with a warning.
//! A2L calibration tables are not needed for this catalog.

use std::collections::BTreeMap;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

const MAX_XML_BYTES: usize = 16 * 1024 * 1024;
const MAX_NODES: usize = 1_000_000;
const MAX_DEPTH: usize = 128;
const MAX_ATTRIBUTE_BYTES: usize = 16 * 1024;
const MAX_TEXT_NODE_BYTES: usize = 1024 * 1024;
const MAX_OWNED_TEXT_BYTES: usize = 32 * 1024 * 1024;
const MAX_LABEL_COPIES: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    F32,
    U32,
    I32,
    I32E7,
    Enum,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDefinition {
    pub name: String,
    pub unit: String,
    pub data_type: ValueType,
    pub enum_labels: BTreeMap<i32, String>,
    pub display_decimals: u8,
    pub display_scale: f64,
    pub display_offset: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    pub channels: BTreeMap<(u32, u32), ChannelDefinition>,
    /// Logging event index to rate in Hz. Firmware update schedules on an item
    /// are distinct from the selected logging packet's event index.
    pub events: BTreeMap<u32, f64>,
    /// No clock field in TuneConfig has been established. The decoder must use
    /// its independently established board-clock/fallback contract.
    pub sys_clock_hz: Option<f64>,
    /// Unknown requested locale conversions, keyed by scalar address. Callers
    /// should surface warnings only for channels actually selected by setups.
    pub unit_conversion_warnings: BTreeMap<(u32, u32), String>,
}

/// Parse a single TuneConfig document without DTDs or external entities.
/// `max_items` bounds retained channel definitions, enum tables/labels and
/// events together. Separate hard byte, XML-node, depth and label-copy budgets
/// also apply. Addressed scalar types outside the supported set are errors.
pub fn parse_catalog(xml: &[u8], max_items: usize) -> Result<Catalog, String> {
    if xml.len() > MAX_XML_BYTES {
        return Err(format!(
            "TuneConfig exceeds the {MAX_XML_BYTES}-byte XML limit"
        ));
    }
    if max_items == 0 {
        return Err("TuneConfig item budget must be nonzero".into());
    }
    let mut reader = Reader::from_reader(xml);
    let mut parser = Parser::new(max_items);
    loop {
        match reader
            .read_event()
            .map_err(|error| format!("Invalid TuneConfig XML: {error}"))?
        {
            Event::Start(element) => {
                let name = element_name(element.name().as_ref())?;
                let attributes = attributes(&element, &reader)?;
                parser.start(name, &attributes)?;
            }
            Event::Empty(element) => {
                let name = element_name(element.name().as_ref())?;
                let attributes = attributes(&element, &reader)?;
                parser.start(name.clone(), &attributes)?;
                parser.end(&name)?;
            }
            Event::End(element) => parser.end(&element_name(element.name().as_ref())?)?,
            Event::DocType(_) => {
                return Err("TuneConfig DTDs and external entities are unsupported".into());
            }
            Event::Decl(declaration) => {
                if parser.root_seen
                    || declaration
                        .version()
                        .map_err(|error| format!("Invalid XML declaration: {error}"))?
                        .as_ref()
                        != b"1.0"
                {
                    return Err("Expected an XML 1.0 declaration before TuneConfig".into());
                }
                if let Some(encoding) = declaration.encoding()
                    && !encoding
                        .map_err(|error| format!("Invalid XML encoding: {error}"))?
                        .eq_ignore_ascii_case(b"utf-8")
                {
                    return Err("TuneConfig must use UTF-8 XML encoding".into());
                }
            }
            Event::Text(text) => {
                if text.len() > MAX_TEXT_NODE_BYTES {
                    return Err("TuneConfig text node exceeds its byte limit".into());
                }
                let text = text
                    .decode()
                    .map_err(|error| format!("Invalid TuneConfig text: {error}"))?;
                if parser.tags.is_empty() && !text.trim().is_empty() {
                    return Err("Unexpected text outside TuneConfig root".into());
                }
            }
            Event::CData(text) => {
                if text.len() > MAX_TEXT_NODE_BYTES || parser.tags.is_empty() {
                    return Err("Invalid or oversized TuneConfig CDATA".into());
                }
                text.decode()
                    .map_err(|error| format!("Invalid TuneConfig CDATA: {error}"))?;
            }
            Event::GeneralRef(reference) => {
                if parser.tags.is_empty() {
                    return Err("Unexpected entity reference outside TuneConfig root".into());
                }
                let name = reference
                    .decode()
                    .map_err(|error| format!("Invalid XML reference: {error}"))?;
                let character = reference
                    .resolve_char_ref()
                    .map_err(|error| format!("Invalid XML character reference: {error}"))?;
                if !matches!(name.as_ref(), "amp" | "lt" | "gt" | "apos" | "quot")
                    && character.is_none()
                {
                    return Err("Custom TuneConfig entity references are unsupported".into());
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    parser.finish()
}

fn element_name(bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > 256 {
        return Err("TuneConfig element name is too long".into());
    }
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|error| format!("Invalid XML element name: {error}"))
}

type Attributes = BTreeMap<String, String>;

fn attributes(element: &BytesStart<'_>, reader: &Reader<&[u8]>) -> Result<Attributes, String> {
    let mut result = BTreeMap::new();
    for attribute in element.attributes() {
        if result.len() >= 64 {
            return Err("Too many attributes on a TuneConfig element".into());
        }
        let attribute =
            attribute.map_err(|error| format!("Invalid TuneConfig attribute: {error}"))?;
        if attribute.value.len() > MAX_ATTRIBUTE_BYTES {
            return Err("TuneConfig attribute exceeds its byte limit".into());
        }
        let key = element_name(attribute.key.as_ref())?;
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| format!("Invalid TuneConfig attribute value: {error}"))?
            .into_owned();
        result.insert(key, value);
    }
    Ok(result)
}

fn required<'a>(attributes: &'a Attributes, key: &str) -> Result<&'a str, String> {
    attributes
        .get(key)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("TuneConfig element is missing {key}"))
}

fn number(text: &str) -> Result<u32, String> {
    let value = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16)
    } else {
        text.parse()
    };
    value.map_err(|_| format!("Invalid TuneConfig unsigned integer: {text}"))
}

struct Value {
    depth: usize,
    kind: Option<String>,
    unit: String,
    enum_id: Option<u32>,
}

struct Item {
    depth: usize,
    kind: String,
    path: String,
    output: Option<String>,
    properties_depth: Option<usize>,
    value: Option<Value>,
    address: Option<(u32, u32)>,
    decimals: Option<u8>,
    display_unit: Option<String>,
}

struct PendingChannel {
    definition: ChannelDefinition,
    enum_id: Option<u32>,
}

struct EnumTable {
    depth: usize,
    id: u32,
    labels: BTreeMap<i32, String>,
}

struct Parser {
    tags: Vec<String>,
    items: Vec<Item>,
    current_enum: Option<EnumTable>,
    enums: BTreeMap<u32, BTreeMap<i32, String>>,
    channels: BTreeMap<(u32, u32), PendingChannel>,
    events: BTreeMap<u32, f64>,
    unit_conversion_warnings: BTreeMap<(u32, u32), String>,
    remaining_items: usize,
    nodes: usize,
    owned_text: usize,
    root_seen: bool,
    root_closed: bool,
}

impl Parser {
    fn new(max_items: usize) -> Self {
        Self {
            tags: Vec::new(),
            items: Vec::new(),
            current_enum: None,
            enums: BTreeMap::new(),
            channels: BTreeMap::new(),
            events: BTreeMap::new(),
            unit_conversion_warnings: BTreeMap::new(),
            remaining_items: max_items,
            nodes: 0,
            owned_text: 0,
            root_seen: false,
            root_closed: false,
        }
    }

    fn consume_item(&mut self) -> Result<(), String> {
        self.remaining_items = self
            .remaining_items
            .checked_sub(1)
            .ok_or("TuneConfig retained-item budget exceeded")?;
        Ok(())
    }

    fn text_budget(&mut self, bytes: usize) -> Result<(), String> {
        self.owned_text = self
            .owned_text
            .checked_add(bytes)
            .filter(|total| *total <= MAX_OWNED_TEXT_BYTES)
            .ok_or("TuneConfig owned text budget exceeded")?;
        Ok(())
    }

    fn start(&mut self, name: String, attributes: &Attributes) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES || self.tags.len() >= MAX_DEPTH {
            return Err("TuneConfig XML node/depth budget exceeded".into());
        }
        if self.tags.is_empty() {
            if self.root_seen || name != "TuneConfig" {
                return Err("Expected one TuneConfig XML root".into());
            }
            self.root_seen = true;
        }
        self.tags.push(name.clone());
        let depth = self.tags.len();
        let parent_is_properties = self
            .tags
            .get(depth.saturating_sub(2))
            .is_some_and(|tag| tag == "Properties");
        let parent_is_value = self
            .tags
            .get(depth.saturating_sub(2))
            .is_some_and(|tag| tag == "Value");
        if name.starts_with("BuiltIn.") {
            let segment = required(attributes, "Name")?;
            let path = match self.items.last() {
                Some(parent) => format!("{}.{}", parent.path, segment),
                None => segment.to_owned(),
            };
            if path.len() > MAX_ATTRIBUTE_BYTES {
                return Err("TuneConfig item path is too long".into());
            }
            self.items.push(Item {
                depth,
                kind: name,
                path,
                output: None,
                properties_depth: None,
                value: None,
                address: None,
                decimals: None,
                display_unit: None,
            });
            return Ok(());
        }
        if self.tags.len() == 2
            && name == "EnumTable"
            && attributes
                .get("Namespace")
                .is_some_and(|namespace| namespace != "_DataType")
        {
            return Err("Unsupported TuneConfig enum namespace".into());
        }
        if self.path_is(&["TuneConfig", "EnumTable", "Enum"]) {
            self.consume_item()?;
            self.current_enum = Some(EnumTable {
                depth,
                id: number(required(attributes, "ID")?)?,
                labels: BTreeMap::new(),
            });
        } else if let Some(table) = &self.current_enum
            && depth == table.depth + 1
            && name == "EnumItem"
        {
            self.consume_item()?;
            let label = required(attributes, "Name")?.to_owned();
            self.text_budget(label.len())?;
            let value = required(attributes, "Value")?
                .parse::<i32>()
                .map_err(|_| "Invalid TuneConfig signed enum value")?;
            if self
                .current_enum
                .as_mut()
                .unwrap()
                .labels
                .insert(value, label)
                .is_some()
            {
                return Err("Duplicate TuneConfig enum value".into());
            }
        } else if self.path_is(&["TuneConfig", "System", "TargetModel", "Events", "Event"]) {
            self.consume_item()?;
            let index = number(required(attributes, "Index")?)?;
            let period: f64 = required(attributes, "Period")?
                .parse()
                .map_err(|_| "Invalid TuneConfig event period")?;
            let rate = period.recip();
            if !period.is_finite() || period <= 0. || !rate.is_finite() || rate <= 0. {
                return Err("TuneConfig event period/rate must be finite and positive".into());
            }
            if self.events.insert(index, rate).is_some() {
                return Err("Duplicate TuneConfig event index".into());
            }
        }
        if let Some(item) = self.items.last_mut() {
            match name.as_str() {
                "Properties" if depth == item.depth + 1 => {
                    if item.properties_depth.replace(depth).is_some() {
                        return Err("Duplicate direct TuneConfig Properties".into());
                    }
                    item.output = attributes.get("ItemValue").cloned();
                }
                "Value" if parent_is_properties && item.properties_depth == Some(depth - 1) => {
                    if item.value.is_some() {
                        return Err("Duplicate direct TuneConfig Value".into());
                    }
                    item.value = Some(Value {
                        depth,
                        kind: attributes.get("Type").cloned(),
                        unit: attributes.get("Unit").cloned().unwrap_or_default(),
                        enum_id: attributes.get("EnumID").map(|id| number(id)).transpose()?,
                    });
                }
                "Address"
                    if parent_is_value
                        && item
                            .value
                            .as_ref()
                            .is_some_and(|value| value.depth == depth - 1) =>
                {
                    if item.address.is_some() {
                        return Err("Duplicate TuneConfig scalar address".into());
                    }
                    item.address = Some((
                        number(required(attributes, "Base")?)?,
                        number(required(attributes, "Offset")?)?,
                    ));
                }
                "DefaultLocale"
                    if parent_is_properties && item.properties_depth == Some(depth - 1) =>
                {
                    item.display_unit = attributes.get("DisplayUnit").cloned();
                    if let Some(decimals) = attributes.get("DisplayDPS") {
                        let decimals: u8 = decimals
                            .parse()
                            .map_err(|_| "Invalid TuneConfig display decimals")?;
                        if decimals > 15 {
                            return Err(
                                "TuneConfig display precision above 15 decimals is unsupported"
                                    .into(),
                            );
                        }
                        item.decimals = Some(decimals);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn path_is(&self, path: &[&str]) -> bool {
        self.tags.len() == path.len()
            && self
                .tags
                .iter()
                .zip(path)
                .all(|(tag, expected)| tag == expected)
    }

    fn end(&mut self, name: &str) -> Result<(), String> {
        if self.tags.last().map(String::as_str) != Some(name) {
            return Err("Mismatched TuneConfig closing element".into());
        }
        let depth = self.tags.len();
        if self
            .current_enum
            .as_ref()
            .is_some_and(|table| table.depth == depth)
        {
            let table = self.current_enum.take().unwrap();
            if self.enums.insert(table.id, table.labels).is_some() {
                return Err("Duplicate TuneConfig enum ID".into());
            }
        }
        if let Some(item) = self.items.pop_if(|item| item.depth == depth) {
            self.add_channel(item)?;
        }
        self.tags.pop();
        if self.tags.is_empty() {
            self.root_closed = true;
        }
        Ok(())
    }

    fn add_channel(&mut self, item: Item) -> Result<(), String> {
        let Some(address) = item.address else {
            return Ok(());
        };
        let value = item
            .value
            .ok_or("TuneConfig scalar address lacks Value metadata")?;
        // Resource selectors have addresses but no scalar type. They cannot
        // silently become channels: a logging request for one is unresolved.
        if value.kind.is_none() && item.kind == "BuiltIn.IOResourceParameter" {
            return Ok(());
        }
        let data_type = match value.kind.as_deref() {
            Some("f32") => ValueType::F32,
            Some("u32") => ValueType::U32,
            Some("s32") => ValueType::I32,
            Some("s32e7") => ValueType::I32E7,
            Some("enum") => ValueType::Enum,
            kind => {
                return Err(format!(
                    "Unsupported addressed TuneConfig scalar type {kind:?} at {}",
                    item.path
                ));
            }
        };
        if data_type == ValueType::Enum && value.enum_id.is_none() {
            return Err(format!("Enum channel {} lacks EnumID", item.path));
        }
        self.consume_item()?;
        let mut path = item.path.as_str();
        // Table output wrappers may form more than one ItemValue alias. Each
        // resolved ancestor is a strictly shorter path, so cycles cannot loop.
        while let Some(ancestor) = self.items.iter().find(|ancestor| {
            ancestor.path.len() < path.len() && ancestor.output.as_deref() == Some(path)
        }) {
            path = &ancestor.path;
        }
        let name = path.replace('.', " ");
        let (unit, display_scale, display_offset) = match item.display_unit.as_deref() {
            Some(display) => match locale_conversion(&value.unit, display) {
                Some((scale, offset)) => (display.to_owned(), scale, offset),
                None => {
                    let warning = format!(
                        "Channel {name}: unsupported display conversion {:?} to {:?}; physical units retained",
                        value.unit, display
                    );
                    self.text_budget(warning.len())?;
                    self.unit_conversion_warnings.insert(address, warning);
                    (value.unit, 1., 0.)
                }
            },
            None => (value.unit, 1., 0.),
        };
        self.text_budget(name.len() + unit.len())?;
        let display_decimals = item.decimals.unwrap_or(match data_type {
            ValueType::F32 => 3,
            ValueType::I32E7 => 7,
            _ => 0,
        });
        let definition = ChannelDefinition {
            name,
            unit,
            data_type,
            enum_labels: BTreeMap::new(),
            display_decimals,
            display_scale,
            display_offset,
        };
        if self
            .channels
            .insert(
                address,
                PendingChannel {
                    definition,
                    enum_id: value.enum_id,
                },
            )
            .is_some()
        {
            return Err(format!(
                "Duplicate TuneConfig mapped address {}:0x{:08x}",
                address.0, address.1
            ));
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Catalog, String> {
        if !self.root_seen || !self.root_closed || !self.tags.is_empty() {
            return Err("Incomplete TuneConfig XML document".into());
        }
        let mut channels = BTreeMap::new();
        let mut label_copies = 0usize;
        for (address, mut channel) in self.channels {
            if channel.definition.data_type == ValueType::Enum {
                let id = channel.enum_id.unwrap();
                let labels = self
                    .enums
                    .get(&id)
                    .ok_or_else(|| format!("Missing TuneConfig enum table {id}"))?;
                label_copies = label_copies
                    .checked_add(labels.len())
                    .filter(|count| *count <= MAX_LABEL_COPIES)
                    .ok_or("TuneConfig enum label-copy budget exceeded")?;
                for label in labels.values() {
                    self.owned_text = self
                        .owned_text
                        .checked_add(label.len())
                        .filter(|count| *count <= MAX_OWNED_TEXT_BYTES)
                        .ok_or("TuneConfig owned text budget exceeded")?;
                }
                channel.definition.enum_labels = labels.clone();
            }
            channels.insert(address, channel.definition);
        }
        Ok(Catalog {
            channels,
            events: self.events,
            sys_clock_hz: None,
            unit_conversion_warnings: self.unit_conversion_warnings,
        })
    }
}

/// Source/display pairs present in the matched TuneConfig and Tune LD fixture.
/// Metric/time factors use their exact definitions. Sign changes and %Trim's
/// affine offset were also checked against nonconstant vendor sample values.
fn locale_conversion(source: &str, display: &str) -> Option<(f64, f64)> {
    if source == display {
        return Some((1., 0.));
    }
    Some(match (source, display) {
        ("ratio", "%") => (100., 0.),
        ("ratio", "%Trim") => (100., -100.),
        ("deg/s", "rpm") => (1. / 6., 0.),
        ("deg", "dBTDC" | "dADV") => (-1., 0.),
        ("deg", "dRTD") => (1., 0.),
        ("Pa", "kPa" | "kPa a") | ("dPa", "dkPa" | "kPa g") => (0.001, 0.),
        ("kg/s", "g/s") | ("m3", "l") | ("m", "mm") => (1000., 0.),
        ("kg", "mg") => (1_000_000., 0.),
        ("m", "km") => (0.001, 0.),
        ("m/s", "km/h") => (3.6, 0.),
        ("m/s", "mph") => (1. / 0.44704, 0.),
        ("m/s/s", "G") => (1. / 9.80665, 0.),
        ("s", "hour") => (1. / 3600., 0.),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<TuneConfig><System><TargetModel><Events><Event Index="4" Period="0.05"/></Events></TargetModel></System><EnumTable Namespace="_DataType"><Enum ID="3"><EnumItem Name="Rückwärts" Value="-1"/><EnumItem Name="Neutral &amp; idle" Value="0"/></Enum></EnumTable><Device><Items><BuiltIn.GroupCompound Name="Gear"><Properties ItemValue="Gear.Value"/><Items><BuiltIn.Channel Name="Value"><Properties><Value Type="enum" EnumID="3"><Address Base="1" Offset="0x0100"/></Value><DefaultLocale DisplayDPS="0"/></Properties></BuiltIn.Channel></Items></BuiltIn.GroupCompound><BuiltIn.Channel Name="GPS Latitude"><Properties><Value Type="s32e7" Unit="deg"><Address Base="1" Offset="260"/></Value><DefaultLocale DisplayDPS="7"/></Properties></BuiltIn.Channel></Items></Device></TuneConfig>"#;

    #[test]
    fn resolves_address_types_aliases_signed_enums_unicode_and_rates() {
        let catalog = parse_catalog(XML.as_bytes(), 20).unwrap();
        assert_eq!(catalog.events[&4], 20.);
        assert_eq!(catalog.channels[&(1, 256)].name, "Gear");
        assert_eq!(catalog.channels[&(1, 256)].enum_labels[&-1], "Rückwärts");
        assert_eq!(
            catalog.channels[&(1, 256)].enum_labels[&0],
            "Neutral & idle"
        );
        let gps = &catalog.channels[&(1, 260)];
        assert_eq!(gps.data_type, ValueType::I32E7);
        assert_eq!(
            (
                gps.unit.as_str(),
                gps.display_decimals,
                gps.display_scale,
                gps.display_offset
            ),
            ("deg", 7, 1., 0.)
        );
        assert_eq!(catalog.sys_clock_hz, None);
    }

    #[test]
    fn rejects_duplicate_addresses_unknown_types_and_unresolved_enums() {
        let duplicate = XML.replace("Offset=\"260\"", "Offset=\"256\"");
        assert!(
            parse_catalog(duplicate.as_bytes(), 20)
                .unwrap_err()
                .contains("Duplicate")
        );
        assert!(
            parse_catalog(XML.replace("s32e7", "f64").as_bytes(), 20)
                .unwrap_err()
                .contains("Unsupported")
        );
        assert!(
            parse_catalog(XML.replace("EnumID=\"3\"", "EnumID=\"4\"").as_bytes(), 20)
                .unwrap_err()
                .contains("Missing")
        );
    }

    #[test]
    fn rejects_dtds_invalid_document_structure_and_invalid_rates() {
        for xml in [
            "<!DOCTYPE TuneConfig SYSTEM 'file:///private/secret'><TuneConfig/>",
            "<TuneConfig>",
            "<TuneConfig/><TuneConfig/>",
            "<Other/>",
            "text<TuneConfig/>",
            "<TuneConfig/>&amp;",
            "<TuneConfig>&#invalid;</TuneConfig>",
        ] {
            assert!(parse_catalog(xml.as_bytes(), 20).is_err(), "{xml}");
        }
        for period in ["0", "-1", "NaN", "inf", "1e-320"] {
            assert!(
                parse_catalog(XML.replace("0.05", period).as_bytes(), 20).is_err(),
                "{period}"
            );
        }
    }

    #[test]
    fn enforces_item_depth_and_attribute_budgets() {
        assert!(parse_catalog(XML.as_bytes(), 2).is_err());
        let deep = format!(
            "<TuneConfig>{}{}</TuneConfig>",
            "<x>".repeat(128),
            "</x>".repeat(128)
        );
        assert!(parse_catalog(deep.as_bytes(), 20).is_err());
        let oversized = format!(
            "<TuneConfig unused=\"{}\"/>",
            "x".repeat(MAX_ATTRIBUTE_BYTES + 1)
        );
        assert!(parse_catalog(oversized.as_bytes(), 20).is_err());
    }

    #[test]
    fn converts_explicit_locale_units_and_ignores_untyped_resource_selectors() {
        let xml = br#"<TuneConfig><Device><Items><BuiltIn.Channel Name="Throttle"><Properties><Value Type="f32" Unit="ratio"><Address Base="1" Offset="0"/></Value><DefaultLocale DisplayUnit="%" DisplayDPS="1"/></Properties></BuiltIn.Channel><BuiltIn.IOResourceParameter Name="Resource"><Properties><Value><Address Base="0" Offset="8"/></Value></Properties></BuiltIn.IOResourceParameter></Items></Device></TuneConfig>"#;
        let catalog = parse_catalog(xml, 10).unwrap();
        assert_eq!(catalog.channels.len(), 1);
        let channel = &catalog.channels[&(1, 0)];
        assert_eq!(channel.unit, "%");
        assert_eq!(channel.display_scale, 100.);
        assert_eq!(channel.display_decimals, 1);
    }

    #[test]
    fn resolves_chained_table_aliases_and_affine_units() {
        let xml = br#"<TuneConfig><Device><Items><BuiltIn.GroupCompound Name="Engine"><Properties ItemValue="Engine.Value"/><Items><BuiltIn.Table Name="Value"><Properties ItemValue="Engine.Value.Value"/><Items><BuiltIn.Channel Name="Value"><Properties><Value Type="f32" Unit="ratio"><Address Base="1" Offset="0"/></Value><DefaultLocale DisplayUnit="%Trim"/></Properties></BuiltIn.Channel></Items></BuiltIn.Table></Items></BuiltIn.GroupCompound></Items></Device></TuneConfig>"#;
        let catalog = parse_catalog(xml, 10).unwrap();
        let channel = &catalog.channels[&(1, 0)];
        assert_eq!(channel.name, "Engine");
        assert_eq!(channel.unit, "%Trim");
        assert_eq!(channel.display_scale, 100.);
        assert_eq!(channel.display_offset, -100.);
        assert!((1.05 * channel.display_scale + channel.display_offset - 5.).abs() < 1e-12);
        assert_eq!(locale_conversion("deg", "dADV"), Some((-1., 0.)));
        assert_eq!(locale_conversion("m/s", "mph"), Some((1. / 0.44704, 0.)));
        assert_eq!(locale_conversion("m/s/s", "G"), Some((1. / 9.80665, 0.)));
        assert_eq!(locale_conversion("s", "hour"), Some((1. / 3600., 0.)));
    }

    #[test]
    fn unknown_locale_conversion_keeps_physical_unit_and_addressed_warning() {
        let xml = br#"<TuneConfig><Device><Items><BuiltIn.Channel Name="Sensor"><Properties><Value Type="f32" Unit="mystery"><Address Base="1" Offset="4"/></Value><DefaultLocale DisplayUnit="%"/></Properties></BuiltIn.Channel></Items></Device></TuneConfig>"#;
        let catalog = parse_catalog(xml, 10).unwrap();
        let channel = &catalog.channels[&(1, 4)];
        assert_eq!(channel.unit, "mystery");
        assert_eq!((channel.display_scale, channel.display_offset), (1., 0.));
        assert!(catalog.unit_conversion_warnings[&(1, 4)].contains("physical units retained"));
        assert!(catalog.unit_conversion_warnings[&(1, 4)].contains("Sensor"));
    }

    #[test]
    fn optional_real_catalog_fixture_matches_known_structure() {
        let Some(path) = std::env::var_os("I3RS_M1_CATALOG_XML") else {
            return;
        };
        let xml = std::fs::read(path).unwrap();
        let catalog = parse_catalog(&xml, 20_000).unwrap();
        assert_eq!(catalog.events[&4], 20.);
        assert_eq!(catalog.channels[&(1, 0x300)].data_type, ValueType::F32);
        assert_eq!(catalog.channels[&(1, 0x300)].name, "Engine Speed");
        assert_eq!(catalog.channels[&(1, 0x300)].unit, "rpm");
        assert_eq!(catalog.channels[&(1, 0x300)].display_scale, 1. / 6.);
        assert_eq!(catalog.channels[&(1, 0x1fac)].data_type, ValueType::I32E7);
        assert!(catalog.channels.len() > 3000);
    }
}
