import { codegenNativeComponent, type ViewProps } from 'react-native';
import type {
    DirectEventHandler,
    Double,
    Int32,
    WithDefault,
} from 'react-native/Libraries/Types/CodegenTypes';

export interface NativeProps extends ViewProps {
    sourceKind?: WithDefault<'json' | 'html', 'json'>;
    source: string;
    configJson: string;
    themeJson?: string;
    imagePolicyJson?: string;
    imagesEnabled?: WithDefault<boolean, true>;
    collapsesWhenEmpty?: WithDefault<boolean, true>;
    enableLinkTaps?: WithDefault<boolean, true>;
    fontEnvironmentRevision: Int32;
    onPressLink?: DirectEventHandler<{ href: string; text: string }>;
    // React Native numbers are IEEE-754 doubles, which exactly represent every
    // UInt32 document position. Int32 would corrupt positions above 2^31 - 1.
    onPressMention?: DirectEventHandler<{ docPos: Double; label: string; attrsJson: string }>;
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
