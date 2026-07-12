import fs from 'node:fs';
import path from 'node:path';

import { NativeEditorBoundaryError } from '../NativeEditorBoundaryError';
import { resolveDocumentDescriptor, type SchemaDefinition } from '../schemas';

const fixturePath =
    process.env.SECURITY_FIXTURE_PATH ??
    path.resolve(__dirname, '../../scripts/tests/security-contract-fixtures.json');
const fixtures = JSON.parse(fs.readFileSync(fixturePath, 'utf8'));

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
});
