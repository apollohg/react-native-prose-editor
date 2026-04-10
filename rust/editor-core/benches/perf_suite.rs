use std::env;
use std::hint::black_box;
use std::time::{Duration, Instant};

use editor_core::collaboration::CollaborationSession;
use editor_core::editor::Editor;
use editor_core::intercept::InterceptorPipeline;
use editor_core::schema::presets::tiptap_schema;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum BenchMode {
    Quick,
    Standard,
}

impl BenchMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Standard => "standard",
        }
    }

    fn profile(self) -> BenchProfile {
        match self {
            Self::Quick => BenchProfile {
                warmup_iterations: 2,
                iterations: 8,
                article_blocks: 48,
                paragraph_chars: 140,
                mapping_points: 192,
                selection_width: 64,
                typing_burst: 24,
                selection_scrub_points: 48,
                awareness_peer_count: 12,
            },
            Self::Standard => BenchProfile {
                warmup_iterations: 4,
                iterations: 20,
                article_blocks: 160,
                paragraph_chars: 220,
                mapping_points: 768,
                selection_width: 160,
                typing_burst: 64,
                selection_scrub_points: 160,
                awareness_peer_count: 32,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct BenchProfile {
    warmup_iterations: usize,
    iterations: usize,
    article_blocks: usize,
    paragraph_chars: usize,
    mapping_points: usize,
    selection_width: u32,
    typing_burst: usize,
    selection_scrub_points: usize,
    awareness_peer_count: usize,
}

struct BenchOptions {
    mode: BenchMode,
    json_output: bool,
    filter: Option<String>,
}

#[derive(Debug)]
struct BenchResult {
    name: &'static str,
    group: &'static str,
    iterations: usize,
    ops_per_iteration: usize,
    min_ms: f64,
    p50_ms: f64,
    mean_ms: f64,
    p95_ms: f64,
    max_ms: f64,
    mean_us_per_op: f64,
}

fn main() {
    let options = parse_options();
    let profile = options.mode.profile();
    let article_doc = build_article_document(profile.article_blocks, profile.paragraph_chars);
    let edited_article_doc = build_edited_article_document(&article_doc);

    let mut results = Vec::new();

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.set_json.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || build_article_document(profile.article_blocks, profile.paragraph_chars),
            |doc| {
                let mut editor = empty_editor();
                black_box(
                    editor
                        .set_json(doc)
                        .expect("set_json benchmark should succeed"),
                );
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.get_current_state.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(&article_doc),
            |editor| {
                black_box(editor.get_current_state());
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.get_selection_state.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(&article_doc),
            |editor| {
                black_box(editor.get_selection_state());
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.get_html.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(&article_doc),
            |editor| {
                black_box(editor.get_html());
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.get_json.article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || editor_with_document(&article_doc),
            |editor| {
                black_box(editor.get_json());
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.insert_text_scalar.middle_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                (editor, total_scalar / 2)
            },
            |(editor, cursor_scalar)| {
                black_box(
                    editor
                        .insert_text_scalar(*cursor_scalar, "!")
                        .expect("insert_text_scalar benchmark should succeed"),
                );
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.insert_text_scalar.typing_burst_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            profile.typing_burst,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                (editor, total_scalar / 2)
            },
            |(editor, cursor_scalar)| {
                let mut next_cursor = *cursor_scalar;
                for _ in 0..profile.typing_burst {
                    black_box(
                        editor
                            .insert_text_scalar(next_cursor, "!")
                            .expect("typing burst benchmark should succeed"),
                    );
                    next_cursor = next_cursor.saturating_add(1);
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.toggle_mark_scalar.selection_article",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let anchor = total_scalar / 3;
                let head = (anchor + profile.selection_width).min(total_scalar.max(anchor));
                (editor, anchor, head.max(anchor + 1))
            },
            |(editor, anchor, head)| {
                black_box(
                    editor
                        .toggle_mark_at_selection_scalar(*anchor, *head, "bold")
                        .expect("toggle_mark_at_selection_scalar benchmark should succeed"),
                );
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.replace_json.article_small_edit",
            "editor",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    editor_with_document(&article_doc),
                    edited_article_doc.clone(),
                )
            },
            |(editor, next_doc)| {
                black_box(
                    editor
                        .replace_json(next_doc)
                        .expect("replace_json benchmark should succeed"),
                );
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "position.doc_to_scalar.article_sweep",
            "position",
            profile.iterations,
            profile.warmup_iterations,
            profile.mapping_points,
            || {
                let editor = editor_with_document(&article_doc);
                let positions = evenly_spaced_positions(
                    editor.document().content_size(),
                    profile.mapping_points,
                );
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    black_box(editor.doc_to_scalar(*position));
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "position.scalar_to_doc.article_sweep",
            "position",
            profile.iterations,
            profile.warmup_iterations,
            profile.mapping_points,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions = evenly_spaced_positions(total_scalar, profile.mapping_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    black_box(editor.scalar_to_doc(*position));
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "editor.set_selection_scalar.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.selection());
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "selection.refresh_toolbar_state_full.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.get_current_state());
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "selection.refresh_toolbar_state_light.article_scrub",
            "selection",
            profile.iterations,
            profile.warmup_iterations,
            profile.selection_scrub_points,
            || {
                let editor = editor_with_document(&article_doc);
                let total_scalar = editor.doc_to_scalar(editor.document().content_size());
                let positions =
                    selection_scrub_positions(total_scalar, profile.selection_scrub_points);
                (editor, positions)
            },
            |(editor, positions)| {
                for position in positions {
                    editor.set_selection_scalar(*position, *position);
                    black_box(editor.get_selection_state());
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "collaboration.apply_local_document.article_small_edit",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                (
                    collaboration_session_with_document(&article_doc),
                    edited_article_doc.clone(),
                )
            },
            |(session, next_doc)| {
                black_box(session.apply_local_document(next_doc.clone()));
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.document_update",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let mut sender = collaboration_session_with_document(&article_doc);
                let receiver = collaboration_session_with_document(&article_doc);
                let message = sender
                    .apply_local_document(edited_article_doc.clone())
                    .messages
                    .into_iter()
                    .next()
                    .expect("document update benchmark should emit a message");
                (receiver, message)
            },
            |(session, message)| {
                black_box(
                    session
                        .handle_message(message.clone())
                        .expect("document update message benchmark should succeed"),
                );
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.awareness_multi_peer_burst",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            profile.awareness_peer_count,
            || {
                (
                    collaboration_session_with_document(&article_doc),
                    awareness_messages_for_document(
                        &article_doc,
                        profile.awareness_peer_count,
                        profile.selection_width,
                    ),
                )
            },
            |(session, messages)| {
                for message in messages {
                    black_box(
                        session
                            .handle_message(message.clone())
                            .expect("multi-peer awareness benchmark should succeed"),
                    );
                }
            },
        ),
    );

    push_case(
        &mut results,
        &options,
        bench_case(
            "collaboration.handle_message.awareness",
            "collaboration",
            profile.iterations,
            profile.warmup_iterations,
            1,
            || {
                let article = build_article_document(12, 96);
                let mut sender = collaboration_session_with_document(&article);
                let receiver = collaboration_session_with_document(&article);
                let message = sender
                    .set_local_awareness(json!({
                        "user": {
                            "name": "Perf Bench",
                            "color": "#007AFF",
                            "userId": "bench-user"
                        },
                        "selection": {
                            "anchor": 1,
                            "head": 1
                        },
                        "focused": true
                    }))
                    .messages
                    .into_iter()
                    .next()
                    .expect("awareness benchmark should emit a message");
                (receiver, message)
            },
            |(session, message)| {
                black_box(
                    session
                        .handle_message(message.clone())
                        .expect("awareness message benchmark should succeed"),
                );
            },
        ),
    );

    if results.is_empty() {
        eprintln!("no benchmarks matched the provided filter");
        std::process::exit(1);
    }

    if options.json_output {
        print_json_summary(options.mode, profile, &results);
    } else {
        print_table(options.mode, profile, &results);
    }
}

fn parse_options() -> BenchOptions {
    let mut mode = BenchMode::Standard;
    let mut json_output = false;
    let mut filter = None;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quick" => mode = BenchMode::Quick,
            "--json" => json_output = true,
            "--bench" => {}
            "--filter" => {
                filter = args.next();
            }
            _ if arg.starts_with("--filter=") => {
                filter = Some(arg["--filter=".len()..].to_string());
            }
            _ => {}
        }
    }

    BenchOptions {
        mode,
        json_output,
        filter,
    }
}

fn push_case(results: &mut Vec<BenchResult>, options: &BenchOptions, result: BenchResult) {
    if let Some(filter) = options.filter.as_deref() {
        if !result.name.contains(filter) && !result.group.contains(filter) {
            return;
        }
    }
    results.push(result);
}

fn bench_case<S, Setup, Run, Output>(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    warmup_iterations: usize,
    ops_per_iteration: usize,
    mut setup: Setup,
    mut run: Run,
) -> BenchResult
where
    Setup: FnMut() -> S,
    Run: FnMut(&mut S) -> Output,
{
    for _ in 0..warmup_iterations {
        let mut state = setup();
        black_box(run(&mut state));
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut state = setup();
        let started_at = Instant::now();
        black_box(run(&mut state));
        samples.push(started_at.elapsed());
    }

    build_result(name, group, iterations, ops_per_iteration, samples)
}

fn build_result(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    ops_per_iteration: usize,
    mut samples: Vec<Duration>,
) -> BenchResult {
    let total_ms = samples
        .iter()
        .map(|duration| duration_to_ms(*duration))
        .sum::<f64>();
    let mean_ms = total_ms / iterations.max(1) as f64;
    samples.sort_unstable();
    let min_ms = duration_to_ms(*samples.first().unwrap_or(&Duration::ZERO));
    let max_ms = duration_to_ms(*samples.last().unwrap_or(&Duration::ZERO));
    let p50_ms = percentile_ms(&samples, 0.50);
    let p95_ms = percentile_ms(&samples, 0.95);
    let mean_us_per_op = (mean_ms * 1_000.0) / ops_per_iteration.max(1) as f64;

    BenchResult {
        name,
        group,
        iterations,
        ops_per_iteration,
        min_ms,
        p50_ms,
        mean_ms,
        p95_ms,
        max_ms,
        mean_us_per_op,
    }
}

fn percentile_ms(samples: &[Duration], percentile: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let clamped = percentile.clamp(0.0, 1.0);
    let index = ((samples.len() - 1) as f64 * clamped).round() as usize;
    duration_to_ms(samples[index])
}

fn duration_to_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn print_table(mode: BenchMode, profile: BenchProfile, results: &[BenchResult]) {
    println!(
        "editor-core performance suite (mode: {}, iterations: {}, warmup: {})",
        mode.as_str(),
        profile.iterations,
        profile.warmup_iterations
    );
    println!(
        "{:<48} {:>5} {:>8} {:>10} {:>10} {:>10} {:>10} {:>11}",
        "benchmark", "iters", "ops", "mean ms", "p50 ms", "p95 ms", "max ms", "mean us/op"
    );
    println!("{}", "-".repeat(118));

    for result in results {
        println!(
            "{:<48} {:>5} {:>8} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>11.2}",
            result.name,
            result.iterations,
            result.ops_per_iteration,
            result.mean_ms,
            result.p50_ms,
            result.p95_ms,
            result.max_ms,
            result.mean_us_per_op
        );
    }
}

fn print_json_summary(mode: BenchMode, profile: BenchProfile, results: &[BenchResult]) {
    let payload = json!({
        "mode": mode.as_str(),
        "iterations": profile.iterations,
        "warmupIterations": profile.warmup_iterations,
        "documentProfile": {
            "articleBlocks": profile.article_blocks,
            "paragraphChars": profile.paragraph_chars,
            "mappingPoints": profile.mapping_points,
            "selectionWidth": profile.selection_width,
            "typingBurst": profile.typing_burst,
            "selectionScrubPoints": profile.selection_scrub_points,
            "awarenessPeerCount": profile.awareness_peer_count,
        },
        "results": results.iter().map(|result| {
            json!({
                "name": result.name,
                "group": result.group,
                "iterations": result.iterations,
                "opsPerIteration": result.ops_per_iteration,
                "minMs": result.min_ms,
                "p50Ms": result.p50_ms,
                "meanMs": result.mean_ms,
                "p95Ms": result.p95_ms,
                "maxMs": result.max_ms,
                "meanUsPerOp": result.mean_us_per_op,
            })
        }).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&payload).expect("benchmark JSON payload should serialize")
    );
}

fn empty_editor() -> Editor {
    Editor::new(tiptap_schema(), InterceptorPipeline::new(), false)
}

fn editor_with_document(doc: &Value) -> Editor {
    let mut editor = empty_editor();
    editor
        .set_json(doc)
        .expect("benchmark fixture document should parse");
    editor
}

fn collaboration_session_with_document(doc: &Value) -> CollaborationSession {
    collaboration_session_with_document_and_client(doc, None)
}

fn collaboration_session_with_document_and_client(
    doc: &Value,
    client_id: Option<u64>,
) -> CollaborationSession {
    let config = json!({
        "clientId": client_id,
        "fragmentName": "default",
        "initialDocumentJson": doc
    })
    .to_string();
    CollaborationSession::new(&config)
}

fn evenly_spaced_positions(max_value: u32, points: usize) -> Vec<u32> {
    if points <= 1 || max_value == 0 {
        return vec![0];
    }

    (0..points)
        .map(|index| ((max_value as u64 * index as u64) / (points - 1) as u64) as u32)
        .collect()
}

fn selection_scrub_positions(total_scalar: u32, points: usize) -> Vec<u32> {
    let upper_bound = total_scalar.saturating_sub(1).max(1);
    evenly_spaced_positions(upper_bound, points)
        .into_iter()
        .map(|position| position.max(1))
        .collect()
}

fn awareness_messages_for_document(
    doc: &Value,
    peer_count: usize,
    selection_width: u32,
) -> Vec<Vec<u8>> {
    let editor = editor_with_document(doc);
    let content_size = editor.document().content_size().saturating_sub(1).max(1);
    let positions = evenly_spaced_positions(content_size, peer_count);

    positions
        .into_iter()
        .enumerate()
        .map(|(index, position)| {
            let client_id = index as u64 + 2;
            let mut session =
                collaboration_session_with_document_and_client(doc, Some(client_id));
            let anchor = position.max(1);
            let head = if selection_width > 0 && index % 2 == 1 {
                anchor.saturating_add(selection_width).min(content_size)
            } else {
                anchor
            };
            session
                .set_local_awareness(json!({
                    "user": {
                        "name": format!("Peer {}", client_id),
                        "color": format!("#{:06X}", (0x3366FFu32 + (index as u32 * 0x111111)) & 0xFFFFFF),
                        "userId": format!("bench-peer-{}", client_id)
                    },
                    "selection": {
                        "anchor": anchor,
                        "head": head
                    },
                    "focused": true
                }))
                .messages
                .into_iter()
                .next()
                .expect("awareness benchmark should emit a message")
        })
        .collect()
}

fn build_article_document(block_count: usize, paragraph_chars: usize) -> Value {
    let mut content = Vec::with_capacity(block_count + (block_count / 12) + 2);
    content.push(json!({
        "type": "h1",
        "content": [text_node(text_fragment(10_000, 42))]
    }));

    for index in 0..block_count {
        if index > 0 && index % 18 == 0 {
            content.push(json!({ "type": "horizontalRule" }));
        }

        if index % 12 == 5 {
            content.push(json!({
                "type": "blockquote",
                "content": [{
                    "type": "paragraph",
                    "content": rich_inline_content(index, paragraph_chars)
                }]
            }));
            continue;
        }

        if index % 9 == 3 {
            content.push(json!({
                "type": "h2",
                "content": [text_node(text_fragment(index + 2_000, paragraph_chars / 3 + 24))]
            }));
            continue;
        }

        content.push(json!({
            "type": "paragraph",
            "content": rich_inline_content(index, paragraph_chars)
        }));
    }

    json!({
        "type": "doc",
        "content": content
    })
}

fn build_edited_article_document(doc: &Value) -> Value {
    let mut next = doc.clone();
    let appended = append_to_last_text_node(&mut next, " sync-update");
    assert!(
        appended,
        "edited benchmark document should contain text nodes"
    );
    next
}

fn append_to_last_text_node(node: &mut Value, suffix: &str) -> bool {
    match node {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(Value::String(text)) = object.get_mut("text") {
                    text.push_str(suffix);
                    return true;
                }
            }

            if let Some(children) = object.get_mut("content").and_then(Value::as_array_mut) {
                for child in children.iter_mut().rev() {
                    if append_to_last_text_node(child, suffix) {
                        return true;
                    }
                }
            }
            false
        }
        Value::Array(array) => {
            for child in array.iter_mut().rev() {
                if append_to_last_text_node(child, suffix) {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn rich_inline_content(seed: usize, total_chars: usize) -> Vec<Value> {
    let full_text = text_fragment(seed, total_chars.max(32));
    let chars: Vec<char> = full_text.chars().collect();
    let len = chars.len();
    let cut_a = len / 4;
    let cut_b = len / 2;
    let cut_c = (len * 3) / 4;

    let plain_lead = char_slice(&chars, 0, cut_a);
    let bold_text = char_slice(&chars, cut_a, cut_b);
    let italic_text = char_slice(&chars, cut_b, cut_c);
    let tail_text = char_slice(&chars, cut_c, len);

    let mut content = Vec::new();
    if !plain_lead.is_empty() {
        content.push(text_node(plain_lead));
    }
    if !bold_text.is_empty() {
        content.push(marked_text_node(bold_text, json!({ "type": "bold" })));
    }
    if !italic_text.is_empty() {
        content.push(marked_text_node(italic_text, json!({ "type": "italic" })));
    }
    if !tail_text.is_empty() {
        content.push(marked_text_node(
            tail_text,
            json!({
                "type": "link",
                "attrs": {
                    "href": format!("https://example.com/item/{seed}"),
                    "target": "_blank",
                    "rel": "noopener noreferrer nofollow",
                    "class": Value::Null,
                    "title": Value::Null
                }
            }),
        ));
    }

    content
}

fn text_node(text: String) -> Value {
    json!({
        "type": "text",
        "text": text
    })
}

fn marked_text_node(text: String, mark: Value) -> Value {
    json!({
        "type": "text",
        "text": text,
        "marks": [mark]
    })
}

fn text_fragment(seed: usize, min_chars: usize) -> String {
    const WORDS: &[&str] = &[
        "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
        "juliet", "kilo", "lima", "mike", "november", "oscar", "papa", "quebec", "romeo", "sierra",
        "tango", "uniform", "victor", "whiskey", "xray", "yankee", "zulu",
    ];

    let mut text = String::new();
    let mut cursor = 0usize;
    while text.chars().count() < min_chars {
        if !text.is_empty() {
            text.push(' ');
        }
        let word = WORDS[(seed + cursor) % WORDS.len()];
        text.push_str(word);
        cursor += 1;
    }
    text.chars().take(min_chars).collect()
}

fn char_slice(chars: &[char], start: usize, end: usize) -> String {
    let bounded_start = start.min(chars.len());
    let bounded_end = end.min(chars.len()).max(bounded_start);
    chars[bounded_start..bounded_end].iter().collect()
}
