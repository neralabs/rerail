<?php
                    class A {
                        public int $a = 0;
                        public int $b = 1;

                        function setPhpVersion(string $version): void {
                            [$a, $b] = explode(".", $version);

                            $this->a = (int) $a;
                            $this->b = (int) $b;
                        }
                    }
                    