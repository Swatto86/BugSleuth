//! OpenCode uses the same verbose model record format as Kilo, but its route
//! names have no Kilo billing meaning. Keep every configured route, including
//! custom local provider IDs, and forward IDs and variants unchanged.
use super::{ModelGroup, VendorCatalogue, verbose_catalogue};
use crate::{ProviderError, opencode, process};
use std::collections::BTreeMap;
use std::time::Duration;

pub(super) async fn available() -> Result<VendorCatalogue, ProviderError> {
    let binary = opencode::binary_path().ok_or_else(|| super::not_installed("opencode"))?;
    let output = process::run(process::Invocation {
        binary: &binary.to_string_lossy(),
        args: &["models".into(), "--pure".into(), "--verbose".into()],
        cwd: &std::env::temp_dir(),
        stdin: None,
        env: &[("OPENCODE_DISABLE_PROJECT_CONFIG".into(), "true".into())],
        timeout: Duration::from_secs(60),
        what: "opencode models",
    })
    .await?;
    from_output(output)
}

fn from_output(output: process::CliOutput) -> Result<VendorCatalogue, ProviderError> {
    if !output.succeeded() {
        return Err(ProviderError::Failed {
            vendor: "opencode",
            code: output.code.unwrap_or(-1),
            message: process::redact_secrets(&process::preview(output.stderr.trim(), 2000)),
        });
    }
    let entries = verbose_catalogue::parse(&output.stdout);
    if entries.is_empty() {
        return Err(ProviderError::Empty("opencode model catalogue"));
    }
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut efforts_by_model = BTreeMap::new();
    for entry in entries {
        let route = entry.id.split('/').next().unwrap_or("OpenCode").to_string();
        let models = groups.entry(route).or_default();
        if !models.contains(&entry.id) {
            models.push(entry.id.clone());
        }
        efforts_by_model.insert(entry.id, entry.efforts);
    }
    Ok(VendorCatalogue {
        groups: groups
            .into_iter()
            .map(|(label, models)| ModelGroup { label, models })
            .collect(),
        efforts_by_model,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalogue_keeps_local_tags_custom_routes_and_variants() {
        let output = process::CliOutput { code: Some(0), stderr: String::new(), stdout:
            "ollama/qwen3:8b\n{\"variants\":{\"thinking\":{}}}\nmy-lmstudio/org/model\n{}\nopenai/new-model\n{}".into() };
        let catalogue = from_output(output).unwrap();
        assert_eq!(catalogue.groups.len(), 3);
        assert!(
            catalogue
                .groups
                .iter()
                .any(|g| g.models == ["my-lmstudio/org/model"])
        );
        assert_eq!(catalogue.efforts_by_model["ollama/qwen3:8b"], ["thinking"]);
        assert!(catalogue.efforts_by_model.contains_key("openai/new-model"));
    }
    #[test]
    fn failed_or_empty_catalogues_are_not_reported_as_success() {
        for (code, stdout) in [(1, "ollama/model\n{}"), (0, "loading...")] {
            assert!(
                from_output(process::CliOutput {
                    code: Some(code),
                    stdout: stdout.into(),
                    stderr: "failed".into()
                })
                .is_err()
            );
        }
    }
}
