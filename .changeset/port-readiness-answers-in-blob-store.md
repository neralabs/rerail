---
"@neralabs/rerail-pack-blade": patch
"@neralabs/rerail-pack-javascript": patch
"@neralabs/rerail-pack-next": patch
"@neralabs/rerail-pack-node": patch
"@neralabs/rerail-pack-php": patch
"@neralabs/rerail-pack-python": patch
"@neralabs/rerail-pack-react": patch
"@neralabs/rerail-pack-rust": patch
"@neralabs/rerail-pack-sql": patch
"@neralabs/rerail-pack-typescript": patch
"@neralabs/rerail-pack-vue": patch
---

The port readiness generator's answers, with their excerpts of upstream's tests, are no longer in the repository or named by a `recording.lock`: Rerail's private Azure blob store holds them, keyed by the upstream version and a digest of the generator's inputs. Each `licenses/*/UPSTREAM.md` says so in the note under its file table and lists no `expected/texts/` file; no README lists a `recording.lock`, and the Rust and SQL READMEs generate the answers with `npm run generate`.
