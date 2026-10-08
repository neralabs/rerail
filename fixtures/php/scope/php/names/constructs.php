<?php
// Language constructs, magic constants and keywords that are not names, next to the names they
// hold.

declare(strict_types=1);

namespace Constructs;

use Library\Tools as T;

#[namespace\Local, T\Marker]
final class Holder
{
    public const string LABEL = __CLASS__;

    public string $name {
        get => PREFIX . $this->name;
        set(string $value) {
            $this->name = strtoupper($value);
        }
    }

    public function &reference(): array
    {
        static $items = [];
        return $items;
    }

    public function run(object $target, array $list, iterable $rows, callable $callback): mixed
    {
        echo __LINE__, __FILE__, __DIR__, __FUNCTION__, __METHOD__, __NAMESPACE__, __TRAIT__;
        echo __COMPILER_HALT_OFFSET__;
        if (isset($list[KEY]) || empty($list[OTHER_KEY])) {
            unset($list[REMOVED]);
        }
        [$first, $second] = $list;
        list($third, , $fourth) = $list;
        $cast = (int) VALUE + (float) $first + (object) [] + (bool) FLAG;
        $copy = clone $target;
        $made = new ($target::class)();
        $constant = $target::VERSION . static::LABEL . T::VERSION;
        $method = T::create(...);
        $chained = T::$registry::instance()?->name;
        $interpolated = "$target->name {$target->name} ${first}";
        $generator = (function () {
            yield KEY => VALUE;
            yield from T\items();
        })();
        $include = include LIBRARY_PATH . '/file.php';
        $evaluated = eval('return 1;');
        $printed = print MESSAGE;
        $check = $target instanceof $callback;
        $named = array_filter(array: $list, callback: null, mode: ARRAY_FILTER_USE_KEY);
        $shell = `ls {$list[SHELL_INDEX]} $list[SHELL_KEY]`;
        $nowdoc = <<<'EOT'
            NOT_A_CONSTANT {$list[NOT_EITHER]}
            EOT;
        goto finish;
        finish:
        return match (true) {
            $first instanceof T\Thing => T\Thing::from($first),
            default => throw new \LogicException(MESSAGE),
        };
    }
}

interface Contract
{
    const LIMIT = T\LIMIT + 1;
}

enum Suit: int
{
    case Hearts = 1;
    case Spades = Contract::LIMIT;

    public function color(): string
    {
        return match ($this) {
            Suit::Hearts => RED,
            self::Spades => BLACK,
        };
    }
}

const COMBINED = Suit::Hearts, OTHER = T\Other::VALUE;

function enum(): int
{
    return exit_code();
}

\Constructs\enum();
die(DIE_CODE);
