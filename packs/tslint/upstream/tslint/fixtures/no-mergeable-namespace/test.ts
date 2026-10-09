// valid case
var foo;
namespace foo{
    namespace foo {}
}
declare module buzz {}
module bar {}

// invalid case
namespace a {}
namespace a {}

namespace b {
    namespace b {}
    namespace b {}
    namespace b {}
}

namespace x.y {}
namespace x { namespace y{} }

namespace m.n {
    namespace l {}
    namespace l {}
}

module "foo" {
    // Different than outer 'a'
    namespace a {}

    namespace q {}
    namespace q {}
}

