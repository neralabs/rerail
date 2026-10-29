import shallowEqual from 'shallowequal'
import { useCallback, useRef, useSyncExternalStore } from 'react'

/**
 *
 * @param monitor The monitor to collect state from
 * @param collect The collecting function
 * @param onUpdate A method to invoke when updates occur
 */
export function useCollector<T, S>(
	monitor: T,
	collect: (monitor: T) => S,
	onUpdate?: () => void,
): [S, () => void] {
	// The last collected value, so the snapshot keeps its identity while it stays shallow-equal
	const collectedRef = useRef<{ value: S } | null>(null)
	const notifyRef = useRef<(() => void) | null>(null)

	const subscribe = useCallback((notify: () => void) => {
		notifyRef.current = notify
		return () => {
			notifyRef.current = null
		}
	}, [])

	const getCollected = useCallback(() => {
		const nextValue = collect(monitor)
		if (!collectedRef.current || !shallowEqual(collectedRef.current.value, nextValue)) {
			collectedRef.current = { value: nextValue }
		}
		return collectedRef.current.value
	}, [monitor])

	// React reads the snapshot again once subscribed, which updates the collected properties
	// after the first render and the components are attached to dnd-core
	const collected = useSyncExternalStore(subscribe, getCollected, getCollected)

	const updateCollected = useCallback(() => {
		const nextValue = collect(monitor)
		if (!shallowEqual(collected, nextValue)) {
			notifyRef.current?.()
			if (onUpdate) {
				onUpdate()
			}
		}
	}, [collected, monitor, onUpdate])

	return [collected, updateCollected]
}
