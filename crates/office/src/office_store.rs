use anyhow::{Context as _, Result, ensure};
use fs::Fs;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::Path};

pub const OFFICE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}

impl Provider {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Post {
    pub id: String,
    pub name: String,
    pub role: String,
    pub provider: Provider,
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfficeDocument {
    pub version: u32,
    #[serde(default = "default_concurrency")]
    pub max_working_agents: usize,
    pub posts: Vec<Post>,
}

fn default_concurrency() -> usize {
    3
}

impl Default for OfficeDocument {
    fn default() -> Self {
        Self {
            version: OFFICE_VERSION,
            max_working_agents: default_concurrency(),
            posts: Vec::new(),
        }
    }
}

impl OfficeDocument {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == OFFICE_VERSION,
            "Versión de Office no compatible"
        );
        ensure!(
            self.max_working_agents > 0,
            "El límite de concurrencia debe ser positivo"
        );
        let mut identifiers = HashSet::new();
        for post in &self.posts {
            ensure!(!post.id.trim().is_empty(), "Puesto sin identificador");
            ensure!(
                identifiers.insert(&post.id),
                "Identificador de puesto duplicado: {}",
                post.id
            );
            ensure!(
                !post.name.trim().is_empty(),
                "El nombre del puesto es obligatorio"
            );
            ensure!(
                !post.role.trim().is_empty(),
                "El rol del puesto es obligatorio"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct OfficeFile {
    pub document: OfficeDocument,
    pub warning: Option<String>,
    original: Option<String>,
}

impl OfficeFile {
    pub fn is_read_only(&self) -> bool {
        self.warning.is_some()
    }

    pub fn exists_on_disk(&self) -> bool {
        self.original.is_some()
    }

    fn decode(original: Option<String>) -> Self {
        let mut file = Self {
            document: OfficeDocument::default(),
            warning: None,
            original,
        };
        if let Some(text) = &file.original {
            let parsed = (|| -> Result<OfficeDocument> {
                let value: serde_json::Value = serde_json::from_str(text)?;
                let version = value.get("version").and_then(|version| version.as_u64());
                if let Some(version) = version {
                    ensure!(
                        version <= u64::from(OFFICE_VERSION),
                        "versión {version} creada por una app más nueva; esta admite hasta {OFFICE_VERSION}"
                    );
                }
                let document: OfficeDocument = serde_json::from_str(text)?;
                document.validate()?;
                Ok(document)
            })();
            match parsed {
                Ok(document) => file.document = document,
                Err(error) => {
                    file.warning = Some(format!("office.json en solo lectura: {error:#}"))
                }
            }
        }
        file
    }
}

pub async fn load(fs: &dyn Fs, path: &Path) -> Result<OfficeFile> {
    let original = if fs.metadata(path).await?.is_some() {
        Some(fs.load(path).await?)
    } else {
        None
    };
    Ok(OfficeFile::decode(original))
}

pub async fn save(
    fs: &dyn Fs,
    path: &Path,
    previous: &OfficeFile,
    document: OfficeDocument,
) -> Result<OfficeFile> {
    ensure!(
        !previous.is_read_only(),
        "Office está en solo lectura; corrige el archivo y pulsa Recargar"
    );
    document.validate()?;
    let current = load(fs, path).await?;
    ensure!(
        current.original == previous.original,
        "office.json cambió en disco; pulsa Recargar antes de guardar"
    );
    let text = serde_json::to_string_pretty(&document)? + "\n";
    let parent = path
        .parent()
        .context("office.json no tiene carpeta padre")?;
    fs.create_dir(parent).await?;
    fs.atomic_write(path.to_path_buf(), text.clone()).await?;
    Ok(OfficeFile {
        document,
        warning: None,
        original: Some(text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> OfficeDocument {
        OfficeDocument {
            posts: vec![Post {
                id: "stable-id".into(),
                name: "Backend".into(),
                role: "backend".into(),
                provider: Provider::Codex,
                account: None,
                model: "example-model".into(),
                instructions: "Primera línea\nSegunda línea".into(),
                archived: true,
            }],
            ..OfficeDocument::default()
        }
    }

    #[test]
    fn serialization_round_trip() -> Result<()> {
        let expected = sample();
        let encoded = serde_json::to_string(&expected)?;
        let actual: OfficeDocument = serde_json::from_str(&encoded)?;
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn invalid_json_reports_line_and_preserves_source() {
        let text = "{\n  \"version\": 1,\n  \"posts\": [}\n";
        let file = OfficeFile::decode(Some(text.into()));
        assert!(
            file.warning
                .as_deref()
                .is_some_and(|warning| warning.contains("line 3"))
        );
        assert_eq!(file.original.as_deref(), Some(text));
    }

    #[test]
    fn future_version_is_read_only_even_with_unknown_schema() {
        let file = OfficeFile::decode(Some(r#"{"version": 999, "posts": "new schema"}"#.into()));
        assert!(file.is_read_only());
        assert!(
            file.warning
                .as_deref()
                .is_some_and(|warning| warning.contains("999"))
        );
    }

    #[test]
    fn duplicate_identifiers_are_rejected() -> Result<()> {
        let mut document = sample();
        document.posts.extend(document.posts.clone());
        assert!(document.validate().is_err());
        Ok(())
    }

    #[test]
    fn unknown_fields_are_rejected_to_avoid_losing_them() {
        let text = r#"{"version":1,"posts":[{"id":"a","name":"A","role":"qa","provider":"claude","color":"red"}]}"#;
        let file = OfficeFile::decode(Some(text.into()));
        assert!(file.is_read_only());
        assert!(
            file.warning
                .as_deref()
                .is_some_and(|warning| warning.contains("color"))
        );
    }

    // `gpui::test` discards the returned value, so failures must surface through `expect`.
    #[gpui::test]
    async fn missing_file_does_not_create_directories(cx: &mut gpui::TestAppContext) {
        let fs = fs::FakeFs::new(cx.background_executor.clone());
        check_missing_file(fs.as_ref())
            .await
            .expect("missing office.json should load without side effects");
    }

    async fn check_missing_file(fs: &dyn Fs) -> Result<()> {
        let path = Path::new("/project/.uacode/office.json");
        let file = load(fs, path).await?;
        assert_eq!(file.document, OfficeDocument::default());
        assert!(!file.is_read_only());
        assert!(fs.metadata(Path::new("/project/.uacode")).await?.is_none());
        Ok(())
    }

    #[gpui::test]
    async fn save_reload_and_external_edit_protection(cx: &mut gpui::TestAppContext) {
        let fs = fs::FakeFs::new(cx.background_executor.clone());
        check_save_and_protection(fs.as_ref())
            .await
            .expect("save, reload and protection checks should succeed");
    }

    async fn check_save_and_protection(fs: &dyn Fs) -> Result<()> {
        let path = Path::new("/project/.uacode/office.json");
        let initial = load(fs, path).await?;
        let saved = save(fs, path, &initial, sample()).await?;
        assert_eq!(load(fs, path).await?.document, sample());
        fs.atomic_write(path.to_path_buf(), "{broken".into())
            .await?;
        assert!(save(fs, path, &saved, sample()).await.is_err());
        assert_eq!(fs.load(path).await?, "{broken");
        let invalid = load(fs, path).await?;
        assert!(save(fs, path, &invalid, sample()).await.is_err());
        let future_text = r#"{"version":999,"posts":[]}"#;
        fs.atomic_write(path.to_path_buf(), future_text.into())
            .await?;
        let future = load(fs, path).await?;
        assert!(save(fs, path, &future, sample()).await.is_err());
        assert_eq!(fs.load(path).await?, future_text);
        Ok(())
    }
}
