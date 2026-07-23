import fs from 'node:fs';
import path from 'node:path';

import {
    NATIVE_EDITOR_ERROR_DOMAINS,
    NATIVE_EDITOR_OPERATION_ERROR_CODES,
    NativeEditorBoundaryError,
    normalizeNativeEditorV2Error,
    parseNativeBoundaryError,
} from '../NativeEditorBoundaryError';
import { resolveDocumentDescriptor, type SchemaDefinition } from '../schemas';

const fixturePath =
    process.env.SECURITY_FIXTURE_PATH ??
    path.resolve(__dirname, '../../scripts/tests/security-contract-fixtures.json');
const fixtures = JSON.parse(fs.readFileSync(fixturePath, 'utf8'));

const V2_U64_ERROR_FIELDS = ['operationIndex', 'limit', 'actual'] as const;

/**
 * Shared security fixtures use JSON numbers for logical counters. The frozen
 * 1.0 FFI v2 wire contract carries every u64 as a canonical decimal string.
 */
function toV2WireError(error: Record<string, unknown>): Record<string, unknown> {
    return Object.fromEntries(
        Object.entries(error).map(([field, value]) => [
            field,
            (V2_U64_ERROR_FIELDS as readonly string[]).includes(field) && typeof value === 'number'
                ? String(value)
                : value,
        ])
    );
}

describe('shared hostile security fixtures', () => {
    it('executes oversized schema admission against the TypeScript boundary', () => {
        const nodeCount = fixtures.oversizedSchema.nodeCount as number;
        const schema: SchemaDefinition = {
            nodes: Array.from({ length: nodeCount }, (_, index) => ({
                name: `n${index}`,
                content: '',
                role: 'block',
            })),
            marks: [],
        };

        try {
            resolveDocumentDescriptor(schema);
            throw new Error('oversized fixture was accepted');
        } catch (error) {
            expect(error).toBeInstanceOf(NativeEditorBoundaryError);
            expect((error as NativeEditorBoundaryError).code).toBe(
                fixtures.oversizedSchema.expectedErrorCode
            );
        }
    });

    it('executes the custom article root through descriptor construction', () => {
        const fixture = fixtures.customArticleRoot;
        const descriptor = resolveDocumentDescriptor(fixture.schema as SchemaDefinition);

        expect(descriptor.documentNodeName).toBe(fixture.expectedRoot);
        expect(descriptor.emptyDocument.type).toBe(fixture.expectedRoot);
    });

    it('normalizes the shared schema fixture exactly as the Rust boundary does', () => {
        const fixture = fixtures.schemaNormalizationParity;
        const descriptor = resolveDocumentDescriptor(fixture.missingFields as SchemaDefinition);

        expect(descriptor.schema.nodes).toEqual([
            {
                name: 'article',
                content: 'paragraph',
                role: 'doc',
                isVoid: false,
            },
            {
                name: 'paragraph',
                content: '',
                group: 'block',
                role: 'textBlock',
                htmlTag: 'p',
                isVoid: false,
            },
            { name: 'text', content: '', role: 'text', isVoid: false },
        ]);
        expect(descriptor.schema.marks).toEqual([{ name: 'highlight', htmlTag: 'mark' }]);
    });

    it.each(['invalidNodeTag', 'invalidAttribute'])(
        'falls back for the shared %s schema fixture',
        (fixtureName) => {
            const fixture = fixtures.schemaNormalizationParity;
            const schema = structuredClone(fixture.missingFields) as SchemaDefinition;
            if (fixtureName === 'invalidNodeTag') {
                schema.nodes[1].htmlTag = fixture.invalidNodeTag;
            } else {
                schema.marks[0].attrs = { [fixture.invalidAttribute]: {} };
            }
            expect(resolveDocumentDescriptor(schema).schema.nodes[0].name).toBe('doc');
        }
    );
});

describe('FFI v2 error contract', () => {
    const contract = fixtures.ffiV2ErrorContract;

    it('freezes all domains, operation codes, and representative domain codes', () => {
        expect(NATIVE_EDITOR_ERROR_DOMAINS).toEqual(contract.domains);
        expect(NATIVE_EDITOR_OPERATION_ERROR_CODES).toEqual(contract.operationCodes);
        expect(contract.representativeCodes).toEqual({
            lifecycle: [
                'ENGINE_DESTROYING',
                'ENGINE_DESTROYED',
                'WHOLE_DOCUMENT_REPLACEMENT_CONNECTED',
            ],
            snapshot: ['SNAPSHOT_RESTORE_CONNECTED'],
            transport: ['TRANSPORT_PROTOCOL_INVALID'],
        });
    });

    it.each(contract.goldenErrors)(
        'normalizes $domain/$code with explicit nullable fields',
        (error: Record<string, unknown>) => {
            const wireError = toV2WireError(error);
            expect(normalizeNativeEditorV2Error({ ok: false, error: wireError })).toEqual(wireError);
        }
    );

    it.each(contract.operationCodes)(
        'normalizes approved operation code %s with its frozen domain and nullability',
        (code: string) => {
            const error = contract.goldenErrors.find(
                (candidate: Record<string, unknown>) => candidate.code === code
            );
            expect(error).toBeDefined();
            expect(error.domain).toBe(contract.operationCodeDomains[code]);
            expect(error.requestId).toMatch(/^(0|[1-9]\d*)$/);
            const wireError = toV2WireError(error);
            expect(normalizeNativeEditorV2Error({ ok: false, error: wireError })).toEqual(wireError);
        }
    );

    it('rejects legacy numeric error counters from the shared fixture', () => {
        const legacyError = contract.goldenErrors.find(
            (error: Record<string, unknown>) => typeof error.operationIndex === 'number'
        );
        expect(legacyError).toBeDefined();
        expect(normalizeNativeEditorV2Error({ ok: false, error: legacyError })).toBeNull();
    });

    it.each(contract.invalidRequestIds)(
        'rejects non-canonical decimal request ID %p',
        (requestId: string) => {
            const error = structuredClone(contract.goldenErrors[0]);
            error.requestId = requestId;
            expect(normalizeNativeEditorV2Error({ ok: false, error })).toBeNull();
        }
    );

    it('normalizes raw UniFFI detailsJson and missing optionals without changing legacy parsing', () => {
        expect(
            normalizeNativeEditorV2Error({
                error: {
                    domain: 'document',
                    code: 'DOCUMENT_INVALID',
                    message: 'invalid document',
                    detailsJson: '{"field":"content"}',
                },
            })
        ).toEqual({
            domain: 'document',
            code: 'DOCUMENT_INVALID',
            message: 'invalid document',
            requestId: null,
            operationIndex: null,
            limit: null,
            actual: null,
            details: { field: 'content' },
        });

        const legacy = parseNativeBoundaryError({
            error: { code: 'CONFIG_INVALID', message: 'invalid config' },
        });
        expect(legacy).toBeInstanceOf(NativeEditorBoundaryError);
        expect(legacy).toMatchObject({
            code: 'CONFIG_INVALID',
            message: 'invalid config',
            limit: undefined,
            actual: undefined,
            details: undefined,
        });
    });

    it('freezes deterministic limit remapping and allocation failure preservation', () => {
        expect(contract.deterministicMappings).toEqual([
            {
                cause: 'traversalWork',
                sourceCode: 'OPERATION_RESOURCE_EXHAUSTED',
                expectedDomain: 'operation',
                expectedCode: 'OPERATION_LIMIT_EXCEEDED',
            },
            {
                cause: 'documentDepth',
                sourceCode: 'OPERATION_RESOURCE_EXHAUSTED',
                expectedDomain: 'document',
                expectedCode: 'DOCUMENT_LIMIT_EXCEEDED',
            },
            {
                cause: 'tryReserve',
                sourceCode: 'OPERATION_RESOURCE_EXHAUSTED',
                expectedDomain: 'operation',
                expectedCode: 'OPERATION_RESOURCE_EXHAUSTED',
            },
        ]);
    });
});
