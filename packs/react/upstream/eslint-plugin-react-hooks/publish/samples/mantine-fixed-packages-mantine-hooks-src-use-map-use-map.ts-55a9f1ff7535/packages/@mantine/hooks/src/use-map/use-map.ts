import { useMemo, useRef, useState } from 'react';
import type { MutableRefObject } from 'react';

// Returns a copy of `source` whose writers copy the latest map, change the copy and store it as
// the new state, so the map held in state is never mutated.
function createObservableMap<T, V>(
  source: Map<T, V>,
  mapRef: MutableRefObject<Map<T, V>>,
  setMap: (map: Map<T, V>) => void
): Map<T, V> {
  const map = new Map<T, V>(source);

  map.set = (...args) => {
    mapRef.current = new Map(mapRef.current);
    Map.prototype.set.apply(mapRef.current, args);
    setMap(mapRef.current);
    return mapRef.current;
  };

  map.clear = (...args) => {
    mapRef.current = new Map(mapRef.current);
    Map.prototype.clear.apply(mapRef.current, args);
    setMap(mapRef.current);
  };

  map.delete = (...args) => {
    mapRef.current = new Map(mapRef.current);
    const res = Map.prototype.delete.apply(mapRef.current, args);
    setMap(mapRef.current);

    return res;
  };

  return map;
}

export function useMap<T, V>(initialState?: [T, V][]): Map<T, V> {
  const [map, setMap] = useState(() => new Map<T, V>(initialState));
  const mapRef = useRef(map);
  mapRef.current = map;

  return useMemo(() => createObservableMap(map, mapRef, setMap), [map]);
}
