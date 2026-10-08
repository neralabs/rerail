<?php
                    /** @param array<string> $arr */
                    function takesArrayOfString(array $arr) : void {
                        foreach ($arr as $a) {
                            echo $a;
                        }
                    }

                    /** @param mixed $a */
                    function takesArray($a) : void {
                        $arr = [$a];
                        takesArrayOfString($arr);
                    }