---
"@neralabs/rerail-pack-javascript": patch
---

JavaScript pack: ships four more ESLint core rules now that they pass the conformance check. `eslint-dot-notation`, `eslint-no-duplicate-imports`, `eslint-no-extra-label` and `eslint-no-lonely-if` start off, since `eslint:recommended` does not turn them on. Their documents share repeated structure through utilities instead of writing it out per clause; what they find is unchanged.

`eslint-no-loss-of-precision`, `eslint-no-prototype-builtins` and `eslint-no-labels` leave the shipped pack and go back to candidates, and `eslint-no-self-assign` stays a candidate instead of shipping. The conformance check now also scores labelled real-code samples (real files from open-source history, with ESLint's own findings), and these four disagree with ESLint there: `eslint-no-loss-of-precision` reports decimal literals of 2^63, which a number stores exactly and ESLint accepts, and the other three miss findings in Flow-annotated files the engine cannot parse yet. `eslint-no-setter-return` now reports an arrow setter's parenthesized body at the expression inside the parentheses, where ESLint reports it.
