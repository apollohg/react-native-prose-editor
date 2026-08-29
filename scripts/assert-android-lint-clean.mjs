import { readFileSync } from 'node:fs';

const reportPath = process.argv[2];
if (!reportPath) {
    console.error('Usage: node scripts/assert-android-lint-clean.mjs <report.xml>');
    process.exit(2);
}

function decodeXml(value) {
    return value
        .replaceAll('&quot;', '"')
        .replaceAll('&apos;', "'")
        .replaceAll('&lt;', '<')
        .replaceAll('&gt;', '>')
        .replaceAll('&amp;', '&');
}

function attributes(source) {
    return Object.fromEntries(
        Array.from(source.matchAll(/([\w:-]+)\s*=\s*"([^"]*)"/g), ([, name, value]) => [
            name,
            decodeXml(value),
        ]),
    );
}

const xml = readFileSync(reportPath, 'utf8');
if (!/<issues\b/.test(xml)) {
    console.error(`Invalid Android lint report: ${reportPath}`);
    process.exit(2);
}

const issues = Array.from(
    xml.matchAll(/<issue\b([^>]*?)(?:\/>|>([\s\S]*?)<\/issue>)/g),
    ([, issueSource, body = '']) => {
        const issue = attributes(issueSource);
        const locationMatch = body.match(/<location\b([^>]*?)\/?\s*>/);
        const location = locationMatch ? attributes(locationMatch[1]) : {};
        const suffix = location.file
            ? ` (${location.file}${location.line ? `:${location.line}` : ''}${location.column ? `:${location.column}` : ''})`
            : '';
        return {
            severity: issue.severity ?? 'Unknown',
            line: `${issue.severity ?? 'Unknown'} ${issue.id ?? 'Unknown'}: ${issue.message ?? ''}${suffix}`,
        };
    },
);

let hasErrors = false;
for (const issue of issues) {
    if (issue.severity === 'Error') {
        hasErrors = true;
        console.error(issue.line);
    } else {
        console.log(issue.line);
    }
}

process.exitCode = hasErrors ? 1 : 0;
