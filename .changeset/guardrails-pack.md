---
"@neralabs/rerail-pack-guardrails": minor
---

Guardrails pack: a conversation rule, `guardrails-no-new-lint-suppression`, steers a coding agent whose file write or edit adds a linter suppression comment (eslint-disable, oxlint-disable, biome-ignore, @ts-ignore, # noqa, # nosec, # type: ignore, # pylint: disable, @phpstan-ignore, @psalm-suppress, phpcs:ignore) to fix the cause or ask the person instead. Set-up recommends the pack for every repository; core ships it inside its own package.
