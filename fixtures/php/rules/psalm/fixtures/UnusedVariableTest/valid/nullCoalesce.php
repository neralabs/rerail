<?php
                    function foo (?bool $b, int $c): void {
                        $b ??= $c;

                        echo $b;
                    }