import { cache } from "react";
import { makeQueryClient } from "./make-query-client";

export const getServerQueryClient = cache(() => makeQueryClient());
