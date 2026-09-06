import type { DocumentJSON } from './NativeEditorBridge';
import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';
import type { SchemaDefinition } from './schemaDefinition';
import { schemaBoundaryError, utf8ByteLengthUpTo } from './schemaNormalization';
import { constructDefaultEmptyDocument } from './schemaDocumentConstruction';
import {
    resolveDescriptorLimits,
    createSchemaWorkBudget,
    admitSchemaCollections,
    resolveDocumentSchema,
} from './schemaResolution';
import {
    defaultSchema,
    type DocumentDescriptorLimits,
    type ResolvedDocumentSchema,
} from './schemaPresets';

export function defaultEmptyDocument(
    schema: SchemaDefinition = defaultSchema,
    limits?: DocumentDescriptorLimits
): DocumentJSON {
    const resolvedLimits = resolveDescriptorLimits(limits);
    if (schema.nodes.length > resolvedLimits.maxSchemaNodes) {
        throw schemaBoundaryError(resolvedLimits.maxSchemaNodes, schema.nodes.length);
    }
    let expressionBytes = 0;
    for (const node of schema.nodes) {
        if (node != null && typeof node.content === 'string') {
            expressionBytes += utf8ByteLengthUpTo(
                node.content,
                resolvedLimits.maxSchemaExpressionBytes - expressionBytes
            );
            if (expressionBytes > resolvedLimits.maxSchemaExpressionBytes) {
                throw schemaBoundaryError(resolvedLimits.maxSchemaExpressionBytes, expressionBytes);
            }
        }
    }
    const admissionBudget = createSchemaWorkBudget(resolvedLimits);
    const admittedCollections = admitSchemaCollections(schema, admissionBudget);
    if (admittedCollections == null) {
        throw new Error('schema cannot construct a default document: invalid schema collections');
    }
    return constructDefaultEmptyDocument(schema, resolvedLimits, admittedCollections);
}

/**
 * Validate a schema and derive its document node name and empty document,
 * mirroring what the Rust core does when a handle is created. Use it to build
 * fragments or empty documents for a custom schema.
 *
 * @param schema Defaults to {@link defaultSchema}.
 * @param limits Schema and document bounds. Defaults to
 * {@link DEFAULT_EDITOR_RESOURCE_LIMITS}.
 * @throws NativeEditorBoundaryError `SCHEMA_INVALID` when the schema is
 * malformed, exceeds its limits, or declares no `role: 'doc'` node.
 */
export function resolveDocumentDescriptor(
    schema?: SchemaDefinition,
    limits?: DocumentDescriptorLimits
): ResolvedDocumentSchema {
    const resolvedLimits = resolveDescriptorLimits(limits);
    const resolvedSchema = resolveDocumentSchema(schema, resolvedLimits);
    const documentNode = resolvedSchema.nodes.find((node) => node.role === 'doc');
    if (!documentNode) {
        throw new NativeEditorBoundaryError('SCHEMA_INVALID', 'schema has no document-role node');
    }
    return {
        schema: resolvedSchema,
        documentNodeName: documentNode.name,
        emptyDocument: defaultEmptyDocument(resolvedSchema, resolvedLimits),
    };
}

export function normalizeDocumentJson(
    doc: DocumentJSON,
    schemaOrDescriptor: SchemaDefinition | ResolvedDocumentSchema = defaultSchema,
    limits?: DocumentDescriptorLimits
): DocumentJSON {
    const descriptor =
        'documentNodeName' in schemaOrDescriptor
            ? schemaOrDescriptor
            : resolveDocumentDescriptor(schemaOrDescriptor, limits);
    const root = doc as { type?: unknown; content?: unknown } | null;
    if (root?.type !== descriptor.documentNodeName) {
        return doc;
    }
    const content = root?.content;
    if (Array.isArray(content) && content.length > 0) {
        return doc;
    }
    return descriptor.emptyDocument;
}
