import fixtures from '../../rust/editor-core/tests/fixtures/schema-fingerprints.json';
import type { SchemaDefinition } from '../schemas';
import { testSchemaFingerprint } from './helpers/schemaFingerprint';

describe('resolved schema fingerprint parity', () => {
    it.each(fixtures.fingerprints)('$name matches the checked-in Rust fingerprint', (fixture) => {
        expect(testSchemaFingerprint(fixture.schema as SchemaDefinition)).toBe(
            fixture.expectedFingerprint
        );
    });

    it.each(fixtures.equivalentSchemas)('$name ignores object key insertion order', (fixture) => {
        for (const schema of fixture.schemas) {
            expect(testSchemaFingerprint(schema as SchemaDefinition)).toBe(
                fixture.expectedFingerprint
            );
        }
    });

    it('keeps the helper test-only', () => {
        expect(require.resolve('./helpers/schemaFingerprint')).toContain('/src/__tests__/helpers/');
    });
});
