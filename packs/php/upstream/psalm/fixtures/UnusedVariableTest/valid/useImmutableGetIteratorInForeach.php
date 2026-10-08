<?php
                    /**
                     * @psalm-immutable
                     * @psalm-suppress MissingTemplateParam
                     */
                    class A implements IteratorAggregate
                    {
                        /**
                         * @return Iterator<int>
                         */
                        public function getIterator() {
                            yield from [1, 2, 3];
                        }
                    }

                    $a = new A();

                    foreach ($a as $v) {
                        echo $v;
                    }