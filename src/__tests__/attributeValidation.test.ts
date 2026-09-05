import {
    validateAttributeSpec,
    validateAttribute,
    validateAttributes,
} from '../attributeValidation';

test.each([
    [{ type: 'string' }, 1],
    [{ type: 'boolean' }, 'true'],
    [{ type: 'object' }, []],
    [{ type: 'object' }, null],
    [{ type: 'array' }, {}],
    [{ type: 'number', min: 0 }, -1],
    [{ type: 'string', max: 1 }, 'ab'],
    [{ type: 'array', min: 1 }, []],
    [{ enum: ['a', 'b'] }, 'c'],
] as const)('rejects invalid value for %j', (spec, value) => {
    expect(() => validateAttribute(value, spec, 'value')).toThrow(/value/);
});

test.each([
    { type: 'other' },
    { type: 'number', min: 2, max: 1 },
    { type: 'string', min: -1 },
    { type: 'array', max: 1.5 },
    { type: 'boolean', min: 1 },
    { enum: [] },
    { type: 'number', enum: ['one'] },
    { type: 'number', default: null },
])('rejects invalid declaration %j', (spec) => {
    expect(() => validateAttributeSpec(spec as never, 'value')).toThrow(/value/);
});

test('defaults, required attrs, unicode lengths and structural enums are honored', () => {
    expect(() => validateAttributes({}, { id: { type: 'string' } })).toThrow(/required/);
    expect(() => validateAttributes({}, { n: { type: 'number', default: 1 } })).not.toThrow();
    expect(() => validateAttribute('🙂', { type: 'string', max: 1 }, 'emoji')).not.toThrow();
    expect(() =>
        validateAttribute({ b: 2, a: 1 }, { type: 'object', enum: [{ a: 1, b: 2 }] }, 'record')
    ).not.toThrow();
});

test('rejects mixed enum types and non-JSON attribute values', () => {
    expect(() => validateAttributeSpec({ enum: [1, '1'] }, 'choice')).toThrow();
    for (const value of [{ n: undefined }, { n: NaN }, new Date(), () => 1]) {
        expect(() => validateAttribute(value, {}, 'value')).toThrow();
    }
    const circular: Record<string, unknown> = {};
    circular.self = circular;
    expect(() => validateAttributeSpec({ default: circular }, 'value')).toThrow();
});
