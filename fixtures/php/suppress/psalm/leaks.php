<?php

function before(): void
{
    echo $beforeFn;
}

final class BeforeClass
{
    public function method(): void
    {
        echo $beforeClass;
    }
}

function inside(): void
{
    echo $insideBefore;
    /** @psalm-suppress MixedArgument */
    /** @psalm-suppress UndefinedVariable */
    echo $insideLeak;
    echo $insideAfter;
    $closure = function (): void {
        echo $closureInherits;
        /** @psalm-suppress ForbiddenCode */
        /** @psalm-suppress UndefinedVariable */
        echo $closureLeak;
        var_dump($closureAfter);
    };
    var_dump($afterClosure);
    $closure();
}

function between(): void
{
    echo $between;
}

trait LeakTrait
{
    public function traitMethod(): void
    {
        echo $traitLeak;
    }
}

final class UsesLeakTrait
{
    use LeakTrait;
}

echo $topBefore;
/** @psalm-suppress MixedArgument */
/** Not a suppression. */
/** @psalm-suppress UndefinedGlobalVariable */
echo $topLeak;
echo $topAfter;

function after(): void
{
    echo $afterFn;
}

final class AfterClass
{
    public function method(): void
    {
        echo $afterClass;
    }
}

function loop(): void
{
    foreach ([1, 2] as $item) {
        var_dump($item);
        /** @psalm-suppress ForbiddenCode */
        /** @psalm-suppress UndefinedVariable */
        echo $loopLeak;
    }
    var_dump(3);
}

trait AfterTrait
{
    public function traitMethod(): void
    {
        echo $afterTrait;
    }
}

final class UsesAfterTrait
{
    use AfterTrait;
}

echo $afterTraits;
