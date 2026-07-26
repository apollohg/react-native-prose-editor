module.exports = {
    dependency: {
        platforms: {
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
