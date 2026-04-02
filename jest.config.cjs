module.exports = {
    preset: '<rootDir>/example/node_modules/react-native',
    testMatch: ['<rootDir>/src/__tests__/**/*.test.ts', '<rootDir>/src/__tests__/**/*.test.tsx'],
    moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json'],
    moduleDirectories: ['node_modules', '<rootDir>/example/node_modules'],
    moduleNameMapper: {
        '^react$': '<rootDir>/example/node_modules/react',
        '^react-native$': '<rootDir>/example/node_modules/react-native',
        '^expo-modules-core$': '<rootDir>/example/node_modules/expo-modules-core',
        '^@expo/vector-icons$': '<rootDir>/test/mocks/expoVectorIcons.js',
    },
    transform: {
        '^.+\\.[jt]sx?$': 'babel-jest',
    },
    transformIgnorePatterns: [
        'node_modules/(?!(react-native|@react-native|expo-modules-core|@expo|expo(nent)?)/)',
    ],
};
