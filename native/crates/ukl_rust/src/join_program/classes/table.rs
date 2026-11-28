//! The classes, by shape key: written by `join-lab classes` from the captures' labels (each run's shape and its
//! stage records' forms), merged with those it held. Not edited by hand: each form is a stage's parts
//! (`stage::<F>()`), each class its shape's key, name and forms.

#[allow(unused_imports)]
use super::{Class, StageClass, stage};

pub const CLASSES: &[Class] = &[
    Class { key: 0x078175d25c73ebf2, name: "s-078175d25c73ebf2", stages: &[&F_0100000400002210, &F_0100000800000000] },
    Class { key: 0x0881a58591ab27c3, name: "s-0881a58591ab27c3", stages: &[&F_0100000000002212, &F_0100000400000000] },
    Class { key: 0x200728769d5bc9b8, name: "s-200728769d5bc9b8", stages: &[&F_0100000400000000, &F_0100400000002214] },
    Class { key: 0x45544bee29625687, name: "s-45544bee29625687", stages: &[&F_0100000800000000, &F_0100400000002214] },
    Class { key: 0x570dda21dab40bf9, name: "s-570dda21dab40bf9", stages: &[&F_0100000000002211, &F_0100000400000210] },
    Class { key: 0x5fcaa27aa961eda0, name: "s-5fcaa27aa961eda0", stages: &[&F_0100000400000000, &F_0100000400002210] },
    Class { key: 0x634ea64cd7e1cd21, name: "s-634ea64cd7e1cd21", stages: &[&F_0100000800000000, &F_0100408400000003, &F_0100608008000003] },
    Class { key: 0x648762e8af7b1444, name: "s-648762e8af7b1444", stages: &[&F_0100000000002212, &F_0100000800000000, &F_0100610418002210] },
    Class { key: 0x711a95bdd88da4c4, name: "s-711a95bdd88da4c4", stages: &[&F_0100000400000000, &F_0100400000000004] },
    Class { key: 0x9e334029caf6f0ac, name: "s-9e334029caf6f0ac", stages: &[&F_0100000000002211, &F_0100000800000000, &F_0100400800000000, &F_0100608008000003] },
    Class { key: 0xd68b5fd8aa369b2e, name: "s-d68b5fd8aa369b2e", stages: &[&F_0100000800000000, &F_0100400000002214] },
    Class { key: 0xd798691931177753, name: "s-d798691931177753", stages: &[&F_0100000400002210, &F_0100100800000000] },
    Class { key: 0xfcd58b4913eb5418, name: "s-fcd58b4913eb5418", stages: &[&F_0100000400000000, &F_0100400000002214] },
];

const F_0100000000002211: StageClass = stage::<0x0100000000002211>();
const F_0100000000002212: StageClass = stage::<0x0100000000002212>();
const F_0100000400000000: StageClass = stage::<0x0100000400000000>();
const F_0100000400000210: StageClass = stage::<0x0100000400000210>();
const F_0100000400002210: StageClass = stage::<0x0100000400002210>();
const F_0100000800000000: StageClass = stage::<0x0100000800000000>();
const F_0100100800000000: StageClass = stage::<0x0100100800000000>();
const F_0100400000000004: StageClass = stage::<0x0100400000000004>();
const F_0100400000002214: StageClass = stage::<0x0100400000002214>();
const F_0100400800000000: StageClass = stage::<0x0100400800000000>();
const F_0100408400000003: StageClass = stage::<0x0100408400000003>();
const F_0100608008000003: StageClass = stage::<0x0100608008000003>();
const F_0100610418002210: StageClass = stage::<0x0100610418002210>();
