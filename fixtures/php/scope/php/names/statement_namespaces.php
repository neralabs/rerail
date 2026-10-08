<?php
// `namespace X;` statements: each runs until the next one, and each starts with no aliases.

namespace App\Http;

use App\Models\User;
use App\Models\{Post, Comment as Remark};
use function App\Support\{format_date, slugify as slug};
use const App\Support\{MAX_ITEMS, MIN_ITEMS as FLOOR};

$user = new User();
$post = new Post();
$remark = new Remark();
$controller = new Controller();
$nested = new Controllers\Admin();
$aliasQualified = new User\Profile();
$relative = new namespace\Kernel();
$fully = new \DateTimeImmutable();

format_date($user);
slug('x');
SLUG('y');
strlen('abc');
exec('ls');
unknown_function();
Controllers\dispatch();
namespace\boot();
\exec('pwd');

echo MAX_ITEMS, FLOOR, floor(1.5);
echo max_items;
echo PHP_VERSION;
echo UNKNOWN_CONSTANT;
echo Controllers\LIMIT;
echo namespace\LIMIT;
echo \PHP_VERSION;
var_dump(true, null, FALSE);

namespace App\Console;

$user = new User();
$command = new Command();
format_date();
echo MAX_ITEMS;
strlen('again');
