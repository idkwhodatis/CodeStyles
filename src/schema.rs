//! Conservative controls generated from the exact bundled plugin schemas.
//!
//! Schema defaults are documentation, not a resolved configuration: presets,
//! global settings, and formatter inheritance can change effective values. A
//! missing key stays missing until an explicit Set or Reset operation.
use crate::config::ConfigDocument;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PinnedPlugin {
    TypeScript,
    Json,
    Markdown,
    Toml,
    Ruff,
    /// Managed native formatter; not a dprint WASM plugin.
    Rustfmt,
    Gofmt,
    Dartfmt,
    Markup,
    Malva,
}

impl PinnedPlugin {
    pub const ALL: [Self; 5] = [
        Self::TypeScript,
        Self::Json,
        Self::Markdown,
        Self::Toml,
        Self::Ruff,
    ];

    pub fn section(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Json => "json",
            Self::Markdown => "markdown",
            Self::Toml => "toml",
            Self::Ruff => "ruff",
            Self::Rustfmt => "rustfmt",
            Self::Gofmt => "gofmt",
            Self::Dartfmt => "dartfmt",
            Self::Markup => "markup",
            Self::Malva => "malva",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::TypeScript => "TypeScript",
            Self::Json => "JSON",
            Self::Markdown => "Markdown",
            Self::Toml => "TOML",
            Self::Ruff => "Python (Ruff)",
            Self::Rustfmt => "Rust (managed rustfmt)",
            Self::Gofmt => "Go (gofmt, fixed style)",
            Self::Dartfmt => "Dart / Flutter",
            Self::Markup => "Vue template",
            Self::Malva => "Vue styles",
        }
    }
    pub fn version(self) -> &'static str {
        match self {
            Self::TypeScript => "0.96.1",
            Self::Json => "0.25.2",
            Self::Markdown => "0.26.0",
            Self::Toml => "0.9.0",
            Self::Ruff => "0.11.3",
            Self::Rustfmt => "1.99.1",
            Self::Gofmt => "1.27.3",
            Self::Dartfmt => "3.13.7",
            Self::Markup => "0.27.6",
            Self::Malva => "0.16.0",
        }
    }
    pub fn url(self) -> &'static str {
        match self {
            Self::TypeScript => "https://plugins.dprint.dev/typescript-0.96.1.wasm",
            Self::Json => "https://plugins.dprint.dev/json-0.25.2.wasm",
            Self::Markdown => "https://plugins.dprint.dev/markdown-0.26.0.wasm",
            Self::Toml => "https://plugins.dprint.dev/toml-0.9.0.wasm",
            Self::Ruff => "https://plugins.dprint.dev/ruff-0.11.3.wasm",
            Self::Rustfmt | Self::Gofmt | Self::Dartfmt => "", // Not dprint plugins.
            Self::Markup => "https://plugins.dprint.dev/g-plane/markup_fmt-v0.27.6.wasm",
            Self::Malva => "https://plugins.dprint.dev/g-plane/malva-v0.16.0.wasm",
        }
    }

    /// Resource filename shared by runtime verification and portable exports.
    pub fn filename(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript-0.96.1.wasm",
            Self::Json => "json-0.25.2.wasm",
            Self::Markdown => "markdown-0.26.0.wasm",
            Self::Toml => "toml-0.9.0.wasm",
            Self::Ruff => "ruff-0.11.3.wasm",
            Self::Rustfmt | Self::Gofmt | Self::Dartfmt => "",
            Self::Markup => "markup-0.27.6.wasm",
            Self::Malva => "malva-0.16.0.wasm",
        }
    }

    /// Identity must be unambiguous. Inherited configuration and unfamiliar
    /// plugins can replace a namespace, so we never guess from a filename.
    pub fn ensure_available(self, document: &ConfigDocument) -> Result<(), String> {
        if matches!(
            self,
            Self::Rustfmt | Self::Gofmt | Self::Dartfmt | Self::Markup | Self::Malva
        ) {
            if document.value(&["extends"])?.is_some()
                || document
                    .value(&["inherit"])?
                    .is_some_and(|v| v != Value::Bool(false))
                || document.value(&["overrides"])?.is_some()
            {
                return Err("Managed language settings do not resolve dprint inheritance or overrides. Use a separate explicit configuration.".into());
            }
            return Ok(());
        }
        if document.value(&["extends"])?.is_some()
            || document
                .value(&["inherit"])?
                .is_some_and(|value| value != Value::Bool(false))
        {
            return Err(
                "Inherited configuration has unresolved plugin identity. Use the raw JSONC editor."
                    .into(),
            );
        }
        let Some(Value::Array(plugins)) = document.value(&["plugins"])? else {
            return Err(
                "No explicit plugin list identifies a bundled schema. Use the raw JSONC editor."
                    .into(),
            );
        };
        if !plugins.iter().all(|value| {
            value.as_str().is_some_and(|url| {
                Self::ALL
                    .iter()
                    .chain([Self::Markup, Self::Malva].iter())
                    .any(|plugin| plugin.url() == url)
            })
        }) {
            return Err("The plugin list includes an unknown identity or version. Use the raw JSONC editor.".into());
        }
        if !plugins
            .iter()
            .any(|value| value.as_str() == Some(self.url()))
        {
            return Err(format!(
                "{} {} is not explicitly configured. In Edit JSONC, add \"{}\" to the plugins array to enable these controls. Existing plugin references are not changed automatically.",
                self.title(),
                self.version(),
                self.url()
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EnumChoice {
    /// Retain JSON types; the string "true" is not the boolean true.
    pub value: Value,
    pub description: String,
}

impl EnumChoice {
    pub fn label(&self) -> String {
        self.value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.value.to_string())
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NumberConstraints {
    pub integer: bool,
    /// Some formatter wire formats reject `1.0` even though JSON Schema calls it an integer.
    pub unsigned_integer_encoding: bool,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub exclusive_minimum: Option<f64>,
    pub exclusive_maximum: Option<f64>,
    pub multiple_of: Option<f64>,
}

impl NumberConstraints {
    pub fn validate(&self, value: &Value) -> Result<(), String> {
        if self.unsigned_integer_encoding && value.as_u64().is_none() {
            return Err("Expected an unsigned integer without a decimal point or exponent.".into());
        }
        let number = value
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or("Expected a finite JSON number.")?;
        if self.integer && number.fract() != 0.0 {
            return Err("Expected a whole number.".into());
        }
        if self.minimum.is_some_and(|minimum| number < minimum) {
            return Err(format!("Must be at least {}.", self.minimum.unwrap()));
        }
        if self.maximum.is_some_and(|maximum| number > maximum) {
            return Err(format!("Must be at most {}.", self.maximum.unwrap()));
        }
        if self
            .exclusive_minimum
            .is_some_and(|minimum| number <= minimum)
        {
            return Err(format!(
                "Must be greater than {}.",
                self.exclusive_minimum.unwrap()
            ));
        }
        if self
            .exclusive_maximum
            .is_some_and(|maximum| number >= maximum)
        {
            return Err(format!(
                "Must be less than {}.",
                self.exclusive_maximum.unwrap()
            ));
        }
        if let Some(multiple) = self.multiple_of {
            let quotient = number / multiple;
            // Decimal JSON multiples (0.3 / 0.1) need a small relative allowance
            // for binary floating-point representation, not a broad epsilon.
            let tolerance = f64::EPSILON * quotient.abs().max(1.0) * 4.0;
            if !quotient.is_finite() || (quotient - quotient.round()).abs() > tolerance {
                return Err(format!("Must be a multiple of {multiple}."));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Boolean,
    Number(NumberConstraints),
    /// An explicit JSON null is distinct from removing the setting (Unset).
    NullableNumber(NumberConstraints),
    String,
    Enum(Vec<EnumChoice>),
    Const(Value),
    /// No partial editor for arrays, objects, compositions, or unknown rules.
    Raw {
        reason: String,
    },
}

impl FieldKind {
    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Raw { .. })
    }
}

#[derive(Clone, Debug)]
pub struct SchemaField {
    /// A dotted name is one literal property, not a nested object path.
    pub name: String,
    pub description: String,
    pub group: String,
    pub default: Option<Value>,
    pub kind: FieldKind,
}

/// Schema identity comes from the actual formatting pipeline, never guessed plugins.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SchemaContext {
    #[default]
    Standalone,
    /// The Vue adapter injects its fixed four trusted plugins in an isolated config.
    ManagedVue,
}

impl SchemaContext {
    fn ensure_available(
        self,
        plugin: PinnedPlugin,
        document: &ConfigDocument,
    ) -> Result<(), String> {
        match self {
            Self::Standalone => plugin.ensure_available(document),
            Self::ManagedVue => {
                if !matches!(
                    plugin,
                    PinnedPlugin::TypeScript
                        | PinnedPlugin::Json
                        | PinnedPlugin::Markup
                        | PinnedPlugin::Malva
                ) {
                    return Err(
                        "This schema is not part of the managed Vue formatter chain.".into(),
                    );
                }
                crate::config::validate_vue_config(document.draft_text())?;
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldState {
    pub explicit: Option<Value>,
    /// Schema annotation only; never presented as a resolved effective value.
    pub schema_default: Option<Value>,
    pub validation_error: Option<String>,
}

impl FieldState {
    pub fn is_explicit(&self) -> bool {
        self.explicit.is_some()
    }
}

/// Stable broad tabs; schema scopes are subgroups rather than separate tabs.
pub const SETTING_CATEGORIES: [&str; 8] = [
    "Tabs and Indents",
    "Spaces",
    "Wrapping and Braces",
    "Blank Lines",
    "Punctuation",
    "Imports",
    "Arrangement",
    "Other",
];

/// JetBrains-style formatting concerns, limited to tabs supported by this schema.
/// No catch-all view: every property belongs to exactly one populated category.
pub fn setting_categories(fields: &[SchemaField]) -> Vec<&'static str> {
    SETTING_CATEGORIES
        .into_iter()
        .filter(|category| fields.iter().any(|field| field.category() == *category))
        .collect()
}

/// Turn schema identifiers into compact labels without losing dotted suffixes.
fn setting_words(name: &str) -> String {
    let mut words = String::new();
    for (index, ch) in name.chars().enumerate() {
        if ch == '.' {
            words.push_str(" · ");
        } else if ch.is_uppercase() {
            if index > 0 {
                words.push(' ');
            }
            words.extend(ch.to_lowercase());
        } else {
            words.push(ch);
        }
    }
    if let Some(first) = words.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    words
        .replace("Jsx", "JSX")
        .replace("Json", "JSON")
        .replace("Html", "HTML")
}

impl SchemaField {
    pub fn category(&self) -> &'static str {
        // Match the option, not its scope: import spacing belongs to Spaces,
        // while import sorting belongs to Imports, just as in WebStorm.
        let name = self.name.to_ascii_lowercase();
        let option = self
            .name
            .split_once('.')
            .map_or(self.name.as_str(), |(_, option)| option)
            .to_ascii_lowercase();
        if option == "reorder_imports" {
            "Imports"
        } else if option == "reorder_modules" {
            "Arrangement"
        } else if matches!(option.as_str(), "edition" | "style_edition") {
            "Other"
        } else if option.contains("sort") {
            if name.contains("import") || name.contains("export") {
                "Imports"
            } else {
                "Arrangement"
            }
        } else if matches!(self.name.as_str(), "locked" | "deno" | "tags")
            || [
                "ignore",
                "skipformat",
                "raisesyntaxerrors",
                "applyconventions",
            ]
            .iter()
            .any(|part| option.contains(part))
        {
            "Other"
        } else if option.contains("indent")
            || matches!(option.as_str(), "usetabs" | "hard_tabs" | "tab_spaces")
        {
            "Tabs and Indents"
        } else if option.contains("blankline") {
            "Blank Lines"
        } else if option.contains("space")
            || option.contains("spacing")
            || option.contains("padding")
        {
            // wrapUnspacedScripts controls line wrapping, not spaces.
            if option.starts_with("wrap") {
                "Wrapping and Braces"
            } else {
                "Spaces"
            }
        } else if [
            "quote",
            "comma",
            "semicolon",
            "separator",
            "parentheses",
            "marker",
            "emphasiskind",
            "strongkind",
            "headingkind",
            "unorderedlistkind",
        ]
        .iter()
        .any(|part| option.contains(part))
            || self.name == "heading.kind"
        {
            "Punctuation"
        } else {
            "Wrapping and Braces"
        }
    }

    pub fn subgroup_label(&self) -> String {
        // Base settings are displayed directly; never repeat the tab heading or
        // add a synthetic General/Defaults heading above them.
        if self.group == "General" {
            String::new()
        } else {
            setting_words(&self.group)
        }
    }

    pub fn short_label(&self) -> String {
        setting_words(
            self.name
                .split_once('.')
                .map_or(self.name.as_str(), |(_, suffix)| suffix),
        )
    }

    pub fn validate(&self, value: &Value) -> Result<(), String> {
        match &self.kind {
            FieldKind::Boolean if value.is_boolean() => Ok(()),
            FieldKind::Boolean => Err("Expected a boolean (true or false).".into()),
            FieldKind::Number(constraints) => constraints.validate(value),
            FieldKind::NullableNumber(_) if value.is_null() => Ok(()),
            FieldKind::NullableNumber(constraints) => constraints.validate(value),
            FieldKind::String if value.is_string() => Ok(()),
            FieldKind::String => Err("Expected a string.".into()),
            FieldKind::Enum(choices) => {
                if choices
                    .iter()
                    .any(|choice| scalar_equal(&choice.value, value))
                {
                    Ok(())
                } else {
                    Err(format!(
                        "Expected one of: {}.",
                        choices
                            .iter()
                            .map(|choice| choice.value.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                }
            }
            FieldKind::Const(expected) if scalar_equal(expected, value) => Ok(()),
            FieldKind::Const(expected) => Err(format!("Expected {expected}.")),
            FieldKind::Raw { reason } => Err(format!("Edit this value in raw JSONC: {reason}")),
        }
    }

    /// Strings are entered literally, including empty strings. Other controls
    /// parse JSON, so malformed, infinite, or wrongly typed input cannot mutate
    /// the draft. Enum UI should normally use its typed `EnumChoice::value`.
    pub fn parse_input(&self, input: &str) -> Result<Value, String> {
        let value = if matches!(self.kind, FieldKind::String) {
            Value::String(input.to_owned())
        } else {
            serde_json::from_str(input).map_err(|error| format!("Invalid JSON value: {error}"))?
        };
        self.validate(&value)?;
        Ok(value)
    }

    pub fn matches(&self, query: &str) -> bool {
        let mut text = format!(
            "{} {} {} {} {} {}",
            self.name,
            self.description,
            self.group,
            self.category(),
            self.subgroup_label(),
            self.short_label()
        );
        if let FieldKind::Enum(choices) = &self.kind {
            for choice in choices {
                text.push_str(&format!(" {} {}", choice.value, choice.description));
            }
        }
        let text = text.to_lowercase();
        query
            .split_whitespace()
            .all(|token| text.contains(&token.to_lowercase()))
    }

    pub fn state(
        &self,
        document: &ConfigDocument,
        plugin: PinnedPlugin,
    ) -> Result<FieldState, String> {
        self.state_in_context(document, plugin, SchemaContext::Standalone)
    }

    pub fn state_in_context(
        &self,
        document: &ConfigDocument,
        plugin: PinnedPlugin,
        context: SchemaContext,
    ) -> Result<FieldState, String> {
        context.ensure_available(plugin, document)?;
        let explicit = document.value(&[plugin.section(), &self.name])?;
        let validation_error = explicit
            .as_ref()
            .filter(|_| self.kind.is_supported())
            .and_then(|value| self.validate(value).err());
        Ok(FieldState {
            explicit,
            schema_default: self.default.clone(),
            validation_error,
        })
    }

    pub fn set(
        &self,
        document: &mut ConfigDocument,
        plugin: PinnedPlugin,
        value: Value,
    ) -> Result<(), String> {
        self.set_in_context(document, plugin, value, SchemaContext::Standalone)
    }

    pub fn set_in_context(
        &self,
        document: &mut ConfigDocument,
        plugin: PinnedPlugin,
        value: Value,
        context: SchemaContext,
    ) -> Result<(), String> {
        context.ensure_available(plugin, document)?;
        self.validate(&value)?;
        document.set(&[plugin.section(), &self.name], value)
    }

    /// Reset writes the documented schema default explicitly. No default means
    /// no reset action; do not make up a zero, false, or empty-string default.
    pub fn reset(&self, document: &mut ConfigDocument, plugin: PinnedPlugin) -> Result<(), String> {
        self.reset_in_context(document, plugin, SchemaContext::Standalone)
    }

    pub fn reset_in_context(
        &self,
        document: &mut ConfigDocument,
        plugin: PinnedPlugin,
        context: SchemaContext,
    ) -> Result<(), String> {
        let value = self
            .default
            .clone()
            .ok_or("This schema does not declare a default. Use Unset to remove the property.")?;
        self.set_in_context(document, plugin, value, context)
    }

    /// Unset removes the property, allowing formatter inheritance/defaults.
    pub fn unset(&self, document: &mut ConfigDocument, plugin: PinnedPlugin) -> Result<(), String> {
        self.unset_in_context(document, plugin, SchemaContext::Standalone)
    }

    pub fn unset_in_context(
        &self,
        document: &mut ConfigDocument,
        plugin: PinnedPlugin,
        context: SchemaContext,
    ) -> Result<(), String> {
        context.ensure_available(plugin, document)?;
        document.remove(&[plugin.section(), &self.name])
    }
}

#[derive(Clone, Debug)]
pub struct PluginSchema {
    pub plugin: PinnedPlugin,
    pub fields: Vec<SchemaField>,
}

impl PluginSchema {
    pub fn from_json(plugin: PinnedPlugin, root: &Value) -> Result<Self, String> {
        if root.get("type").and_then(Value::as_str) != Some("object") {
            return Err("Plugin schema root must describe an object.".into());
        }
        let properties = root
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("Plugin schema has no properties object.")?;
        let fields = properties
            .iter()
            .map(|(name, node)| {
                let resolved = resolve_local(root, node, &mut HashSet::new(), 0);
                let (schema, kind) = match resolved {
                    Ok(schema) => {
                        let kind =
                            parse_kind(&schema).unwrap_or_else(|reason| FieldKind::Raw { reason });
                        (schema, kind)
                    }
                    Err(reason) => (node.clone(), FieldKind::Raw { reason }),
                };
                let description = schema
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let default = schema.get("default").cloned();
                let mut field = SchemaField {
                    name: name.clone(),
                    description,
                    group: name
                        .split_once('.')
                        .map_or("General", |(group, _)| group)
                        .into(),
                    default,
                    kind,
                };
                // The pinned Ruff schema declares these as unconstrained numbers,
                // but the 0.11.3 adapter parses line_length as u16 and indent_width
                // as u8. Supplement the UI contract without altering upstream bytes:
                // https://github.com/dprint/dprint-plugin-ruff/blob/0.11.3/src/configuration/configuration.rs
                // Zero is accepted by the adapter; decimal JSON syntax is not.
                if plugin == PinnedPlugin::Ruff {
                    let maximum = match name.as_str() {
                        "indentWidth" => Some(u8::MAX as f64),
                        "lineLength" => Some(u16::MAX as f64),
                        _ => None,
                    };
                    if let Some(maximum) = maximum {
                        field.kind = FieldKind::Number(NumberConstraints {
                            integer: true,
                            unsigned_integer_encoding: true,
                            minimum: Some(0.0),
                            maximum: Some(maximum),
                            ..Default::default()
                        });
                    }
                }
                if plugin == PinnedPlugin::Rustfmt
                    && matches!(name.as_str(), "max_width" | "tab_spaces")
                    && let FieldKind::Number(constraints) = &mut field.kind
                {
                    constraints.unsigned_integer_encoding = true;
                }
                if field.kind.is_supported()
                    && let Some(default) = &field.default
                    && let Err(error) = field.validate(default)
                {
                    field.kind = FieldKind::Raw {
                        reason: format!("The declared schema default is invalid: {error}"),
                    };
                }
                field
            })
            .collect();
        Ok(Self { plugin, fields })
    }

    pub fn field(&self, name: &str) -> Option<&SchemaField> {
        self.fields.iter().find(|field| field.name == name)
    }

    pub fn matching_fields(&self, query: &str) -> Vec<&SchemaField> {
        self.fields
            .iter()
            .filter(|field| field.matches(query))
            .collect()
    }

    pub fn groups(&self) -> Vec<&str> {
        let groups = self
            .fields
            .iter()
            .map(|field| field.group.as_str())
            .collect::<BTreeSet<_>>();
        let has_general = groups.contains("General");
        std::iter::once("General")
            .filter(|_| has_general)
            .chain(groups.into_iter().filter(|group| *group != "General"))
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct SchemaCatalog {
    schemas: [PluginSchema; 10],
}

impl SchemaCatalog {
    /// Compile the supplied, version-pinned resources into the application.
    /// Loading forms never fetches a schema from the network.
    pub fn bundled() -> Result<Self, String> {
        let typescript =
            serde_json::from_str(include_str!("../resources/schemas/typescript-0.96.1.json"))
                .map_err(|error| format!("Invalid bundled TypeScript schema: {error}"))?;
        let json = serde_json::from_str(include_str!("../resources/schemas/json-0.25.2.json"))
            .map_err(|error| format!("Invalid bundled JSON schema: {error}"))?;
        let markdown =
            serde_json::from_str(include_str!("../resources/schemas/markdown-0.26.0.json"))
                .map_err(|error| format!("Invalid bundled Markdown schema: {error}"))?;
        let toml = serde_json::from_str(include_str!("../resources/schemas/toml-0.9.0.json"))
            .map_err(|error| format!("Invalid bundled TOML schema: {error}"))?;
        let ruff = serde_json::from_str(include_str!("../resources/schemas/ruff-0.11.3.json"))
            .map_err(|error| format!("Invalid bundled Ruff schema: {error}"))?;
        let rustfmt =
            serde_json::from_str(include_str!("../resources/schemas/rustfmt-stable.json"))
                .map_err(|error| format!("Invalid native Rust schema: {error}"))?;
        Ok(Self {
            schemas: [
                PluginSchema::from_json(PinnedPlugin::TypeScript, &typescript)?,
                PluginSchema::from_json(PinnedPlugin::Json, &json)?,
                PluginSchema::from_json(PinnedPlugin::Markdown, &markdown)?,
                PluginSchema::from_json(PinnedPlugin::Toml, &toml)?,
                PluginSchema::from_json(PinnedPlugin::Ruff, &ruff)?,
                PluginSchema::from_json(PinnedPlugin::Rustfmt, &rustfmt)?,
                PluginSchema::from_json(
                    PinnedPlugin::Gofmt,
                    &serde_json::from_str(include_str!("../resources/schemas/gofmt.json"))
                        .map_err(|e| format!("Go schema: {e}"))?,
                )?,
                PluginSchema::from_json(
                    PinnedPlugin::Dartfmt,
                    &serde_json::from_str(include_str!("../resources/schemas/dartfmt.json"))
                        .map_err(|e| format!("Dart schema: {e}"))?,
                )?,
                PluginSchema::from_json(
                    PinnedPlugin::Markup,
                    &serde_json::from_str(include_str!("../resources/schemas/markup-0.27.6.json"))
                        .map_err(|e| format!("Vue schema: {e}"))?,
                )?,
                PluginSchema::from_json(
                    PinnedPlugin::Malva,
                    &serde_json::from_str(include_str!("../resources/schemas/malva-0.16.0.json"))
                        .map_err(|e| format!("Style schema: {e}"))?,
                )?,
            ],
        })
    }

    pub fn schema(&self, plugin: PinnedPlugin) -> &PluginSchema {
        &self.schemas[match plugin {
            PinnedPlugin::TypeScript => 0,
            PinnedPlugin::Json => 1,
            PinnedPlugin::Markdown => 2,
            PinnedPlugin::Toml => 3,
            PinnedPlugin::Ruff => 4,
            PinnedPlugin::Rustfmt => 5,
            PinnedPlugin::Gofmt => 6,
            PinnedPlugin::Dartfmt => 7,
            PinnedPlugin::Markup => 8,
            PinnedPlugin::Malva => 9,
        }]
    }

    pub fn fields(&self, plugin: PinnedPlugin) -> &[SchemaField] {
        &self.schema(plugin).fields
    }

    pub fn available(&self, document: &ConfigDocument, plugin: PinnedPlugin) -> Result<(), String> {
        self.available_in_context(document, plugin, SchemaContext::Standalone)
    }

    pub fn available_in_context(
        &self,
        document: &ConfigDocument,
        plugin: PinnedPlugin,
        context: SchemaContext,
    ) -> Result<(), String> {
        context.ensure_available(plugin, document)
    }

    /// Validate only known, supported values under a provable pinned identity.
    /// Unknown keys/complex fields stay raw-editable and are left to dprint.
    pub fn validate_document(&self, document: &ConfigDocument) -> Result<(), String> {
        for (plugin, language) in [
            (
                PinnedPlugin::Gofmt,
                crate::native_format::NativeLanguage::Go,
            ),
            (
                PinnedPlugin::Dartfmt,
                crate::native_format::NativeLanguage::Dart,
            ),
        ] {
            if document.value(&[plugin.section()])?.is_some()
                && self.available(document, plugin).is_ok()
            {
                crate::native_format::options(document.draft_text(), language)?;
            }
        }
        for plugin in PinnedPlugin::ALL.into_iter().chain([
            PinnedPlugin::Rustfmt,
            PinnedPlugin::Gofmt,
            PinnedPlugin::Dartfmt,
            PinnedPlugin::Markup,
            PinnedPlugin::Malva,
        ]) {
            if self.available(document, plugin).is_err() {
                continue;
            }
            if let Some(section) = document.value(&[plugin.section()])?
                && !section.is_object()
            {
                return Err(format!("{} must be an object.", plugin.section()));
            }
            if plugin == PinnedPlugin::Rustfmt
                && let Some(Value::Object(options)) = document.value(&["rustfmt"])?
            {
                for key in options.keys() {
                    if self.schema(plugin).field(key).is_none() {
                        return Err(format!(
                            "Unsupported stable Rust setting: rustfmt.{key}. Nightly/arbitrary rustfmt options are not enabled."
                        ));
                    }
                }
            }
            for field in self.fields(plugin) {
                if !field.kind.is_supported() {
                    continue;
                }
                if let Some(value) = document.value(&[plugin.section(), &field.name])? {
                    field
                        .validate(&value)
                        .map_err(|error| format!("{}.{}: {error}", plugin.section(), field.name))?;
                }
            }
        }
        Ok(())
    }
}

fn resolve_local(
    root: &Value,
    node: &Value,
    visiting: &mut HashSet<String>,
    depth: usize,
) -> Result<Value, String> {
    if depth > 128 {
        return Err("Schema reference nesting is too deep.".into());
    }
    let object = node
        .as_object()
        .ok_or("Non-object schemas require raw JSONC.")?;
    let Some(reference) = object.get("$ref") else {
        let mut resolved = node.clone();
        if let Some(alternatives) = resolved.get_mut("oneOf").and_then(Value::as_array_mut) {
            for alternative in alternatives {
                *alternative = resolve_local(root, alternative, visiting, depth + 1)?;
            }
        }
        return Ok(resolved);
    };
    let reference = reference.as_str().ok_or("Invalid schema $ref.")?;
    let fragment = reference
        .strip_prefix('#')
        .ok_or("Only local JSON Pointer schema references are supported.")?;
    let pointer = decode_fragment(fragment)?;
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return Err("Only local JSON Pointer schema references are supported.".into());
    }
    if !visiting.insert(pointer.clone()) {
        return Err(format!("Cyclic schema reference: {reference}"));
    }
    let target = root
        .pointer(&pointer)
        .ok_or_else(|| format!("Unresolved schema reference: {reference}"))?;
    let result = resolve_local(root, target, visiting, depth + 1);
    visiting.remove(&pointer);
    // Draft-07 ignores siblings of $ref, including default/type assertions.
    result
}

fn decode_fragment(fragment: &str) -> Result<String, String> {
    let mut output = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex = |byte: Option<u8>| {
                byte.and_then(|byte| (byte as char).to_digit(16))
                    .ok_or("Invalid URI encoding in schema reference.")
            };
            output.push((hex(bytes.next())? * 16 + hex(bytes.next())?) as u8);
        } else {
            output.push(byte);
        }
    }
    String::from_utf8(output).map_err(|_| "Schema reference is not valid UTF-8.".into())
}

fn parse_kind(schema: &Value) -> Result<FieldKind, String> {
    let object = schema.as_object().ok_or("Non-object schema.")?;
    const ALLOWED: &[&str] = &[
        "$id",
        "$schema",
        "$comment",
        "title",
        "description",
        "default",
        "examples",
        "deprecated",
        "readOnly",
        "writeOnly",
        "type",
        "enum",
        "const",
        "oneOf",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ];
    if let Some(key) = object.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!("Unsupported schema rule {key:?}."));
    }
    let (declared_type, nullable) = match object.get("type") {
        None => (None, false),
        Some(Value::String(kind))
            if matches!(kind.as_str(), "boolean" | "number" | "integer" | "string") =>
        {
            (Some(kind.as_str()), false)
        }
        Some(Value::Array(types)) if types.len() == 2 => {
            let primitive = types
                .iter()
                .filter_map(Value::as_str)
                .find(|kind| matches!(*kind, "boolean" | "number" | "integer" | "string"));
            if types
                .iter()
                .filter(|kind| kind.as_str() == Some("null"))
                .count()
                != 1
                || primitive.is_none()
            {
                return Err("Only a primitive plus null union has a typed editor.".into());
            }
            (primitive, true)
        }
        Some(_) => return Err("Complex or union schema types require raw JSONC.".into()),
    };
    let constraints = number_constraints(object, declared_type == Some("integer"))?;
    if !matches!(declared_type, Some("number" | "integer") | None)
        && constraints != NumberConstraints::default()
    {
        return Err("Numeric constraints on a nonnumeric schema require raw JSONC.".into());
    }
    let is_valid_choice = |value: &Value| -> bool {
        if value.is_null() && nullable {
            return true;
        }
        let compatible = match declared_type {
            Some("boolean") => value.is_boolean(),
            Some("number" | "integer") => value.is_number(),
            Some("string") => value.is_string(),
            None => is_scalar(value),
            _ => false,
        };
        compatible && (!value.is_number() || constraints.validate(value).is_ok())
    };
    let alternatives = ["const", "enum", "oneOf"]
        .iter()
        .filter(|key| object.contains_key(**key))
        .count();
    if alternatives > 1 {
        return Err("Combined const/enum/oneOf rules require raw JSONC.".into());
    }
    if let Some(value) = object.get("const") {
        if !is_valid_choice(value) {
            return Err("Const is complex or conflicts with the schema type/constraints.".into());
        }
        return Ok(FieldKind::Const(value.clone()));
    }
    let choices = if let Some(values) = object.get("enum") {
        Some(
            values
                .as_array()
                .ok_or("Invalid enum values.")?
                .iter()
                .map(|value| {
                    Ok(EnumChoice {
                        value: value.clone(),
                        description: String::new(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        )
    } else if let Some(alternatives) = object.get("oneOf") {
        Some(
            alternatives
                .as_array()
                .ok_or("Invalid oneOf alternatives.")?
                .iter()
                .map(|alternative| {
                    let alternative = alternative
                        .as_object()
                        .ok_or("Complex oneOf alternative.")?;
                    if alternative.keys().any(|key| {
                        !["const", "description", "title", "$comment"].contains(&key.as_str())
                    }) {
                        return Err(
                            "Only oneOf alternatives containing scalar const values are supported."
                                .into(),
                        );
                    }
                    Ok(EnumChoice {
                        value: alternative
                            .get("const")
                            .ok_or("Only const oneOf alternatives are supported.")?
                            .clone(),
                        description: alternative
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .into(),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        )
    } else {
        None
    };
    if let Some(choices) = choices {
        if choices.is_empty() || choices.iter().any(|choice| !is_valid_choice(&choice.value)) {
            return Err(
                "Enum choices are empty, complex, or conflict with the schema type/constraints."
                    .into(),
            );
        }
        if choices.iter().enumerate().any(|(index, choice)| {
            choices[..index]
                .iter()
                .any(|previous| scalar_equal(&choice.value, &previous.value))
        }) {
            return Err("Duplicate enum/oneOf choices require raw JSONC.".into());
        }
        if choices.len() == 2 && choices.iter().all(|choice| choice.value.is_boolean()) {
            return Ok(FieldKind::Boolean);
        }
        return Ok(FieldKind::Enum(choices));
    }
    if nullable {
        return match declared_type {
            Some("boolean") => Ok(FieldKind::Enum(
                [Value::Bool(false), Value::Bool(true), Value::Null]
                    .into_iter()
                    .map(|value| EnumChoice {
                        value,
                        description: String::new(),
                    })
                    .collect(),
            )),
            Some("number" | "integer") => Ok(FieldKind::NullableNumber(constraints)),
            _ => Err("Nullable strings without finite choices require raw JSONC.".into()),
        };
    }
    match declared_type {
        Some("boolean") => Ok(FieldKind::Boolean),
        Some("number" | "integer") => Ok(FieldKind::Number(constraints)),
        Some("string") => Ok(FieldKind::String),
        _ => Err("Schema has no supported primitive type or choices.".into()),
    }
}

fn number_constraints(
    object: &Map<String, Value>,
    integer: bool,
) -> Result<NumberConstraints, String> {
    let number = |key: &str| -> Result<Option<f64>, String> {
        object
            .get(key)
            .map(|value| {
                value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| format!("Invalid numeric constraint {key}."))
            })
            .transpose()
    };
    let constraints = NumberConstraints {
        integer,
        unsigned_integer_encoding: false,
        minimum: number("minimum")?,
        maximum: number("maximum")?,
        exclusive_minimum: number("exclusiveMinimum")?,
        exclusive_maximum: number("exclusiveMaximum")?,
        multiple_of: number("multipleOf")?,
    };
    if constraints
        .multiple_of
        .is_some_and(|multiple| multiple <= 0.0)
    {
        return Err("multipleOf must be positive.".into());
    }
    if let (Some(minimum), Some(maximum)) = (constraints.minimum, constraints.maximum)
        && minimum > maximum
    {
        return Err("Numeric schema minimum exceeds maximum.".into());
    }
    Ok(constraints)
}

fn is_scalar(value: &Value) -> bool {
    value.is_boolean() || value.is_number() || value.is_string()
}

fn scalar_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    // JSON Schema treats 1 and 1.0 as the same number. Normalize their decimal
    // spellings rather than rounding large integer choices through f64.
    if let (Some(left), Some(right)) = (left.as_number(), right.as_number()) {
        return normalized_number(left) == normalized_number(right);
    }
    false
}

fn normalized_number(number: &serde_json::Number) -> (bool, String, i32) {
    let text = number.to_string();
    let negative = text.starts_with('-');
    let unsigned = text.trim_start_matches('-');
    let (mantissa, exponent) =
        unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, 0), |(mantissa, exponent)| {
                (
                    mantissa,
                    exponent
                        .parse::<i32>()
                        .expect("finite JSON number exponent"),
                )
            });
    let fractional_digits = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits = mantissa.replace('.', "");
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return (false, String::new(), 0);
    }
    let significant = digits.trim_end_matches('0');
    (
        negative,
        significant.to_owned(),
        exponent - fractional_digits as i32 + (digits.len() - significant.len()) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn schema(properties: Value) -> PluginSchema {
        PluginSchema::from_json(
            PinnedPlugin::Json,
            &json!({"type":"object","properties":properties}),
        )
        .unwrap()
    }

    fn document(value: Value) -> ConfigDocument {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dprint.jsonc");
        fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
        ConfigDocument::load(&path).unwrap()
    }

    #[test]
    fn existing_four_plugin_configuration_explains_python_opt_in_without_mutation() {
        let old_plugins = [
            PinnedPlugin::TypeScript,
            PinnedPlugin::Json,
            PinnedPlugin::Markdown,
            PinnedPlugin::Toml,
        ];
        let document = document(json!({"plugins": old_plugins.map(PinnedPlugin::url)}));
        let before = document.draft_text().to_owned();
        let catalog = SchemaCatalog::bundled().unwrap();
        let error = catalog
            .available(&document, PinnedPlugin::Ruff)
            .unwrap_err();
        assert!(error.contains("Edit JSONC"), "{error}");
        assert!(error.contains("plugins array"), "{error}");
        assert!(error.contains(PinnedPlugin::Ruff.url()), "{error}");
        assert!(error.contains("not changed automatically"), "{error}");
        for plugin in old_plugins {
            assert!(catalog.available(&document, plugin).is_ok());
        }
        assert_eq!(document.draft_text(), before);
        assert!(!document.is_dirty());
    }

    #[test]
    fn ruff_widths_obey_pinned_adapter_integer_wire_contract() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let schema = catalog.schema(PinnedPlugin::Ruff);
        for (name, maximum) in [("indentWidth", 255), ("lineLength", 65535)] {
            let field = schema.field(name).unwrap();
            for value in ["0".to_owned(), "1".to_owned(), maximum.to_string()] {
                assert!(field.parse_input(&value).is_ok(), "{name}: {value}");
            }
            for value in [
                "-1".to_owned(),
                "1.5".to_owned(),
                "1.0".to_owned(),
                "1e1".to_owned(),
                (maximum + 1).to_string(),
            ] {
                assert!(field.parse_input(&value).is_err(), "{name}: {value}");
                let document = document(json!({
                    "plugins": [PinnedPlugin::Ruff.url()],
                    "ruff": {name: serde_json::from_str::<Value>(&value).unwrap()}
                }));
                let error = catalog.validate_document(&document).unwrap_err();
                assert!(error.contains(&format!("ruff.{name}")), "{error}");
            }
        }
        // Do not change the generic JSON Schema integer interpretation.
        let ordinary = PluginSchema::from_json(
            PinnedPlugin::Json,
            &json!({
                "type": "object", "properties": {"width": {"type": "integer"}}
            }),
        )
        .unwrap();
        assert!(ordinary.field("width").unwrap().parse_input("1.0").is_ok());
    }

    #[test]
    fn native_rust_settings_are_explicit_stable_and_do_not_require_a_wasm_url() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let doc = document(json!({"plugins": PinnedPlugin::ALL.map(PinnedPlugin::url)}));
        assert!(catalog.available(&doc, PinnedPlugin::Rustfmt).is_ok());
        assert!(catalog.validate_document(&doc).is_ok());
        for value in [
            json!({"edition":"2027"}),
            json!({"unstable_features":true}),
            json!({"max_width":0}),
            json!({"max_width":1.0}),
            json!({"tab_spaces":17}),
        ] {
            let doc = document(json!({"rustfmt":value}));
            assert!(catalog.validate_document(&doc).is_err());
        }
        let doc =
            document(json!({"rustfmt":{"edition":"2024","style_edition":"2021","max_width":100}}));
        assert!(catalog.validate_document(&doc).is_ok());
        let inherited = document(json!({"extends":"./base.json","rustfmt":{"edition":"2021"}}));
        assert!(
            catalog
                .available(&inherited, PinnedPlugin::Rustfmt)
                .is_err()
        );
        assert!(!PinnedPlugin::ALL.contains(&PinnedPlugin::Rustfmt));
    }

    #[test]
    fn broad_categories_cover_every_field_for_all_languages() {
        let catalog = SchemaCatalog::bundled().unwrap();
        assert_eq!(crate::comparison::SourceLanguage::ALL.len(), 14);
        for language in crate::comparison::SourceLanguage::ALL {
            let fields = catalog.fields(language.plugin());
            let mut seen = std::collections::HashSet::new();
            for category in SETTING_CATEGORIES {
                for field in fields.iter().filter(|field| field.category() == category) {
                    assert!(seen.insert(&field.name), "duplicate {}", field.name);
                    assert_ne!(field.subgroup_label(), "General");
                    assert_ne!(field.subgroup_label(), field.category());
                    assert!(!field.short_label().is_empty());
                    assert!(field.matches(category));
                    assert!(field.matches(&field.short_label()));
                }
            }
            assert_eq!(seen.len(), fields.len(), "{}", language.display_name());
            assert_eq!(seen.is_empty(), language.plugin() == PinnedPlugin::Gofmt);
        }
    }

    #[test]
    fn jetbrains_categories_are_populated_ordered_and_never_all() {
        let catalog = SchemaCatalog::bundled().unwrap();
        for language in crate::comparison::SourceLanguage::ALL {
            let fields = catalog.fields(language.plugin());
            let categories = setting_categories(fields);
            if fields
                .iter()
                .any(|field| field.category() == "Tabs and Indents")
            {
                assert_eq!(categories.first(), Some(&"Tabs and Indents"));
            }
            assert_eq!(
                categories.is_empty(),
                language.plugin() == PinnedPlugin::Gofmt
            );
            assert!(!categories.contains(&"All"));
            assert!(!categories.contains(&"General"));
            assert!(
                categories
                    .iter()
                    .all(|category| fields.iter().any(|field| field.category() == *category))
            );
            assert_eq!(
                categories
                    .iter()
                    .map(|category| fields
                        .iter()
                        .filter(|field| field.category() == *category)
                        .count())
                    .sum::<usize>(),
                fields.len()
            );
        }
        let ts = catalog.schema(PinnedPlugin::TypeScript);
        for (key, category) in [
            ("indentWidth", "Tabs and Indents"),
            ("useTabs", "Tabs and Indents"),
            ("importDeclaration.spaceSurroundingNamedImports", "Spaces"),
            ("importDeclaration.preferHanging", "Wrapping and Braces"),
            ("importDeclaration.trailingCommas", "Punctuation"),
            ("importDeclaration.sortNamedImports", "Imports"),
            ("ifStatement.useBraces", "Wrapping and Braces"),
            ("ignoreFileCommentText", "Other"),
        ] {
            assert_eq!(ts.field(key).unwrap().category(), category, "{key}");
        }
        assert!(ts.field("indentWidth").unwrap().subgroup_label().is_empty());
        assert_eq!(
            catalog
                .schema(PinnedPlugin::Markdown)
                .field("heading.blankLinesAbove")
                .unwrap()
                .category(),
            "Blank Lines"
        );
        assert_eq!(
            catalog
                .schema(PinnedPlugin::Toml)
                .field("sortKeys")
                .unwrap()
                .category(),
            "Arrangement"
        );
        assert!(!setting_categories(catalog.fields(PinnedPlugin::Json)).contains(&"Imports"));
        assert!(
            !setting_categories(catalog.fields(PinnedPlugin::TypeScript)).contains(&"Blank Lines")
        );
    }

    #[test]
    fn short_labels_keep_distinguishing_suffixes_and_original_key_search() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let ts = catalog.schema(PinnedPlugin::TypeScript);
        let field = ts.field("typeLiteral.separatorKind.singleLine").unwrap();
        assert_eq!(field.short_label(), "Separator kind · single line");
        assert_eq!(field.subgroup_label(), "Type literal");
        assert!(field.matches("typeLiteral.separatorKind.singleLine"));
        assert_eq!(
            ts.field("lineWidth").unwrap().category(),
            "Wrapping and Braces"
        );
        assert_eq!(
            ts.field("arguments.spaceAround").unwrap().category(),
            "Spaces"
        );
        assert_eq!(
            ts.field("arrowFunction.useParentheses").unwrap().category(),
            "Punctuation"
        );
        assert_eq!(
            ts.field("module.sortImportDeclarations")
                .unwrap()
                .category(),
            "Imports"
        );
        for plugin in PinnedPlugin::ALL {
            let mut labels = std::collections::HashSet::new();
            for field in catalog.fields(plugin) {
                assert!(
                    labels.insert((
                        field.category(),
                        field.subgroup_label(),
                        field.short_label()
                    )),
                    "ambiguous label: {}",
                    field.name
                );
            }
        }
    }

    #[test]
    fn pinned_resources_expose_every_field_and_exact_defaults() {
        let catalog = SchemaCatalog::bundled().unwrap();
        assert_eq!(catalog.fields(PinnedPlugin::TypeScript).len(), 185);
        assert_eq!(catalog.fields(PinnedPlugin::Json).len(), 16);
        assert_eq!(catalog.fields(PinnedPlugin::Markdown).len(), 37);
        assert_eq!(catalog.fields(PinnedPlugin::Toml).len(), 20);
        assert_eq!(catalog.fields(PinnedPlugin::Ruff).len(), 12);
        assert!(
            catalog
                .fields(PinnedPlugin::TypeScript)
                .iter()
                .all(|field| field.kind.is_supported())
        );
        let json = catalog.schema(PinnedPlugin::Json);
        assert_eq!(
            json.fields
                .iter()
                .filter(|field| field.kind.is_supported())
                .count(),
            15
        );
        assert!(matches!(
            json.field("jsonTrailingCommaFiles").unwrap().kind,
            FieldKind::Raw { .. }
        ));
        for plugin in PinnedPlugin::ALL {
            let schema = catalog.schema(plugin);
            if plugin == PinnedPlugin::Ruff {
                assert_eq!(schema.field("indentWidth").unwrap().default, Some(json!(4)));
                assert_eq!(schema.field("lineLength").unwrap().default, Some(json!(88)));
                assert_eq!(
                    schema.field("quoteStyle").unwrap().default,
                    Some(json!("double"))
                );
                assert!(matches!(
                    schema.field("fixLintErrors").unwrap().kind,
                    FieldKind::Raw { .. }
                ));
                assert!(matches!(
                    schema.field("docstringCodeLineWidth").unwrap().kind,
                    FieldKind::Raw { .. }
                ));
                continue;
            }
            assert_eq!(schema.field("locked").unwrap().default, None);
            if plugin == PinnedPlugin::Markdown {
                assert_eq!(schema.field("lineWidth").unwrap().default, Some(json!(80)));
                assert!(schema.field("indentWidth").is_none());
                assert!(schema.field("useTabs").is_none());
            } else {
                assert_eq!(schema.field("indentWidth").unwrap().default, Some(json!(2)));
                assert_eq!(schema.field("lineWidth").unwrap().default, Some(json!(120)));
                assert_eq!(schema.field("useTabs").unwrap().kind, FieldKind::Boolean);
            }
            for field in &schema.fields {
                assert!(!field.description.is_empty(), "{}", field.name);
                if field.kind.is_supported()
                    && let Some(default) = &field.default
                {
                    assert!(field.validate(default).is_ok(), "{}", field.name);
                }
            }
        }
        let typescript = catalog.schema(PinnedPlugin::TypeScript);
        assert_eq!(
            typescript.field("semiColons").unwrap().default,
            Some(json!("prefer"))
        );
        assert!(matches!(
            typescript.field("semiColons").unwrap().kind,
            FieldKind::Enum(_)
        ));
    }

    #[test]
    fn markdown_and_toml_fields_keep_typed_choices_raw_fallbacks_and_inheritance() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let markdown = catalog.schema(PinnedPlugin::Markdown);
        assert_eq!(
            markdown
                .fields
                .iter()
                .filter(|field| field.kind.is_supported())
                .count(),
            33
        );
        for name in ["tags", "headingKind", "unorderedListKind", "listIndentKind"] {
            assert!(matches!(
                markdown.field(name).unwrap().kind,
                FieldKind::Raw { .. }
            ));
        }
        assert_eq!(markdown.field("heading.kind").unwrap().group, "heading");
        assert_eq!(markdown.field("codeBlock.useTabs").unwrap().default, None);
        assert_eq!(
            markdown.field("codeBlock.indentWidth").unwrap().default,
            None
        );
        assert_eq!(
            markdown.field("textWrap").unwrap().default,
            Some(json!("maintain"))
        );
        assert!(
            markdown
                .field("table.minColumnWidth")
                .unwrap()
                .validate(&json!(0))
                .is_err()
        );
        assert!(
            markdown
                .field("table.minColumnWidth")
                .unwrap()
                .validate(&json!(256))
                .is_err()
        );
        let toml = catalog.schema(PinnedPlugin::Toml);
        assert!(toml.fields.iter().all(|field| field.kind.is_supported()));
        assert_eq!(toml.field("array.preferSingleLine").unwrap().default, None);
        assert_eq!(
            toml.field("sortArrays").unwrap().default,
            Some(json!(false))
        );
        assert!(
            toml.field("sortArrays")
                .unwrap()
                .description
                .contains("meaning")
        );
        for plugin in [PinnedPlugin::Markdown, PinnedPlugin::Toml] {
            let explicit = document(json!({"plugins": [plugin.url()]}));
            assert!(catalog.available(&explicit, plugin).is_ok());
            let unknown = document(json!({"plugins": [plugin.url(), "./custom.wasm"]}));
            assert!(catalog.available(&unknown, plugin).is_err());
            let inherited = document(json!({"plugins": [plugin.url()], "extends": "./base.json"}));
            assert!(catalog.available(&inherited, plugin).is_err());
        }
    }

    #[test]
    fn resolves_chained_and_escaped_local_references_without_sibling_overrides() {
        let root = json!({"type":"object", "definitions":{
            "a/b~c":{"type":"integer","minimum":1,"default":2},
            "alias":{"$ref":"#/definitions/a~1b~0c"}
        }, "properties":{
            "resolved":{"$ref":"#/definitions/alias","default":99,"type":"string"}
        }});
        let schema = PluginSchema::from_json(PinnedPlugin::Json, &root).unwrap();
        let field = schema.field("resolved").unwrap();
        assert_eq!(field.default, Some(json!(2)));
        assert!(field.validate(&json!(1)).is_ok());
        assert!(field.validate(&json!(0)).is_err());
        assert!(field.validate(&json!(1.2)).is_err());
    }

    #[test]
    fn resolves_encoded_and_choice_references_and_defends_nested_cycles() {
        let root = json!({"type":"object", "definitions":{
            "choice a":{"const":"a","description":"First"},
            "choiceB":{"const":"b"},
            "recursive":{"oneOf":[{"$ref":"#/definitions/recursive"}]}
        }, "properties":{
            "choices":{"type":"string","oneOf":[{"$ref":"#/definitions/choice%20a"},{"$ref":"#/definitions/choiceB"}]},
            "cycle":{"$ref":"#/definitions/recursive"},
            "invalidEncoding":{"$ref":"#/definitions/%xz"}
        }});
        let schema = PluginSchema::from_json(PinnedPlugin::Json, &root).unwrap();
        let field = schema.field("choices").unwrap();
        assert!(field.validate(&json!("a")).is_ok());
        assert!(field.validate(&json!("b")).is_ok());
        assert!(field.matches("First"));
        assert!(!schema.field("cycle").unwrap().kind.is_supported());
        assert!(!schema.field("invalidEncoding").unwrap().kind.is_supported());
    }

    #[test]
    fn numeric_choices_do_not_round_large_integer_identities() {
        let schema = schema(json!({"value":{"enum":[9007199254740993u64]}}));
        let field = schema.field("value").unwrap();
        assert!(field.validate(&json!(9007199254740993u64)).is_ok());
        assert!(field.validate(&json!(9007199254740992.0)).is_err());
        for (left, right) in [
            (json!(1), json!(1.0)),
            (json!(100000), json!(1e5)),
            (json!(0), json!(-0.0)),
        ] {
            assert!(scalar_equal(&left, &right));
        }
    }

    #[test]
    fn cycles_missing_and_remote_references_fall_back_per_field() {
        let root = json!({"type":"object", "definitions":{
            "a":{"$ref":"#/definitions/b"},"b":{"$ref":"#/definitions/a"}
        },"properties":{
            "cycle":{"$ref":"#/definitions/a"},
            "self":{"$ref":"#/properties/self"},
            "missing":{"$ref":"#/definitions/missing"},
            "remote":{"$ref":"https://example.test/schema.json"},
            "valid":{"type":"boolean"}
        }});
        let schema = PluginSchema::from_json(PinnedPlugin::Json, &root).unwrap();
        for name in ["cycle", "self", "missing", "remote"] {
            assert!(matches!(
                schema.field(name).unwrap().kind,
                FieldKind::Raw { .. }
            ));
        }
        assert_eq!(schema.field("valid").unwrap().kind, FieldKind::Boolean);
    }

    #[test]
    fn unsupported_rules_and_bad_schema_defaults_remain_raw() {
        let schema = schema(json!({
            "array":{"type":"array","items":{"type":"string"}},
            "object":{"type":"object"},
            "union":{"type":["string","null"]},
            "anyOf":{"anyOf":[{"type":"string"},{"type":"boolean"}]},
            "allOf":{"type":"string","allOf":[{"const":"x"}]},
            "complexOneOf":{"oneOf":[{"type":"string"},{"const":true}]},
            "pattern":{"type":"string","pattern":"x+"},
            "length":{"type":"string","minLength":2},
            "default":{"type":"boolean","default":"false"},
            "complexEnum":{"enum":[{"x":1}]},
            "invalidEnum":{"type":"boolean","enum":["false",true]},
            "duplicateOneOf":{"oneOf":[{"const":1},{"const":1.0}]},
            "invalidMultiple":{"type":"number","multipleOf":0},
            "negativeMultiple":{"type":"number","multipleOf":-2},
            "invalidMinimum":{"type":"number","minimum":"2"},
            "combined":{"enum":[1,2],"const":1},
            "booleanSchema":true
        }));
        assert!(schema.fields.iter().all(|field| !field.kind.is_supported()));
    }

    #[test]
    fn managed_vue_schema_context_does_not_relax_standalone_identity() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let plugin = PinnedPlugin::TypeScript;
        let field = catalog.schema(plugin).field("quoteStyle").unwrap();
        let mut doc = document(json!({}));
        assert!(catalog.available(&doc, plugin).is_err());
        assert!(field.state(&doc, plugin).is_err());
        assert!(field.set(&mut doc, plugin, json!("alwaysSingle")).is_err());
        catalog
            .available_in_context(&doc, plugin, SchemaContext::ManagedVue)
            .unwrap();
        assert_eq!(
            field
                .state_in_context(&doc, plugin, SchemaContext::ManagedVue)
                .unwrap()
                .explicit,
            None
        );
        field
            .set_in_context(
                &mut doc,
                plugin,
                json!("alwaysSingle"),
                SchemaContext::ManagedVue,
            )
            .unwrap();
        assert_eq!(
            doc.value(&["typescript", "quoteStyle"]).unwrap(),
            Some(json!("alwaysSingle"))
        );
        assert_eq!(doc.value(&["plugins"]).unwrap(), None);
        field
            .reset_in_context(&mut doc, plugin, SchemaContext::ManagedVue)
            .unwrap();
        field
            .unset_in_context(&mut doc, plugin, SchemaContext::ManagedVue)
            .unwrap();
        assert!(catalog.available(&doc, plugin).is_err());
        assert!(
            catalog
                .available_in_context(&doc, PinnedPlugin::Rustfmt, SchemaContext::ManagedVue)
                .is_err()
        );

        let mut doc =
            document(json!({"plugins":["custom-local-plugin.wasm"],"future":{"keep":true}}));
        doc.replace_draft_text(&format!("// keep user plugins\n{}", doc.draft_text()))
            .unwrap();
        let original_plugins = doc.value(&["plugins"]).unwrap();
        field
            .set_in_context(
                &mut doc,
                plugin,
                json!("alwaysSingle"),
                SchemaContext::ManagedVue,
            )
            .unwrap();
        assert_eq!(doc.value(&["plugins"]).unwrap(), original_plugins);
        assert!(doc.draft_text().contains("// keep user plugins"));
        assert_eq!(doc.value(&["future", "keep"]).unwrap(), Some(json!(true)));
        assert!(field.state(&doc, plugin).is_err());
    }

    #[test]
    fn managed_vue_context_rejects_ambiguous_or_unsupported_configuration_atomically() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let plugin = PinnedPlugin::TypeScript;
        let field = catalog.schema(plugin).field("quoteStyle").unwrap();
        for value in [
            json!({"extends":"base.json"}),
            json!({"inherit":true}),
            json!({"overrides":[]}),
            json!({"markup":{"scriptFormatter":"prettier"}}),
            json!({"typescript":[]}),
        ] {
            let mut doc = document(value);
            let before = doc.draft_text().to_owned();
            assert!(
                catalog
                    .available_in_context(&doc, plugin, SchemaContext::ManagedVue)
                    .is_err()
            );
            assert!(
                field
                    .state_in_context(&doc, plugin, SchemaContext::ManagedVue)
                    .is_err()
            );
            assert!(
                field
                    .set_in_context(
                        &mut doc,
                        plugin,
                        json!("alwaysSingle"),
                        SchemaContext::ManagedVue
                    )
                    .is_err()
            );
            assert!(
                field
                    .reset_in_context(&mut doc, plugin, SchemaContext::ManagedVue)
                    .is_err()
            );
            assert!(
                field
                    .unset_in_context(&mut doc, plugin, SchemaContext::ManagedVue)
                    .is_err()
            );
            assert_eq!(doc.draft_text(), before);
        }
    }

    #[test]
    fn pinned_vue_nullable_controls_validate_real_schema_values() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let markup = catalog.schema(PinnedPlugin::Markup);
        let number = markup.field("maxAttrsPerLine").unwrap();
        assert!(matches!(number.kind, FieldKind::NullableNumber(_)));
        for valid in ["null", "1", "2"] {
            assert!(number.parse_input(valid).is_ok());
        }
        for invalid in ["0", "-1", "1.5", "true", "\"2\"", "NaN", "1e400", ""] {
            assert!(number.parse_input(invalid).is_err(), "{invalid}");
        }
        for name in ["vue.scriptIndent", "vue.styleIndent"] {
            let field = markup.field(name).unwrap();
            assert!(matches!(field.kind, FieldKind::Enum(_)));
            for valid in [Value::Null, json!(true), json!(false)] {
                field.validate(&valid).unwrap();
            }
            assert!(field.validate(&json!("false")).is_err());
        }
        for name in ["vBindStyle", "vOnStyle"] {
            let field = markup.field(name).unwrap();
            for valid in [Value::Null, json!("short"), json!("long")] {
                field.validate(&valid).unwrap();
            }
            assert!(field.validate(&json!("automatic")).is_err());
        }
        let order = catalog
            .schema(PinnedPlugin::Malva)
            .field("declarationOrder")
            .unwrap();
        for valid in [
            Value::Null,
            json!("alphabetical"),
            json!("smacss"),
            json!("concentric"),
        ] {
            order.validate(&valid).unwrap();
        }
        assert!(order.validate(&json!(["color", "display"])).is_err());
        assert!(matches!(
            markup.field("scriptFormatter").unwrap().kind,
            FieldKind::Raw { .. }
        ));
    }

    #[test]
    fn nullable_union_parser_stays_fail_closed_for_other_unions() {
        let schema = schema(json!({
            "reversed":{"type":["null","integer"],"minimum":1},
            "enum":{"type":["string","null"],"enum":["a",null]},
            "limited":{"type":["boolean","null"],"enum":[true,false]},
            "wrongChoice":{"type":["string","null"],"enum":["a",false,null]},
            "duplicates":{"type":["boolean","boolean"]},
            "complex":{"type":["object","null"]},
            "three":{"type":["boolean","string","null"]},
            "badType":{"type":[{},"null"]},
            "badDefault":{"type":["integer","null"],"minimum":1,"default":0}
        }));
        assert!(matches!(
            schema.field("reversed").unwrap().kind,
            FieldKind::NullableNumber(_)
        ));
        assert!(schema.field("enum").unwrap().validate(&Value::Null).is_ok());
        assert!(
            schema
                .field("limited")
                .unwrap()
                .validate(&Value::Null)
                .is_err()
        );
        for name in [
            "wrongChoice",
            "duplicates",
            "complex",
            "three",
            "badType",
            "badDefault",
        ] {
            assert!(!schema.field(name).unwrap().kind.is_supported(), "{name}");
        }
    }

    #[test]
    fn nullable_settings_keep_unset_null_defaults_and_jsonc_round_trip_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dprint.jsonc");
        fs::write(
            &path,
            r#"// keep root comment
{
  "markup": { /* keep section comment */ "futureOption": {"nested": [1,true]} },
  "malva": {"futureStyle": "unchanged"},
  "custom": {"keep": true}
}
"#,
        )
        .unwrap();
        let mut doc = ConfigDocument::load(&path).unwrap();
        let catalog = SchemaCatalog::bundled().unwrap();
        for (plugin, name, explicit) in [
            (PinnedPlugin::Markup, "maxAttrsPerLine", json!(2)),
            (PinnedPlugin::Markup, "vue.scriptIndent", json!(true)),
            (PinnedPlugin::Markup, "vBindStyle", json!("short")),
            (
                PinnedPlugin::Malva,
                "declarationOrder",
                json!("alphabetical"),
            ),
        ] {
            let field = catalog.schema(plugin).field(name).unwrap();
            assert_eq!(field.state(&doc, plugin).unwrap().explicit, None);
            assert_eq!(
                field.state(&doc, plugin).unwrap().schema_default,
                Some(Value::Null)
            );
            field.set(&mut doc, plugin, explicit.clone()).unwrap();
            let before = doc.draft_text().to_owned();
            assert!(
                field
                    .set(&mut doc, plugin, json!({"invalid":true}))
                    .is_err()
            );
            assert_eq!(doc.draft_text(), before);
            field.reset(&mut doc, plugin).unwrap();
            assert_eq!(
                field.state(&doc, plugin).unwrap().explicit,
                Some(Value::Null)
            );
            field.unset(&mut doc, plugin).unwrap();
            assert_eq!(field.state(&doc, plugin).unwrap().explicit, None);
            field.set(&mut doc, plugin, explicit).unwrap();
        }
        doc.save().unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.contains("// keep root comment"));
        assert!(saved.contains("/* keep section comment */"));
        let reloaded = ConfigDocument::load(&path).unwrap();
        assert_eq!(
            reloaded.value(&["markup", "futureOption"]).unwrap(),
            Some(json!({"nested":[1,true]}))
        );
        assert_eq!(
            reloaded.value(&["malva", "futureStyle"]).unwrap(),
            Some(json!("unchanged"))
        );
        assert_eq!(
            reloaded.value(&["custom", "keep"]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(
            reloaded.value(&["markup", "maxAttrsPerLine"]).unwrap(),
            Some(json!(2))
        );
    }

    #[test]
    fn primitive_validation_and_numeric_constraints_are_atomic() {
        let schema = schema(json!({
            "boolean":{"type":"boolean"},"string":{"type":"string"},
            "number":{"type":"number","minimum":0,"maximum":10,"multipleOf":0.1},
            "integer":{"type":"integer","exclusiveMinimum":0,"exclusiveMaximum":4},
            "freeNumber":{"type":"number"}
        }));
        let boolean = schema.field("boolean").unwrap();
        assert!(boolean.parse_input("true").is_ok());
        for invalid in ["\"true\"", "1", "null", "yes"] {
            assert!(boolean.parse_input(invalid).is_err());
        }
        assert_eq!(
            schema.field("string").unwrap().parse_input("").unwrap(),
            json!("")
        );
        assert!(
            schema
                .field("string")
                .unwrap()
                .validate(&json!(true))
                .is_err()
        );
        let number = schema.field("number").unwrap();
        for valid in ["0", "0.3", "10"] {
            assert!(number.parse_input(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "-1", "10.1", "0.15", "NaN", "Infinity", "1e400", "\"2\"", "true", "",
        ] {
            assert!(number.parse_input(invalid).is_err(), "{invalid}");
        }
        let integer = schema.field("integer").unwrap();
        for valid in ["1", "3", "2.0"] {
            assert!(integer.parse_input(valid).is_ok());
        }
        for invalid in ["0", "4", "1.5"] {
            assert!(integer.parse_input(invalid).is_err());
        }
        assert!(
            schema
                .field("freeNumber")
                .unwrap()
                .parse_input("-0.5")
                .is_ok()
        );
    }

    #[test]
    fn choices_keep_json_types_and_const_values() {
        let schema = schema(json!({
            "mixed":{"enum":[true, "true", 1, "1"]},
            "constant":{"type":"string","const":"fixed"},
            "singleBool":{"type":"boolean","enum":[false]},
            "choice":{"type":"string","oneOf":[{"const":"a","description":"Alpha"},{"const":"b","description":"Beta"}]}
        }));
        let mixed = schema.field("mixed").unwrap();
        for value in [json!(true), json!("true"), json!(1), json!("1"), json!(1.0)] {
            assert!(mixed.validate(&value).is_ok());
        }
        for value in [json!(false), json!("false"), json!(2), json!(null)] {
            assert!(mixed.validate(&value).is_err());
        }
        assert!(
            schema
                .field("singleBool")
                .unwrap()
                .validate(&json!(true))
                .is_err()
        );
        assert!(
            schema
                .field("constant")
                .unwrap()
                .validate(&json!("fixed"))
                .is_ok()
        );
        assert!(
            schema
                .field("constant")
                .unwrap()
                .validate(&json!("other"))
                .is_err()
        );
        assert!(schema.field("choice").unwrap().matches("Beta"));
    }

    #[test]
    fn field_group_search_is_case_insensitive_and_matches_descriptions() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let schema = catalog.schema(PinnedPlugin::TypeScript);
        assert_eq!(schema.groups().first(), Some(&"General"));
        assert!(schema.groups().contains(&"jsx"));
        assert_eq!(schema.field("jsx.quoteStyle").unwrap().group, "jsx");
        assert!(
            schema
                .matching_fields("JSX QUOTE")
                .iter()
                .any(|field| field.name == "jsx.quoteStyle")
        );
        assert_eq!(schema.matching_fields("").len(), schema.fields.len());
        assert!(schema.matching_fields("not-a-real-setting").is_empty());
    }

    #[test]
    fn plugin_identity_version_and_inheritance_gating_is_exact() {
        let pinned = PinnedPlugin::TypeScript.url();
        for configuration in [
            json!({}),
            json!({"plugins":["https://plugins.dprint.dev/typescript-0.97.0.wasm"]}),
            json!({"plugins":["https://example.test/typescript-0.96.1.wasm"]}),
            json!({"plugins":["./typescript-0.96.1.wasm"]}),
            json!({"plugins":[format!("{pinned}#checksum")]}),
            json!({"plugins":[pinned,"./custom.wasm"]}),
            json!({"plugins":[pinned,1]}),
            json!({"plugins":[pinned],"extends":"./base.json"}),
            json!({"plugins":[pinned],"extends":[]}),
            json!({"plugins":[pinned],"inherit":true}),
            json!({"plugins":[pinned],"inherit":null}),
            json!({"plugins":[PinnedPlugin::Json.url()]}),
        ] {
            assert!(
                PinnedPlugin::TypeScript
                    .ensure_available(&document(configuration))
                    .is_err()
            );
        }
        for configuration in [
            json!({"plugins":[pinned]}),
            json!({"plugins":[pinned],"inherit":false}),
            json!({"plugins":[pinned,PinnedPlugin::Json.url()]}),
        ] {
            assert!(
                PinnedPlugin::TypeScript
                    .ensure_available(&document(configuration))
                    .is_ok()
            );
        }
        let mut duplicate = document(json!({}));
        duplicate
            .replace_draft_text(&format!(
                r#"{{"plugins":["{pinned}"],"plugins":["{pinned}"]}}"#
            ))
            .unwrap();
        assert!(
            PinnedPlugin::TypeScript
                .ensure_available(&duplicate)
                .is_err()
        );
    }

    #[test]
    fn absent_explicit_reset_and_unset_are_distinct_and_preserve_dotted_keys() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let plugin = PinnedPlugin::Json;
        let mut document =
            document(json!({"plugins":[plugin.url()],"json":{"unknown":{"kept":true}}}));
        let field = catalog
            .schema(plugin)
            .field("array.preferSingleLine")
            .unwrap();
        let state = field.state(&document, plugin).unwrap();
        assert_eq!(state.explicit, None);
        assert_eq!(state.schema_default, Some(json!(false)));
        assert!(!document.is_dirty());
        field.set(&mut document, plugin, json!(true)).unwrap();
        assert_eq!(
            document.value(&["json", "array.preferSingleLine"]).unwrap(),
            Some(json!(true))
        );
        assert_eq!(document.value(&["json", "array"]).unwrap(), None);
        field.reset(&mut document, plugin).unwrap();
        assert_eq!(
            field.state(&document, plugin).unwrap().explicit,
            Some(json!(false))
        );
        field.unset(&mut document, plugin).unwrap();
        assert_eq!(field.state(&document, plugin).unwrap().explicit, None);
        assert_eq!(
            document.value(&["json", "unknown"]).unwrap(),
            Some(json!({"kept":true}))
        );
        let before = document.draft_text().to_owned();
        assert!(field.set(&mut document, plugin, json!("true")).is_err());
        assert_eq!(before, document.draft_text());
        assert!(
            catalog
                .schema(plugin)
                .field("locked")
                .unwrap()
                .reset(&mut document, plugin)
                .is_err()
        );
        assert_eq!(before, document.draft_text());
    }

    #[test]
    fn every_supported_pinned_field_round_trips_without_touching_unknown_data() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let mut document = document(json!({
            "plugins":PinnedPlugin::ALL.map(PinnedPlugin::url),
            "typescript":{"futureOption":{"preserve":[1,true]}},
            "json":{"jsonTrailingCommaFiles":["*.jsonc"]},
            "markdown":{"tags":{"custom":"ts"},"futureOption":{"preserve":["text"]}},
            "toml":{"futureOption":{"preserve":[1,true]}}
        }));
        let original = format!("// preserve this comment\n{}", document.draft_text());
        document.replace_draft_text(&original).unwrap();
        let mut supported_count = 0;
        for plugin in PinnedPlugin::ALL {
            for field in catalog
                .fields(plugin)
                .iter()
                .filter(|field| field.kind.is_supported())
            {
                assert_eq!(
                    field.state(&document, plugin).unwrap().explicit,
                    None,
                    "{}",
                    field.name
                );
                let value = field.default.clone().unwrap_or_else(|| match &field.kind {
                    FieldKind::Boolean => json!(false),
                    FieldKind::Enum(choices) => choices[0].value.clone(),
                    FieldKind::Const(value) => value.clone(),
                    FieldKind::NullableNumber(_) => Value::Null,
                    FieldKind::Number(constraints) => {
                        json!(constraints.minimum.unwrap_or(0.0))
                    }
                    FieldKind::String => json!("test"),
                    FieldKind::Raw { .. } => unreachable!(),
                });
                field.set(&mut document, plugin, value.clone()).unwrap();
                assert_eq!(
                    field.state(&document, plugin).unwrap().explicit,
                    Some(value)
                );
                field.unset(&mut document, plugin).unwrap();
                assert_eq!(field.state(&document, plugin).unwrap().explicit, None);
                supported_count += 1;
            }
        }
        assert_eq!(supported_count, 263);
        assert!(document.draft_text().contains("// preserve this comment"));
        assert_eq!(
            document.value(&["typescript", "futureOption"]).unwrap(),
            Some(json!({"preserve":[1,true]}))
        );
        assert_eq!(
            document.value(&["json", "jsonTrailingCommaFiles"]).unwrap(),
            Some(json!(["*.jsonc"]))
        );
        assert_eq!(
            document.value(&["markdown", "tags"]).unwrap(),
            Some(json!({"custom":"ts"}))
        );
        assert_eq!(
            document.value(&["markdown", "futureOption"]).unwrap(),
            Some(json!({"preserve":["text"]}))
        );
        assert_eq!(
            document.value(&["toml", "futureOption"]).unwrap(),
            Some(json!({"preserve":[1,true]}))
        );
    }

    #[test]
    fn validates_known_fields_but_preserves_unknown_complex_and_unidentified_config() {
        let catalog = SchemaCatalog::bundled().unwrap();
        let plugin = PinnedPlugin::Json;
        let mut document = document(
            json!({"plugins":[plugin.url()],"json":{"useTabs":"true","unknown":42,"jsonTrailingCommaFiles":{"future":true}}}),
        );
        assert!(
            catalog
                .validate_document(&document)
                .unwrap_err()
                .contains("json.useTabs")
        );
        document.set(&["json", "useTabs"], json!(true)).unwrap();
        assert!(catalog.validate_document(&document).is_ok());
        document.set(&["json"], json!(false)).unwrap();
        assert!(catalog.validate_document(&document).is_err());
        document.set(&["extends"], json!("./base.json")).unwrap();
        assert!(catalog.validate_document(&document).is_ok());
        let before = document.draft_text().to_owned();
        assert!(
            catalog
                .schema(plugin)
                .field("useTabs")
                .unwrap()
                .set(&mut document, plugin, json!(false))
                .is_err()
        );
        assert_eq!(before, document.draft_text());
    }
}
