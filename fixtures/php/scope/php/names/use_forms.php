<?php
// Every `use` form: single, aliased, multiple clauses, grouped, function and const groups,
// mixed groups, and aliases read in other cases.

namespace Library;

use Alpha\One, Alpha\Two as Deux;
use Beta\{Three, Four\Five, Six as Sechs};
use Gamma\{Seven, function eight, const NINE, function ten as tenth, const ELEVEN as Eleven};
use function Delta\twelve, Delta\thirteen as dreizehn;
use const Delta\FOURTEEN, Delta\FIFTEEN as Fifteen;
use \Epsilon\Sixteen;
use Zeta;

new One();
new one();
new ONE\Child();
new Deux();
new deux\Child();
new Three();
new Five();
new Five\Six();
new Sechs();
new Seven();
new Zeta();
new Zeta\Eta();
new Sixteen();

eight();
EIGHT();
tenth();
Tenth();
twelve();
dreizehn();
Dreizehn();
thirteen();
One\helper();

echo NINE, Nine, nine;
echo Eleven, ELEVEN;
echo FOURTEEN, fourteen;
echo Fifteen, FIFTEEN;
echo One\CONSTANT, one\CONSTANT;
echo Eight;
