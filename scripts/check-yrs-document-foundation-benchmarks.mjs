#!/usr/bin/env node

import { readFileSync } from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const REQUIRED_CASES = [
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

const RATIO_GATES = [
    ['yrs.json_import.article.1x', 'legacy.json_import.article.1x', 5.0],
    ['yrs.json_export.article.1x', 'legacy.json_export.article.1x', 3.0],
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

function runBenchmarks() {
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
    const result = spawnSync('cargo', cargoArguments, {
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

function indexResults(payload, label) {
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
    return indexed;
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
        const baselineResults = indexResults(baseline, 'baseline');
        for (const name of REQUIRED_CASES) {
            assertBaselineRatio(benchmark, baselineResults, name, 1.2);
        }
    }
    return benchmark.size;
}

function main() {
    try {
        const options = parseArguments(process.argv.slice(2));
        const input = options.run ? runBenchmarks() : readJsonFile(options.inputPath, 'input');
        const baseline = options.baselinePath
            ? readJsonFile(options.baselinePath, 'baseline')
            : undefined;
        const caseCount = checkBenchmarks(input, baseline);
        console.log(`${caseCount} benchmark cases passed all Yrs foundation gates`);
    } catch (error) {
        console.error(`Yrs foundation benchmark check failed: ${error.message}`);
        process.exitCode = 1;
    }
}

main();
