import { useEffect, useState } from 'react';
import { AccessibilityInfo } from 'react-native';

/** OS "reduce motion", so disclosure animations are skipped rather than shortened. */
export function useReducedMotion(): boolean {
    const [reduceMotion, setReduceMotion] = useState(false);

    useEffect(() => {
        let cancelled = false;

        AccessibilityInfo.isReduceMotionEnabled().then((enabled) => {
            if (!cancelled) {
                setReduceMotion(enabled);
            }
        });

        const subscription = AccessibilityInfo.addEventListener(
            'reduceMotionChanged',
            setReduceMotion
        );

        return () => {
            cancelled = true;
            subscription.remove();
        };
    }, []);

    return reduceMotion;
}
