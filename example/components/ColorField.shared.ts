import type { ExampleAppChrome } from '../themePresets';

/** Shared contract: Metro picks `ColorField.ios.tsx` or `ColorField.tsx`. */
export type ColorFieldProps = {
    label: string;
    value: string;
    chrome: ExampleAppChrome;
    /** Ignored on iOS, where the system picker presents itself. */
    isExpanded: boolean;
    onToggle: () => void;
    onChange: (value: string) => void;
};
