/** randomforest.cc                                             -*- C++ -*-
    Mathieu Marquis Bolduc, 11 Mars 2016
    Copyright (c) 2016 mldb.ai inc.  All rights reserved.

    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.
*/

#include "randomforest.h"
#include "randomforest_kernels.h"
#include "mldb/block/compute_kernel.h"
#include "mldb/types/annotated_exception.h"
#include "mldb/types/basic_value_descriptions.h"
#include "mldb/types/map_description.h"
#include "mldb/plugins/jml/dataset_feature_space.h"
#include "mldb/utils/fixed_point_accum.h"
#include "mldb/plugins/jml/jml/tree.h"
#include "mldb/plugins/jml/jml/stump_training_bin.h"
#include "mldb/base/parallel.h"
#include "mldb/base/map_reduce.h"
#include "mldb/base/thread_pool.h"
#include "mldb/core/bucket.h"
#include "mldb/utils/lightweight_hash.h"
#include "mldb/arch/timers.h"
#include "mldb/utils/environment.h"
#include "mldb/types/span_description.h"
#include "mldb/arch/ansi.h"
#include <any>

using namespace std;

namespace MLDB {
namespace RF {

// compareTrees: in randomforest.rs.

// PartitionData's constructor and clear: in randomforest.rs.

PartitionData PartitionData::
reweightAndCompact(const std::vector<uint8_t> & counts,
                   size_t numNonZero,
                   double scale,
                   MappedSerializer & serializer) const
{
    PartitionData data;
    data.features = this->features;
    data.fs = this->fs;

    ExcAssertEqual(counts.size(), rows.rowCount());
    
    size_t chunkSize = std::min<size_t>(100000, rows.rowCount() / numCpus() / 4);
    chunkSize += (chunkSize == 0);

    // Analyze the weights.  This may allow us to store them in a lot
    // less bits than we would have otherwise.

    auto doWeightChunk = [=] (int chunk)
        -> std::tuple<LightweightHashSet<float> /* uniques */,
                        float /* minWeight */,
                        size_t /* numValues */>
        {
            size_t start = chunk * chunkSize;
            size_t end = std::min(start + chunkSize, rows.rowCount());

            LightweightHashSet<float> uniques;
            float minWeight = INFINITY;
            size_t numValues = 0;
            
            for (size_t i = start;  i < end;  ++i) {
                float weight = rows.getWeight(i) * counts[i] * scale;
                ExcAssert(!std::isnan(weight));
                ExcAssertGreaterEqual(weight, 0);
                if (weight != 0) {
                    numValues += 1;
                    uniques.insert(weight);
                    minWeight = std::min(minWeight, weight);
                }
            }

            return std::make_tuple(std::move(uniques), minWeight,
                                    numValues);
        };

    float minWeight = INFINITY;
    LightweightHashSet<float> allUniques;
    size_t totalNumValues = 0;
    
    auto reduceWeights = [&] (size_t start,
                                std::tuple<LightweightHashSet<float> /* uniques */,
                                float /* minWeight */,
                                size_t /* numValues */> & info)
        {
            minWeight = std::min(minWeight, std::get<1>(info));
            totalNumValues += std::get<2>(info);
            LightweightHashSet<float> & uniques = std::get<0>(info);
            allUniques.insert(uniques.begin(), uniques.end());
        };
    
    parallelMapInOrderReduce
        (0, rows.rowCount() / chunkSize + 1, doWeightChunk, reduceWeights);
    std::vector<float> uniqueWeights(allUniques.begin(), allUniques.end());
    std::sort(uniqueWeights.begin(), uniqueWeights.end());
    
    using namespace std;
    
    bool integerUniqueWeights = true;
    int maxIntFactor = 0;
    for (float w: uniqueWeights) {
        float floatMult = w / minWeight;
        int intMult = round(floatMult);
        bool isInt = intMult * minWeight == w;//floatMult == intMult;
        if (!isInt && false) {
            cerr << "intMult = " << intMult << " floatMult = "
                    << floatMult << " intMult * minWeight = "
                    << intMult * minWeight << endl;
            integerUniqueWeights = false;
        }
        else maxIntFactor = intMult;
    }

    cerr << "total of " << uniqueWeights.size() << " unique weights"
            << endl;
    cerr << "integerUniqueWeights = " << integerUniqueWeights << endl;
    cerr << "maxIntFactor = " << maxIntFactor << endl;

    int numWeightsToEncode = 0;
    if (integerUniqueWeights) {
        data.rows.weightEncoder.weightFormat = WF_INT_MULTIPLE;
        data.rows.weightEncoder.weightMultiplier = minWeight;
        data.rows.weightEncoder.weightBits = MLDB::highest_bit(maxIntFactor, -1) + 1;
    }
    else if (uniqueWeights.size() < 4096) { /* max 16kb of cache */
        data.rows.weightEncoder.weightFormat = WF_TABLE;
        auto mutableWeightFormatTable
            = serializer.allocateWritableT<float>(uniqueWeights.size());
        std::memcpy(mutableWeightFormatTable.data(),
                    uniqueWeights.data(),
                    uniqueWeights.size() * sizeof(float));
        data.rows.weightEncoder.weightFormatTable = serializer.freeze(mutableWeightFormatTable);
        data.rows.weightEncoder.weightBits = MLDB::highest_bit(numWeightsToEncode, -1) + 1;
    }
    else {
        data.rows.weightEncoder.weightFormat = WF_FLOAT;
        data.rows.weightEncoder.weightBits = 32;
    }


    // We split the rows up into tranches to increase parallism
    // To do so, we need to know how many items are in each
    // tranche and where its items start
    size_t numTranches = std::min<size_t>(8, rows.rowCount() / 1024);
    if (numTranches == 0)
        numTranches = 1;
    //numTranches = 1;
    size_t numPerTranche = rows.rowCount() / numTranches;

    // Find the splits such that each one has a multiple of 64
    // non-zero entries (this is a requirement to write them
    // from multiple threads).
    std::vector<size_t> trancheSplits;
    std::vector<size_t> trancheCounts;
    std::vector<size_t> trancheOffsets;
    size_t start = 0;
    size_t offset = 0;

    while (offset < numNonZero) {
        trancheSplits.push_back(start);
        size_t n = 0;
        size_t end = start;
        for (; end < rows.rowCount()
                    && (end < start + numPerTranche
                        || n == 0
                        || n % 64 != 0);  ++end) {
            n += counts[end] != 0;
        }
        trancheCounts.push_back(n);
        trancheOffsets.push_back(offset);
        offset += n;
        start = end;
        ExcAssertLessEqual(start, rows.rowCount());
    }
    ExcAssertEqual(offset, numNonZero);
    trancheOffsets.push_back(offset);
    trancheSplits.push_back(start);

    // Get a contiguous block of memory for all of the feature blocks;
    // this enables a single device transfer and a single device argument
    // list (for when we do devices)
    std::vector<size_t> bucketMemoryOffsets(1, 0);
    size_t bucketMemoryRequired = 0;

    for (int f = 0;  f < features.size();  ++f) {
        size_t bytesRequired = 0;
        if (data.features[f].active) {
            size_t wordsRequired
                = WritableBucketList::wordsRequired
                    (numNonZero,
                        data.features[f].info->distinctValues);
            bytesRequired = wordsRequired * 4;
        }
        bucketMemoryRequired += bytesRequired;
        bucketMemoryOffsets.push_back(bucketMemoryRequired);
    }

    MutableMemoryRegionT<uint32_t> mutableBucketMemory
        = serializer.allocateWritableT<uint32_t>
        (bucketMemoryRequired / 4, 4096 /* page aligned */);

    auto myRange = mutableBucketMemory.rangeBytes(0, bucketMemoryRequired);

    ExcAssertEqual(myRange.length(),  bucketMemoryRequired / 4);

    // This gets called for each feature.  It's further subdivided
    // per tranche.
    auto doFeature = [&] (size_t f)
        {
            if (f == data.features.size()) {
                // Do the row index
                size_t n = 0;

                RowWriter writer
                    = data.rows.getRowWriter(numNonZero, numNonZero,
                                             serializer,
                                             false /* sequential example num */);

                Rows::RowIterator rowIterator
                    = rows.getRowIterator();

                for (size_t i = 0;  i < rows.rowCount();  ++i) {
                    DecodedRow row = rowIterator.getDecodedRow();
                    if (counts[i] == 0 || row.weight == 0)
                        continue;
                    writer.addRow(row.label,
                                  row.weight * counts[i] * scale,
                                  n++);
                }
                
                ExcAssertEqual(n, numNonZero);
                data.rows = writer.freeze(serializer);
                return;
            }

            if (!data.features[f].active)
                return;

            auto mem
                = mutableBucketMemory
                    .rangeBytes(bucketMemoryOffsets[f],
                                bucketMemoryOffsets[f + 1]);
            
            ParallelWritableBucketList featureBuckets
                (numNonZero,
                data.features[f].info->distinctValues,
                mem);

            auto onTranche = [&] (size_t tr)
            {
                size_t start = trancheSplits[tr];
                size_t end = trancheSplits[tr + 1];
                size_t offset = trancheOffsets[tr];
                
                auto writer = featureBuckets.atOffset(offset);

                size_t n = 0;
                for (size_t i = start;  i < end;  ++i) {
                    if (counts[i] == 0)
                        continue;
                    
                    uint32_t bucket = features[f].buckets[rows.getExampleNum(i)];
                    //ExcAssertLess(bucket, features[f].info->distinctValues);
                    writer.write(bucket);
                    ++n;
                }
                
                ExcAssertEqual(n, trancheCounts[tr]);
            };

            parallelMap(0, numTranches, onTranche);

            data.features[f].buckets = featureBuckets.freeze(serializer);
        };

    MLDB::parallelMap(0, data.features.size() + 1, doFeature);

    data.bucketMemory = serializer.freeze(mutableBucketMemory);

    for (size_t i = 0;  i < data.features.size();  ++i) {
        if (features[i].active) {
            ExcAssertGreaterEqual(data.features[i].buckets.storage.data(),
                                    data.bucketMemory.data());
        }
    }
    
    return data;
}

// PartitionData::splitWithoutReindex, splitAndReindex, split and train: in randomforest.rs.

// FeatureSamplingTrainerKernel, FeatureSamplingTrainerPartition (allocate, init, scheduleTraining, finish),
// trainPartitioned, trainPartitionedEndToEnd, PartitionData::trainPartitioned and trainMultipleSamplings: in
// randomforest.rs.

} // namespace RF
} // namespace MLDB
