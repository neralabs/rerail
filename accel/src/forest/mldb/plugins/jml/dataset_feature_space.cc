/** dataset_feature_space.cc
    This file is part of MLDB. Copyright 2015 mldb.ai inc. All rights reserved.
*/

#include "dataset_feature_space.h"
#include "mldb/core/dataset.h"
#include "mldb/plugins/jml/jml/registry.h"
#include "mldb/base/parallel.h"
#include "mldb/plugins/jml/jml/training_index_entry.h"
#include "mldb/types/value_description.h"
#include "mldb/types/vector_description.h"
#include "mldb/types/hash_wrapper_description.h"
#include "mldb/types/annotated_exception.h"
#include "mldb/utils/log.h"

using namespace std;



namespace MLDB {


const ML::Feature labelFeature(0, 0, 0), weightFeature(0, 1, 0);


/*****************************************************************************/
/* DATASET FEATURE SPACE                                                     */
/*****************************************************************************/

DatasetFeatureSpace::
DatasetFeatureSpace() 
    : logger(MLDB::getMldbLog<DatasetFeatureSpace>())
{
}

// DatasetFeatureSpace's construction from a dataset and getColumnInfo (bucketized): in dataset_feature_space.rs
// (DatasetFeatureSpace::from_columns, ColumnInfo::from_values; the unbucketized column info is not brought over,
// as the trainer needs buckets).

float
DatasetFeatureSpace::
encodeFeatureValue(ColumnHash column, const CellValue & value) const
{
    if (value.empty())
        return std::numeric_limits<float>::quiet_NaN();
        
    auto it = columnInfo.find(column);
    if (it == columnInfo.end()) {
        throw MLDB::Exception("Encoding unknown column");
    }

    return encodeValue(value, it->second.columnName, it->second.info);
}

void
DatasetFeatureSpace::
encodeFeature(ColumnHash column, const CellValue & value,
              std::vector<std::pair<ML::Feature, float> > & fset) const
{
    if (value.empty())
        return;
        
    auto it = columnInfo.find(column);
    if (it == columnInfo.end()) {
        throw MLDB::Exception("Encoding unknown column");
    }

    fset.emplace_back(getFeature(column),
                      encodeValue(value, it->second.columnName,
                                  it->second.info));
}

std::pair<int, int>
DatasetFeatureSpace::
getFeatureBucket(ColumnHash column, const CellValue & value) const
{
    if (value.empty())
        throw MLDB::Exception("Encoding empty value");
        
    auto it = columnInfo.find(column);
    if (it == columnInfo.end()) {
        throw MLDB::Exception("Encoding unknown column");
    }

    if (it->second.info.type() == ML::CATEGORICAL
        || it->second.info.type() == ML::STRING) {
        std::string key;
        if (value.isUtf8String())
            key = value.toUtf8String().rawString();
        else 
            key  = value.toString();

        int val = it->second.info.categorical()->lookup(key);
        return { it->second.index, val };
    }

    return { it->second.index, it->second.bucketDescriptions.getBucket(value) };
}

ML::Label
DatasetFeatureSpace::
encodeLabel(const CellValue & value, bool isRegression) const
{
    static ColumnPath LABEL("<<LABEL>>");
    float label = encodeValue(value.isString() ? jsonEncodeStr(value) : value,
                           LABEL, labelInfo);

    if (isRegression) {
        return ML::Label(label);
    }
    else {
        return ML::Label((int)label);
    }
}

float
DatasetFeatureSpace::
encodeValue(const CellValue & value,
            const ColumnPath & columnName,
            const ML::Feature_Info & info) const
{
    if (value.empty() || info.type() == ML::INUTILE)
        return std::numeric_limits<float>::quiet_NaN();

    if (info.type() == ML::CATEGORICAL
        || info.type() == ML::STRING) {
        // Look up the value in the categorical info

        std::string key;
        if (value.isUtf8String())
            key = value.toUtf8String().rawString();
        else 
            key  = value.toString();

        int val = info.categorical()->lookup(key);
        return val;
    }
    if (!value.isNumeric()) {
        throw MLDB::Exception("Value for column '"
                            + columnName.toUtf8String().rawString() + 
                            "' was numeric in training, but is now " +
                            jsonEncodeStr(value));

    }
    return value.toDouble();

}

ML::Feature_Info
DatasetFeatureSpace::
info(const ML::Feature & feature) const
{
    if (feature == labelFeature)
        return labelInfo;
    else if (feature == weightFeature)
        return ML::Feature_Info(ML::REAL, false, true, false);
    else if (feature == ML::MISSING_FEATURE)
        return ML::MISSING_FEATURE_INFO;

    // Look up the feature info we just extracted
    auto it = columnInfo.find(getHash(feature));
    if (it == columnInfo.end())
        throw MLDB::Exception("feature " + feature.print() + " not found");
    return it->second.info;
}

ColumnHash
DatasetFeatureSpace::
getHashRaw(ML::Feature feature)
{
    if (feature == ML::MISSING_FEATURE)
        return ColumnHash();
    ExcAssertEqual(feature.type(), 1);

    uint64_t high = (uint32_t)feature.arg1();
    uint64_t low  = (uint32_t)feature.arg2();

    ColumnHash result(high << 32 | low);
    return result;
}

ColumnHash
DatasetFeatureSpace::
getHash(ML::Feature feature) const
{
    if (feature == ML::MISSING_FEATURE)
        return ColumnHash();
    ExcAssertEqual(feature.type(), 1);

    if (versionTwoMapping.empty())
        return getHashRaw(feature);
    else {
        auto it = versionTwoMapping.find(feature);
        if (it == versionTwoMapping.end()) {
            throw AnnotatedException(400, "feature was not found");
        }
        return it->second;
    }
}

ML::Feature
DatasetFeatureSpace::
getFeatureRaw(ColumnHash hash)
{
    uint32_t high = hash >> 32;
    uint32_t low  = hash;

    ML::Feature result(1, high, low);
    ExcAssertEqual(getHashRaw(result), hash);

    return result;
}

ML::Feature
DatasetFeatureSpace::
getFeature(ColumnHash hash) const
{
    if (versionTwoReverseMapping.empty()) {
        return getFeatureRaw(hash);
    }

    auto it = versionTwoReverseMapping.find(hash);
    if (it == versionTwoReverseMapping.end()) {
        throw AnnotatedException(400, "hash was not found");
    }
    return it->second;
}

static CellValue getValueFromInfo(const ML::Feature_Info & info,
                                  float value)
{
    if (std::isnan(value))
        return CellValue();
    
    if (info.type() == ML::CATEGORICAL) {
        return Utf8String(info.categorical()->print(value));
    }
    
    // Non-categorical just get their face values
    return value;
}

CellValue
DatasetFeatureSpace::
getValue(const ML::Feature & feature, float value) const
{
    if (feature == labelFeature) {
        return getValueFromInfo(labelInfo, value);
    }

    // First, get the column
    ColumnHash column = getHash(feature);

    auto it = columnInfo.find(column);

    if (it == columnInfo.end())
        throw MLDB::Exception("Couldn't find column");

    return getValueFromInfo(it->second.info, value);
}

std::string
DatasetFeatureSpace::
print(const ML::Feature_Set & fs) const
{
    std::string result;

    bool first = true;
    for (auto f: fs) {
        if (!first)
            result += " ";
        first = false;
        result += print(f.first) + "=" + print(f.first, f.second);
    }

    return result;
}

std::string
DatasetFeatureSpace::
print(const ML::Feature & feature) const
{
    if (feature == labelFeature)
        return "LABEL";
    else if (feature == weightFeature)
        return "WEIGHT";
    else if (feature == ML::MISSING_FEATURE)
        return "<<<MISSING>>>";

        
    auto it = columnInfo.find(getHash(feature));
    if (it == columnInfo.end()) {
        throw MLDB::Exception("Couldn't find feature in dataset");
    }
    return it->second.columnName.toUtf8String().rawString();
}

std::string
DatasetFeatureSpace::
print(const ML::Feature & feature, float value) const
{
    if (feature == weightFeature)
        return to_string(value);
    if (feature == ML::MISSING_FEATURE) {
        return "";
    }

    auto val = getValue(feature, value);

    if (val.isAsciiString())
        return val.toString();
    else if (val.isUtf8String())
        return val.toUtf8String().rawString();
    else
        return jsonEncodeStr(getValue(feature, value));
}

void
DatasetFeatureSpace::
serialize(MLDB::DB::Store_Writer & store, const ML::Feature & feature) const
{
    return ML::Feature_Space::serialize(store, feature);
}

void
DatasetFeatureSpace::
reconstitute(MLDB::DB::Store_Reader & store, ML::Feature & feature) const
{
    return ML::Feature_Space::reconstitute(store, feature);
}

void
DatasetFeatureSpace::
serialize(MLDB::DB::Store_Writer & store, const ML::Feature & feature,
          float value) const
{
    auto it = columnInfo.find(getHash(feature));
    if (it == columnInfo.end())
        throw MLDB::Exception("Couldn't find feature in dataset");

    if (it->second.info.categorical()) {
        store << it->second.info.categorical()->print(value);
    }
    else store << value;
}

void
DatasetFeatureSpace::
reconstitute(MLDB::DB::Store_Reader & store,
             const ML::Feature & feature,
             float & value) const
{
    auto it = columnInfo.find(getHash(feature));
    if (it == columnInfo.end())
        throw MLDB::Exception("Couldn't find feature in dataset");

    if (it->second.info.categorical()) {
        string val;
        store >> val;
        value = it->second.info.categorical()->parse(val);
    }
    else store >> value;
}

std::string
DatasetFeatureSpace::
class_id() const
{
    return "MLDB::DatasetFeatureSpace";
}

ML::Feature_Space_Type
DatasetFeatureSpace::
type() const
{
    return ML::SPARSE;
}

ML::Feature_Space *
DatasetFeatureSpace::
make_copy() const
{
    return new DatasetFeatureSpace(*this);
}

void
DatasetFeatureSpace::
reconstitute(MLDB::DB::Store_Reader & store)
{
    char version;
    store >> version;
    if (version > 3)
        throw MLDB::Exception("unexpected version of DatasetFeatureSpace");
    MLDB::DB::compact_size_t numFeatures(store);

    if (version > 1)
        store >> labelInfo;
    else labelInfo = ML::BOOLEAN;
    
    decltype(columnInfo) newColumnInfo;
    decltype(versionTwoMapping) newVersionTwoMapping;
    decltype(versionTwoReverseMapping) newVersionTwoReverseMapping;

    for (unsigned i = 0;  i < numFeatures;  ++i) {
        string s, s2;
        store >> s >> s2;
        ColumnInfo info;
        ColumnHash featureHash = jsonDecodeStr<ColumnHash>(s);
        info.columnName = ColumnPath::parse(s2);
        ColumnHash hash(info.columnName);
        info.index = i;
        store >> info.info;

        if (version < 3) {
            // Version 2 used a different hash, so our feature names
            // operate on that
            ML::Feature feature = getFeatureRaw(featureHash);
            newVersionTwoMapping[feature] = hash;
            newVersionTwoReverseMapping[hash] = feature;

            TRACE_MSG(logger) << "v2 feature " << s2 << " hash " << hash << " featureHash " << featureHash
                              << " feature " << feature;
        }
        else {
            if (hash != featureHash) {
                  TRACE_MSG(logger) << "feature " << info.columnName;
            }
            ExcAssertEqual(hash, featureHash);
        }
        
        newColumnInfo[hash] = std::move(info);
    }

    columnInfo.swap(newColumnInfo);
    versionTwoMapping.swap(newVersionTwoMapping);
    versionTwoReverseMapping.swap(newVersionTwoReverseMapping);
}

void
DatasetFeatureSpace::
serialize(MLDB::DB::Store_Writer & store) const
{
    store << (char)3 // version
          << MLDB::DB::compact_size_t(columnInfo.size());

    store << labelInfo;

    DEBUG_MSG(logger) << "serializing " << columnInfo.size() << " features";
    for (auto & i: columnInfo) {
        store << jsonEncodeStr(i.first);
        const ColumnInfo& columnInfo = i.second;
        store << columnInfo.columnName.toUtf8String().rawString();
        columnInfo.info.serialize(store);
    }

    DEBUG_MSG(logger) << "done serializing feature space";
}

std::ostream & operator << (std::ostream & stream,
                            const DatasetFeatureSpace::ColumnInfo & columnInfo)
{
    stream << columnInfo.print();
    return stream;
}


static ML::Register_Factory<ML::Feature_Space, DatasetFeatureSpace>
DFS_REG("MLDB::DatasetFeatureSpace");


} // namespace MLDB


