<?php
                    /**
                     * @param list<array{string, string}> $arr
                     */
                    function far(array $arr): void {
                        foreach ($arr as [$a, $b]) {
                            echo $a;
                            echo $b;
                        }
                    }