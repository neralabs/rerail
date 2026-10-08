<?php
// A PHPStan 2.2.16 rule that writes CallTargetCollector's records, by file, to the JSON file
// RERAIL_CALL_TARGETS names. It reports nothing. Loaded by phpstan.neon; see generate_php.py.

declare(strict_types=1);

namespace RerailOracle;

use PhpParser\Node;
use PHPStan\Analyser\Scope;
use PHPStan\Node\CollectedDataNode;
use PHPStan\Rules\Rule;

/** @implements Rule<CollectedDataNode> */
final class CallTargetDump implements Rule
{
    public function getNodeType(): string
    {
        return CollectedDataNode::class;
    }

    public function processNode(Node $node, Scope $scope): array
    {
        $records = $node->get(CallTargetCollector::class);
        ksort($records);
        foreach ($records as &$calls) {
            usort($calls, static fn (array $a, array $b): int => [$a[0], $a[1]] <=> [$b[0], $b[1]]);
        }
        unset($calls);
        $path = getenv('RERAIL_CALL_TARGETS');
        if (is_string($path) && $path !== '') {
            file_put_contents($path, json_encode($records, JSON_UNESCAPED_SLASHES | JSON_THROW_ON_ERROR));
        }
        return [];
    }
}
