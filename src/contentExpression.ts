const MAX_NESTING_DEPTH = 128;
const MAX_AUTOMATON_STATES = 10_000;
const MAX_REPETITION_BOUND = 10_000;

type Expression =
    | { kind: 'empty' }
    | { kind: 'symbol'; symbol: string }
    | { kind: 'sequence'; expressions: Expression[] }
    | { kind: 'alternation'; expressions: Expression[] }
    | { kind: 'repeat'; expression: Expression; min: number; max: number | null };

interface State {
    epsilon: number[];
    transitions: Array<{ symbol: string; target: number }>;
}

class ContentExpressionParser {
    private position = 0;
    private depth = 0;

    constructor(private readonly source: string) {}

    parse(): Expression {
        if (this.source.trim() === '') return { kind: 'empty' };
        const expression = this.parseAlternation();
        this.skipWhitespace();
        if (!this.atEnd()) throw new Error(`unexpected '${this.peek()}'`);
        return expression;
    }

    private parseAlternation(): Expression {
        const expressions = [this.parseSequence()];
        while (true) {
            this.skipWhitespace();
            if (!this.consume('|')) break;
            this.skipWhitespace();
            if (this.atEnd() || this.peek() === ')' || this.peek() === '|') {
                throw new Error("missing expression after '|'");
            }
            expressions.push(this.parseSequence());
        }
        return expressions.length === 1 ? expressions[0] : { kind: 'alternation', expressions };
    }

    private parseSequence(): Expression {
        const expressions: Expression[] = [];
        while (true) {
            this.skipWhitespace();
            if (this.atEnd() || this.peek() === ')' || this.peek() === '|') break;
            expressions.push(this.parseRepeated());
        }
        if (expressions.length === 0) throw new Error('expected expression');
        return expressions.length === 1 ? expressions[0] : { kind: 'sequence', expressions };
    }

    private parseRepeated(): Expression {
        const atom = this.parseAtom();
        this.skipWhitespace();
        let expression = atom;
        if (this.consume('?')) expression = { kind: 'repeat', expression: atom, min: 0, max: 1 };
        else if (this.consume('*'))
            expression = { kind: 'repeat', expression: atom, min: 0, max: null };
        else if (this.consume('+'))
            expression = { kind: 'repeat', expression: atom, min: 1, max: null };
        else if (this.consume('{')) expression = this.parseRange(atom);
        this.skipWhitespace();
        if (['?', '*', '+', '{'].includes(this.peek() ?? '')) {
            throw new Error('multiple quantifiers');
        }
        return expression;
    }

    private parseAtom(): Expression {
        this.skipWhitespace();
        if (this.consume('(')) {
            if (this.depth >= MAX_NESTING_DEPTH) throw new Error('expression nesting too deep');
            this.depth += 1;
            try {
                this.skipWhitespace();
                if (this.peek() === ')') throw new Error('empty group');
                const expression = this.parseAlternation();
                this.skipWhitespace();
                if (!this.consume(')')) throw new Error("missing ')'");
                return expression;
            } finally {
                this.depth -= 1;
            }
        }

        const start = this.position;
        while (this.peek() != null && /[A-Za-z0-9_-]/.test(this.peek()!)) this.position += 1;
        if (start === this.position) throw new Error('expected node or group name');
        return { kind: 'symbol', symbol: this.source.slice(start, this.position) };
    }

    private parseRange(expression: Expression): Expression {
        const min = this.parseNumber();
        let max: number | null;
        if (this.consume('}')) max = min;
        else {
            if (!this.consume(',')) throw new Error("expected ',' or '}'");
            if (this.consume('}')) max = null;
            else {
                max = this.parseNumber();
                if (!this.consume('}')) throw new Error("expected '}'");
            }
        }
        if (max != null && max < min) throw new Error('range maximum is smaller than minimum');
        if ((max ?? min) > MAX_REPETITION_BOUND) throw new Error('repetition bound too large');
        return { kind: 'repeat', expression, min, max };
    }

    private parseNumber(): number {
        const start = this.position;
        while (this.peek() != null && /[0-9]/.test(this.peek()!)) this.position += 1;
        if (start === this.position) throw new Error('expected number');
        const value = Number(this.source.slice(start, this.position));
        if (!Number.isSafeInteger(value) || value > 0xffff_ffff) throw new Error('invalid count');
        return value;
    }

    private skipWhitespace(): void {
        while (this.peek() != null && /\s/.test(this.peek()!)) this.position += 1;
    }

    private peek(): string | undefined {
        return this.source[this.position];
    }

    private consume(character: string): boolean {
        if (this.peek() !== character) return false;
        this.position += 1;
        return true;
    }

    private atEnd(): boolean {
        return this.position === this.source.length;
    }
}

class ContentExpressionCompiler {
    readonly states: State[] = [];

    compile(expression: Expression, depth = 0): [number, number] {
        if (depth > MAX_NESTING_DEPTH) throw new Error('expression nesting too deep');
        if (expression.kind === 'empty') {
            const start = this.state();
            const end = this.state();
            this.states[start].epsilon.push(end);
            return [start, end];
        }
        if (expression.kind === 'symbol') {
            const start = this.state();
            const end = this.state();
            this.states[start].transitions.push({ symbol: expression.symbol, target: end });
            return [start, end];
        }
        if (expression.kind === 'sequence') {
            const start = this.state();
            let tail = start;
            for (const nested of expression.expressions) {
                const [nestedStart, nestedEnd] = this.compile(nested, depth + 1);
                this.states[tail].epsilon.push(nestedStart);
                tail = nestedEnd;
            }
            return [start, tail];
        }
        if (expression.kind === 'alternation') {
            const start = this.state();
            const end = this.state();
            for (const nested of expression.expressions) {
                const [nestedStart, nestedEnd] = this.compile(nested, depth + 1);
                this.states[start].epsilon.push(nestedStart);
                this.states[nestedEnd].epsilon.push(end);
            }
            return [start, end];
        }
        return this.compileRepeat(expression.expression, expression.min, expression.max, depth + 1);
    }

    private compileRepeat(
        expression: Expression,
        min: number,
        max: number | null,
        depth: number
    ): [number, number] {
        const start = this.state();
        const end = this.state();
        let tail = start;
        for (let count = 0; count < min; count += 1) {
            const [itemStart, itemEnd] = this.compile(expression, depth);
            this.states[tail].epsilon.push(itemStart);
            tail = itemEnd;
        }
        if (max == null) {
            this.states[tail].epsilon.push(end);
            const [itemStart, itemEnd] = this.compile(expression, depth);
            this.states[tail].epsilon.push(itemStart);
            this.states[itemEnd].epsilon.push(tail);
        } else {
            for (let count = min; count < max; count += 1) {
                this.states[tail].epsilon.push(end);
                const [itemStart, itemEnd] = this.compile(expression, depth);
                this.states[tail].epsilon.push(itemStart);
                tail = itemEnd;
            }
            this.states[tail].epsilon.push(end);
        }
        return [start, end];
    }

    private state(): number {
        if (this.states.length >= MAX_AUTOMATON_STATES)
            throw new Error('too many automaton states');
        this.states.push({ epsilon: [], transitions: [] });
        return this.states.length - 1;
    }
}

function epsilonClosure(states: State[], initial: Iterable<number>): Set<number> {
    const result = new Set<number>();
    const pending = [...initial];
    while (pending.length > 0) {
        const state = pending.pop()!;
        if (result.has(state)) continue;
        result.add(state);
        pending.push(...states[state].epsilon);
    }
    return result;
}

export function acceptingContentSymbols(
    content: string,
    existingChildTypes: readonly string[] = [],
    symbolMatches: (childType: string, symbol: string) => boolean = (child, symbol) =>
        child === symbol
): string[] {
    try {
        const expression = new ContentExpressionParser(content).parse();
        const compiler = new ContentExpressionCompiler();
        const [start] = compiler.compile(expression);
        let current = epsilonClosure(compiler.states, [start]);
        for (const childType of existingChildTypes) {
            const next = new Set<number>();
            for (const state of current) {
                for (const transition of compiler.states[state].transitions) {
                    if (symbolMatches(childType, transition.symbol)) next.add(transition.target);
                }
            }
            current = epsilonClosure(compiler.states, next);
        }
        const symbols = new Set<string>();
        for (const state of current) {
            for (const transition of compiler.states[state].transitions) {
                symbols.add(transition.symbol);
            }
        }
        return [...symbols].sort();
    } catch {
        return [];
    }
}

export function minimalContentMatch<T>(
    content: string,
    choose: (symbol: string) => T | undefined
): T[] | undefined {
    try {
        const expression = new ContentExpressionParser(content).parse();
        const compiler = new ContentExpressionCompiler();
        const [start, accept] = compiler.compile(expression);
        const pending: Array<{ state: number; values: T[] }> = [{ state: start, values: [] }];
        const visited = new Set<number>();
        while (pending.length > 0) {
            const current = pending.shift()!;
            if (visited.has(current.state)) continue;
            visited.add(current.state);
            if (current.state === accept) return current.values;

            const state = compiler.states[current.state];
            for (let index = state.epsilon.length - 1; index >= 0; index -= 1) {
                pending.unshift({ state: state.epsilon[index], values: current.values });
            }
            for (const transition of state.transitions) {
                const value = choose(transition.symbol);
                if (value !== undefined) {
                    pending.push({
                        state: transition.target,
                        values: [...current.values, value],
                    });
                }
            }
        }
        return undefined;
    } catch {
        return undefined;
    }
}
