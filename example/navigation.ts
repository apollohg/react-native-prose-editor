/** Route identity for the harness stack. */
export const HARNESS_ROUTE = 'harness';
export const BENCHMARK_ROUTE = 'benchmark';

/** Navigation bar titles. */
export const ROUTE_TITLES = {
    [HARNESS_ROUTE]: 'React Native Prose Editor',
    [BENCHMARK_ROUTE]: 'Prepared prose benchmark',
} as const;

export type RootStackParamList = {
    [HARNESS_ROUTE]: undefined;
    [BENCHMARK_ROUTE]: undefined;
};
