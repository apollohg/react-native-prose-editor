const path = require('path');
const { getDefaultConfig } = require('expo/metro-config');

const projectRoot = __dirname;
const packageRoot = path.resolve(projectRoot, '..');

/** @type {import('expo/metro-config').MetroConfig} */
const config = getDefaultConfig(projectRoot);

config.watchFolders = [packageRoot];
config.resolver.disableHierarchicalLookup = true;
config.resolver.nodeModulesPaths = [
    path.resolve(projectRoot, 'node_modules'),
    path.resolve(projectRoot, 'node_modules/expo/node_modules'),
    path.resolve(packageRoot, 'node_modules'),
];
config.resolver.extraNodeModules = {
    ...config.resolver.extraNodeModules,
    react: path.resolve(projectRoot, 'node_modules/react'),
    'react-native': path.resolve(projectRoot, 'node_modules/react-native'),
    expo: path.resolve(projectRoot, 'node_modules/expo'),
    'expo-modules-core': path.resolve(
        projectRoot,
        'node_modules/expo/node_modules/expo-modules-core'
    ),
};

module.exports = config;
