#!/usr/bin/env node

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

// Task 16C (user directive 2026-07-20): the legacy runtime was deleted, so
// the legacy.* reference cases and the legacy-relative ratio gates (5.0x
// import, 3.0x export) are removed with it. The absolute import ceiling
// (2.5 ms), the 2x scaling gates, and the 1.20x baseline-regression gate
// are UNCHANGED. Frozen baseline files may still carry legacy.* entries;
// they are ignored here (see indexResults).
const REQUIRED_CASES = [
    'yrs.json_import.article.1x',
    'yrs.json_export.article.1x',
    'yrs.json_import.article.2x',
    'yrs.json_export.article.2x',
    'yrs.candidate_validation.article.1x',
    'yrs.encoded_state.article.1x',
    'yrs.json_import.opaque_large.1x',
    'yrs.json_import.opaque_large.2x',
];

const RATIO_GATES = [
    ['yrs.json_import.article.2x', 'yrs.json_import.article.1x', 2.5],
    ['yrs.json_export.article.2x', 'yrs.json_export.article.1x', 2.5],
    ['yrs.json_import.opaque_large.2x', 'yrs.json_import.opaque_large.1x', 2.5],
];

const ABSOLUTE_GATES = [['yrs.json_import.article.1x', 2.5]];

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function parseArguments(args) {
    let run = false;
    let inputPath;
    let baselinePath;

    for (let index = 0; index < args.length; index += 1) {
        const argument = args[index];
        if (argument === '--run') {
            run = true;
        } else if (argument === '--baseline') {
            baselinePath = args[index + 1];
            index += 1;
            if (!baselinePath) {
                throw new Error('--baseline requires a file path');
            }
        } else if (argument.startsWith('--')) {
            throw new Error(`unknown option: ${argument}`);
        } else if (inputPath) {
            throw new Error('accepts exactly one benchmark JSON file');
        } else {
            inputPath = argument;
        }
    }

    if (run === Boolean(inputPath)) {
        throw new Error('provide either one benchmark JSON file or --run');
    }
    return { run, inputPath, baselinePath };
}

function parseJson(text, label) {
    let payload;
    try {
        payload = JSON.parse(text);
    } catch (error) {
        throw new Error(`failed to parse ${label} JSON: ${error.message}`);
    }
    if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
        throw new Error(`${label} must be a JSON object`);
    }
    return payload;
}

function readJsonFile(filePath, label) {
    let text;
    try {
        text = readFileSync(filePath, 'utf8');
    } catch (error) {
        throw new Error(`failed to read ${label} file ${filePath}: ${error.message}`);
    }
    return parseJson(text, label);
}

function runBenchmarkSample() {
    const cargoArguments = [
        'bench',
        '--manifest-path',
        'rust/editor-core/Cargo.toml',
        '--bench',
        'perf_suite',
        '--',
        '--json',
        '--filter',
        'yrs-foundation',
    ];
    const result = spawnSync(path.join(repositoryRoot, 'rust', 'toolchain-cargo.sh'), cargoArguments, {
        cwd: repositoryRoot,
        encoding: 'utf8',
        maxBuffer: 100 * 1024 * 1024,
    });
    if (result.error) {
        throw new Error(`failed to run Cargo benchmark: ${result.error.message}`);
    }
    if (result.status !== 0) {
        const output = [result.stderr, result.stdout].filter(Boolean).join('\n').trim();
        throw new Error(
            `Cargo benchmark exited with status ${result.status}${output ? `:\n${output}` : ''}`
        );
    }
    return parseJson(result.stdout.trim(), 'input');
}

export function runBenchmarkSamples(sampleRunner = runBenchmarkSample, writeEvidence = () => {}) {
    const samples = [];
    for (let sampleNumber = 1; sampleNumber <= 5; sampleNumber += 1) {
        let sample;
        try {
            sample = sampleRunner(sampleNumber);
        } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            throw new Error(`sample ${sampleNumber}: ${message}`);
        }
        const label = `sample ${sampleNumber}`;
        if (!sample || typeof sample !== 'object' || Array.isArray(sample)) {
            throw new Error(`${label} must be a JSON object`);
        }
        indexResults(sample, label);
        samples.push(sample);
        writeEvidence({
            evidenceType: 'yrs-foundation-raw-sample',
            sampleNumber,
            rawSample: sample,
        });
    }
    return {
        samples,
        aggregate: aggregateBenchmarkSamples(samples),
    };
}

function indexResults(payload, label, { allowExtraCases = false } = {}) {
    if (!Array.isArray(payload.results)) {
        throw new Error(`${label} must contain a results array`);
    }

    const indexed = new Map();
    for (const [index, result] of payload.results.entries()) {
        if (!result || typeof result !== 'object' || Array.isArray(result)) {
            throw new Error(`${label} result at index ${index} must be an object`);
        }
        if (typeof result.name !== 'string' || result.name.length === 0) {
            throw new Error(`${label} result at index ${index} must have a non-empty name`);
        }
        if (!REQUIRED_CASES.includes(result.name)) {
            // Frozen baselines still carry the deleted legacy.* reference
            // cases; only the required cases are read from a baseline.
            if (allowExtraCases) continue;
            throw new Error(`${label} has unexpected benchmark case: ${result.name}`);
        }
        if (indexed.has(result.name)) {
            throw new Error(`${label} has duplicate benchmark case: ${result.name}`);
        }
        if (
            typeof result.p50Ms !== 'number' ||
            !Number.isFinite(result.p50Ms) ||
            result.p50Ms <= 0
        ) {
            throw new Error(`${label} case ${result.name} must have a finite positive p50Ms`);
        }
        indexed.set(result.name, result.p50Ms);
    }

    for (const name of REQUIRED_CASES) {
        if (!indexed.has(name)) {
            throw new Error(`${label} is missing required case: ${name}`);
        }
    }
    if (!allowExtraCases && indexed.size !== REQUIRED_CASES.length) {
        throw new Error(`${label} must contain exactly ${REQUIRED_CASES.length} benchmark cases`);
    }
    return indexed;
}

export function aggregateBenchmarkSamples(samples) {
    if (!Array.isArray(samples) || samples.length !== 5) {
        throw new Error('expected exactly five benchmark samples');
    }

    const indexedSamples = samples.map((sample, index) => {
        const label = `sample ${index + 1}`;
        if (!sample || typeof sample !== 'object' || Array.isArray(sample)) {
            throw new Error(`${label} must be a JSON object`);
        }
        return indexResults(sample, label);
    });

    return {
        mode: 'standard',
        results: REQUIRED_CASES.map((name) => {
            const sortedValues = indexedSamples
                .map((sample) => sample.get(name))
                .sort((left, right) => left - right);
            return { name, p50Ms: sortedValues[2] };
        }),
    };
}

function formatNumber(value) {
    return Number(value.toPrecision(12)).toString();
}

function assertRatio(benchmark, caseName, comparisonName, allowedRatio) {
    const measured = benchmark.get(caseName);
    const comparison = benchmark.get(comparisonName);
    const actualRatio = measured / comparison;
    if (measured > comparison * allowedRatio) {
        throw new Error(
            `${caseName}: measured p50=${formatNumber(measured)} ms, ` +
                `comparison p50=${formatNumber(comparison)} ms (${comparisonName}), ` +
                `actual ratio=${formatNumber(actualRatio)}, allowed ratio=${formatNumber(allowedRatio)}`
        );
    }
}

function assertAbsoluteCeiling(benchmark, caseName, allowedMs) {
    const measured = benchmark.get(caseName);
    if (measured > allowedMs) {
        throw new Error(
            `${caseName}: measured p50=${formatNumber(measured)} ms, ` +
                `allowed p50=${formatNumber(allowedMs)} ms`
        );
    }
}

function assertBaselineRatio(benchmark, baseline, caseName, allowedRatio) {
    const measured = benchmark.get(caseName);
    const comparison = baseline.get(caseName);
    const actualRatio = measured / comparison;
    if (measured > comparison * allowedRatio) {
        throw new Error(
            `${caseName}: measured p50=${formatNumber(measured)} ms, ` +
                `comparison p50=${formatNumber(comparison)} ms (baseline), ` +
                `actual ratio=${formatNumber(actualRatio)}, allowed ratio=${formatNumber(allowedRatio)}`
        );
    }
}

function checkBenchmarks(input, baseline) {
    const benchmark = indexResults(input, 'input');
    for (const [caseName, comparisonName, allowedRatio] of RATIO_GATES) {
        assertRatio(benchmark, caseName, comparisonName, allowedRatio);
    }
    for (const [caseName, allowedMs] of ABSOLUTE_GATES) {
        assertAbsoluteCeiling(benchmark, caseName, allowedMs);
    }

    if (baseline) {
        const baselineResults = indexResults(baseline, 'baseline', { allowExtraCases: true });
        for (const name of REQUIRED_CASES) {
            assertBaselineRatio(benchmark, baselineResults, name, 1.2);
        }
    }
    return benchmark.size;
}

export function checkBenchmarkRun(
    benchmarkRun,
    baseline,
    writeEvidence,
    checker = checkBenchmarks
) {
    writeEvidence({
        evidenceType: 'yrs-foundation-five-run-median',
        sampleCount: benchmarkRun.samples.length,
        rawSamples: benchmarkRun.samples,
        medianAggregate: benchmarkRun.aggregate,
    });
    return checker(benchmarkRun.aggregate, baseline);
}

function main() {
    try {
        const options = parseArguments(process.argv.slice(2));
        const benchmarkRun = options.run
            ? runBenchmarkSamples(runBenchmarkSample, (evidence) =>
                  console.log(JSON.stringify(evidence))
              )
            : undefined;
        const input = benchmarkRun
            ? benchmarkRun.aggregate
            : readJsonFile(options.inputPath, 'input');
        const baseline = options.baselinePath
            ? readJsonFile(options.baselinePath, 'baseline')
            : undefined;
        const caseCount = benchmarkRun
            ? checkBenchmarkRun(benchmarkRun, baseline, (evidence) =>
                  console.log(JSON.stringify(evidence))
              )
            : checkBenchmarks(input, baseline);
        console.log(
            options.run
                ? `${caseCount} benchmark cases from five standard samples aggregated by per-case median passed all Yrs foundation gates`
                : `${caseCount} benchmark cases passed all Yrs foundation gates`
        );
    } catch (error) {
        console.error(`Yrs foundation benchmark check failed: ${error.message}`);
        process.exitCode = 1;
    }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    main();
}
