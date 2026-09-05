import React from 'react';
import { Text, View } from 'react-native';
import { act, fireEvent, render } from '@testing-library/react-native';
import { AtomHost } from '../AtomHost';
import { resolveAtomAttrsUpdate } from '../atomUpdates';

const props = {
    attrs: { count: 1 },
    selected: false,
    readOnly: false,
    interactive: true,
    isViewer: false,
    nodeType: 'card',
    updateAttrs: async () => {},
};

test('composes functional updates into one patch without mutating current attrs', () => {
    const attrs = { count: 2, title: 'a' };
    expect(
        resolveAtomAttrsUpdate(attrs, [
            (current) => ({ count: Number(current.count) + 1 }),
            (current) => ({ count: Number(current.count) + 1, title: 'b' }),
        ])
    ).toEqual({ count: 4, title: 'b' });
    expect(attrs).toEqual({ count: 2, title: 'a' });
});

test('rejects non-JSON partial updates', () => {
    expect(() => resolveAtomAttrsUpdate({}, { count: NaN })).toThrow();
    expect(() => resolveAtomAttrsUpdate({}, () => undefined as never)).toThrow();
});

test('isolates a failed renderer and permits retry without removing its sibling', () => {
    const log = jest.spyOn(console, 'error').mockImplementation(() => {});
    let fail = true;
    const Card = () => {
        if (fail) throw new Error('broken card');
        return <Text>Recovered</Text>;
    };
    const screen = render(
        <View>
            <AtomHost component={Card} atomProps={props} width={200} estimatedHeight={60} />
            <Text>Other card</Text>
        </View>
    );
    expect(screen.getByText('Other card')).toBeTruthy();
    expect(screen.getByText('Unable to display card')).toBeTruthy();
    fail = false;
    fireEvent.press(screen.getByText('Retry'));
    expect(screen.getByText('Recovered')).toBeTruthy();
    log.mockRestore();
});

test('keeps a measured spacer offscreen and pins focused controls', () => {
    const Card = () => <Text>Card content</Text>;
    const screen = render(
        <AtomHost component={Card} atomProps={props} width={200} estimatedHeight={60} visible />
    );
    fireEvent(screen.getByTestId('atom-host'), 'layout', {
        nativeEvent: { layout: { width: 200, height: 140 } },
    });
    fireEvent(screen.getByTestId('atom-host'), 'focus');
    screen.rerender(
        <AtomHost
            component={Card}
            atomProps={props}
            width={200}
            estimatedHeight={60}
            visible={false}
        />
    );
    expect(screen.getByText('Card content')).toBeTruthy();
    fireEvent(screen.getByTestId('atom-host'), 'blur');
    expect(screen.queryByText('Card content')).toBeNull();
    expect(
        screen.getByTestId('atom-host', { includeHiddenElements: true }).props.style
    ).toMatchObject({ height: 140 });
});

test('exposes pending and rejected updates to the renderer', async () => {
    let reject!: (error: Error) => void;
    const updateAttrs = () =>
        new Promise<void>((_, fail) => {
            reject = fail;
        });
    const Card = (value: any) => <View testID='card' atomProps={value} />;
    const screen = render(
        <AtomHost
            component={Card}
            atomProps={{ ...props, updateAttrs }}
            width={200}
            estimatedHeight={60}
        />
    );
    let update!: Promise<void>;
    act(() => {
        update = screen.getByTestId('card').props.atomProps.updateAttrs({ count: 2 });
    });
    expect(screen.getByTestId('card').props.atomProps.updatePending).toBe(true);
    await act(async () => {
        reject(new Error('save failed'));
        await expect(update).rejects.toThrow('save failed');
    });
    expect(screen.getByTestId('card').props.atomProps.updatePending).toBe(false);
    expect(screen.getByTestId('card').props.atomProps.updateError.message).toBe('save failed');
});

test('an older failure cannot overwrite the result of a newer update', async () => {
    const completions: Array<{ resolve: () => void; reject: (error: Error) => void }> = [];
    const updateAttrs = () =>
        new Promise<void>((resolve, reject) => completions.push({ resolve, reject }));
    const Card = (value: any) => <View testID='card' atomProps={value} />;
    const screen = render(
        <AtomHost
            component={Card}
            atomProps={{ ...props, updateAttrs }}
            width={200}
            estimatedHeight={60}
        />
    );
    let first!: Promise<void>, second!: Promise<void>;
    act(() => {
        first = screen.getByTestId('card').props.atomProps.updateAttrs({ count: 2 });
        second = screen.getByTestId('card').props.atomProps.updateAttrs({ count: 3 });
    });
    await act(async () => {
        completions[0].reject(new Error('older failure'));
        await expect(first).rejects.toThrow('older failure');
        completions[1].resolve();
        await second;
    });
    expect(screen.getByTestId('card').props.atomProps.updateError).toBeNull();
});
