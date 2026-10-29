'use client';
import useEnhancedEffect from '../useEnhancedEffect/useEnhancedEffect';
import useLazyRef from '../useLazyRef/useLazyRef';

interface ValueRef<T> {
  current: T;
}

/**
 * Copied from `@base-ui/utils/useValueAsRef`.
 *
 * Stores the latest value in a stable ref. The ref updates after React commits,
 * so effects and delayed callbacks can read the current value without depending
 * on it and rerunning.
 */
export default function useValueAsRef<T>(value: T) {
  const latestRef = useLazyRef(() => createValueRef(value)).current;

  useEnhancedEffect(() => {
    latestRef.current = value;
  });

  return latestRef;
}

function createValueRef<T>(value: T): ValueRef<T> {
  const latest: ValueRef<T> = {
    current: value,
  };

  return latest;
}
