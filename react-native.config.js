module.exports = {
    dependency: {
        platforms: {
            // Both React Native codegen and Expo autolinking scan the package
            // root for ReactNativeProseEditor.podspec. Keep iOS enabled so the
            // root podspec contributes ReactNativeProseEditorSpec/provider
            // metadata; no nested compatibility podspec exists.
            ios: {},
            android: {
                componentDescriptors: ['PreparedProseViewerComponentDescriptor'],
                cmakeListsPath: '../android/src/main/jni/CMakeLists.txt',
                packageImportPath: 'import com.apollohg.editor.viewer.PreparedProseViewerPackage;',
                packageInstance: 'new PreparedProseViewerPackage()',
            },
        },
    },
};
