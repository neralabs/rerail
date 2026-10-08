<?php

/** @psalm-suppress UndefinedVariable */
function suppressedFunction(): void
{
    echo $inFunction;
}

/**
 * @psalm-suppress UndefinedVariable
 */
class SuppressedClass
{
    public function method(): void
    {
        echo $inClassMethod;
    }
}

class PlainClass
{
    /** @psalm-suppress MissingPropertyType */
    public $suppressedProperty;

    public $plainProperty;

    /** @psalm-suppress UndefinedVariable */
    public function suppressedMethod(): void
    {
        echo $inMethod;
    }

    public function plainMethod(): void
    {
        echo $inPlainMethod;
    }
}

final class Box
{
    public function go(): void
    {
    }
}

function possiblyNull(?Box $box): void
{
    $box->go();
}

function parentIssue(?Box $box): void
{
    /** @psalm-suppress NullReference */
    $box->go();
}

function parentIssueWrongWay(Box $box): void
{
    $box = null;
    /** @psalm-suppress PossiblyNullReference */
    $box->go();
}

class Constants
{
    /** @psalm-suppress UndefinedConstant */
    public const SUPPRESSED = UNDEFINED_ONE;

    public const PLAIN = UNDEFINED_TWO;
}

/** @psalm-suppress UndefinedVariable */
/** A later docblock replaces the first for the method's storage. */
function lastDocblockWins(): void
{
    echo $lastDocblockWins;
}

final class LastDoc
{
    /** @psalm-suppress UndefinedVariable */
    /** A later docblock. */
    public function method(): void
    {
        echo $methodLastDoc;
    }

    /** A first docblock. */
    /** @psalm-suppress UndefinedVariable */
    public function second(): void
    {
        echo $methodSecondDoc;
    }
}

/** @psalm-suppress UndefinedVariable */
/** A later docblock. */
final class LastDocClass
{
    public function method(): void
    {
        echo $classLastDoc;
    }
}

/** @psalm-suppress UndefinedVariable */
#[Attribute]
final class AttributedClass
{
    public function method(): void
    {
        echo $attributed;
    }
}

function closures(): void
{
    $doc = /** @psalm-suppress UndefinedVariable */ static function (): void {
        echo $staticClosure;
    };
    $doc();
}

function closureSignatures(): void
{
    $closure = /** @psalm-suppress MissingClosureReturnType */ function () {
        return 1;
    };
    $arrow = /** @psalm-suppress MissingClosureReturnType */ fn () => 1;
    $closure();
    $arrow();
}

/** @psalm-suppress UndefinedVariable */
trait SuppressedTrait
{
    public function traitMethod(): void
    {
        echo $inTrait;
    }
}

final class UsesTrait
{
    use SuppressedTrait;
}

enum Status
{
    case First;

    /** @psalm-suppress UndefinedVariable */
    public function suppressed(): void
    {
        echo $inEnumMethod;
    }
}
