<?php
// A namespace's segments are case-insensitive, a constant's own name is not, and functions are
// case-insensitive throughout.

namespace Mixed\Case;

function Declared(): void
{
}

const Declared_Constant = 1;
define('mixed\case\Defined_Lower', 1);
define('MIXED\CASE\DEFINED_UPPER', 2);

declared();
DECLARED();
Declared();
echo Declared_Constant, DECLARED_CONSTANT, declared_constant;
echo Defined_Lower, defined_lower, DEFINED_UPPER, Defined_Upper;
echo \Mixed\Case\Declared_Constant, \MIXED\CASE\Declared_Constant;
\mixed\case\declared();
\Mixed\Case\Declared();
