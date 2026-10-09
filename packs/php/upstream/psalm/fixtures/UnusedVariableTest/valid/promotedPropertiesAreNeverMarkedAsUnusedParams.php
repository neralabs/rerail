<?php
                    class Container {
                        private function __construct(
                            public float $value
                        ) {}

                        public static function fromValue(float $value): self {
                            return new self($value);
                        }
                    }