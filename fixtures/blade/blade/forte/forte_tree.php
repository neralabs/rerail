<?php
// What Forte 1.1.0 parses a template to, as the oracle files record it (`generate.php` writes
// them, the Rust port's tests read them: `native/src/code_rule_dsl/blade/forte/tests.rs`).
//
// A case is `{ "source" | "source_base64", "registry", "tree" }`. `registry` names the directive
// registry the template was parsed with: "default" (`Directives::withDefaults()`, what
// `Document::parse($template)` uses), "accept-all" (the defaults with `setAcceptAll(true)`, what
// `ParserOptions::defaults()->acceptAllDirectives()` uses) or the key of a registry the file's
// `registries` object spells out (`forte_registry_state`). `tree` is what `forte_tree` returns:
//
// - `t`: the lexer's tokens, flattened: type, start byte, end byte, then the next token.
// - `n`: the tree builder's flat nodes in index order: kind, parent, firstChild, lastChild,
//   nextSibling, tokenStart, tokenCount, genericOffset, data, then (only for nodes that carry
//   them) the record's extra fields as an object: `name`, `args`, `role` (an enum's case name).
// - `e`: the lexer's errors: state, reason (enum case names) and byte offset. Forte's
//   diagnostics are these: its tree builder records no parse errors.
// - `p`: per node index (a string key), what Forte's AST API reads off the flat tree:
//   - elements: `c` the class `Document` wraps it in (element, component, slot), `tag`
//     (`tagNameText()`), `void`, `self_closing`, `paired`, and for components `component`:
//     type, prefix, name, slot name.
//   - directive blocks: `start`, `end` (node indices or null) and `mid` (intermediate directives).
//   - echoes: `content` (between the delimiters, `content()`).
//   - `@php` blocks: `content`; `<?php` tags: `php` (`phpType()`) and `content`.
//   - attributes (of an element's `attributes()`): `type`, `raw_name`, `name`, `value`, `quote`.
// - `x`: instead of the rest, the message of the exception `Document::parse` threw (the tree
//   builder's depth limits).
//
// Requires a composer project requiring fortephp/forte 1.1.0 (laravel/framework 13.34.0 resolved)
// and PHP 8.5.11.
declare(strict_types=1);

use Forte\Ast\Components\ComponentNode;
use Forte\Ast\Components\SlotNode;
use Forte\Ast\DirectiveBlockNode;
use Forte\Ast\Document\Document;
use Forte\Ast\EchoNode;
use Forte\Ast\Elements\ElementNode;
use Forte\Ast\PhpBlockNode;
use Forte\Ast\PhpTagNode;
use Forte\Parser\Directives\Directives;
use Forte\Parser\NodeKind;
use Forte\Parser\ParserOptions;

const FORTE_VERSION = '1.1.0';
const FORTE_COMMIT = '9c9597d0b7080d16c25f53a940a0449ff1aab19b';

function forte_require(string $project): void
{
    require_once $project . '/vendor/autoload.php';
    $installed = \Composer\InstalledVersions::getPrettyVersion('fortephp/forte');
    if (ltrim((string) $installed, 'v') !== FORTE_VERSION) {
        fwrite(STDERR, "fortephp/forte {$installed} is installed, the oracle records " . FORTE_VERSION . "\n");
        exit(1);
    }
    $laravel = \Composer\InstalledVersions::getPrettyVersion('laravel/framework');
    if ($laravel !== 'v13.34.0') {
        fwrite(STDERR, "laravel/framework {$laravel} is installed, the oracle records v13.34.0\n");
        exit(1);
    }
    if (PHP_VERSION !== '8.5.11') {
        fwrite(STDERR, 'PHP ' . PHP_VERSION . " runs, the oracle records 8.5.11\n");
        exit(1);
    }
}

/** A directive registry's whole state, in the order its arrays hold it. */
function forte_registry_state(Directives $directives): array
{
    $read = fn () => [
        'acceptAll' => $this->acceptsAllDirectives(),
        'conditions' => array_map('strval', array_keys($this->conditions)),
        'finalTerminators' => array_map('strval', array_keys($this->finalTerminators)),
        'seen' => array_map('strval', array_keys($this->seenDirectives)),
        'advisoryPairs' => array_map('strval', array_keys($this->advisoryPairs)),
        'advisoryConditions' => array_map('strval', array_keys($this->advisoryConditions)),
        'directives' => array_values(array_map(fn ($d) => [
            'name' => $d->name,
            'args' => $d->args->name,
            'role' => $d->role->name,
            'isCondition' => $d->isCondition,
            'hasConditionLikeBranches' => $d->hasConditionLikeBranches,
            'terminators' => $d->terminators,
            'conditionLikeBranches' => $d->conditionLikeBranches,
            'terminator' => $d->terminator,
            'isSwitch' => $d->isSwitch,
            'switchParent' => $d->switchParent,
            'isSwitchBranch' => $d->isSwitchBranch,
            'isSwitchTerminator' => $d->isSwitchTerminator,
            'isConditionalPair' => $d->isConditionalPair,
            'pairingStrategy' => $d->pairingStrategy,
            'isConditionalClose' => $d->isConditionalClose,
        ], $this->directives)),
        'keys' => array_map('strval', array_keys($this->directives)),
    ];

    return $read->call($directives);
}

/** The registry a state spells out (`forte_registry_state`'s inverse). */
function forte_registry_from_state(array $state): Directives
{
    $directives = new Directives;
    $write = function () use ($state) {
        $this->setAcceptAll($state['acceptAll']);
        $this->conditions = array_fill_keys($state['conditions'], true);
        $this->finalTerminators = array_fill_keys($state['finalTerminators'], 1);
        $this->seenDirectives = array_fill_keys($state['seen'], true);
        $this->advisoryPairs = array_fill_keys($state['advisoryPairs'], true);
        $this->advisoryConditions = array_fill_keys($state['advisoryConditions'], true);
        $this->directives = [];
        foreach ($state['directives'] as $i => $d) {
            $directive = new \Forte\Parser\Directives\DiscoveredDirective;
            $directive->name = $d['name'];
            $directive->args = constant(\Forte\Parser\Directives\ArgumentRequirement::class . '::' . $d['args']);
            $directive->role = constant(\Forte\Parser\Directives\StructureRole::class . '::' . $d['role']);
            $directive->isCondition = $d['isCondition'];
            $directive->hasConditionLikeBranches = $d['hasConditionLikeBranches'];
            $directive->terminators = $d['terminators'];
            $directive->conditionLikeBranches = $d['conditionLikeBranches'];
            $directive->terminator = $d['terminator'];
            $directive->isSwitch = $d['isSwitch'];
            $directive->switchParent = $d['switchParent'];
            $directive->isSwitchBranch = $d['isSwitchBranch'];
            $directive->isSwitchTerminator = $d['isSwitchTerminator'];
            $directive->isConditionalPair = $d['isConditionalPair'];
            $directive->pairingStrategy = $d['pairingStrategy'];
            $directive->isConditionalClose = $d['isConditionalClose'];
            $this->directives[$state['keys'][$i]] = $directive;
        }
    };
    $write->call($directives);

    return $directives;
}

/** The registry a case names. */
function forte_registry(string $name, array $registries): Directives
{
    if ($name === 'default') {
        return Directives::withDefaults();
    }
    if ($name === 'accept-all') {
        $directives = Directives::withDefaults();
        $directives->setAcceptAll(true);

        return $directives;
    }

    return forte_registry_from_state($registries[$name]);
}

/** The name a registry state goes by: "default", "accept-all" or a hash of the state. */
function forte_registry_name(array $state): string
{
    static $default = null, $acceptAll = null;
    $default ??= forte_registry_state(Directives::withDefaults());
    if ($acceptAll === null) {
        $all = Directives::withDefaults();
        $all->setAcceptAll(true);
        $acceptAll = forte_registry_state($all);
    }
    if ($state == $default) {
        return 'default';
    }
    if ($state == $acceptAll) {
        return 'accept-all';
    }

    return substr(sha1(json_encode($state)), 0, 12);
}

/** The JSON form of a template: text when it is UTF-8, else base64. */
function forte_source_json(string $source): array
{
    return mb_check_encoding($source, 'UTF-8') ? ['source' => $source] : ['source_base64' => base64_encode($source)];
}

function forte_source_of(array $case): string
{
    return array_key_exists('source', $case) ? $case['source'] : base64_decode($case['source_base64']);
}

/** What Forte parses `$source` to with `$directives` (see the header). */
function forte_tree(string $source, Directives $directives): array
{
    try {
        $doc = Document::parse($source, ParserOptions::make()->directives($directives));
    } catch (\Throwable $e) {
        return ['x' => $e->getMessage()];
    }

    $tokens = [];
    foreach ($doc->getTokenRecords() as $token) {
        $tokens[] = $token->type;
        $tokens[] = $token->start;
        $tokens[] = $token->end;
    }

    $nodes = [];
    $count = count((fn () => $this->nodes)->call($doc));
    for ($i = 0; $i < $count; $i++) {
        $flat = $doc->getFlatNode($i);
        $row = [
            $flat['kind'], $flat['parent'], $flat['firstChild'], $flat['lastChild'], $flat['nextSibling'],
            $flat['tokenStart'], $flat['tokenCount'], $flat['genericOffset'], $flat['data'],
        ];
        $extra = array_diff_key($flat, array_flip([
            'kind', 'parent', 'firstChild', 'lastChild', 'nextSibling', 'tokenStart', 'tokenCount', 'genericOffset', 'data',
        ]));
        if ($extra !== []) {
            foreach ($extra as $key => $value) {
                if ($value instanceof \UnitEnum) {
                    $extra[$key] = $value->name;
                }
            }
            ksort($extra);
            $row[] = $extra;
        }
        $nodes[] = $row;
    }

    $errors = [];
    foreach ((fn () => $this->lexerErrors)->call($doc) as $error) {
        $errors[] = [$error->state->name, $error->reason->name, $error->offset];
    }

    $props = [];
    for ($i = 1; $i < $count; $i++) {
        $kind = $nodes[$i][0];
        if (! in_array($kind, [
            NodeKind::Element, NodeKind::DirectiveBlock, NodeKind::Echo, NodeKind::RawEcho,
            NodeKind::TripleEcho, NodeKind::PhpBlock, NodeKind::PhpTag,
        ], true)) {
            continue;
        }
        $node = $doc->getNode($i);
        $p = [];
        if ($node instanceof ElementNode) {
            $p['c'] = $node instanceof SlotNode ? 'slot' : ($node instanceof ComponentNode ? 'component' : 'element');
            $p['tag'] = $node->tagNameText();
            $p['void'] = $node->isVoid();
            $p['self_closing'] = $node->isSelfClosing();
            $p['paired'] = $node->isPaired();
            if ($node instanceof ComponentNode) {
                $meta = (fn () => $this->getMetadata())->call($node);
                $p['component'] = [$meta->type, $meta->prefix, $meta->componentName, $meta->slotName];
            }
            foreach ($node->attributes()->all() as $attribute) {
                $index = (fn () => $this->index)->call($attribute);
                $props[(string) $index] = [
                    'type' => $attribute->type(),
                    'raw_name' => $attribute->rawName(),
                    'name' => $attribute->nameText(),
                    'value' => $attribute->valueText(),
                    'quote' => $attribute->quote(),
                ];
            }
        } elseif ($node instanceof DirectiveBlockNode) {
            $p['start'] = $node->startDirective()?->index();
            $p['end'] = $node->endDirective()?->index();
            $p['mid'] = array_map(fn ($d) => $d->index(), $node->getIntermediateDirectives());
        } elseif ($node instanceof EchoNode) {
            $p['content'] = $node->content();
        } elseif ($node instanceof PhpBlockNode) {
            $p['content'] = $node->content();
        } elseif ($node instanceof PhpTagNode) {
            $p['php'] = $node->phpType();
            $p['content'] = $node->content();
        }
        $props[(string) $i] = $p;
    }
    ksort($props, SORT_NUMERIC);

    $tree = ['t' => $tokens, 'n' => $nodes, 'e' => $errors];
    if ($props !== []) {
        $tree['p'] = $props;
    }

    return $tree;
}

/** JSON flags every oracle file is written with. */
const FORTE_JSON = JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_UNESCAPED_LINE_TERMINATORS | JSON_THROW_ON_ERROR | JSON_PRESERVE_ZERO_FRACTION;

/** A case's props and strings may hold bytes that are not UTF-8: substitute as the Rust side does. */
function forte_json(mixed $value): string
{
    return json_encode($value, FORTE_JSON | JSON_INVALID_UTF8_SUBSTITUTE);
}

/** The prefix lengths Forte's `assertIncrementalParsing` lexes a template at. */
function forte_prefix_positions(int $length): array
{
    if ($length > 1000) {
        return array_values(array_unique(array_map(
            fn ($i) => max(1, min($length, (int) (($i * $length) / 100))),
            range(0, 99)
        )));
    }

    return $length === 0 ? [] : range(1, $length);
}

/**
 * A tree's digest, for cases too many or too large to spell out: FNV-1a 64 (hex) of the tree's
 * JSON (`forte_json`), which the Rust side writes byte for byte.
 */
function forte_digest(array $tree): string
{
    return hash('fnv1a64', forte_json($tree));
}
