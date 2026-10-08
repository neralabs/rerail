<?php
                    class Foo
                    {
                        /** @var int|null */
                        public $bar = null;

                        public function setBarRef(int $ref): void
                        {
                            $this->bar = &$ref;
                        }
                    }
                