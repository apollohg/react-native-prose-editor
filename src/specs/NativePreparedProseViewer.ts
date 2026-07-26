import type { ViewProps } from 'react-native';
import codegenNativeComponent from 'react-native/Libraries/Utilities/codegenNativeComponent';
import type {
    DirectEventHandler,
    Int32,
    WithDefault,
} from 'react-native/Libraries/Types/CodegenTypes';

export interface NativeProps extends ViewProps {
    sourceKind: WithDefault<'json' | 'html', 'json'>;
    source: string;
    configJson: string;
    themeJson?: string;
    imagePolicyJson?: string;
    imagesEnabled: WithDefault<boolean, true>;
    collapsesWhenEmpty: WithDefault<boolean, true>;
    enableLinkTaps: WithDefault<boolean, true>;
    fontEnvironmentRevision: Int32;
    onPressLink?: DirectEventHandler<{ href: string; text: string }>;
    onPressMention?: DirectEventHandler<{ docPos: Int32; label: string }>;
    onError?: DirectEventHandler<{
        domain: string;
        code: string;
        message: string;
        fatal: boolean;
    }>;
}

export default codegenNativeComponent<NativeProps>('PreparedProseViewer', {
    interfaceOnly: true,
});
