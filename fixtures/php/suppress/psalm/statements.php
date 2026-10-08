<?php

function plain(): void
{
    echo $kept;
    /** @psalm-suppress UndefinedVariable */
    echo $statement;
    /** @psalm-suppress MixedArgument, UndefinedVariable */
    echo $list;
    /** @psalm-suppress MixedArgument,UndefinedVariable trailing reason */
    echo $compact;
    /** @psalm-suppress UndefinedVariable reason text */
    echo $reason;
    /**
     * @psalm-suppress UndefinedVariable
     */
    echo $multiline;
    /**
     * Description.
     *
     * @psalm-suppress MixedArgument
     * @psalm-suppress UndefinedVariable
     */
    echo $twoTags;
    /**
     * @psalm-suppress MixedArgument,
     *     UndefinedVariable
     */
    echo $continued;
    /** @psalm-suppress InvalidArgument */
    echo $wrong;
    /** @psalm-suppress all */
    echo $all;
    /** @psalm-suppress all, UndefinedVariable */
    echo $allAndName;
    // @psalm-suppress UndefinedVariable
    echo $lineComment;
    # @psalm-suppress UndefinedVariable
    echo $hashComment;
    /* @psalm-suppress UndefinedVariable */
    echo $blockComment;
    /**@psalm-suppress UndefinedVariable */
    echo $noSpaceDoc;
    /** text @psalm-suppress UndefinedVariable */
    echo $notAtLineStart;
    /** @psalm-suppress UndefinedVariable */ echo $sameLine;
    echo $trailing; /** @psalm-suppress UndefinedVariable */
    echo $afterTrailing;
    echo /** @psalm-suppress UndefinedVariable */ $insideExpression;
    /** @psalm-suppress UndefinedVariable */
    // an intervening line comment
    echo $intervening;
    /** @psalm-suppress UndefinedVariable */

    echo $blankLine;
    /** @psalm-suppress UndefinedVariable */
    if (true) {
        echo $nestedInIf;
    }
    if (true) /** @psalm-suppress UndefinedVariable */ echo $unbraced;
    if (rand(0, 1)) {
        echo $thenBranch;
    } /** @psalm-suppress UndefinedVariable */ else {
        echo $beforeElse;
    }
    /** @psalm-suppress UndefinedVariable */
    $f = function (): void {
        echo $closureInSuppressedStatement;
    };
    $g = /** @psalm-suppress UndefinedVariable */ function (): void {
        echo $closureDoc;
    };
    $h = /** @psalm-suppress UndefinedVariable */ fn () => $arrowDoc;
    switch (rand(0, 1)) {
        /** @psalm-suppress UndefinedVariable */
        case 1:
            echo $beforeCase;
            /** @psalm-suppress UndefinedVariable */
            echo $inCase;
    }
    /** @psalm-suppress UndefinedVariable */
    echo $first; echo $secondOnLine;
    /** @psalm-suppress UndefinedVariable */
    echo
        $multilineStatement;
    $literal = '/** @psalm-suppress UndefinedVariable */';
    echo $afterString;
    $heredoc = <<<TEXT
    /** @psalm-suppress UndefinedVariable */
    TEXT;
    echo $afterHeredoc;
    /** @psalm-suppress undefinedvariable */
    echo $lowercase;
    /**	@psalm-suppress	UndefinedVariable */
    echo $tabs;
    $f();
    $g();
    $h();
}

echo $topLevel;
/** @psalm-suppress UndefinedGlobalVariable */
echo $topLevelSuppressed;
