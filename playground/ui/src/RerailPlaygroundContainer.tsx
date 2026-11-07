import { RerailPlayground } from "./RerailPlayground";
import { useRerailPlayground, type RerailPlaygroundOptions } from "./useRerailPlayground";

export const RerailPlaygroundContainer = (props: RerailPlaygroundOptions) => (
  <RerailPlayground {...useRerailPlayground(props)} />
);
