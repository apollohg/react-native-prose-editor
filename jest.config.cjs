const path = require('path');

const EXAMPLE_MODULES = path.join(__dirname, 'example', 'node_modules');
const REACT_NATIVE = path.join(EXAMPLE_MODULES, 'react-native');
const EXPO_MODULES_CORE = path.join(EXAMPLE_MODULES, 'expo', 'node_modules', 'expo-modules-core');

module.exports = {
    haste: {
        defaultPlatform: 'ios',
        platforms: ['android', 'ios', 'native'],
    },
    resolver: require.resolve('@react-native/jest-preset/jest/resolver.js'),
    setupFiles: [require.resolve('@react-native/jest-preset/jest/setup.js')],
    testEnvironment: require.resolve('@react-native/jest-preset/jest/react-native-env.js'),
    testMatch: ['<rootDir>/src/__tests__/**/*.test.ts', '<rootDir>/src/__tests__/**/*.test.tsx'],
    moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json'],
    moduleDirectories: ['node_modules', EXAMPLE_MODULES],
    moduleNameMapper: {
        '^react$': path.join(EXAMPLE_MODULES, 'react'),
        '^react-native$': REACT_NATIVE,
        '^react-native/(.*)$': path.join(REACT_NATIVE, '$1'),
        '^expo-modules-core$': EXPO_MODULES_CORE,
        '^@expo/vector-icons$': '<rootDir>/test/mocks/expoVectorIcons.js',
    },
    transform: {
        '^.+\\.[jt]sx?$': 'babel-jest',
    },
    transformIgnorePatterns: [
        'node_modules/(?!(react-native|@react-native|expo-modules-core|@expo|expo(nent)?)/)',
    ],
};
