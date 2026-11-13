<?php
// Operators with a ? that are no decision, for lizard and Rerail alike: ??, ??=, ?-> and the short ?: (which lizard
// reads as one operator, not a conditional). Each function scores 1, or 2 with its one real decision.

function nullishCoalescing(array $options, string $name): string
{
    $label = $options['label'] ?? $name ?? 'none';
    $options['size'] ??= 10;
    $first = $options['first'] ?: $label;
    return $first;
}

function nullsafeChain(object $user): string
{
    return $user?->profile?->name;
}

function oneRealDecision(array $options, bool $loud): string
{
    $label = $options['label'] ?? 'none';
    $label = $label ?: 'empty';
    return $loud ? strtoupper($label) : $label;
}
