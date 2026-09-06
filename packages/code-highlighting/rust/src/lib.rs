use std::cell::RefCell;
use std::collections::VecDeque;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

uniffi::setup_scaffolding!();

const MAX_BLOCK_BYTES: usize = 65_536;
const MAX_LINE_BYTES: usize = 4_096;
const MAX_LINES: usize = 1_000;
const MAX_CACHE_BYTES: usize = 1_048_576;
const MAX_CACHE_ENTRIES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HighlightRange {
    pub start: u32,
    pub length: u32,
    pub color: u32,
    pub font_style: u8,
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HighlightError {
    #[error("Unknown code-highlighting theme: {theme}")]
    UnknownTheme { theme: String },
}

struct CachedBlock {
    text: String,
    language: String,
    theme: String,
    ranges: Vec<HighlightRange>,
    bytes: usize,
}

struct Engine {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
    cache: VecDeque<CachedBlock>,
    cache_bytes: usize,
}

thread_local! {
    static ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

fn language_extension(language: &str) -> Option<&'static str> {
    Some(match language {
        "javascript" | "js" => "js",
        "typescript" | "ts" => "ts",
        "tsx" | "jsx" => "tsx",
        "swift" => "swift",
        "kotlin" | "kt" => "kt",
        "rust" | "rs" => "rs",
        "python" | "py" => "py",
        "json" => "json",
        "html" => "html",
        "css" => "css",
        "bash" | "shell" | "sh" => "sh",
        _ => return None,
    })
}

#[uniffi::export]
pub fn highlight_code(
    text: String,
    language: Option<String>,
    theme: String,
) -> Result<Vec<HighlightRange>, HighlightError> {
    let Some(extension) = language
        .as_deref()
        .and_then(|value| language_extension(value.trim()))
    else {
        return Ok(Vec::new());
    };
    if text.len() > MAX_BLOCK_BYTES
        || text.lines().take(MAX_LINES + 1).count() > MAX_LINES
        || text.lines().any(|line| line.len() > MAX_LINE_BYTES)
    {
        return Ok(Vec::new());
    }
    ENGINE.with(|cell| {
        let mut engine = cell.borrow_mut();
        let engine = engine.get_or_insert_with(|| Engine {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: ThemeSet::load_defaults(),
            cache: VecDeque::new(),
            cache_bytes: 0,
        });
        engine.highlight(text, extension, theme)
    })
}

impl Engine {
    fn highlight(
        &mut self,
        text: String,
        language: &str,
        theme: String,
    ) -> Result<Vec<HighlightRange>, HighlightError> {
        if let Some(index) = self.cache.iter().position(|entry| {
            entry.text == text && entry.language == language && entry.theme == theme
        }) {
            let entry = self.cache.remove(index).unwrap();
            let ranges = entry.ranges.clone();
            self.cache.push_back(entry);
            return Ok(ranges);
        }
        let selected_theme =
            self.themes
                .themes
                .get(&theme)
                .ok_or_else(|| HighlightError::UnknownTheme {
                    theme: theme.clone(),
                })?;
        let Some(syntax) = self.syntaxes.find_syntax_by_extension(language) else {
            return Ok(Vec::new());
        };
        let mut highlighter = HighlightLines::new(syntax, selected_theme);
        let mut ranges: Vec<HighlightRange> = Vec::new();
        let mut start = 0;
        for line in LinesWithEndings::from(text.as_str()) {
            let Ok(tokens) = highlighter.highlight_line(line, &self.syntaxes) else {
                return Ok(Vec::new());
            };
            for (style, token) in tokens {
                let length = token.encode_utf16().count() as u32;
                if length == 0 {
                    continue;
                }
                let foreground = style.foreground;
                let color =
                    u32::from_be_bytes([foreground.r, foreground.g, foreground.b, foreground.a]);
                let font_style = style.font_style.bits();
                if let Some(last) = ranges
                    .last_mut()
                    .filter(|last| last.color == color && last.font_style == font_style)
                {
                    last.length += length;
                } else {
                    ranges.push(HighlightRange {
                        start,
                        length,
                        color,
                        font_style,
                    });
                }
                start += length;
            }
        }
        let bytes = text.len()
            + language.len()
            + theme.len()
            + ranges.len() * std::mem::size_of::<HighlightRange>();
        while self.cache.len() >= MAX_CACHE_ENTRIES || self.cache_bytes + bytes > MAX_CACHE_BYTES {
            let Some(removed) = self.cache.pop_front() else {
                break;
            };
            self.cache_bytes -= removed.bytes;
        }
        if bytes <= MAX_CACHE_BYTES {
            self.cache_bytes += bytes;
            self.cache.push_back(CachedBlock {
                text,
                language: language.into(),
                theme,
                ranges: ranges.clone(),
                bytes,
            });
        }
        Ok(ranges)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_stays_bounded_under_unique_blocks() {
        for number in 0..100 {
            highlight_code(
                format!("const value = {number};"),
                Some("js".into()),
                "base16-ocean.dark".into(),
            )
            .unwrap();
        }
        ENGINE.with(|cell| {
            let engine = cell.borrow();
            let engine = engine.as_ref().unwrap();
            assert!(engine.cache.len() <= MAX_CACHE_ENTRIES);
            assert!(engine.cache_bytes <= MAX_CACHE_BYTES);
        });
    }
}
