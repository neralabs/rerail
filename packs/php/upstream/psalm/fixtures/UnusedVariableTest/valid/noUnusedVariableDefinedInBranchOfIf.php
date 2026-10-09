<?php
                    abstract class Foo {
                        abstract function validate(): bool|string;
                        abstract function save(): bool|string;

                        function bar(): int {
                            if (($result = $this->validate()) && ($result = $this->save())) {
                                return 0;
                            } elseif (is_string($result)) {
                                return 1;
                            } else {
                                return 2;
                            }
                        }
                    }