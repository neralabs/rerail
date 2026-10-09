<?php
                    class A {
                        public string $value = "";
                        public function writeByRef(string $value): void {
                            $update =& $this->value;
                            $update = $value;
                        }
                    }