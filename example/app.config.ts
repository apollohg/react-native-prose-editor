const rootPackage = require('../package.json');

export default {
  expo: {
    name: 'Native Editor Example',
    slug: 'native-editor-example',
    version: rootPackage.version,
    orientation: 'portrait',
    ios: {
      supportsTablet: true,
      bundleIdentifier: 'com.apollohg.nativeeditorexample',
      appleTeamId: 'UP4U59RPLZ',
    },
    android: {
      package: 'com.apollohg.nativeeditorexample',
    },
    extra: {
      eas: {
        projectId: '623475ea-5d13-42cb-97a4-022072b02441',
      },
    },
  },
};
