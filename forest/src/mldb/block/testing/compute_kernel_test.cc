/* compute_kernel_test.cc
   Jeremy Barnes, 29 June 2011
   Copyright (c) 2011 mldb.ai inc.
   Copyright (c) 2011 Jeremy Barnes.

   This file is part of MLDB. Copyright 2015 mldb.ai inc. All rights reserved.
   Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE
*/

#define BOOST_TEST_MAIN
#define BOOST_TEST_DYN_LINK

#include "mldb/vfs/filter_streams.h"
#include "mldb/block/content_descriptor.h"
#include "mldb/types/value_description.h"
#include "mldb/types/json.h"

#include <boost/test/unit_test.hpp>

using namespace std;
using namespace MLDB;

using boost::unit_test::test_suite;

BOOST_AUTO_TEST_CASE( test_kernel )
{
}
