/**
 * How much of Rerail's footer a host draws: `compact`, or `silent` (nothing until asked). Apart from the storage
 * schema, which stores the preference, so a footer reader can name the values without linking the schema and Drizzle.
 */
export const PRESENTATION_PRESENCES = ["compact", "silent"] as const;
export type PresentationPresence = (typeof PRESENTATION_PRESENCES)[number];
