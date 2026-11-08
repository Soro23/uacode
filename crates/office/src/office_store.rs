use crate::versioned_file::{StoredFile, VersionedDocument};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const OFFICE_VERSION: u32 = 1;

pub type OfficeFile = StoredFile<OfficeDocument>;
pub use crate::versioned_file::{load, save};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    Claude,
    Codex,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Claude, Provider::Codex];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    /// Prefix used for credential folders, e.g. `claude-default`.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
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
    /// Account id from accounts.json; `None` means the provider's default account.
    #[serde(default)]
    pub account: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instructions: String,
    #[serde(default)]
    pub archived: bool,
}

impl Post {
    pub fn set_provider(&mut self, provider: Provider) {
        if self.provider != provider {
            self.provider = provider;
            self.account = None;
        }
    }
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

impl VersionedDocument for OfficeDocument {
    const VERSION: u32 = OFFICE_VERSION;
    const FILE_NAME: &'static str = "office.json";

    fn version(&self) -> u32 {
        self.version
    }

    fn validate(&self) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use fs::Fs;
    use std::path::Path;

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
        let file: OfficeFile = load(fs, path).await?;
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
        let initial: OfficeFile = load(fs, path).await?;
        let saved = save(fs, path, &initial, sample()).await?;
        assert_eq!(load::<OfficeDocument>(fs, path).await?.document, sample());
        fs.atomic_write(path.to_path_buf(), "{broken".into())
            .await?;
        assert!(save(fs, path, &saved, sample()).await.is_err());
        assert_eq!(fs.load(path).await?, "{broken");
        let invalid: OfficeFile = load(fs, path).await?;
        assert!(save(fs, path, &invalid, sample()).await.is_err());
        let future_text = r#"{"version":999,"posts":[]}"#;
        fs.atomic_write(path.to_path_buf(), future_text.into())
            .await?;
        let future: OfficeFile = load(fs, path).await?;
        assert!(save(fs, path, &future, sample()).await.is_err());
        assert_eq!(fs.load(path).await?, future_text);
        Ok(())
    }
}
