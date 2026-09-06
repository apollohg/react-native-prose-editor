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

fn push_case_lazy(
    results: &mut Vec<BenchResult>,
    options: &BenchOptions,
    name: &'static str,
    group: &'static str,
    run: impl FnOnce() -> BenchResult,
) {
    if let Some(result) =
        benchmark_filter::run_if_selected(options.filter.as_deref(), name, group, run)
    {
        results.push(result);
    }
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

#[allow(clippy::too_many_arguments)]
fn verified_bench_case<S, Setup, Run, Output, Verify>(
    name: &'static str,
    group: &'static str,
    iterations: usize,
    warmup_iterations: usize,
    ops_per_iteration: usize,
    mut setup: Setup,
    mut run: Run,
    mut verify: Verify,
) -> BenchResult
where
    Setup: FnMut() -> S,
    Run: FnMut(&mut S) -> Output,
    Verify: FnMut(&S, &Output),
{
    for _ in 0..warmup_iterations {
        let mut state = setup();
        let output = black_box(run(&mut state));
        verify(&state, &output);
    }

    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let mut state = setup();
        let started_at = Instant::now();
        let output = black_box(run(&mut state));
        let elapsed = started_at.elapsed();
        verify(&state, &output);
        samples.push(elapsed);
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
            "editingTypingBurst": EDITING_TYPING_BURST,
            "selectionScrubPoints": profile.selection_scrub_points,
            "awarenessPeerCount": profile.awareness_peer_count,
            "opaquePayloadBytes": profile.opaque_payload_bytes,
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
