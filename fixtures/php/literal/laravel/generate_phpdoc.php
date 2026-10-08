<?php
// Writes phpdoc.expected.json: 1500 doc comments of https://github.com/laravel/framework at e5f3d6265f (checked out at
// /tmp/rerail-php-corpus/laravel), the first distinct ones in SHA-1 order, with the tree
// phpstan/phpdoc-parser 2.3.5 (as PHPStan 2.2.16 bundles it) parses from each; the record is
// `rerail-fixtures/php/literal/phpdoc_tree.php`'s. Run offline, not in CI:
//   /opt/homebrew/Cellar/php/8.5.11/bin/php -n generate_phpdoc.php \
//     /tmp/rerail-php-tools/vendor/phpstan/phpstan/phpstan.phar
// Requires PHP 8.5.11 and PHPStan 2.2.16 (bundling phpstan/phpdoc-parser 2.3.5).

require __DIR__ . '/../phpdoc_tree.php';

phpdoc_sample($argv[1] ?? '', '/tmp/rerail-php-corpus/laravel', 'https://github.com/laravel/framework', __DIR__ . '/phpdoc.expected.json', 1500);
