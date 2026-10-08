// @phpstan-ignore-next-line
<?php echo $outsideTag;
$shell = `ls # @phpstan-ignore-line`; echo $inShell;
$command = `ls // @phpstan-ignore-next-line
`; echo $afterShell;
#[\Deprecated('@phpstan-ignore-line')] function attributed(): void { echo $inAttribute; }
#[\Deprecated('/* @phpstan-ignore-next-line */')]
function nextToAttribute(): void { echo $afterAttribute; }
?>
# @phpstan-ignore-next-line
<?php echo $afterCloseTag;
echo 'first
second' // @phpstan-ignore variable.undefined
, $afterString;
// @phpstan-ignore
echo /* @phpstan-ignore variable.undefined */
$afterFailedIgnore;
/* @phpstan-ignore-line */ echo $lineThenIdentifier; // @phpstan-ignore echo.nonString
echo $identifierThenLine; /* @phpstan-ignore echo.nonString */ // @phpstan-ignore-line
echo $docAfterIdentifier; /* @phpstan-ignore echo.nonString */ /** @phpstan-ignore-line */
// @phpstan-ignore-next-line why?
echo $afterQuestion;
echo $twoAfterQuestion;
?>
<? // @phpstan-ignore-next-line
<?php echo $afterShortTag;
