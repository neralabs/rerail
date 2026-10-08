<?php
                    $a = [
                        "a" => [1],
                        "b" => [2]
                    ];

                    foreach (["a"] as $e){
                        takes_ref($a[$e]);
                    }

                    /** @param array<string|int> $p */
                    function takes_ref(array &$p): void {
                        echo implode(",", $p);
                    }