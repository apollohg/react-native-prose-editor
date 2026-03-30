module.exports = function babelConfig(api) {
  api.cache(true);
  return {
    presets: [require.resolve('./example/node_modules/babel-preset-expo')],
  };
};
