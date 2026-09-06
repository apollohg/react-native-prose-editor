import type { ResolvedEditorResourceLimits } from './ResourceLimits';
import type { DocumentJSON } from './NativeEditorBridge';
import {
    CONTENT_EXPRESSION_MAX_DEPTH,
    DEFAULT_CONTENT_MAX_NODES,
    minimalContentMatch,
} from './contentExpression';
import type { SchemaDefinition, NodeSpec } from './schemaDefinition';
import { type AdmittedSchemaCollections, schemaBoundaryError } from './schemaNormalization';

export function constructDefaultEmptyDocument(
    schema: SchemaDefinition,
    resolvedLimits: ResolvedEditorResourceLimits,
    admittedCollections: AdmittedSchemaCollections
): DocumentJSON {
    const docNode = schema.nodes.find((node) => node.role === 'doc');
    if (!docNode) throw new Error('schema cannot construct a default document: missing doc role');

    const budget = { nodes: 0, work: 0, exhausted: false };
    const workLimit = Math.min(
        DEFAULT_CONTENT_MAX_NODES,
        resolvedLimits.maxSchemaNodes * 64 + resolvedLimits.maxSchemaExpressionBytes * 32
    );
    const consumeWork = (): boolean => {
        if (budget.work >= workLimit) {
            budget.exhausted = true;
            return false;
        }
        budget.work += 1;
        return true;
    };
    const candidatePriority = (candidate: NodeSpec): number => {
        if (
            candidate.role === 'textBlock' &&
            (candidate.htmlTag === 'p' || candidate.name === 'paragraph')
        ) {
            return 0;
        }
        return candidate.role === 'textBlock' ? 1 : 2;
    };
    const candidatesBySymbol = new Map<string, NodeSpec[]>();
    let candidateIndexAdmitted = true;
    const addCandidate = (symbol: string, candidate: NodeSpec): boolean => {
        if (!consumeWork()) return false;
        const candidates = candidatesBySymbol.get(symbol);
        if (candidates) candidates.push(candidate);
        else candidatesBySymbol.set(symbol, [candidate]);
        return true;
    };
    for (const candidate of schema.nodes) {
        if (!addCandidate(candidate.name, candidate)) {
            candidateIndexAdmitted = false;
            break;
        }
        for (const group of admittedCollections.groupsByNode.get(candidate) ?? []) {
            if (group !== candidate.name && !addCandidate(group, candidate)) {
                candidateIndexAdmitted = false;
                break;
            }
        }
        if (!candidateIndexAdmitted) break;
    }
    for (const candidates of candidatesBySymbol.values()) {
        const sortingWork = Math.max(
            candidates.length,
            Math.ceil(candidates.length * Math.log2(Math.max(2, candidates.length)))
        );
        for (let index = 0; index < sortingWork; index += 1) {
            if (!consumeWork()) {
                candidateIndexAdmitted = false;
                break;
            }
        }
        if (!candidateIndexAdmitted) break;
        candidates.sort(
            (left, right) =>
                candidatePriority(left) - candidatePriority(right) ||
                left.name.localeCompare(right.name)
        );
    }
    if (!candidateIndexAdmitted) {
        throw schemaBoundaryError(workLimit, workLimit + 1);
    }
    const constructNode = (
        node: NodeSpec,
        visiting: Set<string>,
        depth: number
    ): DocumentJSON | undefined => {
        if (
            depth > Math.min(CONTENT_EXPRESSION_MAX_DEPTH, resolvedLimits.maxDocumentDepth) ||
            !consumeWork()
        ) {
            return undefined;
        }
        if (node.role === 'text' || visiting.has(node.name)) return undefined;
        const attrs = admittedCollections.attrsByNode.get(node) ?? [];
        for (const _attr of attrs) {
            if (!consumeWork()) {
                budget.exhausted = true;
                return undefined;
            }
        }
        if (attrs.some(([, spec]) => spec.default === undefined)) {
            return undefined;
        }

        visiting.add(node.name);
        const children = minimalContentMatch(
            node.content ?? '',
            (symbol) => {
                for (const candidate of candidatesBySymbol.get(symbol) ?? []) {
                    if (!consumeWork()) return undefined;
                    const constructed = constructNode(candidate, visiting, depth + 1);
                    if (constructed) return constructed;
                }
                return undefined;
            },
            consumeWork
        );
        visiting.delete(node.name);
        if (!children) return undefined;
        if (budget.nodes >= resolvedLimits.maxDocumentNodes) {
            budget.exhausted = true;
            return undefined;
        }
        budget.nodes += 1;

        const result: DocumentJSON = { type: node.json?.type ?? node.name };
        if (children.length > 0) result.content = children;
        const projectedAttrs = Object.entries(node.json?.attrs ?? {});
        if (attrs.length > 0 || projectedAttrs.length > 0) {
            result.attrs = Object.fromEntries([
                ...attrs.map(([name, spec]) => [name, spec.default] as const),
                ...projectedAttrs,
            ]);
        }
        return result;
    };

    const document = constructNode(docNode, new Set(), 0);
    if (!document) {
        if (budget.exhausted || budget.work >= workLimit) {
            throw schemaBoundaryError(workLimit, workLimit + 1);
        }
        throw new Error(`schema cannot construct a default document for '${docNode.name}'`);
    }
    return document;
}
