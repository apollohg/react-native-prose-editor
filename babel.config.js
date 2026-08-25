module.exports = function babelConfig(api) {
    api.cache(true);
    return {
        presets: [require.resolve('babel-preset-expo')],
    };
};
