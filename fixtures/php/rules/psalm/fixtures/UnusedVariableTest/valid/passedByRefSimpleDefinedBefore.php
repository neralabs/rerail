<?php
                    $a = [];
                    takes_ref($a);

                    function takes_ref(?array &$p): void {
                        $p = [0];
                    }