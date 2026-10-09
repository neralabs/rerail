new (Tab.mixins.Controlled('MemberComments',Tab.mixins.Templated('<div mbr-comment-tab=\${a}></div>',Tab.mixins.AlwaysHasData())))

new (Tab.mixins.Controlled('MemberComments',Tab.mixins.Templated('<div mbr-comment-tab=\${b}></div>',Tab.mixins.AlwaysHasData('\${c}'))))

Tab.mixins.Templated('<div mbr-comment-tab=\${which}></div>',Tab.mixins.AlwaysHasData('\\${d}'))

Tab.mixins.Templated(`<div mbr-comment-tab=\${e}></div>`,Tab.mixins.AlwaysHasData('${f}'))

Tab.mixins.Templated('<div mbr-comment-tab=${g}></div>',Tab.mixins.AlwaysHasData('${h}'), Tab.mixin.SomeMethod('${i}'))

new (Tab.mixins.Controlled('MemberComments',Tab.mixins.Templated('<div mbr-comment-tab=\\${j}></div>',Tab.mixins.AlwaysHasData())))

new (Tab.mixins.Templated('<div mbr-comment-tab=\\\\\${b}></div>',Tab.mixins.AlwaysHasData('\\\\${c}'))))

"\${a} = ${a}";

"One plus one is ${1 + 1}.";

'One plus one is ${1 + 1}.';

`One plus one is ${1 + 1}.`;

"${";

"${" + '}';

"${() => {}} ${abc}"

