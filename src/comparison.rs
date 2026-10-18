//! Original source selection and a bounded-memory, line-oriented A/B diff.
//!
//! Neither model accepts formatted output as a replacement for original input.
use crate::schema::PinnedPlugin;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourceLanguage {
    #[default]
    TypeScript,
    JavaScript,
    Tsx,
    Jsx,
    Json,
    Jsonc,
    Markdown,
    Toml,
}

impl SourceLanguage {
    pub const ALL: [Self; 8] = [
        Self::TypeScript,
        Self::JavaScript,
        Self::Tsx,
        Self::Jsx,
        Self::Json,
        Self::Jsonc,
        Self::Markdown,
        Self::Toml,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::TypeScript => "TS",
            Self::JavaScript => "JS",
            Self::Tsx => "TSX",
            Self::Jsx => "JSX",
            Self::Json => "JSON",
            Self::Jsonc => "JSONC",
            Self::Markdown => "Markdown",
            Self::Toml => "TOML",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::TypeScript => "ts",
            Self::JavaScript => "js",
            Self::Tsx => "tsx",
            Self::Jsx => "jsx",
            Self::Json => "json",
            Self::Jsonc => "jsonc",
            Self::Markdown => "md",
            Self::Toml => "toml",
        }
    }

    pub fn plugin(self) -> PinnedPlugin {
        match self {
            Self::TypeScript | Self::JavaScript | Self::Tsx | Self::Jsx => PinnedPlugin::TypeScript,
            Self::Json | Self::Jsonc => PinnedPlugin::Json,
            Self::Markdown => PinnedPlugin::Markdown,
            Self::Toml => PinnedPlugin::Toml,
        }
    }

    pub fn is_json(self) -> bool {
        matches!(self, Self::Json | Self::Jsonc)
    }

    fn source_path(self, custom: bool) -> &'static str {
        match (self, custom) {
            (Self::TypeScript, false) => "sample.ts",
            (Self::JavaScript, false) => "sample.js",
            (Self::Tsx, false) => "sample.tsx",
            (Self::Jsx, false) => "sample.jsx",
            (Self::Json, false) => "sample.json",
            (Self::Jsonc, false) => "sample.jsonc",
            (Self::Markdown, false) => "sample.md",
            (Self::Toml, false) => "sample.toml",
            (Self::TypeScript, true) => "custom.ts",
            (Self::JavaScript, true) => "custom.js",
            (Self::Tsx, true) => "custom.tsx",
            (Self::Jsx, true) => "custom.jsx",
            (Self::Json, true) => "custom.json",
            (Self::Jsonc, true) => "custom.jsonc",
            (Self::Markdown, true) => "custom.md",
            (Self::Toml, true) => "custom.toml",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuiltinSample {
    #[default]
    Overview,
    Collections,
    Comments,
}

impl BuiltinSample {
    pub const ALL: [Self; 3] = [Self::Overview, Self::Collections, Self::Comments];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Collections => "Arrays and objects",
            Self::Comments => "Comments",
        }
    }

    pub fn for_category(category: &str) -> Self {
        let category = category.to_ascii_lowercase();
        if category.contains("comment") {
            Self::Comments
        } else if [
            "array",
            "object",
            "collection",
            "tuple",
            "type",
            "literal",
            "list",
            "table",
        ]
        .iter()
        .any(|word| category.contains(word))
        {
            Self::Collections
        } else {
            Self::Overview
        }
    }

    pub fn source(self, language: SourceLanguage) -> &'static str {
        match (language, self) {
            (SourceLanguage::Markdown, Self::Overview) => MARKDOWN_OVERVIEW,
            (SourceLanguage::Markdown, Self::Collections) => MARKDOWN_COLLECTIONS,
            (SourceLanguage::Markdown, Self::Comments) => MARKDOWN_COMMENTS,
            (SourceLanguage::Toml, Self::Overview) => TOML_OVERVIEW,
            (SourceLanguage::Toml, Self::Collections) => TOML_COLLECTIONS,
            (SourceLanguage::Toml, Self::Comments) => TOML_COMMENTS,
            (SourceLanguage::Json, Self::Overview) => JSON_OVERVIEW,
            (SourceLanguage::Json, Self::Collections) => JSON_COLLECTIONS,
            (SourceLanguage::Json, Self::Comments) => JSON_DESCRIPTIONS,
            (SourceLanguage::Jsonc, Self::Overview) => JSONC_OVERVIEW,
            (SourceLanguage::Jsonc, Self::Collections) => JSONC_COLLECTIONS,
            (SourceLanguage::Jsonc, Self::Comments) => JSONC_COMMENTS,
            (SourceLanguage::Tsx | SourceLanguage::Jsx, Self::Overview) => JSX_OVERVIEW,
            (SourceLanguage::Tsx | SourceLanguage::Jsx, Self::Collections) => JSX_COLLECTIONS,
            (SourceLanguage::Tsx | SourceLanguage::Jsx, Self::Comments) => JSX_COMMENTS,
            (SourceLanguage::TypeScript, Self::Overview) => TS_OVERVIEW,
            (SourceLanguage::TypeScript, Self::Collections) => TS_COLLECTIONS,
            (SourceLanguage::TypeScript | SourceLanguage::JavaScript, Self::Comments) => {
                JS_COMMENTS
            }
            (SourceLanguage::JavaScript, Self::Overview) => JS_OVERVIEW,
            (SourceLanguage::JavaScript, Self::Collections) => JS_COLLECTIONS,
        }
    }
}

/// Custom input is retained independently of language and builtin selection.
/// Changing a language only changes the formatter's virtual source extension;
/// it never attempts to translate, reset, or reformat the custom buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceState {
    language: SourceLanguage,
    sample: BuiltinSample,
    custom: Option<String>,
    custom_active: bool,
}

impl SourceState {
    pub fn language(&self) -> SourceLanguage {
        self.language
    }

    pub fn sample(&self) -> BuiltinSample {
        self.sample
    }

    pub fn select_language(&mut self, language: SourceLanguage) {
        self.language = language;
    }

    /// Explicit sample selection shows that sample but keeps the custom buffer.
    pub fn select_sample(&mut self, sample: BuiltinSample) {
        self.sample = sample;
        self.custom_active = false;
    }

    /// Settings navigation may choose a more relevant builtin, but never
    /// replaces the visible custom input or discards its retained contents.
    pub fn select_category(&mut self, category: &str) {
        self.sample = BuiltinSample::for_category(category);
    }

    pub fn source(&self) -> &str {
        if self.custom_active {
            // Only use_custom/set_custom_source can activate the custom mode.
            self.custom.as_deref().expect("active custom source exists")
        } else {
            self.sample.source(self.language)
        }
    }

    pub fn source_path(&self) -> &'static str {
        self.language.source_path(self.custom_active)
    }

    pub fn custom_source(&self) -> Option<&str> {
        self.custom.as_deref()
    }

    pub fn set_custom_source(&mut self, source: String) {
        self.custom = Some(source);
        self.custom_active = true;
    }

    pub fn use_builtin(&mut self) {
        self.custom_active = false;
    }

    /// Seed from the selected builtin only on the first switch. An intentionally
    /// empty custom source is a real buffer and must not be reseeded.
    pub fn use_custom(&mut self) {
        if self.custom.is_none() {
            self.custom = Some(self.sample.source(self.language).to_owned());
        }
        self.custom_active = true;
    }

    pub fn is_custom(&self) -> bool {
        self.custom_active
    }
}

const JS_OVERVIEW: &str = "import {readFile,writeFile} from 'node:fs/promises';\nconst preferences={theme:'dark',tabs:['source','preview']};\nexport async function greet(name){if(name){return {message:`Hello, ${name}!`,settings:preferences}}return null}\nconsole.log(greet('CodeStyles'));\n";
const TS_OVERVIEW: &str = "type User={name:string;active:boolean};\nconst users:User[]=[{name:'Ada',active:true},{name:'Lin',active:false}];\nexport function activeNames(items:User[]){return items.filter(user=>user.active).map(user=>{return user.name})}\nconsole.log(activeNames(users));\n";
const JSX_OVERVIEW: &str = "const items=['Source','Preview'];\nexport function App(){return <main className='preview'><h1>CodeStyles</h1><ul>{items.map(item=><li key={item}>{item}</li>)}</ul></main>}\n";
const JS_COLLECTIONS: &str = "const items=[{name:'alpha',enabled:true},{name:'beta',enabled:false},];\nconst options={spacing:{width:4,tabs:false},labels:['short','a longer label','preview'],};\nconst result=items.map(({name,enabled})=>({name,enabled,options}));\n";
const TS_COLLECTIONS: &str = "type Item={name:string;enabled:boolean;tags:readonly string[]};\ntype Pair=[string,number];\nconst items:Item[]=[{name:'alpha',enabled:true,tags:['one','two']},{name:'beta',enabled:false,tags:[]},];\nconst options={spacing:{width:4,tabs:false},labels:['short','a longer label','preview'],} satisfies Record<string,unknown>;\n";
const JSX_COLLECTIONS: &str = "const items=[{id:1,name:'alpha',enabled:true},{id:2,name:'beta',enabled:false},];\nexport const List=()=> <section aria-label='Examples' className='sample-list'>{items.map(({id,name,enabled})=><div key={id} data-enabled={enabled} style={{display:'flex',gap:8}}>{name}</div>)}</section>;\n";
const JS_COMMENTS: &str = "/**\n * Build a greeting.\n * @param name The display name.\n */\nexport function greet(name){\n//keep this explanation with the next statement\nconst message='Hello, '+name; //inline explanation\n/* A block comment inside the function. */\nreturn message;\n}\n";
const JSX_COMMENTS: &str = "//Render a compact preview\nexport function Preview(){return <main>{/* A visible section follows. */}<h1 title='Preview'>CodeStyles</h1><p>Compare saved and draft settings.</p></main>}\n";
const JSON_OVERVIEW: &str = "{\"name\":\"CodeStyles\",\"preview\":{\"enabled\":true,\"languages\":[\"TypeScript\",\"JSON\"]}}\n";
const JSON_COLLECTIONS: &str = "{\"items\":[{\"name\":\"alpha\",\"enabled\":true},{\"name\":\"beta\",\"enabled\":false}],\"options\":{\"spacing\":{\"width\":4,\"tabs\":false},\"labels\":[\"short\",\"a longer label\",\"preview\"]}}\n";
// JSON has no comments. Keep this example valid rather than silently switching
// the language's parser to JSONC when the comments category is selected.
const JSON_DESCRIPTIONS: &str = "{\"description\":\"JSON uses fields for descriptions; JSONC also supports comments.\",\"preview\":{\"description\":\"Saved and draft settings\",\"enabled\":true}}\n";
const JSONC_OVERVIEW: &str = "{\n// Formatting preferences\n\"name\":\"CodeStyles\",\"preview\":{\"enabled\":true,\"languages\":[\"TypeScript\",\"JSON\"],},\n}\n";
const JSONC_COLLECTIONS: &str = "{\n// Nested arrays and objects\n\"items\":[{\"name\":\"alpha\",\"enabled\":true},{\"name\":\"beta\",\"enabled\":false},],\n\"options\":{\"spacing\":{\"width\":4,\"tabs\":false},\"labels\":[\"short\",\"a longer label\",\"preview\"],},\n}\n";
const JSONC_COMMENTS: &str = "{\n//keep this line comment\n\"name\":\"CodeStyles\", //inline description\n/* A block comment describing the preview. */\n\"preview\":{\"enabled\":true,\"labels\":[\"saved\",\"draft\"],},\n}\n";

const MARKDOWN_OVERVIEW: &str = "# CodeStyles\n\nPreview **formatting** with the same original text, including a deliberately long paragraph that makes line wrapping changes easy to compare across saved and draft settings.\n\n*   Keep the source unchanged\n*   Compare the preview\n\n```ts\nconst settings={name:'CodeStyles',enabled:true};\n```\n";
const MARKDOWN_COLLECTIONS: &str = "# Collections\n\n*   Source\n    *   Built-in examples\n    *   Custom input\n*   Preview\n\n|Name|Enabled|\n|:--|--:|\n|Saved|true|\n|Draft|false|\n\n```json\n{\"items\":[1,2,3],\"enabled\":true}\n```\n";
const MARKDOWN_COMMENTS: &str = "# Comments\n\n<!-- Keep this explanation with the following paragraph. -->\n\nA paragraph with *emphasis*, a [link](https://dprint.dev/), and `inline code`.\n\n>   Compare the saved settings and draft settings while preserving the original source.\n\n```toml\n# A comment in a fenced TOML example\nname=\"CodeStyles\"\nenabled=true\n```\n";
const TOML_OVERVIEW: &str = "title=\"CodeStyles\"\n\n[preview]\nenabled=true\nlanguages=[\"TypeScript\",\"JSON\",\"Markdown\",\"TOML\"]\nlayout={source=true,preview=true}\n";
const TOML_COLLECTIONS: &str = "[options]\nlabels=[\"source\",\"saved\",\"draft\"]\nspacing={width=4,tabs=false}\n\n[[items]]\nname=\"alpha\"\nenabled=true\n\n[[items]]\nname=\"beta\"\nenabled=false\n";
const TOML_COMMENTS: &str = "# Formatting preferences\ntitle=\"CodeStyles\" # Application name\n\n[preview]\n# Keep saved and draft output visible\nenabled=true\nlabels=[\"saved\",\"draft\"] # Comparison panes\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffKind {
    Unchanged,
    Removed,
    Added,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffKind,
    /// Exact text including any line terminator, so final-newline and CRLF
    /// changes are represented. Strip only the terminator when rendering rows.
    pub text: String,
    /// One-based line numbers; a missing side means insertion or deletion.
    pub before_line: Option<usize>,
    pub after_line: Option<usize>,
}

// No input can allocate an unbounded N × M LCS table. Unique-line anchors split
// most source edits into small regions; large repetitive gaps use replacements.
const MAX_LCS_CELLS: usize = 65_536;

/// A display diff with linear input/output storage and a fixed-size LCS budget.
/// Unique-line patience anchors keep distant edits separate. Small gaps use an
/// exact LCS; oversized unanchored gaps are shown as removed then added lines.
/// The latter is intentionally not guaranteed to be the minimal edit script.
pub fn line_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let left: Vec<_> = before.split_inclusive('\n').collect();
    let right: Vec<_> = after.split_inclusive('\n').collect();
    let mut output = Vec::with_capacity(left.len().max(right.len()));
    let mut positions = (1, 1);
    let prefix = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
    for line in &left[..prefix] {
        emit(&mut output, &mut positions, DiffKind::Unchanged, line);
    }
    let suffix = left[prefix..]
        .iter()
        .rev()
        .zip(right[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let middle_left = &left[prefix..left.len() - suffix];
    let middle_right = &right[prefix..right.len() - suffix];
    let mut previous = (0, 0);
    for (a, b) in unique_anchors(middle_left, middle_right) {
        diff_gap(
            &middle_left[previous.0..a],
            &middle_right[previous.1..b],
            &mut output,
            &mut positions,
        );
        emit(
            &mut output,
            &mut positions,
            DiffKind::Unchanged,
            middle_left[a],
        );
        previous = (a + 1, b + 1);
    }
    diff_gap(
        &middle_left[previous.0..],
        &middle_right[previous.1..],
        &mut output,
        &mut positions,
    );
    for line in &left[left.len() - suffix..] {
        emit(&mut output, &mut positions, DiffKind::Unchanged, line);
    }
    output
}

fn unique_anchors(left: &[&str], right: &[&str]) -> Vec<(usize, usize)> {
    fn counts<'a>(lines: &[&'a str]) -> HashMap<&'a str, (usize, usize)> {
        let mut counts = HashMap::new();
        for (index, &line) in lines.iter().enumerate() {
            let value = counts.entry(line).or_insert((0, index));
            value.0 += 1;
        }
        counts
    }
    let left_counts = counts(left);
    let right_counts = counts(right);
    let candidates: Vec<_> = left
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            if left_counts.get(line)?.0 != 1 {
                return None;
            }
            let &(count, other) = right_counts.get(line)?;
            (count == 1).then_some((index, other))
        })
        .collect();
    // Longest increasing subsequence of right-hand positions, O(N log N).
    let mut tails: Vec<usize> = Vec::new();
    let mut predecessors = vec![None; candidates.len()];
    for (index, &(_, right_index)) in candidates.iter().enumerate() {
        let slot = tails.partition_point(|&tail| candidates[tail].1 < right_index);
        if slot > 0 {
            predecessors[index] = Some(tails[slot - 1]);
        }
        if slot == tails.len() {
            tails.push(index);
        } else {
            tails[slot] = index;
        }
    }
    let mut anchors = Vec::with_capacity(tails.len());
    let mut next = tails.last().copied();
    while let Some(index) = next {
        anchors.push(candidates[index]);
        next = predecessors[index];
    }
    anchors.reverse();
    anchors
}

fn diff_gap(
    left: &[&str],
    right: &[&str],
    output: &mut Vec<DiffLine>,
    positions: &mut (usize, usize),
) {
    let cells = (left.len() + 1).checked_mul(right.len() + 1);
    if left.is_empty() || right.is_empty() || cells.is_none_or(|n| n > MAX_LCS_CELLS) {
        for line in left {
            emit(output, positions, DiffKind::Removed, line);
        }
        for line in right {
            emit(output, positions, DiffKind::Added, line);
        }
        return;
    }
    let columns = right.len() + 1;
    let mut lengths = vec![0_u32; cells.expect("bounded dimensions")];
    for a in (0..left.len()).rev() {
        for b in (0..right.len()).rev() {
            lengths[a * columns + b] = if left[a] == right[b] {
                1 + lengths[(a + 1) * columns + b + 1]
            } else {
                lengths[(a + 1) * columns + b].max(lengths[a * columns + b + 1])
            };
        }
    }
    let (mut a, mut b) = (0, 0);
    while a < left.len() || b < right.len() {
        if a < left.len() && b < right.len() && left[a] == right[b] {
            emit(output, positions, DiffKind::Unchanged, left[a]);
            a += 1;
            b += 1;
        } else if a < left.len()
            && (b == right.len() || lengths[(a + 1) * columns + b] >= lengths[a * columns + b + 1])
        {
            emit(output, positions, DiffKind::Removed, left[a]);
            a += 1;
        } else {
            emit(output, positions, DiffKind::Added, right[b]);
            b += 1;
        }
    }
}

fn emit(output: &mut Vec<DiffLine>, positions: &mut (usize, usize), kind: DiffKind, text: &str) {
    let before_line = (kind != DiffKind::Added).then_some(positions.0);
    let after_line = (kind != DiffKind::Removed).then_some(positions.1);
    positions.0 += usize::from(before_line.is_some());
    positions.1 += usize::from(after_line.is_some());
    output.push(DiffLine {
        kind,
        text: text.to_owned(),
        before_line,
        after_line,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_input_survives_language_category_and_builtin_navigation() {
        let original = "// pasted source\nconst value = { untouched: true };\n";
        let mut source = SourceState::default();
        source.set_custom_source(original.into());
        for language in SourceLanguage::ALL {
            source.select_language(language);
            source.select_category("Object and array literals");
            assert!(source.is_custom());
            assert_eq!(source.source(), original);
            assert!(source.source_path().ends_with(language.extension()));
            source.use_builtin();
            assert_eq!(source.sample(), BuiltinSample::Collections);
            assert_eq!(source.custom_source(), Some(original));
            source.select_sample(BuiltinSample::Comments);
            source.use_custom();
            assert_eq!(source.source(), original);
        }
    }

    #[test]
    fn each_language_selects_its_own_pinned_plugin_and_virtual_paths() {
        for language in SourceLanguage::ALL {
            let expected = match language {
                SourceLanguage::TypeScript
                | SourceLanguage::JavaScript
                | SourceLanguage::Tsx
                | SourceLanguage::Jsx => PinnedPlugin::TypeScript,
                SourceLanguage::Json | SourceLanguage::Jsonc => PinnedPlugin::Json,
                SourceLanguage::Markdown => PinnedPlugin::Markdown,
                SourceLanguage::Toml => PinnedPlugin::Toml,
            };
            assert_eq!(language.plugin(), expected);
            let mut source = SourceState::default();
            source.select_language(language);
            assert_eq!(
                source.source_path(),
                format!("sample.{}", language.extension())
            );
            source.use_custom();
            assert_eq!(
                source.source_path(),
                format!("custom.{}", language.extension())
            );
        }
    }

    #[test]
    fn custom_mode_seeds_once_and_preserves_intentionally_empty_buffer() {
        let mut source = SourceState::default();
        let builtin = source.source().to_owned();
        source.use_custom();
        assert_eq!(source.source(), builtin);
        source.set_custom_source(String::new());
        source.use_builtin();
        source.select_language(SourceLanguage::Jsonc);
        source.select_category("Comments");
        source.use_custom();
        assert_eq!(source.source(), "");
        assert_eq!(source.source_path(), "custom.jsonc");
    }

    #[test]
    fn builtin_selection_is_deterministic_and_json_samples_stay_valid() {
        let mut source = SourceState::default();
        let first = source.source().to_owned();
        source.select_category("Arrays");
        assert_ne!(source.source(), first);
        source.select_category("General");
        assert_eq!(source.source(), first);
        for category in ["list", "table", "inlineTable", "array"] {
            assert_eq!(
                BuiltinSample::for_category(category),
                BuiltinSample::Collections
            );
        }
        for sample in BuiltinSample::ALL {
            for language in SourceLanguage::ALL {
                assert!(!sample.source(language).is_empty());
            }
            serde_json::from_str::<serde_json::Value>(sample.source(SourceLanguage::Json)).unwrap();
        }
    }

    fn assert_reconstructs(before: &str, after: &str) -> Vec<DiffLine> {
        let diff = line_diff(before, after);
        let restored_before: String = diff
            .iter()
            .filter(|line| line.kind != DiffKind::Added)
            .map(|line| line.text.as_str())
            .collect();
        let restored_after: String = diff
            .iter()
            .filter(|line| line.kind != DiffKind::Removed)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(restored_before, before);
        assert_eq!(restored_after, after);
        let mut positions = (1, 1);
        for line in &diff {
            if let Some(number) = line.before_line {
                assert_eq!(number, positions.0);
                positions.0 += 1;
            }
            if let Some(number) = line.after_line {
                assert_eq!(number, positions.1);
                positions.1 += 1;
            }
        }
        diff
    }

    #[test]
    fn diff_marks_replacements_insertions_and_deletions_with_line_numbers() {
        let diff = assert_reconstructs("head\nold\nend\n", "head\nnew\nextra\nend\n");
        assert_eq!(
            diff.iter().map(|line| line.kind).collect::<Vec<_>>(),
            vec![
                DiffKind::Unchanged,
                DiffKind::Removed,
                DiffKind::Added,
                DiffKind::Added,
                DiffKind::Unchanged,
            ]
        );
        assert_eq!(diff[1].before_line, Some(2));
        assert_eq!(diff[1].after_line, None);
        assert_eq!(diff[4].before_line, Some(3));
        assert_eq!(diff[4].after_line, Some(4));
    }

    #[test]
    fn diff_preserves_unicode_empty_inputs_and_line_ending_changes() {
        for (before, after) in [
            ("", ""),
            ("", "insert\n"),
            ("delete", ""),
            ("α\n☃\n", "α\n☃"),
            ("a\r\nb\r\n", "a\nb\n"),
            ("\n\n\n", "\n\n"),
        ] {
            assert_reconstructs(before, after);
        }
        let diff = line_diff("same\n", "same");
        assert_eq!(diff[0].kind, DiffKind::Removed);
        assert_eq!(diff[1].kind, DiffKind::Added);
    }

    #[test]
    fn diff_keeps_separated_changes_and_repeated_braces_aligned() {
        let before = "function a() {\nold_a();\n}\nfunction b() {\nsame();\n}\nfunction c() {\nold_c();\n}\n";
        let after = "function a() {\nnew_a();\n}\nfunction b() {\nsame();\n}\nfunction c() {\nnew_c();\n}\n";
        let diff = assert_reconstructs(before, after);
        assert_eq!(
            diff.iter()
                .filter(|line| line.kind == DiffKind::Removed)
                .count(),
            2
        );
        assert_eq!(
            diff.iter()
                .filter(|line| line.kind == DiffKind::Added)
                .count(),
            2
        );
    }

    #[test]
    fn large_repetitive_diff_uses_bounded_fallback_and_reconstructs_both_sides() {
        let before = "old\n".repeat(20_000);
        let after = "new\n".repeat(20_000);
        let diff = assert_reconstructs(&before, &after);
        assert_eq!(diff.len(), 40_000);
        assert!(
            diff[..20_000]
                .iter()
                .all(|line| line.kind == DiffKind::Removed)
        );
        assert!(
            diff[20_000..]
                .iter()
                .all(|line| line.kind == DiffKind::Added)
        );
    }

    #[test]
    fn exhaustive_small_diffs_reconstruct_even_reordered_repeated_lines() {
        let alphabet = ["a\n", "b\n", "a"];
        let mut values = vec![String::new()];
        for a in alphabet {
            values.push(a.into());
            for b in alphabet {
                values.push(format!("{a}{b}"));
                for c in alphabet {
                    values.push(format!("{a}{b}{c}"));
                }
            }
        }
        for before in &values {
            for after in &values {
                assert_reconstructs(before, after);
            }
        }
    }
}
