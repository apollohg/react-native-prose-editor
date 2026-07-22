import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
    aggregateBenchmarkSamples,
    checkBenchmarkRun,
    runBenchmarkSamples,
} from '../check-yrs-editing-semantics-benchmarks.mjs';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const checker = path.join(repositoryRoot, 'scripts/check-yrs-editing-semantics-benchmarks.mjs');
// Task 16C (user directive 2026-07-20): the legacy runtime and its
// reference benchmarks were removed; the checker contract keeps only the
// Yrs cases, with absolute, scaling, and 1.20x baseline-regression gates
// unchanged. Baselines may still carry the old legacy.* entries; the
// checker must ignore any case it does not require.
const requiredCases = [
    'yrs.edit.insert_char.article.1x',
    'yrs.edit.typing_burst.article.1x',
    'yrs.state.selection_light.article.1x',
    'yrs.command.toggle_mark.article.1x',
    'yrs.command.wrap_list.article.1x',
    'yrs.history.undo.article.1x',
    'yrs.history.redo.article.1x',
    'yrs.edit.insert_char.article.2x',
    'yrs.state.selection_light.article.2x',
    'yrs.command.wrap_list.article.2x',
];

const legacyBaselineEntries = Object.fromEntries(
    [
        'legacy.edit.insert_char.article.1x',
        'legacy.edit.typing_burst.article.1x',
        'legacy.state.selection_light.article.1x',
        'legacy.command.toggle_mark.article.1x',
        'legacy.command.wrap_list.article.1x',
        'legacy.history.undo.article.1x',
        'legacy.history.redo.article.1x',
    ].map((name) => [name, 0.25])
);

function benchmark(overrides = {}) {
    return {
        mode: 'standard',
        results: requiredCases.map((name) => ({ name, p50Ms: overrides[name] ?? 0.5 })),
    };
}

function rawBenchmark(overrides = {}) {
    return {
        ...benchmark(overrides),
        iterations: 20,
        warmupIterations: 4,
        documentProfile: { editingTypingBurst: 20 },
        results: requiredCases.map((name) => ({
            name,
            group: 'yrs-editing',
            opsPerIteration: name.includes('.typing_burst.') ? 20 : 1,
            p50Ms: overrides[name] ?? 0.5,
        })),
    };
}

function runChecker(input, baseline = benchmark()) {
    const directory = mkdtempSync(path.join(tmpdir(), 'yrs-editing-benchmark-check-'));
    const inputPath = path.join(directory, 'input.json');
    const baselinePath = path.join(directory, 'baseline.json');
    writeFileSync(inputPath, JSON.stringify(input));
    writeFileSync(baselinePath, JSON.stringify(baseline));
    return spawnSync(process.execPath, [checker, inputPath, '--baseline', baselinePath], {
        cwd: repositoryRoot,
        encoding: 'utf8',
    });
}

test('retains five exact raw samples and aggregates every case independently', () => {
    const calls = [];
    const samples = [5, 1, 4, 2, 3].map((insertValue, index) =>
        rawBenchmark({
            'yrs.edit.insert_char.article.1x': insertValue,
            'yrs.state.selection_light.article.1x': [1, 2, 4, 3, 5][index],
        })
    );

    const run = runBenchmarkSamples((sampleNumber) => {
        calls.push(sampleNumber);
        return samples[sampleNumber - 1];
    });

    assert.deepEqual(calls, [1, 2, 3, 4, 5]);
    run.samples.forEach((sample, index) => assert.strictEqual(sample, samples[index]));
    assert.equal(samples.includes(run.aggregate), false);
    const medians = Object.fromEntries(
        run.aggregate.results.map(({ name, p50Ms }) => [name, p50Ms])
    );
    assert.equal(medians['yrs.edit.insert_char.article.1x'], 3);
    assert.equal(medians['yrs.state.selection_light.article.1x'], 3);
    assert.equal(
        samples.some((sample) => {
            const values = Object.fromEntries(
                sample.results.map(({ name, p50Ms }) => [name, p50Ms])
            );
            return (
                values['yrs.edit.insert_char.article.1x'] === 3 &&
                values['yrs.state.selection_light.article.1x'] === 3
            );
        }),
        false
    );
});

test('requires exactly five fresh samples', () => {
    assert.throws(
        () => aggregateBenchmarkSamples(Array.from({ length: 4 }, () => rawBenchmark())),
        {
            message: /exactly five benchmark samples/,
        }
    );
    assert.throws(
        () => aggregateBenchmarkSamples(Array.from({ length: 6 }, () => rawBenchmark())),
        {
            message: /exactly five benchmark samples/,
        }
    );
});

test('validates and emits each raw sample immediately, then emits median evidence before gates', () => {
    const events = [];
    const run = runBenchmarkSamples(
        (sampleNumber) => {
            events.push(`run-${sampleNumber}`);
            return rawBenchmark();
        },
        ({ sampleNumber }) => events.push(`raw-${sampleNumber}`)
    );
    const failure = new Error('median gate failed');

    assert.deepEqual(events, [
        'run-1',
        'raw-1',
        'run-2',
        'raw-2',
        'run-3',
        'raw-3',
        'run-4',
        'raw-4',
        'run-5',
        'raw-5',
    ]);
    assert.throws(
        () =>
            checkBenchmarkRun(
                run,
                benchmark(),
                (evidence) => events.push(evidence),
                () => {
                    events.push('check');
                    throw failure;
                }
            ),
        failure
    );
    assert.equal(events.at(-2).evidenceType, 'yrs-editing-semantics-five-run-median');
    assert.strictEqual(events.at(-2).medianAggregate, run.aggregate);
    assert.equal(events.at(-1), 'check');
});

test('stops at the first invalid sample or Cargo failure and retains earlier raw evidence', async (t) => {
    for (const [name, failureSample, runner, message] of [
        [
            'invalid sample',
            4,
            (sampleNumber) => (sampleNumber === 4 ? { results: [] } : rawBenchmark()),
            /sample 4 is missing required case/,
        ],
        [
            'Cargo failure',
            3,
            (sampleNumber) => {
                if (sampleNumber === 3) throw new Error('Cargo benchmark exited with status 1');
                return rawBenchmark();
            },
            /sample 3: Cargo benchmark exited with status 1/,
        ],
    ]) {
        await t.test(name, () => {
            const calls = [];
            const evidence = [];
            assert.throws(
                () =>
                    runBenchmarkSamples(
                        (sampleNumber) => {
                            calls.push(sampleNumber);
                            return runner(sampleNumber);
                        },
                        (sample) => evidence.push(sample)
                    ),
                message
            );
            assert.deepEqual(
                calls,
                Array.from({ length: failureSample }, (_, index) => index + 1)
            );
            assert.deepEqual(
                evidence.map(({ sampleNumber }) => sampleNumber),
                Array.from({ length: failureSample - 1 }, (_, index) => index + 1)
            );
        });
    }
});

test('fresh raw samples pin the exact standard editing workload before retention', async (t) => {
    const mutateResult = (sample, caseName, mutate) => ({
        ...sample,
        results: sample.results.map((result) =>
            result.name === caseName ? mutate({ ...result }) : result
        ),
    });
    for (const [name, sample, message] of [
        ['mode', { ...rawBenchmark(), mode: 'quick' }, /mode must be standard/],
        ['iterations', { ...rawBenchmark(), iterations: 8 }, /iterations must equal 20/],
        ['warmups', { ...rawBenchmark(), warmupIterations: 2 }, /warmupIterations must equal 4/],
        [
            'burst profile',
            { ...rawBenchmark(), documentProfile: { editingTypingBurst: 19 } },
            /editingTypingBurst must equal 20/,
        ],
        [
            'group',
            mutateResult(rawBenchmark(), requiredCases[0], (result) => ({
                ...result,
                group: 'other',
            })),
            /group must be yrs-editing/,
        ],
        [
            'burst operations',
            mutateResult(rawBenchmark(), 'yrs.edit.typing_burst.article.1x', (result) => ({
                ...result,
                opsPerIteration: 19,
            })),
            /opsPerIteration must equal 20/,
        ],
        [
            'single operations',
            mutateResult(rawBenchmark(), 'yrs.edit.insert_char.article.1x', (result) => ({
                ...result,
                opsPerIteration: 2,
            })),
            /opsPerIteration must equal 1/,
        ],
    ]) {
        await t.test(name, () => {
            const evidence = [];
            assert.throws(
                () =>
                    runBenchmarkSamples(
                        () => sample,
                        (event) => evidence.push(event)
                    ),
                message
            );
            assert.deepEqual(evidence, []);
        });
    }
});

test('requires every exact case once with finite positive values', async (t) => {
    const valid = benchmark();
    for (const [name, input, message] of [
        ['missing', { ...valid, results: valid.results.slice(1) }, /missing required case/],
        [
            'duplicate',
            { ...valid, results: [...valid.results, valid.results[0]] },
            /duplicate benchmark case/,
        ],
        [
            'eighteenth',
            { ...valid, results: [...valid.results, { name: 'yrs.uncontracted.extra', p50Ms: 1 }] },
            /unexpected benchmark case/,
        ],
        [
            'non-finite',
            benchmark({ 'yrs.history.redo.article.1x': Number.POSITIVE_INFINITY }),
            /finite positive p50Ms/,
        ],
        ['non-positive', benchmark({ 'yrs.history.redo.article.1x': 0 }), /finite positive p50Ms/],
    ]) {
        await t.test(name, () => {
            const result = runChecker(input);
            assert.notEqual(result.status, 0);
            assert.match(result.stderr, message);
            assert.doesNotMatch(result.stderr, /at .*\.mjs:\d+/);
        });
    }
});

test('file-input mode checks one already-aggregated payload without requiring raw samples', () => {
    const result = runChecker(benchmark(), benchmark());
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /10 benchmark cases passed/);
    assert.doesNotMatch(result.stdout, /raw-sample/);
});

test('accepts every absolute, scaling, and baseline exact boundary', () => {
    const input = benchmark({
        'yrs.edit.insert_char.article.1x': 2,
        'yrs.edit.typing_burst.article.1x': 20,
        'yrs.state.selection_light.article.1x': 1,
        'yrs.command.toggle_mark.article.1x': 5,
        'yrs.command.wrap_list.article.1x': 5,
        'yrs.history.undo.article.1x': 5,
        'yrs.history.redo.article.1x': 5,
        'yrs.edit.insert_char.article.2x': 5,
        'yrs.state.selection_light.article.2x': 2.5,
        'yrs.command.wrap_list.article.2x': 12.5,
    });
    const baseline = benchmark(
        Object.fromEntries(input.results.map(({ name, p50Ms }) => [name, p50Ms / 1.2]))
    );
    const result = runChecker(input, baseline);
    assert.equal(result.status, 0, result.stderr);
});

test('ignores legacy.* entries carried in the frozen baseline', () => {
    const baseline = {
        mode: 'standard',
        benchmarkGroup: 'yrs-editing',
        results: [
            ...benchmark().results,
            ...Object.entries(legacyBaselineEntries).map(([name, p50Ms]) => ({
                name,
                p50Ms,
            })),
        ],
    };
    const result = runChecker(benchmark(), baseline);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /10 benchmark cases passed/);
});

test('rejects each absolute ceiling family with actionable absolute diagnostics', async (t) => {
    for (const [name, caseName, allowed] of [
        ['insert', 'yrs.edit.insert_char.article.1x', 2],
        ['burst', 'yrs.edit.typing_burst.article.1x', 20],
        ['selection', 'yrs.state.selection_light.article.1x', 1],
        ['toggle', 'yrs.command.toggle_mark.article.1x', 5],
        ['wrap', 'yrs.command.wrap_list.article.1x', 5],
        ['undo', 'yrs.history.undo.article.1x', 5],
        ['redo', 'yrs.history.redo.article.1x', 5],
    ]) {
        await t.test(name, () => {
            const result = runChecker(benchmark({ [caseName]: allowed + 0.001 }));
            assert.notEqual(result.status, 0);
            assert.match(result.stderr, /absolute ceiling failure/);
            assert.match(result.stderr, new RegExp(caseName.replaceAll('.', '\\.')));
            assert.match(result.stderr, new RegExp(`allowed p50=${allowed} ms`));
        });
    }
});

test('rejects all three 2x scaling pairs with actionable scaling diagnostics', async (t) => {
    for (const [name, oneX, twoX] of [
        ['insert', 'yrs.edit.insert_char.article.1x', 'yrs.edit.insert_char.article.2x'],
        [
            'selection',
            'yrs.state.selection_light.article.1x',
            'yrs.state.selection_light.article.2x',
        ],
        ['wrap', 'yrs.command.wrap_list.article.1x', 'yrs.command.wrap_list.article.2x'],
    ]) {
        await t.test(name, () => {
            const result = runChecker(benchmark({ [oneX]: 0.1, [twoX]: 0.251 }));
            assert.notEqual(result.status, 0);
            assert.match(result.stderr, /scaling failure/);
            assert.match(result.stderr, /actual ratio=2\.51/);
            assert.match(result.stderr, /allowed ratio=2\.5/);
        });
    }
});

test('rejects a regression above every-case baseline gate with baseline diagnostics', () => {
    const input = benchmark({ 'yrs.history.redo.article.1x': 0.601 });
    const result = runChecker(input, benchmark());
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /baseline regression failure/);
    assert.match(result.stderr, /actual ratio=1\.202/);
    assert.match(result.stderr, /allowed ratio=1\.2/);
});

test('rejects malformed input and baseline JSON cleanly', async (t) => {
    for (const target of ['input', 'baseline']) {
        await t.test(target, () => {
            const directory = mkdtempSync(path.join(tmpdir(), 'yrs-editing-benchmark-json-'));
            const inputPath = path.join(directory, 'input.json');
            const baselinePath = path.join(directory, 'baseline.json');
            writeFileSync(inputPath, target === 'input' ? '{' : JSON.stringify(benchmark()));
            writeFileSync(baselinePath, target === 'baseline' ? '{' : JSON.stringify(benchmark()));
            const result = spawnSync(
                process.execPath,
                [checker, inputPath, '--baseline', baselinePath],
                {
                    cwd: repositoryRoot,
                    encoding: 'utf8',
                }
            );
            assert.notEqual(result.status, 0);
            assert.match(result.stderr, new RegExp(`failed to parse ${target}`));
            assert.doesNotMatch(result.stderr, /at .*\.mjs:\d+/);
        });
    }
});

test('package scripts expose the exact editing benchmark commands', () => {
    const packageJson = JSON.parse(readFileSync(path.join(repositoryRoot, 'package.json'), 'utf8'));
    assert.equal(
        packageJson.scripts['bench:rust:yrs:editing'],
        'cargo bench --manifest-path rust/editor-core/Cargo.toml --bench perf_suite -- --filter yrs-editing'
    );
    assert.equal(
        packageJson.scripts['bench:rust:yrs:editing:check'],
        'node scripts/check-yrs-editing-semantics-benchmarks.mjs --run --baseline rust/editor-core/benches/baselines/yrs-editing-semantics.json'
    );
});
