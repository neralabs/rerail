<?php

use Twig\Environment;
use Twig\ExpressionParser\InfixAssociativity;
use Twig\ExpressionParser\InfixExpressionParserInterface;
use Twig\ExpressionParser\PrefixExpressionParserInterface;
use Twig\Loader\ArrayLoader;

require_once dirname(__DIR__).'/vendor/autoload.php';

function printExpressionParsers($output, array $expressionParsers, bool $withAssociativity = false)
{
    if ($withAssociativity) {
        fwrite($output, "\n=========== =========== =============\n");
        fwrite($output, "Precedence  Operator    Associativity\n");
        fwrite($output, "=========== =========== =============\n");
    } else {
        fwrite($output, "\n=========== ===========\n");
        fwrite($output, "Precedence  Operator\n");
        fwrite($output, "=========== ===========\n");
    }

    usort($expressionParsers, function($a, $b) {
        $aPrecedence = $a->getPrecedenceChange() ? $a->getPrecedenceChange()->getNewPrecedence() : $a->getPrecedence();
        $bPrecedence = $b->getPrecedenceChange() ? $b->getPrecedenceChange()->getNewPrecedence() : $b->getPrecedence();
        return $bPrecedence - $aPrecedence; 
    });

    $current = \PHP_INT_MAX;
    foreach ($expressionParsers as $expressionParser) {
        $precedence = $expressionParser->getPrecedenceChange() ? $expressionParser->getPrecedenceChange()->getNewPrecedence() : $expressionParser->getPrecedence();
        if ($precedence !== $current) {
            $current = $precedence;
            if ($withAssociativity) {
                fwrite($output, \sprintf("\n%-11d %-11s %s", $precedence, $expressionParser->getName(), InfixAssociativity::Left === $expressionParser->getAssociativity() ? 'Left' : 'Right'));
            } else {
                fwrite($output, \sprintf("\n%-11d %s", $precedence, $expressionParser->getName()));
            }
        } else {
            fwrite($output, "\n".str_repeat(' ', 12).$expressionParser->getName());
        }
    }
    fwrite($output, "\n");
}

$output = fopen(dirname(__DIR__).'/doc/operators_precedence.rst', 'w');

$twig = new Environment(new ArrayLoader([]));
$prefixExpressionParsers = [];
$infixExpressionParsers = [];
foreach ($twig->getExpressionParsers() as $expressionParser) {
    if ($expressionParser instanceof PrefixExpressionParserInterface) {
        $prefixExpressionParsers[] = $expressionParser;
    } elseif ($expressionParser instanceof InfixExpressionParserInterface) {
        $infixExpressionParsers[] = $expressionParser;
    }
}

fwrite($output, "Unary operators precedence:\n");
printExpressionParsers($output, $prefixExpressionParsers);

fwrite($output, "\nBinary and Ternary operators precedence:\n");
printExpressionParsers($output, $infixExpressionParsers, true);

fclose($output);
