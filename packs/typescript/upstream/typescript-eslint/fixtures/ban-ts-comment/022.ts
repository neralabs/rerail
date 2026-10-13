
const a = 1;

// @ts-nocheck - should not be reported

// TS error is not actually suppressed
const b: string = a;
    