---
"@neralabs/rerail-pack-php": patch
---

PHP pack: set-up no longer recommends the pack for a repository whose only PHP is tests (`test/`, `tests/`, `Test/`, `Tests/`, `spec/`, `*Test.php`, `*TestCase.php`), scripts and tooling (`scripts/`, `script/`, `bin/`, `tools/`, `ci/`, `.github/`), configuration (`config/`, `*.config.php`, dot-prefixed `.php` files such as `.php-cs-fixer.dist.php`, `rector.php`, `ecs.php`) or documentation (`docs/`, `doc/`), the way the Python and JavaScript packs already leave those out. Blade templates stay excluded. The reason set-up shows reads "PHP source files" instead of "*.php files". Installed packs are unaffected: the rules still run on every `.php` file.
