use native_editor_highlighting::{highlight_code, HighlightError};

fn highlight(text: &str, language: &str) -> Vec<native_editor_highlighting::HighlightRange> {
    highlight_code(
        text.into(),
        Some(language.into()),
        "base16-ocean.dark".into(),
    )
    .unwrap()
}

#[test]
fn covers_advertised_languages_with_real_syntax_colors() {
    for (language, source) in [
        ("javascript", "const answer = 42; // comment\n"),
        ("typescript", "interface User { name: string }\n"),
        ("tsx", "const el = <View title={value} />;\n"),
        ("jsx", "const el = <View title={value} />;\n"),
        ("swift", "let message: String = \"hello\"\n"),
        ("kotlin", "val message: String = \"hello\"\n"),
        ("rust", "let answer: u32 = 42;\n"),
        ("python", "def answer(): return 42\n"),
        ("json", "{\"answer\": 42}\n"),
        ("html", "<div class=\"answer\">hello</div>\n"),
        ("css", ".answer { color: red; }\n"),
        ("bash", "echo \"$HOME\" # comment\n"),
    ] {
        let ranges = highlight(source, language);
        assert!(!ranges.is_empty(), "{language}");
        assert!(
            ranges.iter().any(|range| range.color != ranges[0].color),
            "{language}"
        );
    }
}

#[test]
fn multiline_comments_keep_parser_state_and_utf16_offsets() {
    let source = "/* 🦀\ncontinued */\nconst answer = 42;\n";
    let ranges = highlight(source, "js");
    let next_line = "/* 🦀\n".encode_utf16().count() as u32;
    let comment = ranges
        .iter()
        .find(|range| range.start <= next_line && range.start + range.length > next_line)
        .unwrap();
    assert_eq!(comment.color, ranges[0].color);
    let mut end = 0;
    for range in ranges {
        assert_eq!(range.start, end);
        end += range.length;
    }
    assert_eq!(end, source.encode_utf16().count() as u32);
}

#[test]
fn unsupported_plain_and_bounded_inputs_fall_back() {
    for language in [None, Some("plain".into()), Some("not-a-language".into())] {
        assert!(
            highlight_code("let x = 1".into(), language, "base16-ocean.dark".into())
                .unwrap()
                .is_empty()
        );
    }
    assert!(highlight(&"x".repeat(4097), "js").is_empty());
    assert!(highlight(&"x\n".repeat(1001), "js").is_empty());
    assert!(highlight(&"x\n".repeat(32769), "js").is_empty());
}

#[test]
fn theme_changes_do_not_reuse_stale_colors() {
    let text = "const answer = 42;";
    let dark = highlight(text, "js");
    let light =
        highlight_code(text.into(), Some("js".into()), "base16-ocean.light".into()).unwrap();
    assert_ne!(dark, light);
    assert_eq!(dark, highlight(text, "js"));
    assert!(matches!(
        highlight_code(text.into(), Some("js".into()), "missing".into()),
        Err(HighlightError::UnknownTheme { .. })
    ));
}

#[test]
fn worker_threads_have_independent_state() {
    let handles: Vec<_> = (0..3)
        .map(|_| std::thread::spawn(|| highlight("const answer = 42;", "js")))
        .collect();
    for worker in handles {
        assert_eq!(
            worker.join().unwrap(),
            highlight("const answer = 42;", "js")
        );
    }
}
