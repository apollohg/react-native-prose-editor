use native_editor_highlighting::highlight_code;
use std::time::Instant;

fn main() {
    println!("case,iterations,total_ms");
    for (name, language, source) in [
        (
            "javascript",
            "js",
            "const answer = 42; // comment\n".repeat(30),
        ),
        (
            "typescript",
            "ts",
            "interface User { name: string }\n".repeat(30),
        ),
        (
            "tsx",
            "tsx",
            "const view = <View title={value} />;\n".repeat(30),
        ),
        (
            "swift",
            "swift",
            "let greeting: String = \"hello\"\n".repeat(30),
        ),
        (
            "kotlin",
            "kt",
            "val greeting: String = \"hello\"\n".repeat(30),
        ),
        ("long_parentheses", "js", "(".repeat(4000)),
        ("long_slashes", "js", "/".repeat(4000)),
    ] {
        let start = Instant::now();
        let ranges = highlight_code(
            source.clone(),
            Some(language.into()),
            "base16-ocean.dark".into(),
        )
        .unwrap();
        println!(
            "{name}_first,1,{:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        for _ in 0..100 {
            assert_eq!(
                ranges,
                highlight_code(
                    source.clone(),
                    Some(language.into()),
                    "base16-ocean.dark".into()
                )
                .unwrap()
            );
        }
        println!(
            "{name}_cached,100,{:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
        let start = Instant::now();
        for revision in 0..20 {
            highlight_code(
                format!("{source} {revision}"),
                Some(language.into()),
                "base16-ocean.dark".into(),
            )
            .unwrap();
        }
        println!(
            "{name}_edits,20,{:.3}",
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
