/* compute_promise_test.cc
   Jeremy Barnes, 29 June 2011
   Copyright (c) 2011 mldb.ai inc.
   Copyright (c) 2011 Jeremy Barnes.

   This file is part of MLDB. Copyright 2015 mldb.ai inc. All rights reserved.
   Used here under the Apache License, Version 2.0: MLDB's license is https://github.com/mldbai/mldb/blob/e3caed6fa44782d08eb640702fd60e9a2576d939/LICENSE
*/

#define BOOST_TEST_MAIN
#define BOOST_TEST_DYN_LINK

#include "mldb/block/compute_kernel.h"
#include "mldb/block/compute_kernel_host.h"

#include <boost/test/unit_test.hpp>

using namespace std;
using namespace MLDB;

using boost::unit_test::test_suite;

struct AsyncComputeEvent: public ComputeEvent {
    virtual ~AsyncComputeEvent() = default;

    virtual std::shared_ptr<ComputeProfilingInfo> getProfilingInfo() const override
    {
        return std::make_shared<ComputeProfilingInfo>();
    }

    virtual void await() const override
    {
    }

    virtual std::shared_ptr<ComputeEvent> thenImpl(std::function<void ()> fn) override
    {
        fn();
        return std::make_shared<HostComputeEvent>();
    }
};


BOOST_AUTO_TEST_CASE( test_default_construct )
{
    ComputePromise promise;
    BOOST_CHECK_EQUAL(promise.valid(), false);
    BOOST_CHECK_THROW(promise.event(), MLDB::Exception);
}
