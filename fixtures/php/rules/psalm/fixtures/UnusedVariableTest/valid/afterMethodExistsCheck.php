<?php
                    class A {
                        /**
                         * @param array<string, string> $options
                         */
                        public function __construct(array $options) {
                            $this->setOptions($options);
                        }

                        /**
                         * @param array<string, string> $options
                         */
                        protected function setOptions(array $options): void
                        {
                            foreach ($options as $key => $value) {
                                $normalized = ucfirst($key);
                                $method     = "set" . $normalized;

                                if (method_exists($this, $method)) {
                                    $this->$method($value);
                                }
                            }
                        }
                    }

                    new A(["bar" => "bat"]);