<?php

declare(strict_types=1);

namespace App\Format;

const TWO = '%s %s';
const ONE = '%s';
const JOINED = ONE . ' ' . ONE;
const DOC = <<<EOT
    %s and %s
    EOT;
const NOW = <<<'EOT'
    %d
    EOT;

interface Formats
{
    const LINE = '%s: %s';
}

class Base implements Formats
{
    const BASE = '%s %s';
    protected const GUARDED = '%s %s';
}

class Report extends Base
{
    const ROW = '%s %s';
    const LABELLED = self::ROW . ' (%s)';
    final public const FIXED = '%s %s';
    const HEREDOC = <<<EOT
        %s | %s
        EOT;

    public function render(int $a, int $b): void
    {
        echo sprintf(self::ROW, $a);
        echo sprintf(self::ROW, $a, $b);
        echo sprintf(self::LABELLED, $a, $b);
        echo sprintf(Report::ROW, $a, $b, $a);
        echo sprintf(static::FIXED, $a);
        echo sprintf(self::HEREDOC, $a);
        echo sprintf(self::BASE, $a);
        echo sprintf(parent::BASE, $a, $b, $b);
        echo sprintf(self::LINE, $a);
        echo sprintf(Formats::LINE, $a, $b);
        echo sprintf(self::GUARDED, $a);
        printf(\App\Format\Report::ROW, $a);
        echo vsprintf(self::ROW, [$a]);
        echo sprintf(static::ROW, $a);
        echo sprintf(self::class, $a);
        echo sprintf(Missing::ROW, $a);
    }
}

final class Closed
{
    const ROW = '%s %s';

    public function render(int $a): void
    {
        echo sprintf(static::ROW, $a);
    }
}

enum Level: string
{
    const FORMAT = '%s %s';

    case Low = 'low';

    public function render(int $a): string
    {
        return sprintf(self::FORMAT, $a);
    }
}

function constants(int $a, int $b): void
{
    echo sprintf(TWO, $a);
    echo sprintf(TWO, $a, $b);
    echo sprintf(ONE, $a, $b);
    echo sprintf(\App\Format\TWO, $a);
    echo sprintf(namespace\TWO, $a);
    echo sprintf(JOINED, $a);
    echo sprintf(DOC, $a);
    echo sprintf(NOW, $a, $b);
    echo sprintf(TWO . ' %s', $a, $b);
    $format = Report::LABELLED;
    echo sprintf($format, $a);
    echo sprintf(Level::FORMAT, $a, $b, $b);
    echo sprintf(Closed::ROW, $a);
    echo sprintf(UNDECLARED, $a);
}

trait Shared
{
    const SHARED = '%s %s';
}

class UsesTrait
{
    use Shared;

    public function render(int $a): void
    {
        echo sprintf(self::SHARED, $a);
    }
}

class External extends \Vendor\Base
{
    public function render(int $a): void
    {
        echo sprintf(self::INHERITED, $a);
    }
}
