import type { DocumentJSON } from '../../NativeEditorBridge';

/** Deterministic single-paragraph HTML used by the fake for html round-trips. */
export function fakeHtmlForDoc(doc: DocumentJSON): string {
    const content = Array.isArray(doc.content) ? doc.content : [];
    return content
        .map((block) => {
            const inline = Array.isArray(block?.content) ? block.content : [];
            const text = inline
                .map((node) => (typeof node?.text === 'string' ? node.text : ''))
                .join('');
            return `<p>${text}</p>`;
        })
        .join('');
}

export function fakeDocForHtml(html: string): DocumentJSON {
    const paragraphs: Record<string, unknown>[] = [];
    const pattern = /<p>([\s\S]*?)<\/p>/g;
    let match = pattern.exec(html);
    while (match) {
        const text = match[1].replace(/<[^>]+>/g, '');
        paragraphs.push(
            text.length > 0
                ? { type: 'paragraph', content: [{ type: 'text', text }] }
                : { type: 'paragraph' }
        );
        match = pattern.exec(html);
    }
    if (paragraphs.length === 0) {
        const text = html.replace(/<[^>]+>/g, '');
        paragraphs.push(
            text.length > 0
                ? { type: 'paragraph', content: [{ type: 'text', text }] }
                : { type: 'paragraph' }
        );
    }
    return { type: 'doc', content: paragraphs } as DocumentJSON;
}

export function fakeDocForText(text: string): DocumentJSON {
    return {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text }] }],
    } as DocumentJSON;
}

export function cloneDoc(doc: DocumentJSON): DocumentJSON {
    return JSON.parse(JSON.stringify(doc)) as DocumentJSON;
}

export function appendText(doc: DocumentJSON, text: string): DocumentJSON {
    const next = cloneDoc(doc);
    const content = Array.isArray(next.content) ? next.content : [];
    if (content.length === 0) {
        content.push({ type: 'paragraph', content: [{ type: 'text', text }] });
        return next;
    }
    const last = content[content.length - 1] as Record<string, unknown>;
    const inline = Array.isArray(last.content) ? (last.content as Record<string, unknown>[]) : [];
    const lastText = inline.length > 0 ? inline[inline.length - 1] : null;
    if (lastText && typeof lastText.text === 'string') {
        lastText.text = `${lastText.text}${text}`;
    } else {
        inline.push({ type: 'text', text });
        last.content = inline;
    }
    return next;
}

export type FakeDocumentNode = Record<string, unknown>;

export interface FakeInlineSpan {
    scalarStart: number;
    scalarEnd: number;
    documentStart: number;
    documentEnd: number;
    kind: 'text' | 'atom';
    marks: FakeDocumentNode[];
}

export interface FakePositionBlock {
    scalarStart: number;
    scalarLength: number;
    documentStart: number;
    documentEnd: number;
    isVoid: boolean;
    isPlaceholder: boolean;
    inlineSpans: FakeInlineSpan[];
    ancestors: FakeDocumentNode[];
}

export interface FakeScalarDocumentMap {
    scalarLength: number;
    clampDocumentOffset(offset: number): number;
    scalarToDocument(offset: number): number;
    documentToScalar(offset: number): number;
    activeStateAt(offset: number): {
        marks: Record<string, boolean>;
        markAttrs: Record<string, Record<string, unknown>>;
        nodes: Record<string, boolean>;
    };
}

/** The native snapshot measures text in Unicode scalar values, never UTF-16 code units. */
export function unicodeScalarLength(text: string): number {
    return Array.from(text).length;
}

export function isFakeVoidNode(node: FakeDocumentNode): boolean {
    return (
        node.atom === true ||
        node.type === 'hardBreak' ||
        node.type === 'hard_break' ||
        node.type === 'mention' ||
        node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
    );
}

export function isFakeBlockVoidNode(node: FakeDocumentNode): boolean {
    return (
        node.atom === true ||
        node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
    );
}

export function fakeAtomLabel(node: FakeDocumentNode): string {
    const type = typeof node.type === 'string' ? node.type : '';
    const attrs =
        node.attrs != null && typeof node.attrs === 'object' && !Array.isArray(node.attrs)
            ? (node.attrs as Record<string, unknown>)
            : {};
    let label = typeof attrs.label === 'string' && attrs.label.length > 0 ? attrs.label : type;
    const trigger =
        typeof attrs.mentionSuggestionChar === 'string' ? attrs.mentionSuggestionChar : '';
    if (type === 'mention' && trigger.length > 0 && !label.startsWith(trigger)) {
        label = `${trigger}${label}`;
    }
    return type === 'mention' ? label : `[${label}]`;
}

export function fakeInlineAtomScalarLength(node: FakeDocumentNode): number {
    return node.type === 'hardBreak' || node.type === 'hard_break'
        ? 1
        : unicodeScalarLength(fakeAtomLabel(node));
}

export function fakeBlockAtomScalarLength(node: FakeDocumentNode): number {
    return node.type === 'image' ||
        node.type === 'horizontalRule' ||
        node.type === 'horizontal_rule'
        ? 1
        : unicodeScalarLength(fakeAtomLabel(node));
}

export function fakeDocumentNodeSize(node: FakeDocumentNode): number {
    if (typeof node.text === 'string') return unicodeScalarLength(node.text);
    if (isFakeVoidNode(node)) return 1;
    const content = Array.isArray(node.content) ? node.content : [];
    return (
        2 +
        content.reduce(
            (total, child) =>
                child != null && typeof child === 'object' && !Array.isArray(child)
                    ? total + fakeDocumentNodeSize(child as FakeDocumentNode)
                    : total,
            0
        )
    );
}

/**
 * Model the production PositionMap for the fake's supported top-level block schema.
 * Text blocks expose an empty-block placeholder; block boundaries contribute one
 * rendered separator scalar; and opaque atoms keep their one-token document extent
 * while exposing their rendered scalar width.
 */
export function fakeScalarDocumentMap(doc: DocumentJSON): FakeScalarDocumentMap {
    const blocks: FakePositionBlock[] = [];
    const content = Array.isArray(doc.content) ? doc.content : [];
    let documentLength = 0;

    for (const rawBlock of content) {
        if (rawBlock == null || typeof rawBlock !== 'object' || Array.isArray(rawBlock)) continue;
        const block = rawBlock as FakeDocumentNode;
        if (isFakeBlockVoidNode(block)) {
            blocks.push({
                scalarStart: 0,
                scalarLength: fakeBlockAtomScalarLength(block),
                documentStart: documentLength,
                documentEnd: documentLength,
                isVoid: true,
                isPlaceholder: false,
                inlineSpans: [],
                ancestors: [block],
            });
            documentLength += fakeDocumentNodeSize(block);
            continue;
        }

        const inline = Array.isArray(block.content) ? block.content : [];
        const documentStart = documentLength + 1;
        let inlineDocumentOffset = documentStart;
        let inlineScalarOffset = 0;
        const inlineSpans: FakeInlineSpan[] = [];
        for (const rawInline of inline) {
            if (rawInline == null || typeof rawInline !== 'object' || Array.isArray(rawInline)) {
                continue;
            }
            const inlineNode = rawInline as FakeDocumentNode;
            if (typeof inlineNode.text === 'string') {
                const length = unicodeScalarLength(inlineNode.text);
                inlineSpans.push({
                    scalarStart: inlineScalarOffset,
                    scalarEnd: inlineScalarOffset + length,
                    documentStart: inlineDocumentOffset,
                    documentEnd: inlineDocumentOffset + length,
                    kind: 'text',
                    marks: Array.isArray(inlineNode.marks)
                        ? inlineNode.marks.filter(
                              (mark): mark is FakeDocumentNode =>
                                  mark != null && typeof mark === 'object' && !Array.isArray(mark)
                          )
                        : [],
                });
                inlineScalarOffset += length;
                inlineDocumentOffset += length;
            } else if (isFakeVoidNode(inlineNode)) {
                const length = fakeInlineAtomScalarLength(inlineNode);
                inlineSpans.push({
                    scalarStart: inlineScalarOffset,
                    scalarEnd: inlineScalarOffset + length,
                    documentStart: inlineDocumentOffset,
                    documentEnd: inlineDocumentOffset + 1,
                    kind: 'atom',
                    marks: [],
                });
                inlineScalarOffset += length;
                inlineDocumentOffset += 1;
            } else {
                inlineDocumentOffset += fakeDocumentNodeSize(inlineNode);
            }
        }
        const isPlaceholder = inline.length === 0;
        blocks.push({
            scalarStart: 0,
            scalarLength: isPlaceholder ? 1 : inlineScalarOffset,
            documentStart,
            documentEnd: inlineDocumentOffset,
            isVoid: false,
            isPlaceholder,
            inlineSpans,
            ancestors: [block],
        });
        documentLength += fakeDocumentNodeSize(block);
    }

    let scalarLength = 0;
    for (const [index, block] of blocks.entries()) {
        block.scalarStart = scalarLength;
        scalarLength += block.scalarLength + (index + 1 < blocks.length ? 1 : 0);
    }

    const clampScalar = (offset: number) => Math.min(Math.max(offset, 0), scalarLength);
    const clampDocumentOffset = (offset: number) => Math.min(Math.max(offset, 0), documentLength);
    const blockForDocumentOffset = (offset: number): FakePositionBlock | undefined => {
        let previous: FakePositionBlock | undefined;
        for (const block of blocks) {
            if (block.isVoid) {
                if (offset === block.documentStart) return block;
                if (offset < block.documentStart) {
                    if (!previous) return block;
                    return offset - previous.documentEnd <= block.documentStart - offset
                        ? previous
                        : block;
                }
                previous = block;
                continue;
            }
            if (offset >= block.documentStart && offset <= block.documentEnd) return block;
            if (offset < block.documentStart) {
                if (!previous) return block;
                return offset - previous.documentEnd <= block.documentStart - offset
                    ? previous
                    : block;
            }
            previous = block;
        }
        return previous;
    };
    const scalarToDocument = (offset: number) => {
        const scalar = clampScalar(offset);
        const block = [...blocks].reverse().find((candidate) => candidate.scalarStart <= scalar);
        if (!block) return 0;
        const intraScalar = scalar - block.scalarStart;
        if (block.isVoid) {
            return intraScalar >= block.scalarLength
                ? block.documentStart + 1
                : block.documentStart;
        }
        if (block.isPlaceholder) return block.documentStart;
        const span = block.inlineSpans.find(
            (candidate) => intraScalar >= candidate.scalarStart && intraScalar < candidate.scalarEnd
        );
        if (!span) return block.documentEnd;
        return span.kind === 'text'
            ? span.documentStart + (intraScalar - span.scalarStart)
            : span.documentStart;
    };
    const documentToScalar = (offset: number) => {
        const position = clampDocumentOffset(offset);
        const block = blockForDocumentOffset(position);
        if (!block) return scalarLength;
        if (block.isVoid) {
            return block.scalarStart + (position <= block.documentStart ? 0 : block.scalarLength);
        }
        if (block.isPlaceholder) {
            return block.scalarStart + (position < block.documentStart ? 0 : block.scalarLength);
        }
        if (position < block.documentStart) return block.scalarStart;
        if (position > block.documentEnd) return block.scalarStart + block.scalarLength;
        for (const span of block.inlineSpans) {
            if (position < span.documentStart) return block.scalarStart + span.scalarStart;
            if (position < span.documentEnd) {
                return (
                    block.scalarStart +
                    span.scalarStart +
                    (span.kind === 'text' ? position - span.documentStart : 0)
                );
            }
        }
        return block.scalarStart + block.scalarLength;
    };
    const activeStateAt = (offset: number) => {
        const position = clampDocumentOffset(offset);
        const block = blockForDocumentOffset(position);
        const span =
            block?.inlineSpans.find(
                (candidate) =>
                    position >= candidate.documentStart && position < candidate.documentEnd
            ) ??
            [...(block?.inlineSpans ?? [])]
                .reverse()
                .find((candidate) => position === candidate.documentEnd);
        const marks: Record<string, boolean> = {};
        const markAttrs: Record<string, Record<string, unknown>> = {};
        const nodes: Record<string, boolean> = {};
        for (const mark of span?.marks ?? []) {
            const type = typeof mark.type === 'string' ? mark.type : '';
            if (!type) continue;
            marks[type] = true;
            if (
                mark.attrs != null &&
                typeof mark.attrs === 'object' &&
                !Array.isArray(mark.attrs)
            ) {
                markAttrs[type] = { ...(mark.attrs as Record<string, unknown>) };
            }
        }
        for (const node of block?.ancestors ?? []) {
            const type = typeof node.type === 'string' ? node.type : '';
            if (type === 'heading') {
                const level = (node.attrs as Record<string, unknown> | undefined)?.level;
                if (level != null) nodes[`heading:${String(level)}`] = true;
            } else if (type === 'blockquote' || type === 'bulletList' || type === 'orderedList') {
                nodes[type] = true;
            }
        }
        return { marks, markAttrs, nodes };
    };

    return { scalarLength, clampDocumentOffset, scalarToDocument, documentToScalar, activeStateAt };
}
