use model2vec_rs::model::StaticModel;
use napi::{Error, Result, Status};
use napi_derive::napi;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tokenizers::Tokenizer;

pub const ENGINE_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format_version: u32,
    model_id: String,
    revision: String,
    dimensions: u32,
    files: ManifestFiles,
}

#[derive(Debug, Deserialize)]
struct ManifestFiles {
    #[serde(rename = "model.safetensors")]
    model: ManifestFile,
    #[serde(rename = "tokenizer.json")]
    tokenizer: ManifestFile,
    #[serde(rename = "config.json")]
    config: ManifestFile,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    sha256: String,
}

#[napi(object)]
#[derive(Clone)]
pub struct ModelFingerprint {
    pub model_id: String,
    pub revision: String,
    pub dimensions: u32,
    pub weights_sha256: String,
    pub tokenizer_sha256: String,
    #[napi(js_name = "kernelFormatVersion")]
    pub engine_format_version: u32,
}

pub struct LoadedModel {
    pub model: StaticModel,
    pub fingerprint: ModelFingerprint,
    pub tokenizer: Tokenizer,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

fn sha256(path: &Path) -> Result<String> {
    let bytes = fs::read(path)
        .map_err(|error| invalid(format!("failed to read {}: {error}", path.display())))?;
    Ok(crate::lower_hex(&Sha256::digest(bytes)))
}

fn verify_file(root: &Path, name: &str, expected: &str) -> Result<PathBuf> {
    let path = root.join(name);
    let actual = sha256(&path)?;
    if actual != expected {
        return Err(invalid(format!(
            "model artifact checksum mismatch for {name}: expected {expected}, received {actual}"
        )));
    }
    Ok(path)
}

pub fn load_model(root: &Path) -> Result<LoadedModel> {
    let manifest_path = root.join("manifest.json");
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        invalid(format!(
            "failed to read {}: {error}",
            manifest_path.display()
        ))
    })?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        invalid(format!(
            "failed to parse {}: {error}",
            manifest_path.display()
        ))
    })?;

    if manifest.format_version != ENGINE_FORMAT_VERSION {
        return Err(invalid(format!(
            "unsupported model format version {}; expected {ENGINE_FORMAT_VERSION}",
            manifest.format_version
        )));
    }

    verify_file(root, "model.safetensors", &manifest.files.model.sha256)?;
    verify_file(root, "tokenizer.json", &manifest.files.tokenizer.sha256)?;
    verify_file(root, "config.json", &manifest.files.config.sha256)?;

    let model = StaticModel::from_pretrained(root, None, None, None)
        .map_err(|error| invalid(format!("failed to load Model2Vec artifact: {error:#}")))?;
    let dimensions = model.encode_single("dimension probe").len() as u32;
    if dimensions != manifest.dimensions {
        return Err(invalid(format!(
            "model dimension mismatch: manifest declares {}, model produces {dimensions}",
            manifest.dimensions
        )));
    }
    let tokenizer = Tokenizer::from_file(root.join("tokenizer.json"))
        .map_err(|error| invalid(format!("failed to load tokenizer: {error}")))?;

    Ok(LoadedModel {
        model,
        tokenizer,
        fingerprint: ModelFingerprint {
            model_id: manifest.model_id,
            revision: manifest.revision,
            dimensions,
            weights_sha256: manifest.files.model.sha256,
            tokenizer_sha256: manifest.files.tokenizer.sha256,
            engine_format_version: ENGINE_FORMAT_VERSION,
        },
    })
}
