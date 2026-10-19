
        import { observable } from "mobx";

        export interface ClusterFrameInfo {
          frameId: number;
          processId: number;
        }

        export const clusterFrameMap = observable.map<string, ClusterFrameInfo>();
      
// features: [types,no-babel-old], parser: @typescript-eslint/parser