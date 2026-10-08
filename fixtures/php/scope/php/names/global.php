<?php
// The global namespace: unqualified names are their own global names, qualified ones are
// relative to the global namespace, and relative (`namespace\`) names too.

use Vendor\Package\Client;
use Vendor\Package\Client as HttpClient;
use Vendor\Tools;
use function Vendor\Helpers\render;
use const Vendor\Config\LEVEL;

$client = new Client();
$other = new HttpClient();
$plain = new Plain();
$full = new \Vendor\Package\Server();
$qualified = new Tools\Hammer();
$aliasCase = new CLIENT();
$relative = new namespace\Local();
$nested = new Some\Deep\Thing();

exec('ls');
\exec('pwd');
EXEC('id');
strlen('abc');
render('page');
RENDER('upper');
my_helper();
Tools\helper();
namespace\local_helper();
\Vendor\Helpers\render('full');

echo PHP_EOL;
echo \PHP_EOL;
echo LEVEL;
echo level;
echo MY_CONSTANT;
echo Tools\LIMIT;
echo namespace\LOCAL_LIMIT;
echo E_ALL | E_STRICT;
var_dump(true, false, null, TRUE, False, NULL, \true, \null);

function global_function(Client $a, ?Plain $b, int $c, string|Tools\Wrench $d): HttpClient
{
    return new $a();
}

const GLOBAL_CONSTANT = PHP_INT_MAX;

if ($client instanceof Client || $client instanceof \Other\Thing) {
    Client::create();
    Client::$instances[] = Tools::VERSION;
    echo Client::class, \Vendor\Package\Client::class;
}
