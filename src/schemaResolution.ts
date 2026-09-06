import { NativeEditorBoundaryError } from './NativeEditorBoundaryError';
import { resolveEditorResourceLimits, type ResolvedEditorResourceLimits } from './ResourceLimits';
import { contentExpressionSymbols, minimalContentMatch } from './contentExpression';
import { type DocumentDescriptorLimits, defaultSchema } from './schemaPresets';
import {
    type SchemaWorkBudget,
    type AdmittedSchemaCollections,
    forEachGroupToken,
    consumeSchemaWork,
    collectOwnAttrs,
    consumeSchemaStringWork,
    utf8ByteLengthUpTo,
    normalizeSchemaDefinition,
} from './schemaNormalization';
import { type SchemaDefinition, type NodeSpec, type AttrSpec } from './schemaDefinition';
import { constructDefaultEmptyDocument } from './schemaDocument';

export function schemaBoundaryError(limit: number, actual: number): NativeEditorBoundaryError {
    return new NativeEditorBoundaryError(
        'SCHEMA_INVALID',
        `schema work exceeds configured limit ${limit}`,
        limit,
        actual
    );
}

export function resolveDescriptorLimits(
    limits?: DocumentDescriptorLimits
): ResolvedEditorResourceLimits {
    return resolveEditorResourceLimits(limits);
}

export function createSchemaWorkBudget(limits: ResolvedEditorResourceLimits): SchemaWorkBudget {
    return {
        limit: limits.maxSchemaNodes * 64 + limits.maxSchemaExpressionBytes * 32,
        work: 0,
        exhausted: false,
    };
}

export function admitSchemaCollections(
    schema: SchemaDefinition,
    budget: SchemaWorkBudget
): AdmittedSchemaCollections | null {
    const groupsByNode = new Map<NodeSpec, readonly string[]>();
    const attrsByNode = new Map<NodeSpec, ReadonlyArray<[string, AttrSpec]>>();

    for (let nodeIndex = 0; nodeIndex < schema.nodes.length; nodeIndex += 1) {
        const node = schema.nodes[nodeIndex];
        if (node == null || typeof node !== 'object') return null;
        const groups: string[] = [];
        if (typeof node.group === 'string') {
            if (
                !forEachGroupToken(
                    node.group,
                    (group) => groups.push(group),
                    () => consumeSchemaWork(budget),
                    () => consumeSchemaWork(budget)
                )
            ) {
                throw schemaBoundaryError(budget.limit, budget.limit + 1);
            }
        }
        const attrs = collectOwnAttrs(node.attrs, budget);
        if (attrs == null) return null;
        const projectionAttrs = collectOwnAttrs(node.json?.attrs, budget);
        if (projectionAttrs == null) return null;
        if (
            node.json != null &&
            ((typeof node.json.type === 'string' &&
                !consumeSchemaStringWork(budget, node.json.type)) ||
                projectionAttrs.some(([name, rawValue]) => {
                    const value: unknown = rawValue;
                    return (
                        !consumeSchemaStringWork(budget, name) ||
                        (typeof value === 'string'
                            ? !consumeSchemaStringWork(budget, value)
                            : !consumeSchemaWork(budget))
                    );
                }))
        ) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        if (budget.exhausted) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        groupsByNode.set(node, groups);
        attrsByNode.set(node, attrs);
    }

    for (let markIndex = 0; markIndex < schema.marks.length; markIndex += 1) {
        const mark = schema.marks[markIndex];
        if (!consumeSchemaWork(budget)) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
        if (mark == null || typeof mark !== 'object') return null;
        const attrs = collectOwnAttrs(mark.attrs, budget);
        if (attrs == null) return null;
        if (budget.exhausted) {
            throw schemaBoundaryError(budget.limit, budget.limit + 1);
        }
    }

    return { groupsByNode, attrsByNode };
}

/** Mirror native's invalid-schema fallback after bounded admission succeeds. */
export function resolveDocumentSchema(
    schema?: SchemaDefinition,
    limits?: DocumentDescriptorLimits
): SchemaDefinition {
    if (schema == null) return defaultSchema;
    if (!Array.isArray(schema.nodes)) return defaultSchema;
    if (!Array.isArray(schema.marks)) {
        schema = { ...schema, marks: [] };
    }

    const resolvedLimits = resolveDescriptorLimits(limits);
    if (schema.nodes.length > resolvedLimits.maxSchemaNodes) {
        throw schemaBoundaryError(resolvedLimits.maxSchemaNodes, schema.nodes.length);
    }
    let expressionBytes = 0;
    for (let nodeIndex = 0; nodeIndex < schema.nodes.length; nodeIndex += 1) {
        const node = schema.nodes[nodeIndex];
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
    const schemaBudget = createSchemaWorkBudget(resolvedLimits);
    if (admitSchemaCollections(schema, schemaBudget) == null) return defaultSchema;
    const normalizedSchema = normalizeSchemaDefinition(schema, schemaBudget);
    if (normalizedSchema == null) return defaultSchema;
    schema = normalizedSchema;
    const admittedCollections = admitSchemaCollections(schema, {
        limit: Number.MAX_SAFE_INTEGER,
        work: 0,
        exhausted: false,
    });
    if (admittedCollections == null) return defaultSchema;

    const nodeNames = new Set<string>();
    const markNames = new Set<string>();
    let docRoles = 0;
    let textRoles = 0;
    for (const node of schema.nodes) {
        if (
            node == null ||
            typeof node.name !== 'string' ||
            node.name.length === 0 ||
            typeof node.content !== 'string' ||
            typeof node.role !== 'string' ||
            nodeNames.has(node.name)
        ) {
            return defaultSchema;
        }
        nodeNames.add(node.name);
        if (node.role === 'doc') docRoles += 1;
        if (node.role === 'text') textRoles += 1;
    }
    for (const mark of schema.marks) {
        if (
            mark == null ||
            typeof mark.name !== 'string' ||
            mark.name.length === 0 ||
            markNames.has(mark.name)
        ) {
            return defaultSchema;
        }
        markNames.add(mark.name);
    }
    if (docRoles !== 1 || textRoles !== 1) return defaultSchema;

    const groups = new Set<string>();
    const nodesBySymbol = new Map<string, NodeSpec[]>();
    const addCandidate = (symbol: string, node: NodeSpec): void => {
        const candidates = nodesBySymbol.get(symbol);
        if (candidates) candidates.push(node);
        else nodesBySymbol.set(symbol, [node]);
    };
    for (const node of schema.nodes) {
        addCandidate(node.name, node);
        for (const group of admittedCollections.groupsByNode.get(node) ?? []) {
            groups.add(group);
            addCandidate(group, node);
        }
    }
    for (const node of schema.nodes) {
        const symbols = contentExpressionSymbols(node.content);
        if (
            symbols == null ||
            symbols.some((symbol) => !nodeNames.has(symbol) && !groups.has(symbol))
        ) {
            return defaultSchema;
        }
    }

    const generatable = new Set<string>();
    const consumeWork = (): boolean => consumeSchemaWork(schemaBudget);
    const contentIsConstructible = (node: NodeSpec): boolean => {
        if (!consumeWork()) return false;
        return (
            minimalContentMatch(
                node.content,
                (symbol) => {
                    const candidates = nodesBySymbol.get(symbol) ?? [];
                    for (const candidate of candidates) {
                        if (!consumeWork()) return undefined;
                        if (generatable.has(candidate.name)) return { type: candidate.name };
                    }
                    return undefined;
                },
                consumeWork
            ) != null
        );
    };
    while (true) {
        const before = generatable.size;
        for (const node of schema.nodes) {
            const hasRequiredAttrs = (admittedCollections.attrsByNode.get(node) ?? []).some(
                ([, attr]) => attr.default === undefined
            );
            if (node.role !== 'text' && !hasRequiredAttrs && contentIsConstructible(node)) {
                generatable.add(node.name);
            }
        }
        if (generatable.size === before) break;
    }
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    const hasUnconstructibleNode = schema.nodes.some((node) => !contentIsConstructible(node));
    if (schemaBudget.exhausted) {
        throw schemaBoundaryError(schemaBudget.limit, schemaBudget.limit + 1);
    }
    if (hasUnconstructibleNode) return defaultSchema;

    try {
        constructDefaultEmptyDocument(schema, resolvedLimits, admittedCollections);
        return schema;
    } catch (error) {
        if (error instanceof NativeEditorBoundaryError) throw error;
        return defaultSchema;
    }
}
