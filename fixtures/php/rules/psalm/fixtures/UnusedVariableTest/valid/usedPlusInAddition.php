<?php
                    function takesAnInt(): void {
                        $i = 0;

                        while (rand(0, 1)) {
                            if (($i = $i + 1) > 10) {
                                break;
                            } else {}
                        }
                    }