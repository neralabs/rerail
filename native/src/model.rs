use model2vec_rs::model::StaticModel;
use napi::{Error, Result, Status};
use napi_derive::napi;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
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

/// What `verify_model` found wrong with a model directory: the problem and the file it is in.
#[derive(Debug, PartialEq, Eq)]
pub struct ModelFault {
    /// `missing`, `unreadable`, `invalid-manifest` or `checksum-mismatch`.
    pub problem: &'static str,
    pub file: &'static str,
    /// The whole account, as session attach reports it.
    pub message: String,
}

impl ModelFault {
    fn unread(path: &Path, file: &'static str, error: &std::io::Error) -> Self {
        Self {
            problem: if error.kind() == std::io::ErrorKind::NotFound {
                "missing"
            } else {
                "unreadable"
            },
            file,
            message: format!("failed to read {}: {error}", path.display()),
        }
    }
}

const MANIFEST: &str = "manifest.json";

fn read_manifest(root: &Path) -> std::result::Result<Manifest, ModelFault> {
    let path = root.join(MANIFEST);
    let bytes = fs::read(&path).map_err(|error| ModelFault::unread(&path, MANIFEST, &error))?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| ModelFault {
        problem: "invalid-manifest",
        file: MANIFEST,
        message: format!("failed to parse {}: {error}", path.display()),
    })?;
    if manifest.format_version != ENGINE_FORMAT_VERSION {
        return Err(ModelFault {
            problem: "invalid-manifest",
            file: MANIFEST,
            message: format!(
                "unsupported model format version {}; expected {ENGINE_FORMAT_VERSION}",
                manifest.format_version
            ),
        });
    }
    Ok(manifest)
}

fn verify_file(
    root: &Path,
    name: &'static str,
    expected: &str,
) -> std::result::Result<(), ModelFault> {
    let path = root.join(name);
    let bytes = fs::read(&path).map_err(|error| ModelFault::unread(&path, name, &error))?;
    let actual = crate::lower_hex(&Sha256::digest(bytes));
    if actual != expected {
        return Err(ModelFault {
            problem: "checksum-mismatch",
            file: name,
            message: format!(
                "model artifact checksum mismatch for {name}: expected {expected}, received {actual}"
            ),
        });
    }
    Ok(())
}

fn verify_files(root: &Path, manifest: &Manifest) -> std::result::Result<(), ModelFault> {
    verify_file(root, "model.safetensors", &manifest.files.model.sha256)?;
    verify_file(root, "tokenizer.json", &manifest.files.tokenizer.sha256)?;
    verify_file(root, "config.json", &manifest.files.config.sha256)
}

/// Checks the model directory `root` against its `manifest.json`, as loading it does, without
/// loading it: the manifest reads, and every file it lists has its checksum.
pub fn verify_model(root: &Path) -> std::result::Result<(), ModelFault> {
    verify_files(root, &read_manifest(root)?)
}

pub fn load_model(root: &Path) -> Result<LoadedModel> {
    let manifest = read_manifest(root).map_err(|fault| invalid(fault.message))?;
    verify_files(root, &manifest).map_err(|fault| invalid(fault.message))?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct ModelDir(PathBuf);

    impl ModelDir {
        /// A model directory whose manifest lists `files`' checksums, `format_version` 1.
        fn new(tag: &str, files: [(&str, &[u8]); 3]) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("rerail-model-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&root).unwrap();
            let mut listed = serde_json::Map::new();
            for (name, bytes) in files {
                fs::write(root.join(name), bytes).unwrap();
                let sha256 = crate::lower_hex(&Sha256::digest(bytes));
                listed.insert(name.to_owned(), serde_json::json!({ "sha256": sha256 }));
            }
            let manifest = serde_json::json!({
                "formatVersion": 1,
                "modelId": "test",
                "revision": "test",
                "dimensions": 1,
                "files": listed,
            });
            fs::write(root.join(MANIFEST), manifest.to_string()).unwrap();
            Self(root)
        }
    }

    impl Drop for ModelDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const FILES: [(&str, &[u8]); 3] = [
        ("model.safetensors", b"weights"),
        ("tokenizer.json", b"{}"),
        ("config.json", b"{}"),
    ];

    #[test]
    fn verification_names_the_file_that_breaks_the_manifest() {
        let directory = ModelDir::new("verify", FILES);
        assert_eq!(verify_model(&directory.0), Ok(()));

        fs::write(
            directory.0.join("model.safetensors"),
            "version https://git-lfs.github.com/spec/v1\noid sha256:a4264cfe\nsize 7559256\n",
        )
        .unwrap();
        let fault = verify_model(&directory.0).unwrap_err();
        assert_eq!(
            (fault.problem, fault.file),
            ("checksum-mismatch", "model.safetensors")
        );
        // Loading reports the same fault, as session attach always has.
        let error = load_model(&directory.0).err().expect("a mismatched model");
        assert!(
            error
                .reason
                .contains("checksum mismatch for model.safetensors")
        );

        fs::write(directory.0.join("model.safetensors"), b"weights").unwrap();
        fs::remove_file(directory.0.join("config.json")).unwrap();
        let fault = verify_model(&directory.0).unwrap_err();
        assert_eq!((fault.problem, fault.file), ("missing", "config.json"));

        fs::write(directory.0.join(MANIFEST), "not json").unwrap();
        let fault = verify_model(&directory.0).unwrap_err();
        assert_eq!((fault.problem, fault.file), ("invalid-manifest", MANIFEST));

        fs::remove_file(directory.0.join(MANIFEST)).unwrap();
        let fault = verify_model(&directory.0).unwrap_err();
        assert_eq!((fault.problem, fault.file), ("missing", MANIFEST));
    }
}
