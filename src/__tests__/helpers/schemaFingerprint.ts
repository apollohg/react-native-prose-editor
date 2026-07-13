import { createHash } from 'node:crypto';
import {
    resolveDocumentSchema,
    type AttrSpec,
    type MarkSpec,
    type NodeSpec,
    type SchemaDefinition,
} from '../../schemas';

type CanonicalJson = null | boolean | number | string | CanonicalJson[] | CanonicalJsonObject;
type CanonicalJsonObject = { [name: string]: CanonicalJson };

function compareStrings(left: string, right: string): number {
    return left < right ? -1 : left > right ? 1 : 0;
}

function sortedObject<T>(entries: Iterable<readonly [string, T]>): Record<string, T> {
    return Object.fromEntries([...entries].sort(([left], [right]) => compareStrings(left, right)));
}

function canonicalJson(value: unknown): CanonicalJson {
    if (
        value == null ||
        typeof value === 'boolean' ||
        typeof value === 'number' ||
        typeof value === 'string'
    ) {
        return value;
    }
    if (Array.isArray(value)) return value.map(canonicalJson);
    return sortedObject(
        Object.entries(value as Record<string, unknown>).map(([name, nested]) => [
            name,
            canonicalJson(nested),
        ])
    );
}

function canonicalAttrs(attrs: Record<string, AttrSpec> | undefined) {
    return sortedObject(
        Object.entries(attrs ?? {}).map(([name, attr]) => {
            const hasDefault = Object.prototype.hasOwnProperty.call(attr, 'default');
            return [
                name,
                {
                    hasDefault,
                    default: hasDefault ? canonicalJson(attr.default) : null,
                },
            ] as const;
        })
    );
}

function canonicalRole(node: NodeSpec): string {
    if (node.role !== 'list') return node.role;
    return node.name.includes('ordered') || node.name.includes('Ordered')
        ? 'listOrdered'
        : 'listUnordered';
}

function canonicalNode(node: NodeSpec) {
    const group = [...new Set((node.group ?? '').split(/\s+/u).filter(Boolean))].sort(
        compareStrings
    );
    return {
        content: node.content.trim(),
        group,
        attrs: canonicalAttrs(node.attrs),
        role: canonicalRole(node),
        htmlTag: node.htmlTag ?? null,
        isVoid: node.isVoid ?? false,
        allowUndeclaredAttrs: node.allowUndeclaredAttrs ?? false,
    };
}

function canonicalMark(mark: MarkSpec) {
    return {
        htmlTag: mark.htmlTag ?? null,
        attrs: canonicalAttrs(mark.attrs),
        excludes: mark.excludes ?? null,
        allowUndeclaredAttrs: mark.allowUndeclaredAttrs ?? false,
    };
}

function firstHtmlTags(specs: ReadonlyArray<NodeSpec | MarkSpec>): Record<string, string> {
    const tags = new Map<string, string>();
    for (const spec of specs) {
        if (spec.htmlTag != null && !tags.has(spec.htmlTag)) tags.set(spec.htmlTag, spec.name);
    }
    return sortedObject(tags);
}

function preferredTextBlockName(nodes: NodeSpec[]): string | null {
    const candidates = nodes.filter(
        (node) =>
            node.role === 'textBlock' &&
            Object.values(node.attrs ?? {}).every((attr) =>
                Object.prototype.hasOwnProperty.call(attr, 'default')
            )
    );
    candidates.sort((left, right) => {
        const leftPriority = left.htmlTag === 'p' || left.name === 'paragraph' ? 0 : 1;
        const rightPriority = right.htmlTag === 'p' || right.name === 'paragraph' ? 0 : 1;
        return leftPriority - rightPriority || compareStrings(left.name, right.name);
    });
    return candidates[0]?.name ?? null;
}

function canonicalResolvedSchema(schema: SchemaDefinition) {
    const resolved = resolveDocumentSchema(schema);
    const documentNodeName = resolved.nodes.find((node) => node.role === 'doc')!.name;
    const textNodeName = resolved.nodes.find((node) => node.role === 'text')!.name;
    const fallbackListItemName =
        resolved.nodes
            .filter((node) => node.role === 'listItem')
            .map((node) => node.name)
            .sort(compareStrings)[0] ?? null;

    return {
        nodes: sortedObject(resolved.nodes.map((node) => [node.name, canonicalNode(node)])),
        marks: sortedObject(resolved.marks.map((mark) => [mark.name, canonicalMark(mark)])),
        nodeHtmlTags: firstHtmlTags(resolved.nodes),
        markHtmlTags: firstHtmlTags(resolved.marks),
        preferredTextBlockName: preferredTextBlockName(resolved.nodes),
        fallbackListItemName,
        documentNodeName,
        textNodeName,
    };
}

export function testSchemaFingerprint(schema: SchemaDefinition): string {
    const canonical = canonicalResolvedSchema(schema);
    return createHash('sha256').update(JSON.stringify(canonical), 'utf8').digest('hex');
}
