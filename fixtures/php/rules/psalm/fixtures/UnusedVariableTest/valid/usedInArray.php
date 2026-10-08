<?php
                    /**
                     * @psalm-suppress MixedMethodCall
                     * @psalm-suppress MissingParamType
                     */
                    function foo($a) : void {
                        $b = "b";
                        $a->bar([$b]);
                    }