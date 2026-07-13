import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
    aggregateBenchmarkSamples,
    runBenchmarkSamples,
} from '../check-yrs-document-foundation-benchmarks.mjs';

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const checker = path.join(repositoryRoot, 'scripts/check-yrs-document-foundation-benchmarks.mjs');
const requiredCases = [
    'legacy.json_import.article.1x',
    'yrs.json_import.article.1x',
    'legacy.json_export.article.1x',
    'yrs.json_export.article.1x',
    'yrs.json_import.article.2x',
    'yrs.json_export.article.2x',
    'yrs.candidate_validation.article.1x',
    'yrs.encoded_state.article.1x',
    'yrs.json_import.opaque_large.1x',
    'yrs.json_import.opaque_large.2x',
];

function benchmark(overrides = {}) {
    return {
        mode: 'standard',
        results: requiredCases.map((name) => ({
            name,
            p50Ms: overrides[name] ?? 1,
        })),
    };
}

function runChecker(input, baseline = benchmark()) {
    const directory = mkdtempSync(path.join(tmpdir(), 'yrs-benchmark-check-'));
    const inputPath = path.join(directory, 'input.json');
    const baselinePath = path.join(directory, 'baseline.json');
    writeFileSync(inputPath, JSON.stringify(input));
    writeFileSync(baselinePath, JSON.stringify(baseline));
    return spawnSync(process.execPath, [checker, inputPath, '--baseline', baselinePath], {
        cwd: repositoryRoot,
        encoding: 'utf8',
    });
}

test('executes exactly five samples and returns per-case medians', () => {
    const calls = [];
    const samples = [5, 1, 4, 2, 3].map((p50Ms) =>
        benchmark({ 'yrs.candidate_validation.article.1x': p50Ms })
    );

    const aggregated = runBenchmarkSamples((sampleNumber) => {
        calls.push(sampleNumber);
        return samples[sampleNumber - 1];
    });

    assert.deepEqual(calls, [1, 2, 3, 4, 5]);
    assert.equal(
        aggregated.results.find(({ name }) => name === 'yrs.candidate_validation.article.1x').p50Ms,
        3
    );
});

test('an individual threshold failure is not gated when the five-sample median passes', () => {
    const samples = [
        benchmark({ 'yrs.json_export.article.1x': 4 }),
        benchmark({ 'yrs.json_export.article.1x': 2 }),
        benchmark({ 'yrs.json_export.article.1x': 2.5 }),
        benchmark({ 'yrs.json_export.article.1x': 1.5 }),
        benchmark({ 'yrs.json_export.article.1x': 2 }),
    ];

    const aggregated = aggregateBenchmarkSamples(samples);
    const result = runChecker(aggregated, aggregated);

    assert.equal(result.status, 0, result.stderr);
    assert.equal(
        aggregated.results.find(({ name }) => name === 'yrs.json_export.article.1x').p50Ms,
        2
    );
});

test('aggregates every benchmark case independently', () => {
    const candidateValues = [1, 2, 3, 4, 5];
    const encodedStateValues = [1, 2, 4, 3, 5];
    const samples = candidateValues.map((candidateValue, index) =>
        benchmark({
            'yrs.candidate_validation.article.1x': candidateValue,
            'yrs.encoded_state.article.1x': encodedStateValues[index],
        })
    );

    const aggregated = aggregateBenchmarkSamples(samples);
    const values = Object.fromEntries(aggregated.results.map(({ name, p50Ms }) => [name, p50Ms]));

    assert.equal(values['yrs.candidate_validation.article.1x'], 3);
    assert.equal(values['yrs.encoded_state.article.1x'], 3);
    assert.equal(
        samples.some((sample) => {
            const sampleValues = Object.fromEntries(
                sample.results.map(({ name, p50Ms }) => [name, p50Ms])
            );
            return (
                sampleValues['yrs.candidate_validation.article.1x'] === 3 &&
                sampleValues['yrs.encoded_state.article.1x'] === 3
            );
        }),
        false
    );
});

test('requires exactly five samples for aggregation', () => {
    assert.throws(() => aggregateBenchmarkSamples(Array.from({ length: 4 }, () => benchmark())), {
        message: /exactly five benchmark samples/,
    });
    assert.throws(() => aggregateBenchmarkSamples(Array.from({ length: 6 }, () => benchmark())), {
        message: /exactly five benchmark samples/,
    });
});

test('labels malformed data in any raw sample with its one-based sample number', async (t) => {
    const invalidSamples = [
        ['malformed payload', null, /sample 4 must be a JSON object/],
        ['malformed results', {}, /sample 4 must contain a results array/],
        [
            'missing case',
            { ...benchmark(), results: benchmark().results.slice(1) },
            /sample 4 is missing required case/,
        ],
        [
            'duplicate case',
            { ...benchmark(), results: [...benchmark().results, benchmark().results[0]] },
            /sample 4 has duplicate benchmark case/,
        ],
        [
            'non-finite case',
            benchmark({ 'yrs.encoded_state.article.1x': Number.POSITIVE_INFINITY }),
            /sample 4 case .* must have a finite positive p50Ms/,
        ],
        [
            'non-positive case',
            benchmark({ 'yrs.encoded_state.article.1x': 0 }),
            /sample 4 case .* must have a finite positive p50Ms/,
        ],
    ];

    for (const [name, invalidSample, message] of invalidSamples) {
        await t.test(name, () => {
            const samples = Array.from({ length: 5 }, () => benchmark());
            samples[3] = invalidSample;
            assert.throws(() => aggregateBenchmarkSamples(samples), message);
        });
    }
});

test('sample runner failures identify the sample and stop later samples', async (t) => {
    for (const [name, failureSample, failure] of [
        ['start failure', 2, new Error('failed to start Cargo benchmark')],
        ['nonzero failure', 3, new Error('Cargo benchmark exited with status 1')],
    ]) {
        await t.test(name, () => {
            const calls = [];

            assert.throws(
                () =>
                    runBenchmarkSamples((sampleNumber) => {
                        calls.push(sampleNumber);
                        if (sampleNumber === failureSample) {
                            throw failure;
                        }
                        return benchmark();
                    }),
                new RegExp(`sample ${failureSample}: ${failure.message}`)
            );
            assert.deepEqual(
                calls,
                Array.from({ length: failureSample }, (_, index) => index + 1)
            );
        });
    }
});

test('accepts benchmark results at every exact threshold', () => {
    const input = benchmark({
        'legacy.json_import.article.1x': 0.5,
        'yrs.json_import.article.1x': 2.5,
        'legacy.json_export.article.1x': 2,
        'yrs.json_export.article.1x': 6,
        'yrs.json_import.article.2x': 6.25,
        'yrs.json_export.article.2x': 15,
        'yrs.json_import.opaque_large.1x': 4,
        'yrs.json_import.opaque_large.2x': 10,
    });
    const baseline = benchmark(
        Object.fromEntries(input.results.map(({ name, p50Ms }) => [name, p50Ms / 1.2]))
    );

    const result = runChecker(input, baseline);

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /10 benchmark cases passed/);
});

test('rejects import ratios above five even when the absolute ceiling passes', () => {
    const input = benchmark({
        'legacy.json_import.article.1x': 0.4,
        'yrs.json_import.article.1x': 2.1,
    });

    const result = runChecker(input, input);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /actual ratio=5\.25/);
    assert.match(result.stderr, /allowed ratio=5/);
});

test('rejects import p50 above 2.5 ms even when the relative ratio passes', () => {
    const input = benchmark({
        'legacy.json_import.article.1x': 1,
        'yrs.json_import.article.1x': 2.500001,
    });

    const result = runChecker(input, input);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /yrs\.json_import\.article\.1x/);
    assert.match(result.stderr, /measured p50=2\.500001 ms/);
    assert.match(result.stderr, /allowed p50=2\.5 ms/);
});

test('ratio failures report the case, p50 values, actual ratio, and allowed ratio', () => {
    const input = benchmark({
        'legacy.json_import.article.1x': 0.4,
        'yrs.json_import.article.1x': 2.04,
    });

    const result = runChecker(input, input);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /yrs\.json_import\.article\.1x/);
    assert.match(result.stderr, /measured p50=2\.04/);
    assert.match(result.stderr, /comparison p50=0\.4/);
    assert.match(result.stderr, /actual ratio=5\.1/);
    assert.match(result.stderr, /allowed ratio=5/);
});

test('baseline regression failures use the same actionable diagnostics', () => {
    const input = benchmark({ 'yrs.encoded_state.article.1x': 1.21 });
    const baseline = benchmark();

    const result = runChecker(input, baseline);

    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /yrs\.encoded_state\.article\.1x/);
    assert.match(result.stderr, /measured p50=1\.21/);
    assert.match(result.stderr, /comparison p50=1/);
    assert.match(result.stderr, /actual ratio=1\.21/);
    assert.match(result.stderr, /allowed ratio=1\.2/);
});

test('rejects missing, duplicate, non-finite, and non-positive cases cleanly', async (t) => {
    const invalidInputs = [
        [
            'missing',
            { ...benchmark(), results: benchmark().results.slice(1) },
            /missing required case/,
        ],
        [
            'duplicate',
            { ...benchmark(), results: [...benchmark().results, benchmark().results[0]] },
            /duplicate benchmark case/,
        ],
        [
            'non-finite',
            benchmark({ 'yrs.encoded_state.article.1x': 'NaN' }),
            /finite positive p50Ms/,
        ],
        ['non-positive', benchmark({ 'yrs.encoded_state.article.1x': 0 }), /finite positive p50Ms/],
    ];

    for (const [name, input, message] of invalidInputs) {
        await t.test(name, () => {
            const result = runChecker(input);
            assert.notEqual(result.status, 0);
            assert.match(result.stderr, message);
            assert.doesNotMatch(result.stderr, /at .*\.mjs:\d+/);
        });
    }
});

test('rejects malformed benchmark and baseline JSON cleanly', async (t) => {
    for (const target of ['input', 'baseline']) {
        await t.test(target, () => {
            const directory = mkdtempSync(path.join(tmpdir(), 'yrs-benchmark-check-'));
            const inputPath = path.join(directory, 'input.json');
            const baselinePath = path.join(directory, 'baseline.json');
            writeFileSync(inputPath, target === 'input' ? '{' : JSON.stringify(benchmark()));
            writeFileSync(baselinePath, target === 'baseline' ? '{' : JSON.stringify(benchmark()));

            const result = spawnSync(
                process.execPath,
                [checker, inputPath, '--baseline', baselinePath],
                { cwd: repositoryRoot, encoding: 'utf8' }
            );

            assert.notEqual(result.status, 0);
            assert.match(result.stderr, new RegExp(`failed to parse ${target}`));
            assert.doesNotMatch(result.stderr, /at .*\.mjs:\d+/);
        });
    }
});

test('package scripts expose the Yrs benchmark entry points', () => {
    const packageJson = JSON.parse(
        execFileSync(
            process.execPath,
            ['-e', "process.stdout.write(require('fs').readFileSync('package.json'))"],
            {
                cwd: repositoryRoot,
                encoding: 'utf8',
            }
        )
    );

    assert.equal(
        packageJson.scripts['bench:rust:yrs'],
        'cargo bench --manifest-path rust/editor-core/Cargo.toml --bench perf_suite -- --filter yrs-foundation'
    );
    assert.equal(
        packageJson.scripts['bench:rust:yrs:quick'],
        'cargo bench --manifest-path rust/editor-core/Cargo.toml --bench perf_suite -- --quick --filter yrs-foundation'
    );
    assert.equal(
        packageJson.scripts['bench:rust:yrs:check'],
        'node scripts/check-yrs-document-foundation-benchmarks.mjs --run --baseline rust/editor-core/benches/baselines/yrs-document-foundation.json'
    );
});
