<?php
                    function foo(string $limit) : void {
                        /**
                         * @psalm-suppress InvalidOperand
                         */
                        for ($i = $limit; $i > 0; $i--) {
                            echo $i . "\n";
                        }
                    }