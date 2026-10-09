<?php
                    /** @psalm-suppress MixedAssignment */
                    function foo(array $args) : void {
                        foreach ($args as $key => $value) {
                            $_GET[$key] = $value;
                        }
                    }