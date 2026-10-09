<?php
                    function foo(string $limit) : void {
                        /**
                         * @psalm-suppress InvalidOperand
                         */
                        for ($i = $limit; $i < 50; $i++) {
                            echo $i . "\n";
                        }
                    }