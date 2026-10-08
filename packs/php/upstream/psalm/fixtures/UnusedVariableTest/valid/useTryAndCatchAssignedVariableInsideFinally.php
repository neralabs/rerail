<?php
                    function foo() : void {
                        try {
                            // do something dangerous
                            $a = 5;
                        } catch (Exception $e) {
                            $a = 4;
                            throw new Exception("bad");
                        } finally {
                            /** @psalm-suppress PossiblyUndefinedVariable */
                            echo $a;
                        }
                    }