
import { Nullable } from 'nullable';
import { SomeOther } from 'some';
import { Component, Component2 } from 'react';
class Mmu implements Component<Nullable<SomeOther>, {}>, Component2 {}
new Mmu();
    