/** randomforest_kernels_common.h                                     -*- C++ -*-
    Jeremy Barnes, 13 October 2018
    Copyright (c) 2018 mldb.ai inc.  All rights reserved.
    This file is part of MLDB. Copyright 2016 mldb.ai inc. All rights reserved.

    Common files for OpenCL kernels.
*/

#include "mldb/block/ukl/ukl_shim.h"

#define W W32

static const __constant float VAL_2_HL = 1.0f * (1UL << 30);
static const __constant float HL_2_VAL = 1.0f / (1UL << 30);
static const __constant float ADD_TO_ROUND = 0.5f / (1UL << 30);

UKL_FUNCTION(v1,uint32_t,createMask32)(uint32_t numBits)
{
    return numBits >= 32 ? -1 : (((uint32_t)1 << numBits) - 1);
}

UKL_FUNCTION(v1,uint64_t,createMask64)(uint32_t numBits)
{
    return numBits >= 64 ? -1 : (((uint64_t)1 << numBits) - 1);
}

UKL_FUNCTION(v1,uint64_t,extractBitRange64)(ROBUFFER(uint64_t) data,
                                  uint32_t numBits,
                                  uint32_t entryNumber,
                                  uint64_t mask,
                                  uint32_t dataLength)
{
    long bitNumber = numBits * (long)entryNumber;
    //if (bitNumber >= INT_MAX) {
    //    printf("bit number requires > 32 bits: %ld\n", bitNumber); 
    //}
    uint32_t wordNumber = bitNumber >> 6;
    uint32_t wordOffset = bitNumber - (wordNumber << 6);

    if (wordNumber >= dataLength) {
        //printf("OUT OF RANGE WORD %d vs %d\n", wordNumber, dataLength);
        return 0;
    }
    
    //return data[wordNumber];
    
    //printf("reading word number %d in worker %ld\n",
    //       wordNumber, get_global_id(0));
    
    //printf("wordNumber = %d, bitNumber = %d\n", wordNumber, wordOffset);
    
    uint32_t bottomBits = min(numBits, 64 - wordOffset);
    uint32_t topBits = numBits - bottomBits;

    //printf("numBits = %d, bottomBits = %d, topBits = %d\n",
    //       numBits, bottomBits, topBits);
    
    //uint64_t mask = createMask64(numBits);

    //printf("mask = %016lx\n", mask);
    
    uint64_t val = data[wordNumber];

    //printf("val = %016lx\n", val);

    val >>= wordOffset;

    if (topBits > 0) {
        uint64_t val2 = data[wordNumber + 1];
        val = val | val2 << bottomBits;
    }
    val = val & mask;
    //printf("val out = %016lx\n", val);
    return val;
}

UKL_FUNCTION(v1,uint16_t,extract16BitRange32)(ROBUFFER(uint32_t) data,
                                    uint16_t numBits,
                                    uint32_t entryNumber)
{
    uint32_t bitNumber = numBits * entryNumber;
    uint32_t wordNumber = bitNumber / 32;
    uint16_t wordOffset = bitNumber % 32;

    uint16_t bottomBits = min((uint16_t)numBits, (uint16_t)(32 - wordOffset));
    uint16_t topBits = numBits - bottomBits;

    if (topBits == 0) {
        return extract_bits(data[wordNumber], wordOffset, bottomBits);
    }
    else {
        uint2 words = CAST_ROBUFFER(data + wordNumber, uint2)[0];
        uint16_t bottom = extract_bits(ukl_vec_el(words, x), wordOffset, bottomBits);
        return bottom | extract_bits(ukl_vec_el(words, y), 0, topBits) << bottomBits;
    }
}

typedef enum WeightFormat {
    WF_INT_MULTIPLE,  ///< Stored as an int multiplied by a constant
    WF_TABLE,         ///< Stored as an index into a fixed table
    WF_FLOAT          ///< Stored directly as a 32 bit float
} WeightFormat;

UKL_FUNCTION(v1,float,decodeWeight)(uint32_t bits, WeightFormat floatEncoding, float baseMultiplier,
                   ROBUFFER(float) table)
{
    // Upstream forced this branch (`|| true`), so rows with table or float weights decoded as garbage on the
    // device.
    if (floatEncoding == WF_INT_MULTIPLE) {
        return bits * baseMultiplier;
    }
    else if (floatEncoding == WF_FLOAT) {
        return reinterpretIntAsFloat(bits);
    }
    else if (floatEncoding == WF_TABLE) {
        return table[bits];
    }
    else return INFINITY;
}

typedef struct DecodedRow {
    uint32_t example;
    float weight;
    bool label;
} DecodedRow;

UKL_FUNCTION(v1,DecodedRow,getDecodedRow)(uint32_t rowNumber,

                   ROBUFFER(uint64_t) rowData,
                   uint32_t rowDataLength,
                   uint32_t totalBits,
                   uint32_t weightBits,
                   uint32_t exampleBits,
                   uint32_t numRows,

                   WeightFormat weightEncoding,
                   float weightMultiplier,
                   ROBUFFER(float) weightTable,
                   
                   uint64_t mask,
                   uint32_t exampleMask,
                   uint32_t weightMask,
                   uint64_t labelMask)
{
    //*example = rowNumber;
    //*weight = weightMultiplier;
    //*label = (rowNumber % 4 == 0);
    //return;
    uint64_t bits = extractBitRange64(rowData, totalBits, rowNumber, mask,
                                      rowDataLength);
    //printf("rowNum %d bits = %016lx weightBits = %d exampleBits = %d\n",
    //       rowNumber, bits, weightBits, exampleBits);
    //printf("exampleMask = %016lx example = %016lx\n",
    //       createMask64(exampleBits),
    //       bits & createMask64(exampleBits));
    DecodedRow result;
    result.example = exampleBits == 0 ? rowNumber : bits & exampleMask;
    result.weight = decodeWeight((bits >> exampleBits) & weightMask,
                                 weightEncoding, weightMultiplier, weightTable);
    result.label = (bits & labelMask) != 0;
    return result;
}

UKL_FUNCTION(v1,uint16_t,getBucket)(uint32_t exampleNum,
                          ROBUFFER(uint32_t) bucketData,
                          uint32_t bucketDataLength,
                          uint32_t bucketBits,
                          uint32_t numBuckets)
{
    return extract16BitRange32(bucketData, bucketBits, exampleNum);
}

typedef struct W32 {
    int32_t vals[2];
    int32_t count;
} W32;

typedef struct W32A {
    atomic_int vals[2];
    atomic_int count;
} W32A;

typedef W32A WAtomic;

// Struct with an extra index field used for sorting (local only)
typedef struct WIndexed {
    W w;
    int32_t index;
} WIndexed;

#define W_INIT { { 0, 0}, 0 }
#define W_INDEXED_INIT { W_INIT, 0 }

UKL_FUNCTION(v1,void,zeroW)(RWLOCAL(W) w)
{
    w->vals[0] = 0;
    w->vals[1] = 0;
    w->count = 0;
}

UKL_FUNCTION(v1,int32_t,encodeW)(float f)
{
    int32_t result = (f + ADD_TO_ROUND) * VAL_2_HL;
    //printf("encoding %g (%08x) to %g (%08x) to %ld, ADD_TO_ROUND = %g, VAL_2_HL = %g\n",
    //       f, as_int(f),
    //       (f + ADD_TO_ROUND) * VAL_2_HL, as_int((f + ADD_TO_ROUND) * VAL_2_HL),
    //       result, ADD_TO_ROUND, VAL_2_HL);
    return result;
}

UKL_FUNCTION(v1,float,decodeW)(int32_t v)
{
    return v * HL_2_VAL;
}

UKL_FUNCTION(v1,float,decodeWf)(int32_t v)
{
    return v * HL_2_VAL;
}

UKL_FUNCTION(v1,void,incrementWLocal)(__local W * wIn, bool label, float weight)
{
    UKL_LOCAL_PTR(WAtomic) w = UKL_LOCAL_PTR_CAST(WAtomic)wIn;
    int32_t inc = encodeW(weight);
    atom_add_local(&w->vals[label ? 1 : 0], inc);
    atom_inc_local(&w->count);
}

UKL_FUNCTION(v1,void,decrementWLocal)(__local W * wIn, bool label, float weight)
{
    UKL_LOCAL_PTR(WAtomic) w = UKL_LOCAL_PTR_CAST(WAtomic)wIn;
    int32_t inc = encodeW(weight);
    ukl_assert(wIn->count > 0);
    atom_sub_local(&w->vals[label ? 1 : 0], inc);
    atom_dec_local(&w->count);
}

UKL_FUNCTION(v1,void,incrementWGlobal)(UKL_GLOBAL_PTR(W) wIn, bool label, float weight)
{
    UKL_GLOBAL_PTR(WAtomic) w = UKL_GLOBAL_PTR_CAST(WAtomic)wIn;
    int32_t inc = encodeW(weight);
    atom_add(&w->vals[label ? 1 : 0], inc);
    atom_inc(&w->count);
}

// A row's weights to false and true (decodedRows holds two floats a row): each side's weight added where it is not
// zero, so a classification row (whose weight is all on its label's side) costs what upstream's did.
UKL_FUNCTION(v1,void,addRowLocal)(__local W * wIn, float wFalse, float wTrue)
{
    UKL_LOCAL_PTR(WAtomic) w = UKL_LOCAL_PTR_CAST(WAtomic)wIn;
    if (wFalse != 0.0f)
        atom_add_local(&w->vals[0], encodeW(wFalse));
    if (wTrue != 0.0f)
        atom_add_local(&w->vals[1], encodeW(wTrue));
    atom_inc_local(&w->count);
}

UKL_FUNCTION(v1,void,addRowGlobal)(UKL_GLOBAL_PTR(W) wIn, float wFalse, float wTrue)
{
    UKL_GLOBAL_PTR(WAtomic) w = UKL_GLOBAL_PTR_CAST(WAtomic)wIn;
    if (wFalse != 0.0f)
        atom_add(&w->vals[0], encodeW(wFalse));
    if (wTrue != 0.0f)
        atom_add(&w->vals[1], encodeW(wTrue));
    atom_inc(&w->count);
}

UKL_FUNCTION(v1,void,incrementWOut)(UKL_GLOBAL_PTR(W) wOut_, __local const W * wIn)
{
    UKL_GLOBAL_PTR(WAtomic) wOut = UKL_GLOBAL_PTR_CAST(WAtomic)wOut_;
    if (wIn->count == 0)
        return;
    if (wIn->vals[0] != 0)
        atom_add(&wOut->vals[0], wIn->vals[0]);
    if (wIn->vals[1] != 0)
        atom_add(&wOut->vals[1], wIn->vals[1]);
    atom_add(&wOut->count,   wIn->count);
}

UKL_FUNCTION(v1,void,decrementWOutGlobal)(UKL_GLOBAL_PTR(W) wOut_, UKL_GLOBAL_PTR(const W) wIn)
{
    UKL_GLOBAL_PTR(WAtomic) wOut = UKL_GLOBAL_PTR_CAST(WAtomic)wOut_;
    if (wIn->count == 0)
        return;
    ukl_assert_greater_equal(wOut->count, wIn->count);
    if (wIn->vals[0] != 0)
        atom_sub(&wOut->vals[0], wIn->vals[0]);
    if (wIn->vals[1] != 0)
        atom_sub(&wOut->vals[1], wIn->vals[1]);
    atom_sub(&wOut->count, wIn->count);
}

// Take a bit-compressed representation of rows, and turn it into a
// decompressed version with one float per row (the sign gives the
// label, and the magnitude gives the weight).
//
// This is a 1D kernel over the array of rows, not very complicated

UKL_FUNCTION(v1,void,
decompressRowsImpl)(ROBUFFER(uint64_t) rowData,
                         uint32_t rowDataLength,
                         uint16_t weightBits,
                         uint16_t exampleNumBits,
                         uint32_t numRows,
                     
                         WeightFormat weightFormat,
                         float weightMultiplier,
                         ROBUFFER(float) weightData,
                         uint32_t numTargets,
                         ROBUFFER(float) targets,
                     
                         RWBUFFER(float) decodedRowsOut,
                         uint32_t workerId, uint32_t numWorkers)
{
    uint32_t totalBits = weightBits + exampleNumBits + 1;  // 1 is for the label

    uint64_t mask = createMask64(totalBits);
    uint32_t exampleMask = createMask32(exampleNumBits);
    uint32_t weightMask = createMask32(weightBits);
    // The label is above the weight and example bits, which with 32 weight bits (WF_FLOAT) is past bit 31: upstream
    // shifted a 32 bit 1, so those labels were lost.
    uint64_t labelMask = ((uint64_t)1 << (weightBits + exampleNumBits));

    for (uint32_t rowNumber = workerId;  rowNumber < numRows; rowNumber += numWorkers) {

        DecodedRow row = getDecodedRow(rowNumber, rowData, rowDataLength,
                      totalBits, weightBits, exampleNumBits, numRows,
                      weightFormat, weightMultiplier, weightData,
                      mask, exampleMask, weightMask, labelMask);

        // The row's weights to false and true: its label's side has its weight, or a regression row's target t
        // (by example) gives w (1 - t) and w t.
        float t = numTargets != 0 ? targets[row.example] : (row.label ? 1.0f : 0.0f);
        // One rounding (as the host's `row_weights`): fast math may not contract it differently.
        decodedRowsOut[2 * rowNumber] = fma(-row.weight, t, row.weight);
        decodedRowsOut[2 * rowNumber + 1] = row.weight * t;
    }
}

uint32_t testRow(uint32_t rowId,
                 ROBUFFER(float) rowData,
                 uint32_t numRows,
                 
                 ROBUFFER(uint32_t) bucketData,
                 uint32_t bucketDataLength,
                 uint32_t bucketBits,
                 uint32_t numBuckets,

                 RWLOCAL(W) w,
                 RWBUFFER(W) wOut,
                 uint32_t maxLocalBuckets);

UKL_FUNCTION(v1,void,printW)(RWBUFFER(W) w)
{
    //printf("{\"v\":[%f,%f],\"c\":%d}", decodeWf(w->vals[0]), decodeWf(w->vals[1]), w->count);
}

UKL_FUNCTION(v1,uint32_t,testRow)
                (uint32_t rowId,
                 ROBUFFER(float) rowData,
                 uint32_t numRows,
                 
                 ROBUFFER(uint32_t) bucketData,
                 uint32_t bucketDataLength,
                 uint32_t bucketBits,
                 uint32_t numBuckets,

                 RWLOCAL(W) w,
                 RWBUFFER(W) wOut,
                 uint32_t maxLocalBuckets)
{
    uint32_t exampleNum = rowId;
    float wFalse = rowData[2 * rowId];
    float wTrue = rowData[2 * rowId + 1];
    //int f = get_global_id(0);
    uint16_t bucket
        = getBucket(exampleNum, bucketData, bucketDataLength,
                    bucketBits, numBuckets);

#if 0
    if (bucket >= numBuckets) {
        printf("ERROR BUCKET NUMBER: got %d numBuckets %d row %d feature %d\n",
               bucket, numBuckets, rowId, (int)get_global_id(0));
        return 0;
    }
#endif

    //if (f == 0 && bucket == 1) {
    //    printf("feat %d row %d bucket %d of %d weight %f label %d\n",
    //            f, rowId, bucket, numBuckets, weight, label);
    //}

    if (bucket < maxLocalBuckets) {
        addRowLocal(&w[bucket], wFalse, wTrue);
    }
    else {
        addRowGlobal(&wOut[bucket], wFalse, wTrue);
    }

    return bucket;
}

SYNC_FUNCTION(v1,
void, testFeatureImpl)
                 (ROBUFFER(float) decodedRows,
                  uint32_t numRows, 
                  ROBUFFER(uint32_t) bucketData,
                  ROBUFFER(uint32_t) bucketDataOffsets,
                  ROBUFFER(uint32_t) bucketNumbers,
                  ROBUFFER(uint32_t) bucketEntryBits,
 
                  ROBUFFER(uint32_t) activeFeatureList,
 
                  RWLOCAL(W) w,
                  uint32_t maxLocalBuckets,

                  RWBUFFER(W) partitionBuckets,

                  uint32_t featureIndex,
                  uint32_t threadGroupId, uint32_t threadGroupSize,
                  uint32_t gridId, uint32_t gridSize)
{
    const uint32_t f = activeFeatureList[featureIndex];

    uint32_t bucketDataOffset = bucketDataOffsets[f];
    uint32_t bucketDataLength = bucketDataOffsets[f + 1] - bucketDataOffset;
    uint32_t numBuckets = bucketNumbers[f + 1] - bucketNumbers[f];
    bucketData += bucketDataOffset;
    uint32_t bucketBits = bucketEntryBits[f];

    RWBUFFER(W) wOut = partitionBuckets + bucketNumbers[f];

    //maxLocalBuckets = 0;  // TODO DEBUG ONLY

    for (uint32_t i = threadGroupId;  i < numBuckets && i < maxLocalBuckets;  i += threadGroupSize) {
        zeroW(w + i);
    }

    ukl_threadgroup_barrier();

    for (uint32_t rowId = gridId;  rowId < numRows;  rowId += gridSize) {
        //if (f == 0 && workGroupId == 0) {
        //    printf("testing row %d\n", rowId);
        //}
        testRow(rowId,
                decodedRows, numRows,
                bucketData, bucketDataLength, bucketBits, numBuckets,
                w, wOut, maxLocalBuckets);
    }

    ukl_threadgroup_barrier();
    
    for (uint32_t i = threadGroupId;  i < numBuckets && i < maxLocalBuckets;
         i += threadGroupSize) {
        incrementWOut(&wOut[i], &w[i]);
    }

    SYNC_RETURN();
}

// A categorical split's buckets on the left (CART's subset split), a bit per bucket (at most 512 buckets).
#define CATEGORY_SET_WORDS 16
#define MAX_CATEGORICAL_BUCKETS 512

typedef struct PartitionSplit {
    float score;
    int16_t feature;
    int16_t value;      // An ordinal split's bucket; a categorical split's number of buckets on the left
    W left;
    W right;
    uint32_t subset[CATEGORY_SET_WORDS];  // A categorical split's buckets on the left
} PartitionSplit;

#define CATEGORY_SET_INIT { 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0 }
#define PARTITION_SPLIT_INIT { INFINITY, -1, -1, W_INIT, W_INIT, CATEGORY_SET_INIT }

typedef struct IndexedPartitionSplit {
    float score;
    int16_t feature;
    int16_t value;
    W left;
    W right;
    uint32_t subset[CATEGORY_SET_WORDS];
    uint32_t index;
} IndexedPartitionSplit;

#define INDEXED_PARTITION_SPLIT_INIT { INFINITY, -1, -1, W_INIT, W_INIT, CATEGORY_SET_INIT, 0 }

UKL_FUNCTION(v1,bool,inCategorySet)(ROBUFFER(uint32_t) set, uint32_t b)
{
    return b < MAX_CATEGORICAL_BUCKETS && ((set[b >> 5] >> (b & 31)) & 1) != 0;
}

// CART's order of a categorical feature's buckets: by their mean target (the true side's share of the weight),
// the empty buckets last, ties by bucket number. Exact, in the fixed point: t_a (f_b + t_b) < t_b (f_a + t_a), as
// the host's `category_before`.
UKL_FUNCTION(v1,bool,categoryBefore)(W wa, uint32_t a, W wb, uint32_t b)
{
    if ((wa.count == 0) != (wb.count == 0))
        return wb.count == 0;
    if (wa.count == 0)
        return a < b;
    int64_t fa = wa.vals[0], ta = wa.vals[1], fb = wb.vals[0], tb = wb.vals[1];
    int64_t l = ta * (fb + tb), r = tb * (fa + ta);
    return l < r || (l == r && a < b);
}

UKL_FUNCTION(v1,bool,partitionSplitDirection)(PartitionSplit split)
{
    return split.feature != -1 && split.left.count < split.right.count;
}

UKL_FUNCTION(v1,bool,indexedPartitionSplitDirection)(IndexedPartitionSplit split)
{
    return split.feature != -1 && split.left.count < split.right.count;
}

UKL_FUNCTION(v1,bool,partitionSplitDirectionGlobal)(ROBUFFER(PartitionSplit) split)
{
    return partitionSplitDirection(*split);
}

UKL_FUNCTION(v1,float,nextFloat)(float val, int32_t n)
{
    int32_t ival = reinterpretFloatAsInt(val);
    ival += n;
    return reinterpretIntAsFloat(ival);
}

// Stable SQRT: take the largest value which when squared is less than or equal to the desired value
UKL_FUNCTION(v1,float,sqrt3)(float val)
{
    constexpr float SQRT3_MULT_BEFORE = 65536.0f;
    constexpr float SQRT3_MULT_AFTER  = 1.0f / 256.0f;

    val *= SQRT3_MULT_BEFORE;
    float approx = nextFloat(sqrt(val), 3);
    while (approx * approx > val) {
        approx = nextFloat(approx, -1);
    }
    return approx * SQRT3_MULT_AFTER;
}

// SplitCriterion: upstream's score, or the variance (regression; the Gini impurity for classification).
#define SPLIT_CRITERION_SQRT 0
#define SPLIT_CRITERION_VARIANCE 1

// A side's squared error about its mean target, less what every split shares: f t / (f + t) in the accumulators'
// fixed point, by integer division (exact, as the host's `variance_fixed`; the device's float division is not).
UKL_FUNCTION(v1,int64_t,varianceFixed)(W w)
{
    int64_t f = w.vals[0], t = w.vals[1];
    return f + t <= 0 ? 0 : f * t / (f + t);
}

UKL_FUNCTION(v1,float,scoreSplit)(W wFalse, W wTrue, uint32_t criterion);

#if 1
UKL_FUNCTION(v1,float,scoreSplit)(W wFalse, W wTrue, uint32_t criterion)
{
    if (criterion == SPLIT_CRITERION_VARIANCE)
        return (float)(varianceFixed(wFalse) + varianceFixed(wTrue)) * HL_2_VAL;
    float arg1 = decodeWf(wFalse.vals[0]) * decodeWf(wFalse.vals[1]);
    float arg2 = decodeWf(wTrue.vals[0])  * decodeWf(wTrue.vals[1]);
    float sr1 = sqrt3(arg1);
    float sr2 = sqrt3(arg2);
    float score = 2.0f * (sr1 + sr2);
    return score;
};
#endif

UKL_FUNCTION(v1,void,incrementW)(__local W * out, __local const W * in);

UKL_FUNCTION(v1,void,incrementW)(__local W * out, __local const W * in)
{
    if (in->count == 0)
        return;
    out->vals[0] += in->vals[0];
    out->vals[1] += in->vals[1];
    out->count += in->count;
}

UKL_FUNCTION(v1,float,scoreSplitWAll)(W wTrue, W wAll, uint32_t criterion);

UKL_FUNCTION(v1,float,scoreSplitWAll)(W wTrue, W wAll, uint32_t criterion)
{
    W wFalse = { { wAll.vals[0] - wTrue.vals[0],
                   wAll.vals[1] - wTrue.vals[1] },
                 wAll.count - wTrue.count };
    if (wFalse.count == 0 || wTrue.count == 0)
        return INFINITY;
    return scoreSplit(wFalse, wTrue, criterion);
}

#if 1
UKL_FUNCTION(v1,void,minW)(RWLOCAL(WIndexed) out, RWLOCAL(const WIndexed) in, W wAll, uint32_t criterion);

/* In this function, we identify empty buckets (that should not be scored)
   by marking their index with -1.
*/
UKL_FUNCTION(v1,void,minW)(RWLOCAL(WIndexed) out, RWLOCAL(const WIndexed) in, W wAll, uint32_t criterion)
{
    if (in->w.count == 0 || in->index < 0)
        return;
    if (out->w.count == 0 || out->index < 0) {
        *out = *in;
        return;
    }

    float scoreIn = scoreSplitWAll(in->w, wAll, criterion);
    float scoreOut = scoreSplitWAll(out->w, wAll, criterion);

#if 0
    if (debug) {
        printf("score %d vs %d: %f(%08lx) vs %f(%08lx): (%f,%f,%d) vs (%f,%f,%d): %d\n",
               in->index, out->index, scoreIn, scoreIn, scoreOut, scoreOut,
               decodeW(in->vals[0]), decodeW(in->vals[1]), in->count,
               decodeW(out->vals[0]), decodeW(out->vals[1]), out->count,
               scoreIn >= scoreOut);
    }
#endif

    // No need to check the indexes if less; the way it's structured out will
    // always be on the higher index.
    if (scoreIn <= scoreOut)
        *out = *in;
}
#endif

SYNC_FUNCTION(v1, void, prefixSumW) (RWLOCAL(WIndexed) w, uint32_t n, uint16_t threadIdx, uint16_t numThreads);

#if 1
// Post: start[0] <- start[0] + init
//       start[n] <- sum(start[0...n]) + init
// This is a low latency prefix sum, not work-minimizing
SYNC_FUNCTION(v1,
void, prefixSumW) (RWLOCAL(WIndexed) w, uint32_t n, uint16_t threadIdx, uint16_t numThreads)
{
    // iter 0
    // a b c d e f g h i j k l m n o p

    // iter 1
    // 0 0+1 2 2+3 4 4+5 6 6+7 8 8+9 10 10+11 12 12+13 14 14+15
    // a a+b c c+d e e+f g g+h i i+j  k   k+l   m  m+n  o   o+p
    //     0     1     2     3     4        5        6        7
    
    // j = 2, blocks of 2, one thread per block, read from 0
    // n odd: n-1 + n
    // n even:      n
    
    // iter 2
    // 0   1   1+2     1+3 4   5   5+6     5+7    
    // a a+b a+b+c a+b+c+d e e+f e+f+g e+f+g+h ...
    //           0       1           2       3
    // 8   9  9+10    9+11 12    13 13+14   13+15
    // i i+j i+j+k i+j+k+l  m   m+n m+n+o m+n+o+p
    //           4       5              6       7
    
    // j = 4, blocks of 4, two threads per block, read from 1
    // n % 4 == 0,1:         n
    // n % 4 == 2,3: n%4+1 + n
    
    // iter 3
    // 0   1     2       3        3+4           3+5           3+6             3+7
    // a a+b a+b+c a+b+c+d  a+b+c+d+e   a+b+c+d+e+f a+b+c+d+e+f+g a+b+c+d+e+f+g+h
    //                              0             1             2               3
    // 8   9    10      11      11+12         11+13         11+14           11+15
    // i i+j i+j+k i+j+k+l  i+j+k+l+m   i+j+k+l+m+n i+j+k+l+m+n+o i+j+k+l+m+n+o+p
    //                              4             5             6               7
    
    // j = 8, blocks of 8, four threads per block, read from 3
    // n % 8 == 0,1,2,3:         n
    // n % 8 == 4,5,6,7: n%8+3 + n
    
    // iter 4, blocks of 16, 8 threads per block, read from 7
    // n % 16 < 8: n
    // n % 16 >= 8: n%16+7+n

    // iter i, j = 2^i, blocks of j, j/2 threads per block, read from j/2-1

    for (uint32_t iter = 0, blockSize = 2;  blockSize < n * 2;  blockSize *= 2, iter += 1) {
        // Each thread has one increment to do.  Here we calculate the index of
        // the result and the index of the argument.

        // If we need more than one round from our warp to process all elements,
        // then do  it
        for (uint32_t i = threadIdx;  i < n / 2;  i += numThreads) {
            // j is the number of elements in the block.
            uint32_t blockNum = i * 2 / blockSize;
            uint32_t threadNumInBlock = i % (blockSize / 2);
            uint32_t blockStart = blockNum * blockSize;
            uint32_t halfBlockSize = blockSize/2;

            // All read from this same one
            uint32_t argIndex = blockStart + halfBlockSize - 1;

            // We add to the second half of the block
            uint32_t resultIndex = blockStart + halfBlockSize + threadNumInBlock;

            if (resultIndex < n) {
#if 0
                if (debug) {
                    printf("b %d = %d + %d = (%f,%f,%d,i%d) + (%f,%f,%d,i%d) = (%f,%f,%d)\n",
                           resultIndex, resultIndex, argIndex,
                           decodeW(w[resultIndex].vals[0]),
                           decodeW(w[resultIndex].vals[1]),
                           w[resultIndex].count, w[resultIndex].index,
                           decodeW(w[argIndex].vals[0]),
                           decodeW(w[argIndex].vals[1]),
                           w[argIndex].count, w[argIndex].index,
                           decodeW(w[resultIndex].vals[0] + w[argIndex].vals[0]),
                           decodeW(w[resultIndex].vals[1] + w[argIndex].vals[1]),
                           w[resultIndex].count + w[argIndex].count
                           );
                }
#endif

                // All threads read the same value (argIndex) and write a different
                // element of the block (resultIndex)
                // RWLOCAL_VALUE turns an expression giving an object with an address into a length
                // one RWLOCAL array with the object.
                incrementW(&w[resultIndex].w, &w[argIndex].w);
            }
        }

        // Make sure everything has finished on this iteration before we do the
        // next one
        ukl_threadgroup_barrier();
    }

    SYNC_RETURN();
}
#endif

SYNC_FUNCTION(v1,
uint32_t, argMinScanW) (RWLOCAL(WIndexed) w, uint32_t n, W wAll, uint32_t criterion, uint16_t threadIdx, uint16_t numThreads);

#if 1
// Return the lowest index of the W array with a score equal to the minimum
// score.  Only get_local_id(0) knows the actual correct result; the result
// from the others should be ignored.
SYNC_FUNCTION(v1,
uint32_t, argMinScanW) (RWLOCAL(WIndexed) w, uint32_t n, W wAll, uint32_t criterion, uint16_t threadIdx, uint16_t numThreads)
{
    if (n == 0)
        SYNC_RETURN(0);

    for (uint32_t iter = 0, blockSize = 2;  blockSize < n * 2;  blockSize *= 2, iter += 1) {
        // If we need more than one round from our warp to process all elements,
        // then do  it
        for (uint32_t i = threadIdx;  i < n / 2;  i += numThreads) {
            // j is the number of elements in the block.
            uint32_t blockNum = i * 2 / blockSize;
            uint32_t threadNumInBlock = i % (blockSize / 2);
            uint32_t blockStart = blockNum * blockSize;
            uint32_t halfBlockSize = blockSize/2;

            // All read from this same one
            uint32_t argIndex = blockStart + halfBlockSize - 1;

            // We add to the second half of the block
            uint32_t resultIndex = blockStart + halfBlockSize + threadNumInBlock;

            if (resultIndex <  n) {
                // Compare them, taking the minimum and pushing it right
                minW(w + resultIndex, w + argIndex, wAll, criterion);
            }
        }

        // Make sure everything has finished on this iteration before we do the
        // next one
        ukl_threadgroup_barrier();
    }

    SYNC_RETURN(w[n - 1].index);
}
#elif 0
uint32_t argMinScanW(RWLOCAL(WIndexed) w, uint32_t n, W wAll)
{
    if (get_local_id(0) == 0) {
        for (uint32_t i = 1;  i < n;  ++i) {
            minW(w, w + i, wAll);
        }
        return w[0].index;
    }
    else {
        return 0;
    }
}
#endif

// Function that uses a single workgroup to scan all of the buckets in a split
SYNC_FUNCTION(v1,
PartitionSplit, chooseSplit)
    (ROBUFFER(W) w,
     uint32_t numBuckets,
     W wAll,
     RWLOCAL(WIndexed) wLocal,
     uint32_t wLocalSize,
     RWLOCAL(WIndexed) wStartBest,  // length 2
     RWLOCAL(WIndexed) wRanked,  // [MAX_CATEGORICAL_BUCKETS]: a categorical feature's buckets in CART's order
     RWLOCAL(uint16_t) ranked,   // [MAX_CATEGORICAL_BUCKETS]: the bucket at each place of that order
     bool ordinal,
     uint32_t criterion,
     uint16_t threadIdx, uint16_t numThreads,
     uint16_t f,
     uint16_t p)
{
    PartitionSplit result = PARTITION_SPLIT_INIT;

    //uint32_t f = get_global_id(1);
    //uint32_t p = get_global_id(2);

    // We need some weight in both true and false for a split to make sense
    if (wAll.vals[0] == 0 || wAll.vals[1] == 0)
        SYNC_RETURN(result);
    
    // Read our W array into shared memory
    
    // If we have more W values than local buckets, we need to
    // do the prefix sum multiple times to fill it up completely

    // Contains the prefix sum from the previous iteration
    RWLOCAL(WIndexed) wStart = wStartBest + 0;

    // Contains the best split from the previous iteration
    RWLOCAL(WIndexed) wBest = wStartBest + 1;
    
    if (!ordinal && numBuckets > MAX_CATEGORICAL_BUCKETS)
        SYNC_RETURN(result);

    if (threadIdx == 0) {
        // Initialize wBest
        {
            wStart->w.vals[0] = wStart->w.vals[1] = wStart->w.count = 0;
            wStart->index = -1;
        }
        wBest->w.vals[0] = wBest->w.vals[1] = wBest->w.count = 0;
        wBest->index = -1;
    }

    // Shouldn't be necessary; just in case
    ukl_threadgroup_barrier();

    //bool debug = false;//(f == 0 && p == 2 && get_global_size(2) == 8);
 
    //using namespace std;
    //cerr << "threadIdx = " << threadIdx << " numThreads = " << numThreads << " numBuckets = " << numBuckets << endl;

    if (!ordinal) {
        // CART: a categorical feature's buckets ranked by mean target (categoryBefore), then scanned as an ordinal
        // feature's are, in one pass: the best prefix of the order is the best subset. The buckets are staged in
        // wRanked, each ranked against all (a bounded loop); then wRanked holds them in rank order, each indexed by
        // its place (-1 for an empty one), and `ranked` the bucket at each place.
        for (uint32_t i = threadIdx;  i < numBuckets;  i += numThreads)
            wRanked[i].w = w[i];
        ukl_threadgroup_barrier();
        for (uint32_t i = threadIdx;  i < numBuckets;  i += numThreads) {
            W wi = wRanked[i].w;
            uint32_t rank = 0;
            for (uint32_t j = 0;  j < numBuckets;  ++j) {
                if (categoryBefore(wRanked[j].w, j, wi, i))
                    ++rank;
            }
            ranked[rank] = i;
        }
        ukl_threadgroup_barrier();
        for (uint32_t k = threadIdx;  k < numBuckets;  k += numThreads) {
            wRanked[k].w = w[ranked[k]];
            wRanked[k].index = wRanked[k].w.count > 0 ? (int32_t)k : -1;
        }
        ukl_threadgroup_barrier();
        SYNC_CALL_4(prefixSumW, wRanked, numBuckets, threadIdx, numThreads);
        SYNC_CALL_6(argMinScanW, wRanked, numBuckets, wAll, criterion, threadIdx, numThreads);
        minW(wBest, wRanked + (numBuckets - 1), wAll, criterion);
        ukl_threadgroup_barrier();
    }

    // An ordinal feature's buckets in order, a block of wLocalSize at a time.
    for (uint32_t startBucket = 0;  ordinal && startBucket < numBuckets;  startBucket += wLocalSize) {

        //if (startBucket != 0) {
        //    printf("f %d p %d startBucket %d\n",
        //           f, p, startBucket);
        //}
        
        uint32_t endBucket = min(startBucket + wLocalSize, numBuckets);
        uint32_t numBucketsInIteration = endBucket - startBucket;
        
        //cerr << "threadIdx = " << threadIdx << " startBucket = " << startBucket << " endBucket = " << endBucket
        //     << "numBucketsInIteration = " << numBucketsInIteration << " numBuckets = " << numBuckets << endl;

        for (uint32_t i = threadIdx;  i < numBucketsInIteration;  i += numThreads) {
            //using namespace std;
            //cerr << "i = " << i << " numBucketsInIteration = " << numBucketsInIteration << endl;
            wLocal[i].w = w[startBucket + i];

            // We don't want buckets with zero count to participate in the
            // scoring, so those get an index of -1 to mark them as empty.
            // They still accumulate their weight, however.
            if (wLocal[i].w.count > 0)
                wLocal[i].index = startBucket + i;
            else wLocal[i].index = -1;

            //if (debug) {
                //printf("bucket %d has count %d index %d\n",
                //       startBucket + i, wLocal[i].count, wLocal[i].index);
            //}
            
            if (i == 0 && startBucket != 0) {
                // Add the previous prefix to the first element
                if (ordinal) {
                    incrementW(&wLocal[0].w, &wStart[0].w);

                    // if (debug) {
                    //     printf("f %d p %d sb %d new iter (%f,%f,%d) min (%f,%f,%d,i%d) first (%f,%f,%d,i%d) in (%f,%f,%d)\n",
                    //            f, p, startBucket,
                    //            decodeW(wStart->vals[0]), decodeW(wStart->vals[1]),
                    //            wStart->count,
                    //            decodeW(wBest->vals[0]), decodeW(wBest->vals[1]),
                    //            wBest->count, wBest->index,
                    //            decodeW(wLocal->vals[0]), decodeW(wLocal->vals[1]),
                    //            wLocal->count, wLocal->index,
                    //            decodeW(w[startBucket].vals[0]),
                    //            decodeW(w[startBucket].vals[1]),
                    //            w[startBucket].count
                    //            );
                    // }
                }
            }
        }

        ukl_threadgroup_barrier();

        RWLOCAL(WIndexed) wScan = wLocal;
        {
            // Now we've read it in, we can do our prefix sum
            SYNC_CALL_4(prefixSumW, wScan, numBucketsInIteration, threadIdx, numThreads);

            // Seed for the next one
            *wStart = wScan[numBucketsInIteration - 1];
        }

        //using namespace std;
        //cerr << "numBucketsInIteration = " << numBucketsInIteration << endl;

        // And then scan that to find the index of the best score
        SYNC_CALL_6(argMinScanW, wScan, numBucketsInIteration, wAll, criterion, threadIdx, numThreads);

        // Find if our new bucket is better than the last champion
        minW(wBest, wScan + (numBucketsInIteration - 1), wAll, criterion);

        ukl_threadgroup_barrier();
    }

#if 0
    if (get_local_id(0) == 0 && ordinal) {
        if (wStart->vals[0] != wAll.vals[0]
            || wStart->vals[1] != wAll.vals[1]
            || wStart->count != wAll.count) {
            //printf("Accumulation error: wAll != wStart: (%f,%f,%d) != (%f,%f,%d)\n",
            //       decodeW(wAll.vals[0]), decodeW(wAll.vals[1]), wAll.count,
            //       decodeW(wStart->vals[0]), decodeW(wStart->vals[1]), wStart->count);
        }
    }
#endif

#if 0
    if (p < 256 && get_local_id(0) == 0 && false) {
        //printf("f %d p %d numBuckets = %d\n",
        //       f, p, numBuckets);

#if 0        
        printf("wAll (%f,%f,%d)\n",
               decodeW(wAll.vals[0]), decodeW(wAll.vals[1]), wAll.count);

        for (uint32_t i = 0;  i < numBuckets;  ++i) {
            printf("  b%d in (%f,%f,%d) cum (%f,%f,%d)\n",
                   i, decodeW(w[i].vals[0]), decodeW(w[i].vals[1]),
                   w[i].count,
                   decodeW(wLocal[i].vals[0]), decodeW(wLocal[i].vals[1]),
                   wLocal[i].count);
        }
#endif
        float score = scoreSplitWAll(*wBest, wAll);

        
        printf("f %d p %d nb %5d wBest %4d ord %d (%f,%f,%7d) %f\n",
               f, p, numBuckets, wBest->index, ordinal,
               decodeW(wBest->vals[0]), decodeW(wBest->vals[1]), wBest->count,
               score);
    }
#endif

    //cerr << "FINISHED chooseSplit f=" << f << " p=" << p << " threadIdx " << threadIdx << endl;

    result.score = scoreSplitWAll(wBest->w, wAll, criterion);
    result.feature = f;
    result.value = wBest->index + 1;  // the buckets (or ranks) below the split's value are on the left
    if (!ordinal) {
        // The subset: the buckets ranked below the split's value.
        for (int32_t k = 0;  k < result.value;  ++k) {
            uint32_t b = ranked[k];
            result.subset[b >> 5] |= 1u << (b & 31);
        }
    }
    result.left = wBest->w;
    result.right = wAll;

    result.right.vals[0] -= wBest->w.vals[0];
    result.right.vals[1] -= wBest->w.vals[1];
    result.right.count   -= wBest->w.count;

    SYNC_RETURN(result);
}

typedef struct TreeDepthInfo {
    uint32_t depth;
    uint32_t prevNumFinishedPartitions;
    uint32_t prevNumActivePartitions;
    uint32_t numFinishedPartitions;
    uint32_t numActivePartitions;
    uint32_t numSmallSideRows;
    uint32_t status;
} TreeDepthInfo;

typedef struct TreeTrainingInfo {
    uint32_t numRows;             ///< Number of rows in our training set
    uint16_t maxDepth;            ///< Maximum depth we're willing to get to
    uint16_t numActiveFeatures;   ///< Number of active features
    uint16_t numFeatures;         ///< Number of total features
    uint16_t maxNumActivePartitions; ///< Maximum width of active partition tree
    uint32_t numActiveBuckets;    ///< Number of buckets across all active features
    uint16_t featureSampling;     ///< Which feature sampling bag we belong to
    uint16_t featureVectorSampling; ///< Which feature vector sampling bag we belong to
    uint32_t maxNumFinishedPartitions; ///< Length of the pool of every level's partition splits
    uint32_t splitCriterion;      ///< How splits are scored (SPLIT_CRITERION_*)
} TreeTrainingInfo;

// TreeDepthInfo's status: 0 while the device trains; 1 when it has stopped (the next level is wider than its
// partitions, or its splits would not fit the pool), leaving that level for the host, and every later kernel does
// nothing; below 0 for an overflow (the tree is wrong).
#define TREE_DEPTH_STATUS_STOPPED 1

// [bucket, feature, partition]
SYNC_FUNCTION(v1,
void, 
getPartitionSplitsImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                         ROBUFFER(uint32_t) bucketNumbers, // [nf]
                         
                         ROBUFFER(uint32_t) activeFeatureList, // [naf]
                         ROBUFFER(uint32_t) featureIsOrdinal, // [nf]
                         
                         ROBUFFER(W) buckets, // [np x totalBuckets]

                         ROBUFFER(W) wAll,  // one per partition
                         RWBUFFER(PartitionSplit) featurePartitionSplitsOut, // [np x nf]

                         ROBUFFER(TreeDepthInfo) treeDepthInfo,

                         RWLOCAL(WIndexed) wLocal,  // [wLocalSize]
                         uint32_t wLocalSize,
                         RWLOCAL(WIndexed) wStartBest, //[2]
                         RWLOCAL(WIndexed) wRanked, //[MAX_CATEGORICAL_BUCKETS]
                         RWLOCAL(uint16_t) ranked, //[MAX_CATEGORICAL_BUCKETS]

                         uint16_t featureId, uint16_t numFeatureIds,
                         uint16_t workerId, uint16_t workGroupSize,
                         uint16_t partitionWorkerId, uint16_t partitionWorkerSize)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        SYNC_RETURN();

    //using namespace std;
    //cerr << "getPartitionSplitsImpl: featureId " << featureId << " numActiveFeatures " << numActiveFeatures
    //    << " workerId " << workerId << " workGroupSize " << workGroupSize
    //    << " partitionWorkerId " << partitionWorkerId << " partitionWorkerSize " << partitionWorkerSize
    //    << endl;
    //printf("getPartitionSplits %d\n", bucket);

    ukl_assert(workGroupSize > 0);
    ukl_assert(workGroupSize % 32 == 0);
    ukl_assert(partitionWorkerSize > 0);

    const uint32_t totalBuckets = treeTrainingInfo[0].numActiveBuckets;
    const uint32_t numActivePartitions = treeDepthInfo[0].numActivePartitions;

    const uint16_t fidx = featureId;
    const uint16_t naf = treeTrainingInfo[0].numActiveFeatures;
    if (fidx >= naf)
        SYNC_RETURN();

    const uint16_t f = activeFeatureList[fidx];

    uint32_t bucketStart = bucketNumbers[f];
    uint32_t bucketEnd = bucketNumbers[f + 1];
    uint32_t numBuckets = bucketEnd - bucketStart;

    //printf("partitionWorkerId = %d bucket %d featureId %d\n\n\n", partitionWorkerId, bucket, featureId);

    // We dimension as the feature with the highest number of buckets, so
    // for most of them we have too many and can stop
    //if (bucket >= numBuckets)
    //    return;

    const PartitionSplit NONE = PARTITION_SPLIT_INIT;

    //printf("getPartitionSplits done2 %d %d %d %d\n", partitionWorkerId, bucket, fidx, naf);

    for (uint16_t partition = partitionWorkerId;  partition < numActivePartitions;  partition += partitionWorkerSize) {
        //printf("partition %d\n", (int)partition);
        PartitionSplit best = NONE;

        // Don't do inactive partitions
        if (wAll[partition].count == 0 || wAll[partition].vals[0] == 0 || wAll[partition].vals[1] == 0) {
            if (workerId == 0) {
                //printf("writing out inactive PartitionSplit part %d f %d idx %x\n", partition, f, partition * nf + f);
                featurePartitionSplitsOut[partition * naf + fidx] = best;            
            }
            continue;
        }
        
        // Find where our bucket data starts
        ROBUFFER(W) myW
            = buckets
            + (totalBuckets * partition)
            + bucketStart;
        
        bool ordinal = featureIsOrdinal[f];

        best = SYNC_CALL_14(chooseSplit, myW, numBuckets, wAll[partition],
                         wLocal, wLocalSize, wStartBest, wRanked, ranked, ordinal, treeTrainingInfo[0].splitCriterion,
                         workerId, workGroupSize, f, partition);
        
        if (workerId == 0) {
            if (best.score == INFINITY) {
                best = NONE;
            }
            featurePartitionSplitsOut[partition * naf + fidx] = best;
            //printf("writing out active PartitionSplit part %d f %d idx %x score %f\n", partition, f, partition * nf + f, best.score);
        }

    }

    //using namespace std;
    //cerr << "FINISHED getPartitionSplit" << " f = " << f << endl;

    SYNC_RETURN();
}

typedef struct PartitionIndex {
    uint32_t index;
} PartitionIndex;

// id 0: partition number
UKL_FUNCTION(v1,void,
bestPartitionSplitImpl)(CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                       ROBUFFER(TreeDepthInfo) treeDepthInfo,
                       ROBUFFER(uint32_t) activeFeatureList, // [numActiveFeatures]
                       ROBUFFER(PartitionSplit) featurePartitionSplits,
                       ROBUFFER(PartitionIndex) partitionIndexes,
                       RWBUFFER(IndexedPartitionSplit) allPartitionSplitsOut,
                       uint16_t workerId, uint16_t numWorkers)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        return;

    const uint16_t numActiveFeatures = treeTrainingInfo->numActiveFeatures;
    const uint16_t numActivePartitions = treeDepthInfo->numActivePartitions;
    const uint16_t depth = treeDepthInfo->depth;

    allPartitionSplitsOut += treeDepthInfo->numFinishedPartitions;

    for (uint16_t partition = workerId;  partition < numActivePartitions;  partition += numWorkers) {
        IndexedPartitionSplit best = INDEXED_PARTITION_SPLIT_INIT;
        best.index = (depth == 0 ? 1 : partitionIndexes[partition].index);
        //printf("partition %d has index %d\n", p, best.index);
        if (best.index == 0) {
            allPartitionSplitsOut[partition] = best;
            continue;
        }
        
        //printf("bestPartitionSplitKernel for partition %d\n", p);

        // TODO: with lots of features this will become slow; we can have a workgroup cooperate
        for (uint32_t fidx = 0;  fidx < numActiveFeatures;  ++fidx) {
            const PartitionSplit split = featurePartitionSplits[partition * numActiveFeatures + fidx];

#if UKL_CPU_BACKEND
            if (false) {
                using namespace std;
                std::ofstream splitsStream("tree-best-splits-" + std::to_string(treeTrainingInfo[0].featureSampling)
                                        + "-" + std::to_string(treeTrainingInfo[0].featureVectorSampling)
                                        + "-depth" + std::to_string(treeDepthInfo[0].depth)
                                        + ".txt", std::ios_base::app);
                splitsStream << "feat " << fidx << " part " << partition << " " << MLDB::RF::printPartitionSplit(split) << endl;
            }
#endif

            //printf("feature %d active %d split %d score %f\n", f, activeFeatureList[f], featurePartitionSplits[f].value, featurePartitionSplits[f].score);
            if (split.value < 0)
                continue;
            //printf("doing feature %d\n", f);
            if (split.score < best.score) {
                best.score = split.score;
                best.feature = split.feature;
                best.value = split.value;
                best.left = split.left;
                best.right = split.right;
                for (uint32_t i = 0;  i < CATEGORY_SET_WORDS;  ++i)
                    best.subset[i] = split.subset[i];
            }
        }

        //printf("partition %d offset %d feature %d value %d score %f\n", p, best.feature, best.value, best.score);

        allPartitionSplitsOut[partition] = best;

#if UKL_CPU_BACKEND
        if (false) {
            using namespace std;
            std::ofstream splitsStream("tree-best-splits-" + std::to_string(treeTrainingInfo[0].featureSampling)
                                    + "-" + std::to_string(treeTrainingInfo[0].featureVectorSampling)
                                    + "-depth" + std::to_string(treeDepthInfo[0].depth)
                                    + ".txt", std::ios_base::app);
            splitsStream << "part " << partition << " best " << printIndexedPartitionSplit(best) << endl;
        }
#endif

    }
}

typedef struct PartitionInfo {
    int32_t left;
    int32_t right;
} PartitionInfo;

UKL_FUNCTION(v1,bool,discardSplit)(IndexedPartitionSplit split)
{
    return split.feature == -1
        || ((split.left.vals[0] == 0 || split.left.vals[1] == 0)
             && (split.right.vals[0] == 0 || split.right.vals[1] == 0));
}

UKL_FUNCTION(v1,int32_t,indexDepth)(uint32_t index)
{
    return index == 0 ? -1 : 31 - ukl_clz(index);
}

UKL_FUNCTION(v1,uint32_t,leftChildIndex)(uint32_t parent)
{
    return parent + (1 << indexDepth(parent));
}

UKL_FUNCTION(v1,uint32_t,rightChildIndex)(uint32_t parent)
{
    return parent + (2 << indexDepth(parent));
}

// one instance
// SmallSideIndex[numPartitionsOut]:
// Used to a) know when we need to clear partitions (those on the small side are cleared,
// those on the big side are not) and b) provide a local index for partition buckets for
// the UpdateBuckets kernel
// - 0 means it's not on the small side
// - 1-254 means we're partition number (n-1) on the small side
// - 255 means we're partition number 254 or greater on the small side

#define INACTIVE_PARTITIONS_LENGTH 16300

typedef struct AssignPartitionNumbersLocalState {
    atomic_uint numInactivePartitions;
    atomic_uint numSmallSideRows;
    atomic_uint n;
    atomic_uint n2;
    atomic_uint skippedRows;
    atomic_uint skippedPartitions;
    atomic_uint ssi;
    atomic_uint scratch;  // for SIMD reductions
    uint16_t inactivePartitions[INACTIVE_PARTITIONS_LENGTH];
} AssignPartitionNumbersLocalState;

SYNC_FUNCTION(v1,
void,
assignPartitionNumbersImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                             RWBUFFER(TreeDepthInfo) treeDepthInfo,
                             ROBUFFER(IndexedPartitionSplit) allPartitionSplits,
                             RWBUFFER(PartitionIndex) partitionIndexesOut,
                             RWBUFFER(PartitionInfo) partitionInfoOut,
                             RWBUFFER(uint8_t) smallSideIndexesOut,
                             RWBUFFER(uint16_t) smallSideIndexToPartitionOut,
                             RWLOCAL(AssignPartitionNumbersLocalState) localState,
                             uint16_t workerId,
                             uint16_t workgroupSize)
{
    const uint16_t depth = treeDepthInfo->depth;
    const uint16_t numActivePartitions = depth == 0 ? 1 : treeDepthInfo->numActivePartitions;
    const uint16_t maxNumActivePartitions = treeTrainingInfo->maxNumActivePartitions;

    // A previous level stopped the device: nothing more to do.
    if (depth != 0 && treeDepthInfo->status != 0)
        SYNC_RETURN();

    allPartitionSplits += depth == 0 ? 0 : treeDepthInfo->numFinishedPartitions;

    if (workerId == 0) {
        atom_store(&localState->numInactivePartitions, 0);
        atom_store(&localState->numSmallSideRows, 0);
        if (depth == 0)
            treeDepthInfo->status = 0;  // for now
    }

    ukl_threadgroup_barrier();

    // Note that we have to ensure that all 32 threads enter this loop, so we have a uniform
    // condition and then selectively enable for p < numActivePartitions
    for (uint16_t i = 0;  i < numActivePartitions;  i += workgroupSize) {
        uint16_t p = i + workerId;

        bool discard = false;

        if (p < numActivePartitions) {
            IndexedPartitionSplit split = allPartitionSplits[p];
            discard = discardSplit(split);
        }

        //using namespace std;
        //cerr << "i = " << i << " p = " << p << " nap = " << numActivePartitions << " discard = " << discard << " workGroupSize = " << workgroupSize << endl;

        uint32_t discardWhich = ukl_simdgroup_ballot(discard, workerId, &localState->scratch);

        uint32_t numDiscarded = ukl_popcount(discardWhich);

        //cerr << "i = " << i << " discardWhich = " << discardWhich << " numDiscarded = " << numDiscarded << endl;

        if (numDiscarded + atom_load(&localState->numInactivePartitions) >= INACTIVE_PARTITIONS_LENGTH) {
            // OVERFLOW
            treeDepthInfo->numActivePartitions = -1;
            treeDepthInfo->status = -1;
            SYNC_RETURN();
        }

        uint32_t base = atom_load(&localState->numInactivePartitions);
        uint32_t index = base + SYNC_CALL_3(ukl_simdgroup_prefix_exclusive_sum_bools, discard, workerId, &localState->scratch);

        if (p < numActivePartitions) {
            if (discard) {
                smallSideIndexesOut[p] = 255;
                localState->inactivePartitions[index] = p;
            }
            else {
                smallSideIndexesOut[p] = 0;
            }

            if (workerId == 0 && numDiscarded > 0) {
                atom_add(&localState->numInactivePartitions, numDiscarded);
            }
        }

        ukl_simdgroup_barrier();
    }

    ukl_threadgroup_barrier();

    // Before writing anything for this level: each partition that goes on splitting needs a second number, from
    // the stopped partitions' first and then past the active ones. When the next level would be wider than the
    // partitions, or its splits would not fit the pool, the device stops here (the level's state is still the
    // previous level's, which the host continues from); upstream dropped the partitions it had no number for.
    {
        uint32_t numInactive = atom_load(&localState->numInactivePartitions);
        uint32_t numContinuing = numActivePartitions - numInactive;
        uint32_t nextNumActive = numActivePartitions + (numContinuing > numInactive ? numContinuing - numInactive : 0);
        uint32_t nextNumFinished = (depth == 0 ? 0 : treeDepthInfo->numFinishedPartitions) + numActivePartitions;
        if (nextNumActive > maxNumActivePartitions
            || nextNumFinished + nextNumActive > treeTrainingInfo->maxNumFinishedPartitions) {
            ukl_threadgroup_barrier();
            if (workerId == 0)
                treeDepthInfo->status = TREE_DEPTH_STATUS_STOPPED;
            SYNC_RETURN();
        }
    }

    //using namespace std;
    //cerr << "workerId = " << workerId << " numActivePartitions = " << numActivePartitions << endl;

    if (workerId == 0) {
        atom_store(&localState->n, 0);
        atom_store(&localState->skippedRows, 0);
        atom_store(&localState->skippedPartitions, 0);
        atom_store(&localState->ssi, 0);
        atom_store(&localState->n2, numActivePartitions);
        smallSideIndexToPartitionOut[0] = smallSideIndexToPartitionOut[255] = 0;  // we never use a small side index of 0, as this means "no longer active"
    }

    ukl_threadgroup_barrier();  // for when we use more than one SIMD group

    for (uint16_t i = 0;  i < numActivePartitions;  i += workgroupSize) {
        uint16_t p = i + workerId;
        RWBUFFER(PartitionInfo) info = partitionInfoOut + p;

        bool discard = true;
        bool direction = false;
        uint32_t partitionSmallSideRows = 0;
        IndexedPartitionSplit split;

        if (p < numActivePartitions) {
            split = allPartitionSplits[p];
            discard = discardSplit(split);
            if (discard) {
                info->left = info->right = -1;
            }
            else {
                partitionSmallSideRows = min(split.left.count, split.right.count);
                direction = indexedPartitionSplitDirection(split);
            }
        }

        uint32_t smallSideRowsIncrement = ukl_simdgroup_sum(partitionSmallSideRows, workerId, &localState->scratch);

        if (workerId == 0) {
            atom_add(&localState->numSmallSideRows, smallSideRowsIncrement);
        }

        if (!discard) {
            bool skipped = false;
            uint32_t minorPartitionIndex = atom_add(&localState->n, 1);
            uint32_t minorPartitionNumber;
            if (minorPartitionIndex < atom_load(&localState->numInactivePartitions)) {
                minorPartitionNumber = localState->inactivePartitions[minorPartitionIndex];
            }
            else {
                minorPartitionNumber = atom_add(&localState->n2, 1);
            }

            //cerr << "workerId = " << workerId << " minorPartitionIndex = " << minorPartitionIndex
            //     << " numInactivePartitions " << localState->numInactivePartitions
            //     << " minorPartitionNumber = " << minorPartitionNumber << " n2 " << localState->n2 << endl;

            if (minorPartitionNumber >= maxNumActivePartitions) {
                skipped = true;
            }
            else {
                // Attempt to allocate a small side number, and if it's possible record the
                // mapping.

                uint32_t idx = 1 + atom_add(&localState->ssi, 1);
                //cerr << "workerId = " << workerId << " idx = " << idx << " minorPartitionNumber = " << minorPartitionNumber << endl;
                if (idx < 255) {
                    smallSideIndexesOut[minorPartitionNumber] = idx;
                    smallSideIndexToPartitionOut[idx] = minorPartitionNumber;
                }
                else {
                    smallSideIndexesOut[minorPartitionNumber] = 255;
                }
            }

            if (skipped) {
                info->left = -1;
                info->right = -1;
                atom_add(&localState->skippedRows, split.left.count + split.right.count);
                atom_add(&localState->skippedPartitions, 1);
            }
            else {
                if (direction == 0) {
                    info->left = p;
                    info->right = minorPartitionNumber;
                }
                else {
                    info->left = minorPartitionNumber;
                    info->right = p;
                }

                //printf("partition %d direction %d minor %d n %d n2 %d\n", p, direction, minorPartitionNumber, n, n2);

                partitionIndexesOut[info->left].index = leftChildIndex(split.index);
                partitionIndexesOut[info->right].index = rightChildIndex(split.index);
            }
        }
    }

    // Make sure the previous loop is finished before things get updated...
    ukl_threadgroup_barrier();

    if (workerId == 0) {
        uint32_t nap = min((uint32_t)atom_load(&localState->n2), (uint32_t)maxNumActivePartitions);
        treeDepthInfo->prevNumActivePartitions = treeDepthInfo->numActivePartitions;
        treeDepthInfo->numActivePartitions = nap;
        treeDepthInfo->numSmallSideRows = atom_load(&localState->numSmallSideRows);
        treeDepthInfo->prevNumFinishedPartitions = depth == 0 ? 0 : treeDepthInfo->numFinishedPartitions;
        treeDepthInfo->numFinishedPartitions = treeDepthInfo->prevNumFinishedPartitions + numActivePartitions;
        treeDepthInfo->depth += 1;
        if (nap < atom_load(&localState->n2)) {
            treeDepthInfo->status = -2;
        }

        // Clear partition indexes for gap partitions
        for (uint32_t n = atom_load(&localState->n), nap = atom_load(&localState->numInactivePartitions); n < nap;  ++n) {
            uint32_t part = localState->inactivePartitions[n];
            smallSideIndexesOut[part] = 0;
            partitionIndexesOut[part].index = 0;
        }
        for (uint32_t ssi = atom_load(&localState->ssi); ssi < 255 && ssi < nap/2;  ++ssi) {
            smallSideIndexToPartitionOut[ssi+1] = 0;
        }
    }

#if UKL_CPU_BACKEND
    if (workerId == 0) {
        using namespace std;
        cerr << printTreeDepthInfo(*treeDepthInfo) << endl;
    }
#endif

    SYNC_RETURN();
}

typedef struct UpdateWorkEntry {
    uint32_t row;
    uint16_t partition;
    uint16_t smallSideIndex;
    float decodedRow[2];  // The row's weights to false and true
} UpdateWorkEntry;

// [partition, bucket]
UKL_FUNCTION(v1,void,
clearBucketsImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                   ROBUFFER(TreeDepthInfo) treeDepthInfo,
                   RWBUFFER(W) bucketsOut,
                   RWBUFFER(W) wAllOut,
                   RWBUFFER(uint32_t) numNonZeroDirectionIndices,
                   CONSTBUFFER(uint8_t) smallSideIndexes,
                   uint16_t partitionWorkerId, uint16_t partitionWorkgroupSize,
                   uint32_t bucket)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        return;

    uint32_t numActiveBuckets = treeTrainingInfo->numActiveBuckets;
    uint16_t numActivePartitions = treeDepthInfo->numActivePartitions;
        
    // Clear the number of non zero directions once per kernel
    if (partitionWorkerId == 0 && bucket == 0) {
        numNonZeroDirectionIndices[0] = 0;
    }

    if (bucket >= numActiveBuckets)
        return;
        
    const W empty = W_INIT;
        
    for (uint32_t partition = partitionWorkerId;  partition < numActivePartitions;  partition += partitionWorkgroupSize) {
        if (!smallSideIndexes[partition])
            continue;

        RWBUFFER(W) buckets = bucketsOut + partition * numActiveBuckets;
        
        buckets[bucket] = empty;
        if (bucket == 0) {
            wAllOut[partition] = empty;
        }
    }
}

// Structure for a partition number
typedef struct RowPartitionInfo {
    int16_t num;  // partition number
} RowPartitionInfo;

#define SSI_CACHE_SIZE 512
#define MAX_WORKGROUP_THREADS 1024

typedef struct UpdatePartitionNumbersLocalState {
    atomic_uint scratch[MAX_WORKGROUP_THREADS / 32];  // One per workgroup
    // Most rows live in partitions with a number < 256; avoid main memory accesses for these
    // by caching in local memory
    uint16_t smallSideIndexesCache[SSI_CACHE_SIZE];
} UpdatePartitionNumbersLocalState;


// Second part: per feature plus wAll, transfer examples
//[rowNumber]
SYNC_FUNCTION(v1,
void,
updatePartitionNumbersImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                             ROBUFFER(TreeDepthInfo) treeDepthInfo,

                             RWBUFFER(RowPartitionInfo) partitions,
                             RWBUFFER(uint32_t) directions,
                             RWBUFFER(uint32_t) numNonZeroDirectionIndices,
                             RWBUFFER(UpdateWorkEntry) nonZeroDirectionIndices,
                             ROBUFFER(uint8_t) smallSideIndexes,

                             ROBUFFER(IndexedPartitionSplit) allPartitionSplits,
                             ROBUFFER(PartitionInfo) partitionInfo,

                             ROBUFFER(uint32_t) bucketData,
                             CONSTBUFFER(uint32_t) bucketDataOffsets,
                             CONSTBUFFER(uint32_t) bucketNumbers,
                             CONSTBUFFER(uint32_t) bucketEntryBits,
                             CONSTBUFFER(uint32_t) featureIsOrdinal,

                             ROBUFFER(float) decodedRows,
                             RWLOCAL(UpdatePartitionNumbersLocalState) localState,
                             uint32_t workerIdInGrid, uint32_t numWorkersInGrid,
                             uint16_t workerIdInWorkgroup, uint16_t numWorkersInWorkgroup)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        SYNC_RETURN();

    uint16_t depth = treeDepthInfo->depth - 1;
    uint32_t numRows = treeTrainingInfo->numRows;
    uint16_t numActivePartitions = treeDepthInfo->numActivePartitions;

    RWLOCAL(uint16_t) smallSideIndexesCache = localState->smallSideIndexesCache;

    // Populate the cache of smallSideIndexes
    for (uint32_t b = workerIdInWorkgroup;  b < SSI_CACHE_SIZE && b < numActivePartitions; b += numWorkersInWorkgroup) {
        smallSideIndexesCache[b] = smallSideIndexes[b];
    }

    // Wait for all buckets to be clear
    ukl_threadgroup_barrier();

    allPartitionSplits += treeDepthInfo->prevNumFinishedPartitions;

    //uint16_t numSimdGroups = get_local_size(0) / 32;
    //uint16_t simdGroupNum = get_local_id(0) / 32;
    //uint16_t simdLaneNum = get_local_id(0) % 32;

    //RWLOCAL(uint32_t work_group_directions[32];  // 32) 32 = 1024 bits at once, which is the SIMD width

    UKL_GLOBAL_PTR(atomic_uint) nonZeroDirectionCount = &CAST_RWBUFFER(numNonZeroDirectionIndices, atomic_uint)[0];

    // Scratch for my simdgroup
    const uint16_t simdgroup_lane = workerIdInWorkgroup % 32;
    const uint16_t simdgroup_num  = workerIdInWorkgroup / 32;
    UKL_LOCAL_PTR(atomic_uint) simdgroup_scratch = localState->scratch + simdgroup_num;

    for (uint32_t i = 0;  i < numRows;  i += numWorkersInGrid) {
        uint32_t r = i + workerIdInGrid;

        uint16_t storedPartition = r < numRows ? partitions[r].num : 0;
        uint16_t oldPartition = depth == 0 ? 0 : storedPartition;
        uint16_t partition = oldPartition;

        if (r < numRows && partition != (uint16_t)-1) {
            PartitionInfo info = partitionInfo[partition];

            if (info.left == -1 || info.right == -1) {
                partition = -1;
            }
            else {
                int16_t splitFeature = allPartitionSplits[partition].feature;
                uint16_t splitValue = allPartitionSplits[partition].value;
                uint16_t ordinal = featureIsOrdinal[splitFeature];

                // Split feature values go here
                uint32_t splitBucketDataOffset = bucketDataOffsets[splitFeature];
                //uint32_t splitBucketDataLength = bucketDataOffsets[splitFeature + 1] - splitBucketDataOffset;
                //uint32_t splitNumBuckets = bucketNumbers[splitFeature + 1] - bucketNumbers[splitFeature];
                ROBUFFER(uint32_t) splitBucketData = bucketData + splitBucketDataOffset;
                uint16_t splitBucketBits = bucketEntryBits[splitFeature];

                uint16_t bucket = getBucket(r,
                                            splitBucketData, 0 /*splitBucketDataLength*/,
                                            splitBucketBits, 0 /*splitNumBuckets*/);
                
                // A categorical split's left is its subset.
                uint16_t side = ordinal ? bucket >= splitValue : !inCategorySet(allPartitionSplits[partition].subset, bucket);

                // Set the new partition number
                uint16_t newPartitionNumber = side ? info.right : info.left;
                partition = newPartitionNumber;
            }
        }

        bool direction = r < numRows && partition != oldPartition && partition != (uint16_t)-1;
        if (r < numRows && partition != storedPartition)
            partitions[r].num = partition;

        //if (r + 32 >= numRows)
        //    break;

        uint32_t simdDirections = ukl_simdgroup_ballot(direction, simdgroup_lane, simdgroup_scratch);

        //if (simd_is_first() && (r / 32) * 32 < numRows) {
        //    directions[r/32] = simdDirections;
        //}

        uint16_t numDirections = ukl_popcount(simdDirections);

        //using namespace std;
        //cerr << "lane " << simdgroup_lane << " numDirections = " << numDirections << endl;

        if (numDirections == 0)
            continue;

        uint32_t nonZeroDirectionBase = 0;
        if (simdgroup_lane == 0) {
            nonZeroDirectionBase = atom_add(nonZeroDirectionCount, numDirections);
        }

        nonZeroDirectionBase = ukl_simdgroup_broadcast_first(nonZeroDirectionBase, simdgroup_lane, simdgroup_scratch);

        uint16_t n = ukl_prefix_exclusive_sum_bitmask(simdDirections, simdgroup_lane);

        if (direction) {
            uint16_t smallPartitionIndex = /*partition < SSI_CACHE_SIZE ? smallSideIndexesCache[partition] :*/ smallSideIndexes[partition];
            const UpdateWorkEntry work = { r, partition, smallPartitionIndex, { decodedRows[2 * r], decodedRows[2 * r + 1] } };
            nonZeroDirectionIndices[nonZeroDirectionBase + n] = work;
        }
    }

    SYNC_RETURN();
}

typedef struct UpdateBucketsLocalState {
    uint16_t smallSideIndexesCache[SSI_CACHE_SIZE];
} UpdateBucketsLocalState;

// [row, feature]
SYNC_FUNCTION(v1,
void,
updateBucketsImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                    ROBUFFER(TreeDepthInfo) treeDepthInfo,
                    ROBUFFER(RowPartitionInfo) partitions,
                    ROBUFFER(uint32_t) directions,
                    ROBUFFER(uint32_t) numNonZeroDirectionIndices,
                    RWBUFFER(UpdateWorkEntry) nonZeroDirectionIndices,
                    RWBUFFER(W) buckets,
                    RWBUFFER(W) wAll,
                    CONSTBUFFER(uint8_t) smallSideIndexes,
                    CONSTBUFFER(uint16_t) smallSideIndexToPartition,

                    // Row data
                    ROBUFFER(float) decodedRows,
                    
                    // Feature data
                    ROBUFFER(uint32_t) bucketData,
                    ROBUFFER(uint32_t) bucketDataOffsets,
                    ROBUFFER(uint32_t) bucketNumbers,
                    ROBUFFER(uint32_t) bucketEntryBits,

                    ROBUFFER(uint32_t) activeFeatureList,
                    ROBUFFER(uint32_t) featureIsOrdinal,

                    RWLOCAL(W) wLocal,
                    uint16_t maxLocalBuckets,
                    RWLOCAL(UpdateBucketsLocalState) localState,

                    int16_t featureWorkerId,
                    uint16_t workerIdInWorkgroup, uint16_t numWorkersInWorkgroup,
                    uint16_t workerIdInGrid, uint16_t numWorkersInGrid)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        SYNC_RETURN();

    int16_t fidx = featureWorkerId - 1;  // -1 means wAll, otherwise it's the feature number
    const uint16_t numActivePartitions = treeDepthInfo[0].numActivePartitions;
    const uint32_t numActiveBuckets = treeTrainingInfo[0].numActiveBuckets;
    const uint16_t numActiveFeatures = treeTrainingInfo[0].numActiveFeatures;

    if (fidx >= numActiveFeatures)
        SYNC_RETURN();

    int16_t f = fidx == -1 ? -1 : activeFeatureList[fidx];

    // We have to set up to access two different features:
    // 1) The feature we're splitting on for this example (splitFeature)
    // 2) The feature we're updating for the split (f)
    // Thus, we need to perform the same work twice, once for each of the
    // features.
    
    uint16_t numBucketsPerPartition;        // How many buckets for this feature?
    uint16_t bucketBits;
    uint16_t numLocalBuckets;
            
    // Pointer to the global array we eventually want to update
    RWBUFFER(W) wGlobal;

    if (f == -1) {
        numBucketsPerPartition = 1;
        wGlobal = wAll;
        numLocalBuckets = min((uint16_t)(numActivePartitions / 2), maxLocalBuckets);
        numLocalBuckets = min(numLocalBuckets, (uint16_t)254);
    }
    else {
        uint32_t bucketDataOffset = bucketDataOffsets[f];
        numBucketsPerPartition = bucketNumbers[f + 1] - bucketNumbers[f];
        bucketData = bucketData + bucketDataOffset;
        bucketBits = bucketEntryBits[f];
        numLocalBuckets = min((uint32_t)numBucketsPerPartition * numActivePartitions / 2, (uint32_t)maxLocalBuckets);
        wGlobal = buckets + bucketNumbers[f];
    }

    // We call getBucket, but since our stride is the workgroup width (which is always a multiple of 32),
    // we always use the same bit number and mask.  This code allows us to avoid most of the logic by
    // pre-computing the constants.

    // Most rows live in partitions with a number < 256; avoid main memory accesses for these
    // by caching in local memory
    RWLOCAL(uint16_t) smallSideIndexCache = localState->smallSideIndexesCache;

    // Clear our local accumulation buckets to get started
    for (uint32_t b = workerIdInWorkgroup;  b < numLocalBuckets; b += numWorkersInWorkgroup) {
        zeroW(wLocal + b);
    }

    // Populate the cache of smallSideIndexes
    for (uint32_t b = workerIdInWorkgroup;  b < SSI_CACHE_SIZE && b < numActivePartitions; b += numWorkersInWorkgroup) {
        smallSideIndexCache[b] = smallSideIndexes[b];
    }

    // Wait for all buckets to be clear
    ukl_threadgroup_barrier();
    
    //numLocalBuckets = 0;  // DEBUG DO NOT COMMIT

    uint32_t numNonZero = numNonZeroDirectionIndices[0];

    for (uint32_t i = workerIdInGrid;  i < numNonZero;  i += numWorkersInGrid) {

        const UpdateWorkEntry work = nonZeroDirectionIndices[i];

        const uint32_t r = work.row;
        const uint16_t partition = work.partition;
        const uint16_t smallPartitionIndex = work.smallSideIndex;
        const float wFalse = work.decodedRow[0];
        const float wTrue = work.decodedRow[1];

        uint32_t toBucketLocal;
        uint32_t toBucketGlobal;
        //uint8_t smallPartitionIndex = partition < 256 ? smallSideIndexesCache[partition] : smallSideIndexes[partition];
        
        if (f == -1) {
            // Since we don't touch the buckets on the left side of the
            // partition, we don't need to have local accumulators for them.
            // Hence, the partition number for local is the left partition.
            if (smallPartitionIndex == 255)
                toBucketLocal = -1;
            else
                toBucketLocal = smallPartitionIndex - 1;
            toBucketGlobal = partition;
        }
        else {
            uint16_t bucket = getBucket(r /*exampleNum*/,
                                        bucketData, 0 /* bucketDataLength */,
                                        bucketBits, 0 /* numBucketsPerPartition */);
            toBucketGlobal = partition * numActiveBuckets + bucket;
            if (smallPartitionIndex == 255)
                toBucketLocal = -1;
            else
                toBucketLocal = (smallPartitionIndex - 1) * numBucketsPerPartition + bucket;
        }

#if 0
        using namespace std;
        cerr << "transferring " << i << " of " << numNonZero << " partition " << partition << " feature " << f 
             << " ssr " << smallPartitionIndex
             << " row " << r << ":" << decodedRow << " toBucketLocal " << toBucketLocal << " toBucketGlobal " << toBucketGlobal << endl;
#endif

        if (toBucketLocal < numLocalBuckets) {
            //atom_inc(&numLocalUpdates);
            addRowLocal(&wLocal[toBucketLocal], wFalse, wTrue);
        }
        else {
            //atom_inc(&numGlobalUpdates);
            addRowGlobal(&wGlobal[toBucketGlobal], wFalse, wTrue);
        }
    }

    // Wait for all buckets to be updated, before we write them back to the global mem
    ukl_threadgroup_barrier();
    
    // Now write our local changes to the global
    for (uint32_t b = workerIdInWorkgroup;  b < numLocalBuckets;  b += numWorkersInWorkgroup) {
        if (wLocal[b].count != 0) {
            //++numCopyLocalToGlobal;
            if (f == -1) {
                uint32_t bucketNumberGlobal = smallSideIndexToPartition[b + 1];
                incrementWOut(&wGlobal[bucketNumberGlobal], &wLocal[b]);
                continue;
            }

            // TODO: avoid div/mod if possible in these calculations
            uint32_t partition = b / numBucketsPerPartition;
            uint32_t bucket = b % numBucketsPerPartition;

            uint32_t bucketNumberGlobal = smallSideIndexToPartition[partition + 1] * numActiveBuckets + bucket;

            if (f == 0 && false) {
                printf("f %d local bucket %d part %d buck %d nb %d nab %d global bucket %d cnt %d\n",
                    f, b, partition, bucket, numBucketsPerPartition, numActiveBuckets, bucketNumberGlobal,
                    wLocal[b].count);
            }

            incrementWOut(&wGlobal[bucketNumberGlobal], &wLocal[b]);
            //atomic_inc(&numGlobalUpdates);
        }
    }
}

// For each partition and each bucket, we up to now accumulated just the
// weight that needs to be transferred in the right hand side of the
// partition splits.  We need to fix this up by:
// 1.  Subtracting this weight from the left hand side
// 2.  Flipping the buckets where the big amount of weight belongs on the
//     right not on the left.
//
// This is a 2 dimensional kernel:
// Dimension 0 = partition number (from 0 to the old number of partitions)
// Dimension 1 = bucket number (from 0 to the number of active buckets); may be padded

UKL_FUNCTION(v1,void,
fixupBucketsImpl) (CONSTBUFFER(TreeTrainingInfo) treeTrainingInfo,
                   ROBUFFER(TreeDepthInfo) treeDepthInfo,
                   RWBUFFER(W) buckets,
                   RWBUFFER(W) wAll,
                   ROBUFFER(PartitionInfo) partitionInfo,
                   ROBUFFER(uint8_t) smallSideIndexes,
                   uint32_t bucket,
                   uint16_t partitionWorkerIdInGrid,
                   uint16_t partitionNumWorkersInGrid)
{
    // The device has stopped: the host continues from this level.
    if (treeDepthInfo->status != 0)
        return;

    uint32_t numActiveBuckets = treeTrainingInfo->numActiveBuckets;
    uint16_t numActivePartitions = treeDepthInfo->prevNumActivePartitions;
    uint32_t j = bucket;  // bucket number in partition
    if (j >= numActiveBuckets)
        return;

    for (uint32_t partition = partitionWorkerIdInGrid;  partition < numActivePartitions;  partition += partitionNumWorkersInGrid) {

        PartitionInfo info = partitionInfo[partition];

#if UKL_CPU_BACKEND
        using namespace std;
        if (j == 0 && false) {
            cerr << "partition " << partition << " info.left = " << info.left << " info.right = " << info.right << endl;
        }
#endif

        if (info.left == -1 || info.right == -1)
            continue;

        // The small side partition always gets subtracted from the large size partition
        int32_t to, from;
        if (smallSideIndexes[info.left]) {
            to = info.right;
            from = info.left;
        }
        else {
            to = info.left;
            from = info.right;
        }

#if UKL_CPU_BACKEND
        if (j == 0 && false) {
            using namespace std;
            cerr << "moving buckets from " << from << " to " << to << endl;
        }
#endif

        RWBUFFER(W) bucketsFrom = buckets + from * numActiveBuckets;
        RWBUFFER(W) bucketsTo = buckets + to * numActiveBuckets;

#if UKL_CPU_BACKEND
        if (bucketsFrom[j].count > 0 && false) {
            cerr << "  bucket " << j << " subtracting "
                 << printWYouFucker(&bucketsFrom[j]) << " from " << printWYouFucker(&bucketsTo[j]) << endl;
        }
#endif

        decrementWOutGlobal(&bucketsTo[j], &bucketsFrom[j]);
        if (j == 0) {
            decrementWOutGlobal(&wAll[to], &wAll[from]);
        }
    }
}


// Prediction (rerail's addition): a forest flattened into nodes, routing each row by its buckets as training did.

// One node of a flattened forest. A child is a node's index when it is at least 0, and leaf -(child + 1) when it is
// below 0.
typedef struct PredictNode {
    uint16_t feature;       // The feature's index (its bucket column)
    uint16_t bucketBits;    // The bits of each of the feature's buckets
    uint16_t splitBucket;   // The bucket the trainer split at; for a subset, its place in the subsets
    uint16_t ordinal;       // 1: true is the buckets below splitBucket; 0: true is splitBucket itself; 2: true is
                            // the buckets of subset splitBucket (CART's categorical split)
    int32_t childTrue;
    int32_t childFalse;
} PredictNode;

// [row group]: the forest's prediction for each row, the trees' leaf distributions (two labels) weighted. A row has
// lanesPerRow threads of the group (a power of 2, at most the group's threads): each lane walks the trees lane,
// lane + lanesPerRow, ..., and the row's first lane adds the lanes' sums in lane order, so a row's result depends on
// lanesPerRow only. Many lanes a row keep a small batch's walks short (a walk is a chain of dependent loads); one
// lane a row suits a batch that fills the device by itself. Each walk is at most maxDepth steps; a walk that has not
// reached a leaf by then gives NaN.
SYNC_FUNCTION(v1,void,
predictForestImpl) (uint32_t numRows,
                    uint32_t numTrees,
                    uint32_t maxDepth,
                    uint32_t lanesPerRow,
                    uint32_t columnStride,
                    ROBUFFER(uint32_t) bucketData,
                    ROBUFFER(PredictNode) nodes,
                    ROBUFFER(uint32_t) subsets,  // [numSubsets * CATEGORY_SET_WORDS]
                    ROBUFFER(int32_t) treeRoots,
                    ROBUFFER(float) treeWeights,
                    ROBUFFER(float) leafPreds,
                    RWBUFFER(float) predictionsOut,
                    RWLOCAL(float) partials,  // [2 * threadsPerGroup]
                    uint32_t groupId,
                    uint16_t threadId,
                    uint16_t threadsPerGroup)
{
    uint32_t rowsPerGroup = threadsPerGroup / lanesPerRow;
    uint32_t lane = threadId % lanesPerRow;
    uint32_t r = groupId * rowsPerGroup + threadId / lanesPerRow;

    float p0 = 0.0f, p1 = 0.0f;
    if (r < numRows) {
        for (uint32_t t = lane;  t < numTrees;  t += lanesPerRow) {
            int32_t at = treeRoots[t];
            // Each step: the node, then its feature's bucket for the row (column `feature` starts at
            // feature * columnStride; the bits are the node's).
            for (uint32_t d = 0;  d < maxDepth && at >= 0;  ++d) {
                PredictNode node = nodes[at];
                uint16_t bucket = extract16BitRange32(bucketData + (uint32_t)node.feature * columnStride,
                                                      node.bucketBits, r);
                bool isTrue = node.ordinal == 1 ? bucket < node.splitBucket
                    : node.ordinal == 0 ? bucket == node.splitBucket
                    : inCategorySet(subsets + (uint32_t)node.splitBucket * CATEGORY_SET_WORDS, bucket);
                at = isTrue ? node.childTrue : node.childFalse;
            }
            if (at >= 0) {
                p0 = p1 = NAN;
                break;
            }
            uint32_t leaf = (uint32_t)(-(at + 1));
            float w = treeWeights[t];
            p0 += w * leafPreds[2 * leaf];
            p1 += w * leafPreds[2 * leaf + 1];
        }
    }
    partials[2 * threadId] = p0;
    partials[2 * threadId + 1] = p1;

    ukl_threadgroup_barrier();

    if (r < numRows && lane == 0) {
        float s0 = 0.0f, s1 = 0.0f;
        for (uint32_t l = 0;  l < lanesPerRow;  ++l) {
            s0 += partials[2 * (threadId + l)];
            s1 += partials[2 * (threadId + l) + 1];
        }
        predictionsOut[2 * r] = s0;
        predictionsOut[2 * r + 1] = s1;
    }
}


// The feature space's columns built on the device (rerail's addition): each row's bucket, then the rows' buckets
// bit-packed as WritableBucketList writes them (least significant bits first, entryBits each).

// A row's key that stands for a null (no number orders below it).
#define NULL_ORDER_KEY ((int64_t)0x8000000000000000UL)

// [row]: a numeric column's bucket: 0 for a null; else the numeric buckets' offset plus the split points below the
// value (the keys order as the values do, so the comparison is exact). A bounded binary search.
UKL_FUNCTION(v1,void,
numericBucketsImpl) (uint32_t numRows,
                     ROBUFFER(int64_t) keys,
                     ROBUFFER(int64_t) splitKeys,
                     uint32_t numSplits,
                     uint32_t numericOffset,
                     RWBUFFER(uint32_t) bucketsOut,
                     uint32_t r)
{
    if (r >= numRows)
        return;
    int64_t k = keys[r];
    if (k == NULL_ORDER_KEY) {
        bucketsOut[r] = 0;
        return;
    }
    uint32_t lo = 0, hi = numSplits;
    for (uint32_t step = 0;  step < 32 && lo < hi;  ++step) {
        uint32_t mid = (lo + hi) / 2;
        if (splitKeys[mid] < k)
            lo = mid + 1;
        else
            hi = mid;
    }
    bucketsOut[r] = numericOffset + lo;
}

// [row]: marks the codes a categorical column holds (code numCodes for a null).
UKL_FUNCTION(v1,void,
markCodesImpl) (uint32_t numRows,
                uint32_t numCodes,
                ROBUFFER(uint32_t) codes,
                RWBUFFER(uint32_t) presentOut,
                uint32_t r)
{
    if (r >= numRows)
        return;
    uint32_t c = codes[r];
    presentOut[c < numCodes ? c : numCodes] = 1;
}

// [row]: a categorical column's bucket, its code's (code numCodes for a null).
UKL_FUNCTION(v1,void,
categoricalBucketsImpl) (uint32_t numRows,
                         uint32_t numCodes,
                         ROBUFFER(uint32_t) codes,
                         ROBUFFER(uint32_t) codeBuckets,
                         RWBUFFER(uint32_t) bucketsOut,
                         uint32_t r)
{
    if (r >= numRows)
        return;
    uint32_t c = codes[r];
    bucketsOut[r] = codeBuckets[c < numCodes ? c : numCodes];
}

// [word]: one 32-bit word of the packed buckets: the entries whose bits fall in it (at most 32 / entryBits + 2, a
// bounded loop), zero past the last entry.
UKL_FUNCTION(v1,void,
packBucketsImpl) (uint32_t numRows,
                  uint32_t entryBits,
                  uint32_t numWords,
                  ROBUFFER(uint32_t) buckets,
                  RWBUFFER(uint32_t) wordsOut,
                  uint32_t w)
{
    if (w >= numWords)
        return;
    uint32_t firstBit = 32 * w;
    uint32_t first = firstBit / entryBits;
    uint32_t word = 0;
    for (uint32_t i = 0;  i < 34;  ++i) {
        uint32_t e = first + i;
        if (e >= numRows || e * entryBits >= firstBit + 32)
            break;
        int32_t shift = (int32_t)(e * entryBits) - (int32_t)firstBit;
        uint32_t b = buckets[e];
        word |= shift >= 0 ? b << shift : b >> (-shift);
    }
    wordsOut[w] = word;
}
