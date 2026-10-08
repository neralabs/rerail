<?php
                    function foo(int $counter) : void {
                        foreach ([1, 2, 3] as $_) {
                            echo ($counter = $counter + 1);
                            echo rand(0, 1) ? 1 : 0;
                        }
                    }